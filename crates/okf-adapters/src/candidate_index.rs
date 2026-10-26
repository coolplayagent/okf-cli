use super::*;
use okf_application::{
    candidate_data, trigrams, CandidateRequest, CandidateSelection, MAX_DOCUMENT_GRAMS,
    MAX_POSTINGS,
};
use okf_core::Document;
use std::collections::BTreeSet;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS candidate_snapshots(bundle TEXT PRIMARY KEY,generation TEXT NOT NULL,version INTEGER NOT NULL,documents INTEGER NOT NULL,postings INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS candidate_documents(bundle TEXT NOT NULL,path TEXT NOT NULL,hash TEXT NOT NULL,metadata TEXT NOT NULL,metadata_hash TEXT NOT NULL,fallback INTEGER NOT NULL,PRIMARY KEY(bundle,path));
CREATE INDEX IF NOT EXISTS candidate_fallback ON candidate_documents(bundle,fallback,path);
CREATE TABLE IF NOT EXISTS candidate_postings(bundle TEXT NOT NULL,gram TEXT NOT NULL,path TEXT NOT NULL,PRIMARY KEY(bundle,gram,path)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS candidate_paths ON candidate_postings(bundle,path,gram);
CREATE TRIGGER IF NOT EXISTS candidate_invalidate_delete AFTER DELETE ON documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_invalidate_update AFTER UPDATE ON documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_invalidate_insert AFTER INSERT ON documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=new.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_metadata_delete AFTER DELETE ON candidate_documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_metadata_update AFTER UPDATE ON candidate_documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_metadata_insert AFTER INSERT ON candidate_documents BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=new.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_postings_delete AFTER DELETE ON candidate_postings BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_postings_update AFTER UPDATE ON candidate_postings BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=old.bundle; END;
CREATE TRIGGER IF NOT EXISTS candidate_postings_insert AFTER INSERT ON candidate_postings BEGIN UPDATE candidate_snapshots SET version=0 WHERE bundle=new.bundle; END;";

pub(super) fn clear(tx: &Transaction<'_>, key: &str) -> Result<()> {
    for table in [
        "candidate_snapshots",
        "candidate_documents",
        "candidate_postings",
    ] {
        tx.execute(&format!("DELETE FROM {table} WHERE bundle=?1"), [key])
            .map_err(Error::io)?;
    }
    Ok(())
}

pub(super) fn publish(
    tx: &Transaction<'_>,
    key: &str,
    snapshot: &Snapshot,
    control: &RequestControl,
) -> Result<()> {
    clear(tx, key)?;
    let mut documents = 0;
    let mut postings = 0;
    let mut insert = tx
        .prepare("INSERT INTO candidate_postings VALUES(?1,?2,?3)")
        .map_err(Error::io)?;
    for doc in &snapshot.documents {
        control.check()?;
        if okf_core::reserved(&doc.path) {
            continue;
        }
        let (metadata, grams) = candidate_data(&doc.path, &doc.raw)?;
        let grams = grams.filter(|g| g.len() <= MAX_POSTINGS - postings);
        tx.execute(
            "INSERT INTO candidate_documents VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                key,
                doc.path,
                doc.hash,
                metadata,
                okf_core::sha256(metadata.as_bytes()),
                grams.is_none()
            ],
        )
        .map_err(Error::io)?;
        if let Some(grams) = grams {
            for gram in grams {
                if postings % 256 == 0 {
                    control.check()?;
                }
                insert
                    .execute(params![key, gram, doc.path])
                    .map_err(Error::io)?;
                postings += 1;
            }
        }
        documents += 1;
    }
    tx.execute(
        "INSERT INTO candidate_snapshots VALUES(?1,?2,1,?3,?4)",
        params![key, snapshot.generation, documents, postings],
    )
    .map_err(Error::io)?;
    Ok(())
}

fn corrupt() -> Error {
    Error::new(
        "INDEX_CANDIDATES_CORRUPT",
        "candidate index is inconsistent; run sync to rebuild",
        5,
    )
}

pub(super) fn header_valid(tx: &Transaction<'_>, key: &str, generation: &str) -> Result<bool> {
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='candidate_snapshots' AND type='table')", [], |r| r.get(0)).map_err(Error::io)?;
    if !exists {
        return Ok(false);
    }
    let header: Option<(String, usize, usize, usize)> = tx
        .query_row(
            "SELECT generation,version,documents,postings FROM candidate_snapshots WHERE bundle=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(Error::io)?;
    let Some((stored, version, documents, postings)) = header else {
        return Ok(false);
    };
    if version != 1 || stored != generation {
        return Err(corrupt());
    }
    let (actual_docs, actual_postings, concepts): (usize,usize,usize) = tx.query_row(
        &format!("SELECT (SELECT COUNT(*) FROM candidate_documents WHERE bundle=?1),(SELECT COUNT(*) FROM candidate_postings WHERE bundle=?1),(SELECT COUNT(*) FROM documents WHERE bundle=?1 AND ({CONCEPT}))"), [key],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(Error::io)?;
    if documents != actual_docs
        || documents > MAX_INDEX_DOCUMENTS
        || documents != concepts
        || postings != actual_postings
        || postings > MAX_POSTINGS
    {
        return Err(corrupt());
    }
    Ok(true)
}

pub(super) fn verify_document(
    tx: &Transaction<'_>,
    key: &str,
    path: &str,
    raw: &str,
    hash: &str,
) -> Result<()> {
    if okf_core::reserved(path) {
        return Ok(());
    }
    let (expected_metadata, expected_grams) = candidate_data(path, raw)?;
    let row: Option<(String,String,String,bool)> = tx.query_row("SELECT hash,metadata,metadata_hash,fallback FROM candidate_documents WHERE bundle=?1 AND path=?2",params![key,path],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(Error::io)?;
    let Some((stored_hash, metadata, metadata_hash, fallback)) = row else {
        return Err(corrupt());
    };
    if stored_hash != hash
        || metadata != expected_metadata
        || metadata_hash != okf_core::sha256(metadata.as_bytes())
    {
        return Err(corrupt());
    }
    let mut statement = tx.prepare("SELECT gram FROM candidate_postings WHERE bundle=?1 AND path=?2 ORDER BY gram LIMIT ?3").map_err(Error::io)?;
    let grams = statement
        .query_map(params![key, path, (MAX_DOCUMENT_GRAMS + 1) as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(Error::io)?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(Error::io)?;
    if (fallback && !grams.is_empty()) || (!fallback && expected_grams.as_ref() != Some(&grams)) {
        return Err(corrupt());
    }
    Ok(())
}

pub(super) fn select(
    index: &SqliteIndex,
    key: &str,
    request: &CandidateRequest<'_>,
) -> Result<Option<CandidateSelection>> {
    request.control.check()?;
    if request.row_limit > MAX_POSTINGS || request.body_limit > MAX_INDEX_DOCUMENTS {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "candidate request exceeds index limits",
            2,
        ));
    }
    let tx = index.0.unchecked_transaction().map_err(Error::io)?;
    let Some(header) = stats(&tx, key, request.filter.directory.as_deref())? else {
        return Ok(None);
    };
    if header.generation != request.generation {
        return Err(Error::new(
            "INDEX_CHANGED",
            "index refreshed during query; repeat search",
            6,
        ));
    }
    if !header_valid(&tx, key, request.generation)? {
        return Err(Error::new(
            "INDEX_CANDIDATES_MISSING",
            "run sync to build candidate data",
            3,
        ));
    }
    let (lower, upper) = directory_bounds(request.filter.directory.as_deref())?;
    let mut rows_read = 0;
    let mut complete = true;
    let tokens: BTreeSet<_> = request.terms.iter().flat_map(|t| trigrams(t)).collect();
    let mut paths: Option<BTreeSet<String>> = None;
    for token in &tokens {
        request.control.check()?;
        let remaining = request.row_limit.saturating_sub(rows_read);
        if remaining == 0 {
            complete = false;
            break;
        }
        let mut statement = tx.prepare("SELECT path FROM candidate_postings WHERE bundle=?1 AND gram=?2 AND path>=?3 AND (?4 IS NULL OR path<?4) ORDER BY path LIMIT ?5").map_err(Error::io)?;
        let selected = statement
            .query_map(params![key, token, lower, upper, remaining as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(Error::io)?
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .map_err(Error::io)?;
        rows_read += selected.len();
        if selected.len() == remaining {
            complete = false;
        }
        paths = Some(match paths {
            Some(paths) => paths.intersection(&selected).cloned().collect(),
            None => selected,
        });
        if !complete || paths.as_ref().is_some_and(BTreeSet::is_empty) {
            break;
        }
    }
    let mut paths = paths.unwrap_or_default();
    // Overflow documents are always included; short queries scan candidate metadata.
    let remaining = request.row_limit.saturating_sub(rows_read);
    if remaining == 0 {
        complete = false;
    } else {
        let mut statement = tx.prepare("SELECT path FROM candidate_documents WHERE bundle=?1 AND (?2 OR fallback=1) AND path>=?3 AND (?4 IS NULL OR path<?4) ORDER BY path LIMIT ?5").map_err(Error::io)?;
        let extra = statement
            .query_map(
                params![key, tokens.is_empty(), lower, upper, remaining as i64],
                |r| r.get::<_, String>(0),
            )
            .map_err(Error::io)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::io)?;
        rows_read += extra.len();
        if extra.len() == remaining {
            complete = false;
        }
        paths.extend(extra);
    }
    let mut documents = vec![];
    for path in paths {
        request.control.check()?;
        if rows_read >= request.row_limit {
            complete = false;
            break;
        }
        let row: Option<(String,String,String)> = tx.query_row("SELECT hash,metadata,metadata_hash FROM candidate_documents WHERE bundle=?1 AND path=?2", params![key,path],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(Error::io)?;
        rows_read += 1;
        let Some((hash, metadata, digest)) = row else {
            return Err(corrupt());
        };
        if okf_core::sha256(metadata.as_bytes()) != digest {
            return Err(corrupt());
        }
        let doc = Document {
            frontmatter: serde_json::from_str(&metadata).map_err(Error::io)?,
            body_start: 0,
        };
        let card = doc.card(
            path.strip_suffix(".md").unwrap_or(&path),
            request.evaluated_at,
        );
        if !request
            .filter
            .matches(&serde_json::to_value(card).map_err(Error::io)?)
        {
            continue;
        }
        if documents.len() >= request.body_limit {
            complete = false;
            break;
        }
        let (raw, source_hash): (String, String) = tx
            .query_row(
                "SELECT raw,hash FROM documents WHERE bundle=?1 AND path=?2",
                params![key, path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(Error::io)?;
        if source_hash != hash || okf_core::sha256(raw.as_bytes()) != hash {
            return Err(corrupt());
        }
        okf_core::logical_path(&path)?;
        documents.push(okf_application::IndexedDocument { path, hash, raw });
    }
    request.control.check()?;
    Ok(Some(CandidateSelection {
        snapshot: Snapshot {
            generation: header.generation,
            upstream_revision: header.upstream_revision,
            checked_at: header.checked_at,
            documents,
        },
        complete,
        rows_read,
    }))
}
