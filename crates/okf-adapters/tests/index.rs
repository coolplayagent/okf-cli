use okf_adapters::SqliteIndex;
use okf_application::{Index, IndexedDocument, Snapshot};
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
