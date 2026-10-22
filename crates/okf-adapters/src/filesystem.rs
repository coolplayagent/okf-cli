use okf_application::{BundleBinding, BundleReader, Entry, Listing, RequestControl, ScanBudget};
use okf_core::{logical_path, Error, Result};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub struct Filesystem;

impl Filesystem {
    fn root(binding: &BundleBinding) -> Result<PathBuf> {
        if binding.kind != "fs" {
            return Err(Error::new(
                "UNSUPPORTED_CAPABILITY",
                format!("{} adapter is not implemented yet", binding.kind),
                2,
            ));
        }
        let root = fs::canonicalize(&binding.root).map_err(map_io)?;
        if !root.is_dir() {
            return Err(Error::new(
                "NOT_A_DIRECTORY",
                "bundle root is not a directory",
                2,
            ));
        }
        Ok(root)
    }

    fn contained(root: &Path, logical: &str) -> Result<PathBuf> {
        logical_path(logical)?;
        let actual = fs::canonicalize(root.join(logical)).map_err(map_io)?;
        if !actual.starts_with(root) {
            return Err(Error::new(
                "PATH_ESCAPE",
                "resolved path escapes bundle root",
                4,
            ));
        }
        Ok(actual)
    }
}

impl BundleReader for Filesystem {
    fn probe(&self, binding: &BundleBinding) -> Result<()> {
        Self::root(binding).map(|_| ())
    }

    fn list(
        &self,
        binding: &BundleBinding,
        directory: &str,
        budget: &mut ScanBudget<'_>,
    ) -> Result<Listing> {
        budget.charge()?;
        let root = Self::root(binding)?;
        let folder = Self::contained(&root, directory)?;
        let mut entries = vec![];
        let mut names = BTreeMap::new();
        let mut iterator = fs::read_dir(&folder).map_err(map_io)?;
        let complete = loop {
            match budget.charge() {
                Ok(()) => {}
                Err(error) if error.code == "SCAN_LIMIT" => break false,
                Err(error) => return Err(error),
            }
            let Some(item) = iterator.next() else {
                break true;
            };
            let item = item.map_err(map_io)?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| Error::new("INVALID_FILENAME", "filename must be UTF-8", 2))?;
            let path = if directory.is_empty() {
                name.clone()
            } else {
                format!("{directory}/{name}")
            };
            logical_path(&path)?;
            // Do not follow directory links. Even internal links could create traversal cycles.
            let link_meta = fs::symlink_metadata(item.path()).map_err(map_io)?;
            let actual = Self::contained(&root, &path)?;
            let meta = fs::metadata(&actual).map_err(map_io)?;
            if meta.is_dir() && (link_meta.file_type().is_symlink() || actual != folder.join(&name))
            {
                return Err(Error::new(
                    "DIRECTORY_LINK",
                    "directory links and junctions are not traversed",
                    4,
                ));
            }
            if !meta.is_dir() && !name.ends_with(".md") {
                continue;
            }
            if let Some(previous) = names.insert(name.to_lowercase(), name.clone()) {
                return Err(Error::new(
                    "CASE_COLLISION",
                    format!("case-colliding paths: {previous}, {name}"),
                    2,
                ));
            }
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_nanos())
                .unwrap_or(0);
            entries.push(Entry {
                path,
                directory: meta.is_dir(),
                bytes: meta.len(),
                revision: format!("{}:{modified}", meta.len()),
            });
        };
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        budget.control.check()?;
        Ok(Listing { entries, complete })
    }

    fn read(
        &self,
        binding: &BundleBinding,
        path: &str,
        max_bytes: usize,
        control: &RequestControl,
    ) -> Result<String> {
        control.check()?;
        let root = Self::root(binding)?;
        let actual = Self::contained(&root, path)?;
        let mut file = fs::File::open(&actual).map_err(map_io)?;
        let before = file.metadata().map_err(map_io)?;
        if !before.is_file() {
            return Err(Error::new(
                "NOT_A_FILE",
                "requested path is not a regular file",
                3,
            ));
        }
        if before.len() > max_bytes as u64 {
            return Err(Error::new(
                "DOCUMENT_TOO_LARGE",
                "document exceeds the 2 MiB read limit",
                7,
            ));
        }
        let mut bytes = vec![];
        let mut chunk = [0_u8; 8192];
        while bytes.len() <= max_bytes {
            control.check()?;
            let length = chunk.len().min(max_bytes + 1 - bytes.len());
            let count = file.read(&mut chunk[..length]).map_err(map_io)?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        control.check()?;
        let after = file.metadata().map_err(map_io)?;
        if bytes.len() > max_bytes {
            return Err(Error::new(
                "DOCUMENT_TOO_LARGE",
                "document exceeds read limit",
                7,
            ));
        }
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(Error::new(
                "REVISION_CHANGED",
                "file changed during read",
                6,
            ));
        }
        String::from_utf8(bytes).map_err(|_| Error::new("INVALID_OKF", "document is not UTF-8", 1))
    }
}

fn map_io(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::NotFound => Error::new("NOT_FOUND", e.to_string(), 3),
        std::io::ErrorKind::PermissionDenied => Error::new("ACCESS_DENIED", e.to_string(), 4),
        _ => Error::io(e),
    }
}
