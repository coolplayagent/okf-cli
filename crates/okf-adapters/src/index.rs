use okf_application::{Index, IndexedDocument, Snapshot, MAX_INDEX_BYTES};
use okf_core::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};

pub struct SqliteIndex(Connection);
impl SqliteIndex {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::io)?;
        }
        let connection = Connection::open(path).map_err(Error::io)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(Error::io)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(Error::io)?;
        if version != 0 && version != 1 {
            return Err(Error::new(
                "INDEX_VERSION",
                "unsupported content index schema",
                2,
            ));
        }
        connection.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS snapshots (bundle TEXT PRIMARY KEY, generation TEXT NOT NULL, revision TEXT, checked_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS documents (bundle TEXT NOT NULL, path TEXT NOT NULL, hash TEXT NOT NULL, raw TEXT NOT NULL, generation TEXT NOT NULL, PRIMARY KEY(bundle,path));
            PRAGMA user_version=1;").map_err(Error::io)?;
        Ok(Self(connection))
    }
}
impl Index for SqliteIndex {
    fn load(&self, key: &str) -> Result<Option<Snapshot>> {
        let tx = self.0.unchecked_transaction().map_err(Error::io)?;
        let header: Option<(String, Option<String>, String)> = tx
            .query_row(
                "SELECT generation,revision,checked_at FROM snapshots WHERE bundle=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(Error::io)?;
        let Some((generation, upstream_revision, checked_at)) = header else {
            return Ok(None);
        };
        let (bytes,count):(i64,i64)=tx.query_row("SELECT COALESCE(SUM(length(CAST(raw AS BLOB))),0),COUNT(*) FROM documents WHERE bundle=?1",[key],|r|Ok((r.get(0)?,r.get(1)?))).map_err(Error::io)?;
        if bytes > MAX_INDEX_BYTES as i64 || count > 10000 {
            return Err(Error::new(
                "INDEX_LIMIT",
                "stored index exceeds load budget",
                7,
            ));
        }
        let mut statement = tx
            .prepare("SELECT path,hash,raw FROM documents WHERE bundle=?1 ORDER BY path")
            .map_err(Error::io)?;
        let documents = statement
            .query_map([key], |r| {
                Ok(IndexedDocument {
                    path: r.get(0)?,
                    hash: r.get(1)?,
                    raw: r.get(2)?,
                })
            })
            .map_err(Error::io)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::io)?;
        for doc in &documents {
            okf_core::logical_path(&doc.path)?;
            if okf_core::sha256(doc.raw.as_bytes()) != doc.hash {
                return Err(Error::new(
                    "INDEX_CORRUPT",
                    "stored content hash mismatch; rebuild index",
                    5,
                ));
            }
        }
        Ok(Some(Snapshot {
            generation,
            upstream_revision,
            checked_at,
            documents,
        }))
    }
    fn publish(&self, key: &str, expected: Option<&str>, snapshot: &Snapshot) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.0, TransactionBehavior::Immediate)
            .map_err(Error::io)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT generation FROM snapshots WHERE bundle=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Error::io)?;
        if current.as_deref() != expected {
            return Err(Error::new(
                "INDEX_CHANGED",
                "index refreshed concurrently; retry sync",
                6,
            ));
        }
        for doc in &snapshot.documents {
            tx.execute("INSERT INTO documents(bundle,path,hash,raw,generation) VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(bundle,path) DO UPDATE SET raw=CASE WHEN documents.hash=excluded.hash THEN documents.raw ELSE excluded.raw END,hash=excluded.hash,generation=excluded.generation",
                params![key,doc.path,doc.hash,doc.raw,snapshot.generation]).map_err(Error::io)?;
        }
        tx.execute(
            "DELETE FROM documents WHERE bundle=?1 AND generation<>?2",
            params![key, snapshot.generation],
        )
        .map_err(Error::io)?;
        tx.execute("INSERT INTO snapshots(bundle,generation,revision,checked_at) VALUES(?1,?2,?3,?4)
            ON CONFLICT(bundle) DO UPDATE SET generation=excluded.generation,revision=excluded.revision,checked_at=excluded.checked_at",
            params![key,snapshot.generation,snapshot.upstream_revision,snapshot.checked_at]).map_err(Error::io)?;
        tx.commit().map_err(Error::io)
    }
    fn clear(&self, key: &str) -> Result<()> {
        let tx = self.0.unchecked_transaction().map_err(Error::io)?;
        tx.execute("DELETE FROM documents WHERE bundle=?1", [key])
            .map_err(Error::io)?;
        tx.execute("DELETE FROM snapshots WHERE bundle=?1", [key])
            .map_err(Error::io)?;
        tx.commit().map_err(Error::io)
    }
}
