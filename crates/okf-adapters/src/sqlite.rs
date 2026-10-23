use okf_application::Cache;
use okf_core::{Card, Error, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};

pub struct SqliteCache {
    connection: Connection,
    // Fields drop in declaration order: close SQLite before releasing the lock.
    _guard: fs::File,
}
impl SqliteCache {
    pub fn open(path: &Path) -> Result<Self> {
        let guard = Self::guard(path)?;
        guard
            .try_lock_shared()
            .map_err(|e| Error::new("CACHE_LOCKED", e.to_string(), 5))?;
        Ok(Self {
            connection: Self::connect(path)?,
            _guard: guard,
        })
    }

    fn guard(path: &Path) -> Result<fs::File> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(Error::io)?;
        }
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("guard"))
            .map_err(Error::io)
    }

    /// Archive the cache and SQLite sidecars under an exclusive process lock.
    /// Bindings and knowledge files are never touched.
    pub fn reset(path: &Path) -> Result<Value> {
        let guard = Self::guard(path)?;
        guard
            .try_lock()
            .map_err(|e| Error::new("CACHE_LOCKED", e.to_string(), 5))?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("INVALID_CONFIG", "cache needs a parent", 2))?;
        let archive = tempfile::Builder::new()
            .prefix("cache-backup-")
            .tempdir_in(parent)
            .map_err(Error::io)?
            .keep();
        let mut moved = vec![];
        for source in [
            path.to_path_buf(),
            path.with_file_name(format!(
                "{}-wal",
                path.file_name().unwrap().to_string_lossy()
            )),
            path.with_file_name(format!(
                "{}-shm",
                path.file_name().unwrap().to_string_lossy()
            )),
        ] {
            if source.try_exists().map_err(Error::io)? {
                let destination = archive.join(source.file_name().unwrap());
                if let Err(error) = fs::rename(&source, &destination) {
                    for (old, new) in moved.iter().rev() {
                        let _ = fs::rename(new, old);
                    }
                    return Err(Error::new(
                        "CACHE_REPAIR_FAILED",
                        format!("{error}; recovery files: {}", archive.display()),
                        5,
                    ));
                }
                moved.push((source, destination));
            }
        }
        let connection = Self::connect(path).map_err(|e| {
            Error::new(
                "CACHE_REPAIR_FAILED",
                format!(
                    "{}; original cache archived at {}",
                    e.message,
                    archive.display()
                ),
                5,
            )
        })?;
        drop(connection);
        drop(guard);
        Ok(json!({"archive":archive,"cached_cards":0,"metadata_coverage":"unknown"}))
    }

    fn connect(path: &Path) -> Result<Connection> {
        let conn = Connection::open(path).map_err(Error::io)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(Error::io)?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(Error::io)?;
        if version != 0 && version != 2 {
            return Err(Error::new(
                "CACHE_VERSION",
                "unsupported cache schema; choose a fresh runtime directory",
                2,
            ));
        }
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS cards (
                bundle_id TEXT NOT NULL, path TEXT NOT NULL, hash TEXT NOT NULL,
                card_json TEXT NOT NULL, checked_at TEXT NOT NULL,
                PRIMARY KEY(bundle_id, path)
            );
            PRAGMA user_version=2;",
        )
        .map_err(Error::io)?;
        Ok(conn)
    }
}
impl Cache for SqliteCache {
    fn remember(
        &self,
        bundle_id: &str,
        path: &str,
        hash: &str,
        card: &Card,
        checked_at: &str,
    ) -> Result<()> {
        let text = serde_json::to_string(card).map_err(Error::io)?;
        self.connection.execute("INSERT INTO cards(bundle_id,path,hash,card_json,checked_at) VALUES(?1,?2,?3,?4,?5)
            ON CONFLICT(bundle_id,path) DO UPDATE SET hash=excluded.hash,card_json=excluded.card_json,checked_at=excluded.checked_at",
            params![bundle_id, path, hash, text, checked_at]).map_err(Error::io)?;
        Ok(())
    }
    fn status(&self, bundle_id: &str) -> Result<Value> {
        let (count, checked): (i64, Option<String>) = self
            .connection
            .query_row(
                "SELECT COUNT(*),MAX(checked_at) FROM cards WHERE bundle_id=?1",
                [bundle_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(Error::io)?;
        Ok(json!({"cached_cards": count, "last_observed_at": checked,
            "metadata_coverage": "unknown", "body_coverage": "none",
            "note": "Observed cards are historical metadata, not proof of a complete or current index"}))
    }
}
