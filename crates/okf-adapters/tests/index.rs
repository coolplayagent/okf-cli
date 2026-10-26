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
