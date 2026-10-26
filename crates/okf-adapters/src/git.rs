//! Read immutable Git objects. No checkout, filters, hooks, LFS or submodules.
use okf_application::{
    BundleBinding, BundleReader, Entry, GitRevision, Listing, RequestControl, ResourceMetadata,
    ScanBudget,
};
use okf_core::{logical_path, Error, Result};
use std::{
    collections::BTreeSet,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub struct Git;

fn command(root: &Path) -> Command {
    let mut cmd = Command::new(std::env::var_os("OKF_GIT").unwrap_or_else(|| "git".into()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    // Ambient Git plumbing variables must not redirect the repository or inject config.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg("--no-pager")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "protocol.version=2",
            "-c",
            "credential.helper=",
            "-c",
            "http.followRedirects=false",
            "-c",
            "http.lowSpeedLimit=1024",
            "-c",
            "http.lowSpeedTime=30",
        ]);
    cmd
}

fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // Let Git's signal handlers remove lockfiles before forcing the whole
        // group down. Keep the child unreaped so its process-group ID stays owned.
        unsafe {
            kill(-(child.id() as i32), 15);
        }
        thread::sleep(Duration::from_millis(500));
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let executable = std::path::PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:/Windows".into()),
        )
        .join("System32/taskkill.exe");
        let _ = Command::new(executable)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn run(root: &Path, args: &[&str], max: usize, control: &RequestControl) -> Result<Vec<u8>> {
    run_bounded(root, args, max, control, None)
}

fn run_bounded(
    root: &Path,
    args: &[&str],
    max: usize,
    control: &RequestControl,
    storage_limit: Option<u64>,
) -> Result<Vec<u8>> {
    control.check()?;
    if let Some(limit) = storage_limit {
        check_storage(root, limit, control)?;
    }
    let mut child = command(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            Error::new(
                "GIT_UNAVAILABLE",
                format!("cannot start Git: {e}; install Git or set OKF_GIT"),
                5,
            )
        })?;
    let overflow = Arc::new(AtomicBool::new(false));
    fn collect(
        mut input: impl Read,
        max: usize,
        overflow: Arc<AtomicBool>,
    ) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = input.read(&mut chunk)?;
            if n == 0 {
                return Ok(bytes);
            }
            if n > max.saturating_sub(bytes.len()) {
                overflow.store(true, Ordering::Relaxed);
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let over = overflow.clone();
    let out = thread::spawn(move || collect(stdout, max, over));
    let over = overflow.clone();
    let err = thread::spawn(move || collect(stderr, 65536, over));
    let mut last_storage_check = std::time::Instant::now();
    let status = loop {
        let storage_error = if last_storage_check.elapsed() >= Duration::from_millis(50) {
            last_storage_check = std::time::Instant::now();
            storage_limit.and_then(|limit| check_storage(root, limit, control).err())
        } else {
            None
        };
        let stopped = control.check().err().or(storage_error).or_else(|| {
            overflow.load(Ordering::Relaxed).then(|| {
                Error::new(
                    "GIT_OUTPUT_LIMIT",
                    "Git output exceeded the request limit",
                    7,
                )
            })
        });
        if let Some(error) = stopped {
            stop(&mut child);
            let _ = out.join();
            let _ = err.join();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                stop(&mut child);
                let _ = out.join();
                let _ = err.join();
                return Err(Error::io(error));
            }
        }
    };
    let bytes = out
        .join()
        .map_err(|_| Error::new("GIT_ERROR", "output reader failed", 5))?
        .map_err(Error::io)?;
    let stderr = err
        .join()
        .map_err(|_| Error::new("GIT_ERROR", "error reader failed", 5))?
        .map_err(Error::io)?;
    control.check()?;
    if let Some(limit) = storage_limit {
        check_storage(root, limit, control)?;
    }
    if overflow.load(Ordering::Relaxed) {
        return Err(Error::new(
            "GIT_OUTPUT_LIMIT",
            "Git output exceeded the request limit",
            7,
        ));
    }
    if !status.success() {
        // stderr can contain credential material or repository-specific commands.
        if args.first() == Some(&"fetch") {
            return Err(transport_error(&String::from_utf8_lossy(&stderr)));
        }
        return Err(Error::new(
            "GIT_OBJECT_UNAVAILABLE",
            "Git operation failed; repository, ref, object or transport is unavailable",
            5,
        ));
    }
    Ok(bytes)
}

