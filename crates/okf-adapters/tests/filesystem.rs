use okf_adapters::{ConfigRegistry, Filesystem, SqliteCache};
use okf_application::{
    BundleBinding, BundleReader, BundleRegistry, Cache, RequestControl, ScanBudget,
};
use okf_core::Document;
use std::fs;
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

fn control() -> RequestControl {
    RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(false)))
}

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
        git: None,
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
        Filesystem
            .read(&b, "note.md", 3, &control())
            .unwrap_err()
            .code,
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
        Filesystem
            .read(&b, "leak.md", 1024, &control())
            .unwrap_err()
            .code,
        "PATH_ESCAPE"
    );
    fs::remove_file(root.path().join("leak.md")).unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    assert_eq!(
        Filesystem
            .list(&b, "", &mut ScanBudget::new(100, &control()))
            .unwrap_err()
            .code,
        "DIRECTORY_LINK"
    );
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
            Filesystem
                .list(
                    &binding(root.path()),
                    "",
                    &mut ScanBudget::new(100, &control())
                )
                .unwrap_err()
                .code,
            "CASE_COLLISION"
        );
    }
}

#[test]
fn scan_budget_bounds_enumeration_including_ignored_files() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..40 {
        fs::write(root.path().join(format!("ignored-{index}.txt")), "ignored").unwrap();
    }
    let control = control();
    let mut budget = ScanBudget::new(4, &control);
    let listing = Filesystem
        .list(&binding(root.path()), "", &mut budget)
        .unwrap();
    assert!(!listing.complete);
    assert!(listing.entries.is_empty());
    assert_eq!(budget.used(), 4);
    let mut budget = ScanBudget::new(42, &control);
    assert!(
        Filesystem
            .list(&binding(root.path()), "", &mut budget)
            .unwrap()
            .complete
    );
    assert_eq!(budget.used(), 42); // Open + 40 entries + end-of-directory.
}

#[test]
fn expired_or_cancelled_requests_stop_before_filesystem_access() {
    let root = tempfile::tempdir().unwrap();
    let b = binding(root.path());
    let expired = RequestControl::new(Duration::ZERO, Arc::new(AtomicBool::new(false)));
    assert_eq!(
        Filesystem
            .list(&b, "missing", &mut ScanBudget::new(100, &expired))
            .unwrap_err()
            .code,
        "DEADLINE_EXCEEDED"
    );
    let cancelled = RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(true)));
    assert_eq!(
        Filesystem
            .read(&b, "missing.md", 1024, &cancelled)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
}

#[test]
fn resource_metadata_distinguishes_missing_files_and_directories_without_reading_bytes() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("tools")).unwrap();
    fs::write(root.path().join("tools/raw.bin"), [0xff, 0x00, 0xfe]).unwrap();
    let b = binding(root.path());
    let metadata = Filesystem
        .stat(&b, "tools/raw.bin", &control())
        .unwrap()
        .unwrap();
    assert!(!metadata.directory);
    assert_eq!(metadata.bytes, 3);
    assert!(
        Filesystem
            .stat(&b, "tools", &control())
            .unwrap()
            .unwrap()
            .directory
    );
    assert!(Filesystem
        .stat(&b, "absent.py", &control())
        .unwrap()
        .is_none());
    assert!(Filesystem.stat(&b, "../outside", &control()).is_err());
}

#[test]
fn cache_reset_preserves_corrupt_bytes_and_refuses_active_consumers() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("index.sqlite");
    let cache = SqliteCache::open(&path).unwrap();
    assert_eq!(SqliteCache::reset(&path).unwrap_err().code, "CACHE_LOCKED");
    drop(cache);
    fs::write(&path, b"corrupt database").unwrap();
    assert!(SqliteCache::open(&path).is_err());
    let result = SqliteCache::reset(&path).unwrap();
    let archive = std::path::Path::new(result["archive"].as_str().unwrap());
    assert_eq!(
        fs::read(archive.join("index.sqlite")).unwrap(),
        b"corrupt database"
    );
    assert_eq!(
        SqliteCache::open(&path).unwrap().status("book").unwrap()["cached_cards"],
        0
    );
}

#[test]
fn config_os_lock_rejects_active_owner_and_reuses_abandoned_file() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".okf-cli")).unwrap();
    let path = root.path().join(".okf-cli/config.lock");
    let file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.try_lock().unwrap();
    let registry = ConfigRegistry::discover(home.path().into(), root.path().into(), None);
    let b = binding(root.path());
    assert_eq!(registry.add(&b).unwrap_err().code, "CONFIG_LOCKED");
    drop(file); // Simulates an owner exiting without deleting its lock file.
    fs::write(&path, "pid=dead-owner").unwrap();
    registry.add(&b).unwrap();
    assert!(path.exists());
    assert_eq!(registry.list().unwrap().len(), 1);
}
