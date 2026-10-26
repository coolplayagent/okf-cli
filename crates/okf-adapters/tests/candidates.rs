use okf_adapters::SqliteIndex;
use okf_application::{
    index_generation, CandidateRequest, Index, IndexedDocument, RequestControl, SearchFilter,
    Snapshot,
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

fn control() -> RequestControl {
    RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(false)))
}
fn snapshot(entries: &[(&str, &str)]) -> Snapshot {
    let documents: Vec<_> = entries
        .iter()
        .map(|(path, raw)| IndexedDocument {
            path: path.to_string(),
            raw: raw.to_string(),
            hash: okf_core::sha256(raw.as_bytes()),
        })
        .collect();
    Snapshot {
        generation: index_generation(
            None,
            documents.iter().map(|d| (d.path.as_str(), d.hash.as_str())),
        )
        .unwrap(),
        upstream_revision: None,
        checked_at: "2030-01-01T00:00:00Z".into(),
        documents,
    }
}

#[test]
fn candidate_budgets_filters_and_time_dependent_metadata_preserve_evidence() {
    let home = tempfile::tempdir().unwrap();
    let index = SqliteIndex::open(&home.path().join("content.sqlite")).unwrap();
    let data = snapshot(&[("a.md", "---\ntype: Note\n---\nnoise"), ("z/evidence.md", "---\ntype: Service\nstale_after: 2031-01-01T00:00:00Z\n---\n认证流程 İSTANBUL late-evidence")]);
    index.publish("book", None, &data).unwrap();
    let filter = SearchFilter {
        types: vec!["Service".into()],
        ..Default::default()
    };
    let terms = vec!["认证流程".into()];
    let control = control();
    let mut request = CandidateRequest {
        generation: &data.generation,
        terms: &terms,
        filter: &filter,
        evaluated_at: "2030-01-01T00:00:00Z".parse().unwrap(),
        body_limit: 1,
        row_limit: 1000,
        control: &control,
    };
    let selected = index.candidates("book", &request).unwrap().unwrap();
    assert!(selected.complete);
    assert_eq!(selected.snapshot.documents[0].path, "z/evidence.md");
    assert!(selected.rows_read <= request.row_limit);
    let expiry = vec!["expired".into()];
    request.terms = &expiry;
    request.evaluated_at = "2032-01-01T00:00:00Z".parse().unwrap();
    assert!(
        index
            .candidates("book", &request)
            .unwrap()
            .unwrap()
            .complete
    );
    request.row_limit = 0;
    let selected = index.candidates("book", &request).unwrap().unwrap();
    assert!(!selected.complete);
    assert_eq!(selected.rows_read, 0);
    assert!(selected.snapshot.documents.is_empty());
    request.row_limit = 1000;
    request.body_limit = 0;
    assert!(
        !index
            .candidates("book", &request)
            .unwrap()
            .unwrap()
            .complete
    );
    request.row_limit = usize::MAX;
    assert_eq!(
        index.candidates("book", &request).err().unwrap().code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn mutations_invalidate_candidates_and_resync_repairs_same_generation() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let data = snapshot(&[("a.md", "---\ntype: Note\n---\nneedle")]);
    index.publish("book", None, &data).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    for mutation in [
        "DELETE FROM candidate_postings WHERE gram='nee'",
        "UPDATE candidate_documents SET metadata='{}'",
        "UPDATE documents SET raw=raw",
        "UPDATE candidate_snapshots SET postings=0",
    ] {
        sql.execute(mutation, []).unwrap();
        let report = index.verify("book", 100, &control()).unwrap().unwrap();
        assert!(report
            .findings
            .iter()
            .any(|(_, e)| e.code == "INDEX_CANDIDATES_CORRUPT"));
        let terms = vec!["needle".into()];
        let filter = SearchFilter::default();
        let control = control();
        let request = CandidateRequest {
            generation: &data.generation,
            terms: &terms,
            filter: &filter,
            evaluated_at: chrono::DateTime::UNIX_EPOCH,
            body_limit: 1,
            row_limit: 100,
            control: &control,
        };
        assert!(index.candidates("book", &request).is_err());
        index
            .publish("book", Some(&data.generation), &data)
            .unwrap();
        assert_eq!(
            index
                .verify("book", 100, &control)
                .unwrap()
                .unwrap()
                .failures,
            0
        );
        assert!(
            index
                .candidates("book", &request)
                .unwrap()
                .unwrap()
                .complete
        );
    }
}

#[test]
fn candidate_failure_rolls_back_original_and_derived_publication() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let data = snapshot(&[("a.md", "---\ntype: Note\n---\nneedle")]);
    index.publish("book", None, &data).unwrap();
    let invalid = snapshot(&[("a.md", "missing frontmatter")]);
    assert!(index
        .publish("book", Some(&data.generation), &invalid)
        .is_err());
    assert_eq!(
        index.load("book").unwrap().unwrap().generation,
        data.generation
    );
    assert_eq!(
        index
            .verify("book", 100, &control())
            .unwrap()
            .unwrap()
            .failures,
        0
    );
    let cancelled = RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(true)));
    assert_eq!(
        index
            .publish_controlled("book", Some(&data.generation), &data, &cancelled)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(
        index
            .verify("book", 100, &control())
            .unwrap()
            .unwrap()
            .failures,
        0
    );
}

#[test]
fn oversized_gram_sets_fall_back_and_old_content_schema_stays_readable() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let raw = format!(
        "---\ntype: Note\n---\n{} needle",
        (0..5000)
            .map(|i| char::from_u32(0x4e00 + i).unwrap())
            .collect::<String>()
    );
    let data = snapshot(&[("a.md", &raw)]);
    index.publish("book", None, &data).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("SELECT fallback FROM candidate_documents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let terms = vec!["needle".into()];
    let filter = SearchFilter::default();
    let control = control();
    let request = CandidateRequest {
        generation: &data.generation,
        terms: &terms,
        filter: &filter,
        evaluated_at: chrono::DateTime::UNIX_EPOCH,
        body_limit: 1,
        row_limit: 100,
        control: &control,
    };
    let selected = index.candidates("book", &request).unwrap().unwrap();
    assert!(selected.complete);
    assert_eq!(selected.snapshot.documents.len(), 1);
    sql.execute("DELETE FROM candidate_snapshots", []).unwrap();
    assert_eq!(
        index.candidates("book", &request).err().unwrap().code,
        "INDEX_CANDIDATES_MISSING"
    );
    assert_eq!(
        index.read("book", "a.md").unwrap().unwrap().documents[0].raw,
        raw
    );
    index
        .publish("book", Some(&data.generation), &data)
        .unwrap();
    assert_eq!(
        index
            .verify("book", 100, &control)
            .unwrap()
            .unwrap()
            .failures,
        0
    );
}