fn transport_error(stderr: &str) -> Error {
    let text = stderr.to_ascii_lowercase();
    let (code, message) = if text.contains("returned error: 401")
        || text.contains("returned error: 403")
        || text.contains("authentication failed")
        || text.contains("could not read username")
    {
        (
            "GIT_AUTH_FAILED",
            "remote denied access or requires authentication",
        )
    } else if text.contains("certificate")
        || text.contains("ssl peer")
        || text.contains("sec_e_untrusted_root")
        || text.contains("sec_e_wrong_principal")
        || text.contains("sec_e_cert_expired")
    {
        ("GIT_TLS_FAILED", "remote TLS verification failed")
    } else if text.contains("couldn't find remote ref")
        || text.contains("not our ref")
        || text.contains("returned error: 404")
        || (text.contains("repository") && text.contains("not found"))
    {
        (
            "GIT_REMOTE_NOT_FOUND",
            "remote repository or requested ref is unavailable",
        )
    } else {
        (
            "GIT_TRANSPORT_FAILED",
            "remote transfer failed or was interrupted",
        )
    };
    Error::new(code, message, 5)
}

// A sampled disk quota, not a wire-byte limit: Git may write one chunk before
// the next checkpoint. No content is read and symbolic directories are rejected.
fn object_metadata(path: &Path) -> Result<std::fs::Metadata> {
    let metadata = path.symlink_metadata().map_err(Error::io)?;
    validate_object_metadata(metadata)
}

fn validate_object_metadata(metadata: std::fs::Metadata) -> Result<std::fs::Metadata> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::new(
                "PATH_ESCAPE",
                "object cache must not contain reparse points",
                4,
            ));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            "PATH_ESCAPE",
            "object cache must not contain symbolic links",
            4,
        ));
    }
    Ok(metadata)
}

fn check_storage(root: &Path, limit: u64, control: &RequestControl) -> Result<u64> {
    let mut pending = vec![root.join("objects")];
    let mut bytes = 0u64;
    let mut entries = 0;
    while let Some(path) = pending.pop() {
        control.check()?;
        object_metadata(&path)?;
        for entry in std::fs::read_dir(path).map_err(Error::io)? {
            control.check()?;
            entries += 1;
            if entries > 100_000 {
                return Err(Error::new(
                    "GIT_STORAGE_LIMIT",
                    "object inventory exceeds entry budget",
                    7,
                ));
            }
            let entry = entry.map_err(Error::io)?;
            // Git renames temporary pack/loose objects during a fetch. A sampled
            // inventory may see an entry just before that name disappears; the
            // final inventory after Git exits still enforces the storage quota.
            let metadata = match entry.path().symlink_metadata() {
                Ok(metadata) => validate_object_metadata(metadata)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::io(error)),
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
            if bytes > limit {
                return Err(Error::new(
                    "GIT_STORAGE_LIMIT",
                    "Git object storage budget exceeded; existing binding retained",
                    7,
                ));
            }
        }
    }
    Ok(bytes)
}

fn text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes)
        .map_err(|_| Error::new("INVALID_OKF", "Git object or path is not UTF-8", 1))
}

