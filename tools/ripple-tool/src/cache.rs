use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub struct Cache {
    connection: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_cache (
                cache_key TEXT PRIMARY KEY,
                expires_at INTEGER NOT NULL,
                value_json TEXT NOT NULL
             );",
        )?;
        Ok(Self { connection })
    }

    pub fn get(&self, key: &str) -> anyhow::Result<Option<Value>> {
        let now = chrono::Utc::now().timestamp();
        let raw = self
            .connection
            .query_row(
                "SELECT value_json FROM tool_cache WHERE cache_key = ?1 AND expires_at > ?2",
                params![key, now],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        raw.map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn put(&self, key: &str, ttl_seconds: i64, value: &Value) -> anyhow::Result<()> {
        let expires_at = chrono::Utc::now().timestamp() + ttl_seconds;
        self.connection.execute(
            "INSERT INTO tool_cache(cache_key, expires_at, value_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(cache_key) DO UPDATE SET expires_at=excluded.expires_at, value_json=excluded.value_json",
            params![key, expires_at, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
}

pub fn cache_key(command: &str, input: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ripple-tool:v1\0");
    hasher.update(command.as_bytes());
    hasher.update(b"\0");
    hasher.update(serde_json::to_vec(input).expect("JSON values serialize"));
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn caches_json_and_keys_include_the_command() {
        let directory = tempdir().unwrap();
        let cache = Cache::open(&directory.path().join("cache.sqlite3")).unwrap();
        let input = json!({"query": "Ripple"});
        let search_key = cache_key("web-search", &input);
        assert_ne!(search_key, cache_key("web-fetch", &input));
        assert!(cache.get(&search_key).unwrap().is_none());
        cache.put(&search_key, 60, &json!({"ok": true})).unwrap();
        assert_eq!(cache.get(&search_key).unwrap().unwrap()["ok"], true);
    }
}
