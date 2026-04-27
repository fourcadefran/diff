use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecentRepo {
    pub host: String,
    pub path: String,
    pub last_opened_at: i64, // unix epoch seconds
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    /// Open `~/Library/Application Support/diff/state.db` (macOS) or the
    /// equivalent on other platforms. Creates the file and schema if missing.
    pub fn open_default() -> Result<Self, String> {
        let path = default_db_path()?;
        Self::open_at(&path)
    }

    pub fn open_at(path: &std::path::Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create state dir: {e}"))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS repos (
                host TEXT NOT NULL,
                path TEXT NOT NULL,
                last_opened_at INTEGER NOT NULL,
                PRIMARY KEY (host, path)
            );
            "#,
        )
        .map_err(|e| format!("create schema: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list_for_host(&self, host: &str) -> Result<Vec<RecentRepo>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT host, path, last_opened_at FROM repos WHERE host = ?1 ORDER BY last_opened_at DESC, rowid DESC")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![host], |row| {
                Ok(RecentRepo {
                    host: row.get(0)?,
                    path: row.get(1)?,
                    last_opened_at: row.get(2)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    pub fn list_all_recent(&self, limit: usize) -> Result<Vec<RecentRepo>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT host, path, last_opened_at FROM repos ORDER BY last_opened_at DESC, rowid DESC LIMIT ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(RecentRepo {
                    host: row.get(0)?,
                    path: row.get(1)?,
                    last_opened_at: row.get(2)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    pub fn record_open(&self, host: &str, path: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("clock: {e}"))?
            .as_secs() as i64;
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        conn.execute(
            r#"INSERT INTO repos (host, path, last_opened_at) VALUES (?1, ?2, ?3)
               ON CONFLICT (host, path) DO UPDATE SET last_opened_at = excluded.last_opened_at"#,
            params![host, path, now],
        )
        .map_err(|e| format!("insert: {e}"))?;
        Ok(())
    }
}

fn default_db_path() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or_else(|| "could not resolve data dir".to_string())?;
    Ok(base.join("diff").join("state.db"))
}