impl Git {
    pub fn pin(
        root: &Path,
        reference: &str,
        bundle_path: &str,
        control: &RequestControl,
    ) -> Result<GitRevision> {
        logical_path(bundle_path)?;
        if reference.is_empty()
            || reference.starts_with('-')
            || reference.chars().any(char::is_control)
        {
            return Err(Error::new("INVALID_REFERENCE", "invalid Git ref", 2));
        }
        let commit = text(run(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{reference}^{{commit}}"),
            ],
            256,
            control,
        )?)?
        .trim()
        .to_owned();
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::new(
                "INVALID_REFERENCE",
                "Git did not return a commit object ID",
                2,
            ));
        }
        let revision = GitRevision {
            reference: reference.into(),
            commit,
            bundle_path: bundle_path.into(),
            remote: None,
        };
        let b = BundleBinding {
            id: String::new(),
            name: String::new(),
            kind: "git".into(),
            root: root.to_string_lossy().into_owned(),
            scope: String::new(),
            git: Some(revision.clone()),
        };
        let tree = Self::object(&b, "", control)?
            .ok_or_else(|| Error::new("NOT_FOUND", "bundle tree missing", 3))?;
        if tree.1 != "tree" {
            return Err(Error::new(
                "NOT_A_DIRECTORY",
                "bundle root must be a tree",
                2,
            ));
        }
        Ok(revision)
    }

    fn spec(binding: &BundleBinding, path: &str) -> Result<String> {
        let git = binding
            .git
            .as_ref()
            .ok_or_else(|| Error::new("INVALID_CONFIG", "Git binding has no pinned revision", 2))?;
        if !matches!(git.commit.len(), 40 | 64)
            || !git.commit.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::new(
                "INVALID_CONFIG",
                "Git commit must be a full object ID",
                2,
            ));
        }
        logical_path(&git.bundle_path)?;
        logical_path(path)?;
        let full = [git.bundle_path.as_str(), path]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        Ok(format!("{}:{full}", git.commit))
    }

    // Traverse trees component by component, never follow symlinks or gitlinks.
    fn object(
        binding: &BundleBinding,
        path: &str,
        control: &RequestControl,
    ) -> Result<Option<(String, String, u64)>> {
        let spec = Self::spec(binding, path)?;
        let (commit, full) = spec.split_once(':').unwrap();
        let root = Path::new(&binding.root);
        let mut oid = text(run(
            root,
            &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
            256,
            control,
        )?)?
        .trim()
        .to_owned();
        let mut kind = "tree".to_owned();
        let mut size = 0;
        for component in full.split('/').filter(|p| !p.is_empty()) {
            if kind != "tree" {
                return Ok(None);
            }
            let data = run(
                root,
                &[
                    "--literal-pathspecs",
                    "ls-tree",
                    "-z",
                    "-l",
                    &oid,
                    "--",
                    component,
                ],
                65536,
                control,
            )?;
            if data.is_empty() {
                return Ok(None);
            }
            let record = text(data)?;
            let (header, _) = record
                .split_once('\t')
                .ok_or_else(|| Error::new("GIT_ERROR", "invalid tree record", 5))?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 4 {
                return Err(Error::new("GIT_ERROR", "invalid tree metadata", 5));
            }
            check_mode(fields[0])?;
            kind = fields[1].into();
            oid = fields[2].into();
            size = fields[3].parse().unwrap_or(0);
        }
        Ok(Some((oid, kind, size)))
    }

    /// HTTPS only; no interactive prompts, repository hooks or recursive submodules.
    pub fn fetch_bounded(
        root: &Path,
        remote: &str,
        reference: &str,
        control: &RequestControl,
        max_bytes: u64,
    ) -> Result<String> {
        if !remote.starts_with("https://")
            || remote[8..].contains('@')
            || remote.contains(['\n', '\r', '?', '#'])
        {
            return Err(Error::new(
                "INVALID_REMOTE",
                "remote must be an HTTPS URL without embedded credentials, query or fragment",
                2,
            ));
        }
        if reference.is_empty()
            || reference.starts_with('-')
            || reference.chars().any(char::is_control)
        {
            return Err(Error::new("INVALID_REFERENCE", "invalid Git ref", 2));
        }
        let token = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(Error::io)?
            .as_nanos();
        let destination = format!("refs/okf/fetch-{}-{token}", std::process::id());
        run_bounded(
            root,
            &[
                "fetch",
                "--depth=1",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                remote,
                &format!("+{reference}:{destination}"),
            ],
            65536,
            control,
            Some(max_bytes),
        )?;
        Ok(destination)
    }
    pub fn init(root: &Path, control: &RequestControl) -> Result<()> {
        std::fs::create_dir_all(root).map_err(Error::io)?;
        run(root, &["init", "--bare", "--template="], 65536, control)?;
        Ok(())
    }

    /// Serialize writes to an already initialized, tool-managed bare repository.
    pub fn lock(root: &Path) -> Result<std::fs::File> {
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("okf-cache.guard"))
            .map_err(Error::io)?;
        guard
            .try_lock()
            .map_err(|_| Error::new("GIT_CACHE_LOCKED", "another cache writer is active", 5))?;
        Ok(guard)
    }

    /// Repack without expiring refs or unreachable objects; all pinned commits survive.
    pub fn cache(
        root: &Path,
        compact: bool,
        control: &RequestControl,
    ) -> Result<serde_json::Value> {
        let _guard = Self::lock(root)?;
        let before = check_storage(root, u64::MAX, control)?;
        if compact {
            run(
                root,
                &["repack", "-a", "-d", "--keep-unreachable"],
                65536,
                control,
            )?;
        }
        let after = check_storage(root, u64::MAX, control)?;
        Ok(
            serde_json::json!({"before_bytes":before,"object_bytes":after,"compacted":compact,"historical_objects_retained":true}),
        )
    }
}

