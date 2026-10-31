use okf_adapters::Git;
use okf_application::{BundleBinding, BundleReader, RequestControl, ScanBudget};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new(std::env::var_os("OKF_GIT").unwrap_or_else(|| "git".into()))
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("Git is required for Git adapter tests");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn control() -> RequestControl {
    RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(false)))
}
#[test]
fn pinned_tree_ignores_worktree_and_branch_updates_and_bounds_reads() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--initial-branch=main"]);
    fs::create_dir(repo.path().join("knowledge")).unwrap();
    let raw = "---\ntype: Note\n---\n认证🙂\n";
    fs::write(repo.path().join("knowledge/认证.md"), raw).unwrap();
    fs::write(repo.path().join("knowledge/run.py"), b"do not execute").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "first"]);
    let pinned = Git::pin(repo.path(), "main", "knowledge", &control()).unwrap();
    let b = BundleBinding {
        id: "git-book".into(),
        name: "book".into(),
        kind: "git".into(),
        root: repo.path().to_string_lossy().into_owned(),
        scope: "project".into(),
        git: Some(pinned.clone()),
    };
    fs::write(
        repo.path().join("knowledge/认证.md"),
        "---\ntype: Note\n---\nnew",
    )
    .unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "second"]);
    assert_eq!(Git.read(&b, "认证.md", 1024, &control()).unwrap(), raw);
    assert_ne!(
        Git::pin(repo.path(), "main", "knowledge", &control())
            .unwrap()
            .commit,
        pinned.commit
    );
    assert_eq!(
        Git.read(&b, "认证.md", 1, &control()).unwrap_err().code,
        "DOCUMENT_TOO_LARGE"
    );
    assert!(Git.stat(&b, "run.py", &control()).unwrap().is_some());
    assert!(Git.stat(&b, "missing.py", &control()).unwrap().is_none());
    let c = control();
    let listing = Git.list(&b, "", &mut ScanBudget::new(100, &c)).unwrap();
    assert!(listing.complete);
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].path, "认证.md");
    assert!(
        !Git.list(&b, "", &mut ScanBudget::new(1, &c))
            .unwrap()
            .complete
    );
    assert_eq!(
        Git.read(&b, "../secret.md", 1024, &c).unwrap_err().code,
        "INVALID_PATH"
    );
    assert!(Git::pin(repo.path(), "--help", "", &c).is_err());
}

#[test]
fn git_symlink_objects_are_not_followed_even_without_worktree_links() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init"]);
    fs::write(repo.path().join("target.md"), "---\ntype: Note\n---\n").unwrap();
    git(repo.path(), &["add", "target.md"]);
    let blob = git(repo.path(), &["hash-object", "-w", "target.md"]);
    git(
        repo.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{blob},link.md"),
        ],
    );
    git(repo.path(), &["commit", "-m", "symlink"]);
    let b = BundleBinding {
        id: "book".into(),
        name: "book".into(),
        kind: "git".into(),
        root: repo.path().to_string_lossy().into_owned(),
        scope: "project".into(),
        git: Some(Git::pin(repo.path(), "HEAD", "", &control()).unwrap()),
    };
    assert_eq!(
        Git.read(&b, "link.md", 1024, &control()).unwrap_err().code,
        "GIT_UNSUPPORTED_ENTRY"
    );
}

#[test]
fn cache_compaction_keeps_pinned_objects_and_serializes_writers() {
    let source = tempfile::tempdir().unwrap();
    git(source.path(), &["init", "--initial-branch=main"]);
    let raw = "---\ntype: Note\n---\nold commit\n";
    fs::write(source.path().join("note.md"), raw).unwrap();
    git(source.path(), &["add", "."]);
    git(source.path(), &["commit", "-m", "old"]);
    let old = git(source.path(), &["rev-parse", "HEAD"]);
    fs::write(source.path().join("note.md"), "---\ntype: Note\n---\nnew").unwrap();
    git(source.path(), &["add", "."]);
    git(source.path(), &["commit", "-m", "new"]);
    let bare = tempfile::tempdir().unwrap();
    git(
        source.path(),
        &["clone", "--bare", ".", bare.path().to_str().unwrap()],
    );
    let binding = BundleBinding {
        id: "cache".into(),
        name: "cache".into(),
        kind: "git".into(),
        root: bare.path().to_string_lossy().into_owned(),
        scope: "project".into(),
        git: Some(Git::pin(bare.path(), &old, "", &control()).unwrap()),
    };
    let lock = Git::lock(bare.path()).unwrap();
    assert_eq!(
        Git::cache(bare.path(), true, &control()).unwrap_err().code,
        "GIT_CACHE_LOCKED"
    );
    drop(lock);
    let stats = Git::cache(bare.path(), true, &control()).unwrap();
    assert_eq!(stats["compacted"], true);
    assert_eq!(
        Git.read(&binding, "note.md", 1024, &control()).unwrap(),
        raw
    );
    assert_eq!(
        Git::fetch_bounded(
            bare.path(),
            "https://localhost:1/unreachable",
            "main",
            None,
            &control(),
            1,
            None
        )
        .unwrap_err()
        .code,
        "GIT_STORAGE_LIMIT"
    );
}
