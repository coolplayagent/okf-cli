use okf_adapters::{ConfigRegistry, Filesystem, SqliteCache};
use okf_application::{BundleBinding, BundleReader, BundleRegistry, Cache};
use okf_core::Document;
use std::fs;

fn binding(root: &std::path::Path) -> BundleBinding {
    BundleBinding {
        id: "test-bundle".into(),
        name: "book".into(),
        kind: "fs".into(),
        root: fs::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        scope: "project".into(),
    }
}

#[test]
fn config_is_relative_to_project_and_duplicate_alias_does_not_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    let registry = ConfigRegistry::discover(home.path().into(), root.path().into(), None);
    let b = binding(&root.path().join("docs"));
    registry.add(&b).unwrap();
    assert!(registry.add(&b).is_err());
    let stored = fs::read_to_string(root.path().join(".okf-cli/config.toml")).unwrap();
    assert!(stored.contains("root = \"docs\""));
    let child = ConfigRegistry::discover(home.path().into(), root.path().join("docs"), None);
    assert_eq!(
        fs::canonicalize(&child.list().unwrap()[0].root).unwrap(),
        fs::canonicalize(root.path().join("docs")).unwrap()
    );
    registry.remove("book", "project").unwrap();
    assert!(root.path().join("docs").exists());
}

#[test]
fn size_limits_and_cache_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("note.md"), "---\ntype: Note\n---\nbody").unwrap();
    let b = binding(root.path());
    assert_eq!(
        Filesystem.read(&b, "note.md", 3).unwrap_err().code,
        "DOCUMENT_TOO_LARGE"
    );
    let cache = SqliteCache::open(&root.path().join("cache.sqlite")).unwrap();
    let card = Document::parse("---\ntype: Note\n---\n")
        .unwrap()
        .card("note", chrono::Utc::now());
    cache
        .remember(&b.id, "note.md", "hash", &card, "2026-10-21T00:00:00Z")
        .unwrap();
    cache
        .remember(&b.id, "note.md", "newhash", &card, "2026-10-22T00:00:00Z")
        .unwrap();
    assert_eq!(cache.status(&b.id).unwrap()["cached_cards"], 1);
    assert_eq!(cache.status("other-bundle").unwrap()["cached_cards"], 0);
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape_or_form_directory_cycles() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        root.path().join("leak.md"),
    )
    .unwrap();
    let b = binding(root.path());
    assert_eq!(
        Filesystem.read(&b, "leak.md", 1024).unwrap_err().code,
        "PATH_ESCAPE"
    );
    fs::remove_file(root.path().join("leak.md")).unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    assert_eq!(Filesystem.list(&b, "").unwrap_err().code, "DIRECTORY_LINK");
}

#[cfg(unix)]
#[test]
fn case_collisions_are_not_silently_merged() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("A.md"), "a").unwrap();
    fs::write(root.path().join("a.md"), "b").unwrap();
    // macOS may use a case-insensitive filesystem.
    if fs::read_dir(root.path()).unwrap().count() == 2 {
        assert_eq!(
            Filesystem.list(&binding(root.path()), "").unwrap_err().code,
            "CASE_COLLISION"
        );
    }
}
