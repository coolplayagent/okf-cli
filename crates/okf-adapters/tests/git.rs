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
