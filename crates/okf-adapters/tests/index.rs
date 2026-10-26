use okf_adapters::SqliteIndex;
use okf_application::{Index, IndexedDocument, Snapshot};
fn control() -> okf_application::RequestControl {
    okf_application::RequestControl::new(
        std::time::Duration::from_secs(30),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
}

#[test]
fn verification_detects_corruption_bounds_coverage_and_preserves_storage() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    assert_eq!(
        SqliteIndex::open_read_only(&path).err().unwrap().code,
        "INDEX_MISSING"
    );
    assert!(!path.exists());
    let index = SqliteIndex::open(&path).unwrap();
    let mut original = snapshot("", "---\ntype: Note\n---\nevidence");
    original.documents.push(IndexedDocument {
        path: "z.md".into(),
        ..original.documents[0].clone()
    });
    original.generation = okf_application::index_generation(
        None,
        original
            .documents
            .iter()
            .map(|d| (d.path.as_str(), d.hash.as_str())),
    )
    .unwrap();
    index.publish("book", None, &original).unwrap();
    let reader = SqliteIndex::open_read_only(&path).unwrap();
    let verify = || reader.verify("book", 100, &control()).unwrap().unwrap();
    assert!(reader.verify("missing", 100, &control()).unwrap().is_none());
    assert!(verify().complete);
    assert_eq!(verify().failures, 0);
    assert!(reader.clear("book").is_err());
    let partial = reader.verify("book", 1, &control()).unwrap().unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.documents_checked, 1);
    assert_eq!(partial.failures, 0);
    assert_eq!(partial.interruption.unwrap().code, "SCAN_LIMIT");
    let sql = rusqlite::Connection::open(&path).unwrap();
    for (mutation, expected) in [
        (
            "UPDATE documents SET raw='damaged' WHERE path='a.md'",
            "INDEX_CORRUPT",
        ),
        (
            "UPDATE documents SET hash='invalid' WHERE path='a.md'",
            "INDEX_CORRUPT",
        ),
        (
            "UPDATE documents SET generation='wrong' WHERE path='a.md'",
            "INDEX_GENERATION_MISMATCH",
        ),
        (
            "UPDATE documents SET path='../a.md' WHERE path='a.md'",
            "INDEX_PATH_INVALID",
        ),
        (
            "DELETE FROM documents WHERE path='a.md'",
            "INDEX_SNAPSHOT_MISMATCH",
        ),
    ] {
        sql.execute(mutation, []).unwrap();
        let report = verify();
        assert!(report.complete);
        assert!(
            report.findings.iter().any(|(_, e)| e.code == expected),
            "{report:?}"
        );
        assert_eq!(verify().failures, report.failures);
        index
            .publish("book", Some(&original.generation), &original)
            .unwrap();
        assert_eq!(verify().failures, 0);
    }
    sql.execute("UPDATE snapshots SET generation='wrong'", [])
        .unwrap();
    let report = verify();
    assert!(report
        .findings
        .iter()
        .any(|(_, e)| e.code == "INDEX_SNAPSHOT_MISMATCH"));
    let cancelled = okf_application::RequestControl::new(
        std::time::Duration::from_secs(30),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    assert_eq!(
        reader.verify("book", 100, &cancelled).unwrap_err().code,
        "CANCELLED"
    );
}
fn snapshot(generation: &str, raw: &str) -> Snapshot {
    Snapshot {
        generation: generation.into(),
        upstream_revision: None,
        checked_at: "2026-10-23T00:00:00Z".into(),
        documents: vec![IndexedDocument {
            path: "a.md".into(),
            hash: okf_core::sha256(raw.as_bytes()),
            raw: raw.into(),
        }],
    }
}
#[test]
fn publication_is_atomic_and_rejects_stale_writers() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let first = SqliteIndex::open(&path).unwrap();
    let second = SqliteIndex::open(&path).unwrap();
    let old = snapshot("old", "---\ntype: Note\n---\nold");
    first.publish("book", None, &old).unwrap();
    let next = snapshot("next", "---\ntype: Note\n---\nnext");
    second.publish("book", Some("old"), &next).unwrap();
    assert_eq!(
        first
            .publish("book", Some("old"), &snapshot("stale", "stale"))
            .unwrap_err()
            .code,
        "INDEX_CHANGED"
    );
    assert_eq!(
        first.load("book").unwrap().unwrap().documents[0].raw,
        next.documents[0].raw
    );
    let mut invalid = next.clone();
    invalid.documents.push(invalid.documents[0].clone());
    assert!(first.publish("book", Some("next"), &invalid).is_err());
    assert_eq!(first.load("book").unwrap().unwrap().documents.len(), 1);
    let empty = Snapshot {
        documents: vec![],
        generation: "empty".into(),
        ..next
    };
    first.publish("book", Some("next"), &empty).unwrap();
    assert!(second.load("book").unwrap().unwrap().documents.is_empty());
    first.clear("book").unwrap();
    assert!(second.load("book").unwrap().is_none());
}