fn check_mode(mode: &str) -> Result<()> {
    if !matches!(mode, "040000" | "100644" | "100755") {
        return Err(Error::new(
            "GIT_UNSUPPORTED_ENTRY",
            "symlinks and submodules are not consumed",
            4,
        ));
    }
    Ok(())
}
impl BundleReader for Git {
    fn probe(&self, binding: &BundleBinding) -> Result<()> {
        let control =
            RequestControl::new(Duration::from_secs(30), Arc::new(AtomicBool::new(false)));
        self.probe_controlled(binding, &control)
    }
    fn probe_controlled(&self, binding: &BundleBinding, control: &RequestControl) -> Result<()> {
        match Self::object(binding, "", control)? {
            Some((_, kind, _)) if kind == "tree" => Ok(()),
            _ => Err(Error::new("NOT_A_DIRECTORY", "bundle tree unavailable", 3)),
        }
    }
    fn stat(
        &self,
        b: &BundleBinding,
        path: &str,
        c: &RequestControl,
    ) -> Result<Option<ResourceMetadata>> {
        Ok(
            Self::object(b, path, c)?.map(|(_, kind, bytes)| ResourceMetadata {
                directory: kind == "tree",
                bytes,
            }),
        )
    }
    fn list(
        &self,
        b: &BundleBinding,
        directory: &str,
        budget: &mut ScanBudget<'_>,
    ) -> Result<Listing> {
        budget.charge()?;
        let (oid, kind, _) = Self::object(b, directory, budget.control)?
            .ok_or_else(|| Error::new("NOT_FOUND", "directory missing", 3))?;
        if kind != "tree" {
            return Err(Error::new(
                "NOT_A_DIRECTORY",
                "requested path is not a tree",
                3,
            ));
        }
        let data = run(
            Path::new(&b.root),
            &["ls-tree", "-z", "-l", &oid],
            2 * 1024 * 1024,
            budget.control,
        )?;
        let raw = text(data)?;
        let mut entries = vec![];
        let mut names = BTreeSet::new();
        for record in raw.split('\0') {
            match budget.charge() {
                Ok(()) => {}
                Err(e) if e.code == "SCAN_LIMIT" => {
                    return Ok(Listing {
                        entries,
                        complete: false,
                    })
                }
                Err(e) => return Err(e),
            }
            if record.is_empty() {
                break;
            }
            let (header, name) = record
                .split_once('\t')
                .ok_or_else(|| Error::new("GIT_ERROR", "invalid tree record", 5))?;
            logical_path(name)?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 4 {
                return Err(Error::new("GIT_ERROR", "invalid tree record", 5));
            }
            check_mode(fields[0])?;
            let is_dir = fields[1] == "tree";
            if !is_dir && !name.ends_with(".md") {
                continue;
            }
            if !names.insert(name.to_lowercase()) {
                return Err(Error::new("CASE_COLLISION", "case-colliding Git paths", 2));
            }
            entries.push(Entry {
                path: if directory.is_empty() {
                    name.into()
                } else {
                    format!("{directory}/{name}")
                },
                directory: is_dir,
                bytes: fields[3].parse().unwrap_or(0),
                revision: fields[2].into(),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Listing {
            entries,
            complete: true,
        })
    }
    fn read(
        &self,
        b: &BundleBinding,
        path: &str,
        max: usize,
        c: &RequestControl,
    ) -> Result<String> {
        let (oid, kind, size) = Self::object(b, path, c)?
            .ok_or_else(|| Error::new("NOT_FOUND", "Git file missing", 3))?;
        if kind != "blob" {
            return Err(Error::new("NOT_A_FILE", "Git path is not a file", 3));
        }
        if size > max as u64 {
            return Err(Error::new(
                "DOCUMENT_TOO_LARGE",
                "Git blob exceeds read limit",
                7,
            ));
        }
        text(run(
            Path::new(&b.root),
            &["cat-file", "blob", &oid],
            max,
            c,
        )?)
    }
}

pub struct Readers;
impl Readers {
    fn reader(&self, b: &BundleBinding) -> Result<&dyn BundleReader> {
        match b.kind.as_str() {
            "fs" => Ok(&super::Filesystem),
            "git" => Ok(&Git),
            _ => Err(Error::new(
                "UNSUPPORTED_CAPABILITY",
                "unknown bundle adapter",
                2,
            )),
        }
    }
}
impl BundleReader for Readers {
    fn probe(&self, b: &BundleBinding) -> Result<()> {
        self.reader(b)?.probe(b)
    }
    fn probe_controlled(&self, b: &BundleBinding, c: &RequestControl) -> Result<()> {
        self.reader(b)?.probe_controlled(b, c)
    }
    fn stat(
        &self,
        b: &BundleBinding,
        p: &str,
        c: &RequestControl,
    ) -> Result<Option<ResourceMetadata>> {
        self.reader(b)?.stat(b, p, c)
    }
    fn list(&self, b: &BundleBinding, p: &str, s: &mut ScanBudget<'_>) -> Result<Listing> {
        self.reader(b)?.list(b, p, s)
    }
    fn read(&self, b: &BundleBinding, p: &str, m: usize, c: &RequestControl) -> Result<String> {
        self.reader(b)?.read(b, p, m, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_status_codes_must_not_match_repository_paths() {
        for status in [401, 403, 404] {
            let error = transport_error(&format!(
                "Unable to create '/cache/abc{status}def/shallow.lock': File exists"
            ));
            assert_eq!(error.code, "GIT_TRANSPORT_FAILED");
            let error = transport_error(&format!("The requested URL returned error: {status}"));
            assert_eq!(
                error.code,
                if status == 404 {
                    "GIT_REMOTE_NOT_FOUND"
                } else {
                    "GIT_AUTH_FAILED"
                }
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn stop_allows_lock_cleanup_and_forces_unresponsive_children() {
        use std::os::unix::process::CommandExt;
        use std::time::Instant;
        for cooperative in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let script = if cooperative {
                "trap 'rm -f shallow.lock; exit 0' TERM; touch shallow.lock ready; while :; do :; done"
            } else {
                "trap '' TERM; touch shallow.lock ready; while :; do :; done"
            };
            let mut child = Command::new("sh")
                .args(["-c", script])
                .current_dir(dir.path())
                .process_group(0)
                .spawn()
                .unwrap();
            let start = Instant::now();
            while !dir.path().join("ready").exists() && start.elapsed() < Duration::from_secs(5) {
                thread::sleep(Duration::from_millis(10));
            }
            let ready = dir.path().join("ready").exists();
            let start = Instant::now();
            stop(&mut child);
            assert!(ready);
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(child.try_wait().unwrap().is_some());
            assert_eq!(dir.path().join("shallow.lock").exists(), !cooperative);
        }
    }
}
