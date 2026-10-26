use okf_application::{
    Index, IndexStats, IndexedDocument, Snapshot, MAX_INDEX_BYTES, MAX_INDEX_DOCUMENTS,
};
use okf_core::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};

pub struct SqliteIndex(Connection);
impl SqliteIndex {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::io)?;
        }
        let connection = Connection::open(path).map_err(Error::io)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(Error::io)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(Error::io)?;
        if version != 0 && version != 1 {
            return Err(Error::new(
                "INDEX_VERSION",
                "unsupported content index schema",
                2,
            ));
        }
        connection.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS snapshots (bundle TEXT PRIMARY KEY, generation TEXT NOT NULL, revision TEXT, checked_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS documents (bundle TEXT NOT NULL, path TEXT NOT NULL, hash TEXT NOT NULL, raw TEXT NOT NULL, generation TEXT NOT NULL, PRIMARY KEY(bundle,path));
            PRAGMA user_version=1;").map_err(Error::io)?;
        Ok(Self(connection))
    }
}
// Exact, case-sensitive reserved filenames, matching okf_core::reserved.
const CONCEPT: &str = "path NOT IN ('index.md','log.md') AND substr(path,-9)<>'/index.md' AND substr(path,-7)<>'/log.md'";

fn header(tx: &Transaction<'_>, key: &str) -> Result<Option<IndexStats>> {
    let header: Option<(String, Option<String>, String)> = tx
        .query_row(
            "SELECT generation,revision,checked_at FROM snapshots WHERE bundle=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(Error::io)?;
    let Some((generation, upstream_revision, checked_at)) = header else {
        return Ok(None);
    };
    Ok(Some(IndexStats {
        generation,
        upstream_revision,
        checked_at,
        documents: 0,
        concepts: 0,
        bytes: 0,
    }))
}

fn stats(tx: &Transaction<'_>, key: &str) -> Result<Option<IndexStats>> {
    let Some(header) = header(tx, key)? else {
        return Ok(None);
    };
    let (bytes, documents, concepts): (usize, usize, usize) = tx.query_row(
        &format!("SELECT COALESCE(SUM(length(CAST(raw AS BLOB))),0),COUNT(*),COALESCE(SUM(CASE WHEN {CONCEPT} THEN 1 ELSE 0 END),0) FROM documents WHERE bundle=?1"),
        [key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(Error::io)?;
    if bytes > MAX_INDEX_BYTES || documents > MAX_INDEX_DOCUMENTS {
        return Err(Error::new(
            "INDEX_LIMIT",
            "stored index exceeds load budget",
            7,
        ));
    }
    Ok(Some(IndexStats {
        bytes,
        documents,
        concepts,
        ..header
    }))
}

impl SqliteIndex {
    fn selected(
        &self,
        key: &str,
        path: Option<&str>,
        expected: Option<&str>,
        limit: usize,
    ) -> Result<Option<Snapshot>> {
        let tx = self.0.unchecked_transaction().map_err(Error::io)?;
        let metadata = if path.is_some() {
            header(&tx, key)?
        } else {
            stats(&tx, key)?
        };
        let Some(header) = metadata else {
            return Ok(None);
        };
        if let Some(path) = path {
            let bytes: Option<usize> = tx
                .query_row(
                    "SELECT length(CAST(raw AS BLOB)) FROM documents WHERE bundle=?1 AND path=?2",
                    params![key, path],
                    |r| r.get(0),
                )
                .optional()
                .map_err(Error::io)?;
            if bytes.is_some_and(|bytes| bytes > 2 * 1024 * 1024) {
                return Err(Error::new(
                    "INDEX_LIMIT",
                    "stored document exceeds read budget",
                    7,
                ));
            }
        }
        if expected.is_some_and(|generation| generation != header.generation) {
            return Err(Error::new(
                "INDEX_CHANGED",
                "index refreshed during query; repeat search",
                6,
            ));
        }
        let predicate = if expected.is_some() { CONCEPT } else { "1" };
        let mut statement = tx.prepare(&format!(
            "SELECT path,hash,raw FROM documents WHERE bundle=?1 AND (?2 IS NULL OR path=?2) AND ({predicate}) ORDER BY path LIMIT ?3"
        )).map_err(Error::io)?;
        let documents = statement
            .query_map(params![key, path, limit as i64], |r| {
                Ok(IndexedDocument {
                    path: r.get(0)?,
                    hash: r.get(1)?,
                    raw: r.get(2)?,
                })
            })
            .map_err(Error::io)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::io)?;
        for doc in &documents {
            okf_core::logical_path(&doc.path)?;
            if okf_core::sha256(doc.raw.as_bytes()) != doc.hash {
                return Err(Error::new(
                    "INDEX_CORRUPT",
                    "stored content hash mismatch; rebuild index",
                    5,
                ));
            }
        }
        Ok(Some(Snapshot {
            generation: header.generation,
            upstream_revision: header.upstream_revision,
            checked_at: header.checked_at,
            documents,
        }))
    }
}
impl Index for SqliteIndex {
    fn load(&self, key: &str) -> Result<Option<Snapshot>> {
        self.selected(key, None, None, MAX_INDEX_DOCUMENTS)
    }
    fn stats(&self, key: &str) -> Result<Option<IndexStats>> {
        let tx = self.0.unchecked_transaction().map_err(Error::io)?;
        stats(&tx, key)
    }
    fn read(&self, key: &str, path: &str) -> Result<Option<Snapshot>> {
        self.selected(key, Some(path), None, 1)
    }
    fn search(&self, key: &str, generation: &str, limit: usize) -> Result<Option<Snapshot>> {
        self.selected(key, None, Some(generation), limit.min(MAX_INDEX_DOCUMENTS))
    }
    fn publish(&self, key: &str, expected: Option<&str>, snapshot: &Snapshot) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.0, TransactionBehavior::Immediate)
            .map_err(Error::io)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT generation FROM snapshots WHERE bundle=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Error::io)?;
        if current.as_deref() != expected {
            return Err(Error::new(
                "INDEX_CHANGED",
                "index refreshed concurrently; retry sync",
                6,
            ));
        }
        for doc in &snapshot.documents {
            tx.execute("INSERT INTO documents(bundle,path,hash,raw,generation) VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(bundle,path) DO UPDATE SET raw=excluded.raw,hash=excluded.hash,generation=excluded.generation",
                params![key,doc.path,doc.hash,doc.raw,snapshot.generation]).map_err(Error::io)?;
        }
        tx.execute(
            "DELETE FROM documents WHERE bundle=?1 AND generation<>?2",
            params![key, snapshot.generation],
        )
        .map_err(Error::io)?;
        tx.execute("INSERT INTO snapshots(bundle,generation,revision,checked_at) VALUES(?1,?2,?3,?4)
            ON CONFLICT(bundle) DO UPDATE SET generation=excluded.generation,revision=excluded.revision,checked_at=excluded.checked_at",
            params![key,snapshot.generation,snapshot.upstream_revision,snapshot.checked_at]).map_err(Error::io)?;
        tx.commit().map_err(Error::io)
    }
    fn clear(&self, key: &str) -> Result<()> {
        let tx = self.0.unchecked_transaction().map_err(Error::io)?;
        tx.execute("DELETE FROM documents WHERE bundle=?1", [key])
            .map_err(Error::io)?;
        tx.execute("DELETE FROM snapshots WHERE bundle=?1", [key])
            .map_err(Error::io)?;
        tx.commit().map_err(Error::io)
    }
}
