use okf_application::Cache;
use okf_core::{Card, Error, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

pub struct SqliteCache(Connection);
impl SqliteCache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::io)?;
        }
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
        Ok(Self(conn))
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
        self.0.execute("INSERT INTO cards(bundle_id,path,hash,card_json,checked_at) VALUES(?1,?2,?3,?4,?5)
            ON CONFLICT(bundle_id,path) DO UPDATE SET hash=excluded.hash,card_json=excluded.card_json,checked_at=excluded.checked_at",
            params![bundle_id, path, hash, text, checked_at]).map_err(Error::io)?;
        Ok(())
    }
    fn status(&self, bundle_id: &str) -> Result<Value> {
        let (count, checked): (i64, Option<String>) = self
            .0
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
