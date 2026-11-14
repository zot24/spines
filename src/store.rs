use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::domain::{Item, PluginId};
use crate::error::{Error, Result};

pub struct PartRow {
    pub source_key: String,
    pub item_id: String,
    pub mtime_ns: i64,
    pub size: u64,
    pub content_hash: String,
    pub body: String,
}

pub struct Store {
    conn: Mutex<Connection>,
    data_dir: PathBuf,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        fs::create_dir_all(data_dir.join("covers"))?;
        fs::create_dir_all(data_dir.join("spines"))?;
        fs::create_dir_all(data_dir.join("cache"))?;
        let conn = Connection::open(data_dir.join("spines.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS parts (
              plugin TEXT NOT NULL,
              source_key TEXT NOT NULL,
              item_id TEXT NOT NULL,
              mtime_ns INTEGER NOT NULL,
              size INTEGER NOT NULL,
              content_hash TEXT NOT NULL,
              body TEXT NOT NULL,
              PRIMARY KEY (plugin, source_key, item_id)
            );
            CREATE TABLE IF NOT EXISTS items (
              id TEXT PRIMARY KEY,
              plugin TEXT NOT NULL,
              sort_key TEXT NOT NULL,
              body TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS stamps (
              plugin TEXT PRIMARY KEY,
              stamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS items_plugin_sort ON items (plugin, sort_key);
            ",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            data_dir: data_dir.to_path_buf(),
        })
    }

    /// Read a catalog another process already created. Does not scan and does not write.
    pub fn open_readonly(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("spines.db");
        if !path.is_file() {
            return Err(Error::msg(format!("no catalog in {}", data_dir.display())));
        }
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _ = conn.pragma_update(None, "query_only", true);
        Ok(Self {
            conn: Mutex::new(conn),
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn cover_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("covers").join(format!("{id}.jpg"))
    }

    pub fn spine_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("spines").join(format!("{id}.png"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn stamp(&self, plugin: PluginId) -> Result<Option<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT stamp FROM stamps WHERE plugin = ?1")?;
        let mut rows = stmt.query([plugin.as_str()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn parts(&self, plugin: PluginId) -> Result<Vec<PartRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT source_key, item_id, mtime_ns, size, content_hash, body FROM parts WHERE plugin = ?1",
        )?;
        let rows = stmt.query_map([plugin.as_str()], |row| {
            Ok(PartRow {
                source_key: row.get(0)?,
                item_id: row.get(1)?,
                mtime_ns: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                content_hash: row.get(4)?,
                body: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn content_hash(&self, plugin: PluginId, source_key: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT content_hash FROM parts WHERE plugin = ?1 AND source_key = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![plugin.as_str(), source_key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn item(&self, id: &str) -> Result<Option<Item>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT body FROM items WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        if let Some(row) = rows.next()? {
            let body: String = row.get(0)?;
            Ok(Some(serde_json::from_str(&body)?))
        } else {
            Ok(None)
        }
    }

    pub fn items(&self, plugin: Option<PluginId>, words: &[String]) -> Result<Vec<Item>> {
        let conn = self.lock();
        let mut out = Vec::new();
        if let Some(plugin) = plugin {
            let mut stmt =
                conn.prepare("SELECT body FROM items WHERE plugin = ?1 ORDER BY sort_key")?;
            let rows = stmt.query_map([plugin.as_str()], |row| row.get::<_, String>(0))?;
            for body in rows {
                out.push(serde_json::from_str(&body?)?);
            }
        } else {
            let mut stmt = conn.prepare("SELECT body FROM items ORDER BY plugin, sort_key")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for body in rows {
                out.push(serde_json::from_str(&body?)?);
            }
        }
        if words.is_empty() {
            return Ok(out);
        }
        Ok(out
            .into_iter()
            .filter(|item| {
                words.iter().all(|w| {
                    let word = w.to_lowercase();
                    !word.is_empty() && item.haystack.contains(&word)
                })
            })
            .collect())
    }

    /// Replace this plugin's ledger and stamp, upsert changed items, drop items that no longer have parts.
    pub fn commit_plugin(
        &self,
        plugin: PluginId,
        stamp: &str,
        parts: &[NewPart],
        upserts: &[Item],
        remove_files_for: &[String],
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            conn.execute("DELETE FROM parts WHERE plugin = ?1", [plugin.as_str()])?;
            for part in parts {
                conn.execute(
                    "INSERT INTO parts (plugin, source_key, item_id, mtime_ns, size, content_hash, body)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        plugin.as_str(),
                        part.source_key,
                        part.item_id,
                        part.mtime_ns,
                        part.size as i64,
                        part.content_hash,
                        part.body,
                    ],
                )?;
            }
            for item in upserts {
                let body = serde_json::to_string(item)?;
                conn.execute(
                    "INSERT INTO items (id, plugin, sort_key, body) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(id) DO UPDATE SET plugin = excluded.plugin, sort_key = excluded.sort_key, body = excluded.body",
                    params![item.id, plugin.as_str(), item.sort_key, body],
                )?;
            }
            conn.execute(
                "DELETE FROM items WHERE plugin = ?1 AND id NOT IN (SELECT DISTINCT item_id FROM parts WHERE plugin = ?1)",
                [plugin.as_str()],
            )?;
            conn.execute(
                "INSERT INTO stamps (plugin, stamp) VALUES (?1, ?2)
                 ON CONFLICT(plugin) DO UPDATE SET stamp = excluded.stamp",
                params![plugin.as_str(), stamp],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute("COMMIT", [])?;
                for id in remove_files_for {
                    let _ = fs::remove_file(self.cover_path(id));
                    let _ = fs::remove_file(self.spine_path(id));
                }
                Ok(())
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", []);
                Err(err)
            }
        }
    }
}

pub struct NewPart {
    pub source_key: String,
    pub item_id: String,
    pub mtime_ns: i64,
    pub size: u64,
    pub content_hash: String,
    pub body: String,
}