#[test]
fn selective_reads_preserve_versions_and_do_not_load_unrelated_bodies() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let mut data = snapshot("first", "---\ntype: Note\n---\n认证🙂");
    for path in ["index.md", "nested/log.md", "nested/Index.md", "z.md"] {
        let raw = "---\ntype: Note\n---\nevidence".to_string();
        data.documents.push(IndexedDocument {
            path: path.into(),
            hash: okf_core::sha256(raw.as_bytes()),
            raw,
        });
    }
    index.publish("book", None, &data).unwrap();
    let stats = index.stats("book").unwrap().unwrap();
    assert_eq!(stats.documents, 5);
    assert_eq!(stats.concepts, 3);
    assert_eq!(
        stats.bytes,
        data.documents.iter().map(|d| d.raw.len()).sum::<usize>()
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE documents SET raw='tampered' WHERE path='z.md'", [])
        .unwrap();
    assert_eq!(index.load("book").unwrap_err().code, "INDEX_CORRUPT");
    assert_eq!(
        index.read("book", "a.md").unwrap().unwrap().documents[0].raw,
        data.documents[0].raw
    );
    assert_eq!(
        index
            .search("book", "first", 2)
            .unwrap()
            .unwrap()
            .documents
            .len(),
        2
    );
    assert_eq!(
        index.search("book", "first", 3).unwrap_err().code,
        "INDEX_CORRUPT"
    );
    assert!(index
        .read("book", "missing.md")
        .unwrap()
        .unwrap()
        .documents
        .is_empty());
    assert_eq!(index.stats("book").unwrap().unwrap().documents, 5);
    index
        .publish("book", Some("first"), &snapshot("second", "new"))
        .unwrap();
    assert_eq!(
        index.search("book", "first", 1).unwrap_err().code,
        "INDEX_CHANGED"
    );
    assert_eq!(
        index.read("book", "a.md").unwrap().unwrap().generation,
        "second"
    );
    assert!(index.read("absent", "a.md").unwrap().is_none());
}

#[test]
fn republishing_unchanged_sources_repairs_corrupted_stored_content() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let original = snapshot("same-generation", "---\ntype: Note\n---\noriginal evidence");
    index.publish("book", None, &original).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE documents SET raw='corrupted' WHERE bundle='book'",
            [],
        )
        .unwrap();
    assert_eq!(
        index.read("book", "a.md").unwrap_err().code,
        "INDEX_CORRUPT"
    );
    let previous = index.stats("book").unwrap().unwrap();
    index
        .publish("book", Some(&previous.generation), &original)
        .unwrap();
    assert_eq!(
        index.read("book", "a.md").unwrap().unwrap().documents[0].raw,
        original.documents[0].raw
    );
}

#[test]
fn verification_caps_findings_and_rejects_oversized_bodies() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("content.sqlite");
    let index = SqliteIndex::open(&path).unwrap();
    let mut data = snapshot("v1", "small");
    for i in 0..110 {
        data.documents.push(IndexedDocument {
            path: format!("note-{i:03}.md"),
            hash: "wrong".into(),
            raw: "small".into(),
        });
    }
    index.publish("book", None, &data).unwrap();
    let report = index.verify("book", 1000, &control()).unwrap().unwrap();
    assert!(report.complete);
    assert_eq!(report.findings.len(), 100);
    assert!(report.failures > 100);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE documents SET raw=?1 WHERE path='a.md'",
        ["x".repeat(2 * 1024 * 1024 + 1)],
    )
    .unwrap();
    let report = index.verify("book", 1, &control()).unwrap().unwrap();
    assert!(!report.complete);
    assert_eq!(report.findings[0].1.code, "INDEX_DOCUMENT_LIMIT");
    assert_eq!(report.documents_checked, 1);
}
