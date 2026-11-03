//! The file index: one SQLite file holding every indexed file and folder,
//! with a trigram full-text index over names for substring search.
//!
//! Each thread opens its own [`Index`] on the same file. SQLite's WAL mode
//! lets searches read while the crawler or the watcher writes.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// SQLite allows one writer at a time. Every write in this process takes
/// this lock first, so the crawler, the watcher and settings never compete
/// for SQLite's own lock, which in WAL mode can fail with "database is
/// locked" rather than wait. busy_timeout covers other processes.
static WRITER: Mutex<()> = Mutex::new(());

pub(crate) fn writer() -> MutexGuard<'static, ()> {
    WRITER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One file or folder as stored.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileRecord {
    pub path: String,
    pub root: String,
    pub name: String,
    pub ext: String,
    pub is_dir: bool,
    pub size: i64,
    pub modified: i64,
    pub inode: i64,
}

pub struct Index {
    pub(crate) conn: Connection,
}

/// The meta key marking a folder whose files are being removed, before its path.
const REMOVING: &str = "removing:";

/// A condition on files f: not under a folder being removed. Searches use
/// it, so a large folder's files stop being found as soon as it is removed,
/// not when the last batch goes. There are few marks, so it is cheap.
pub(crate) const NOT_REMOVING: &str =
    "f.root NOT IN (SELECT substr(key, 10) FROM meta WHERE key >= 'removing:' AND key < 'removing;')";

/// Version 0 of the schema; later versions are in migrate.rs.
pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    id       INTEGER PRIMARY KEY,
    path     TEXT NOT NULL UNIQUE,
    root     TEXT NOT NULL,
    name     TEXT NOT NULL,
    ext      TEXT NOT NULL,
    is_dir   INTEGER NOT NULL,
    size     INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    inode    INTEGER NOT NULL,
    seen     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_root ON files(root, seen);
CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
    name, content='files', content_rowid='id', tokenize='trigram case_sensitive 0'
);
CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, name) VALUES (new.id, new.name);
END;
CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, name) VALUES ('delete', old.id, old.name);
END;
CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE OF name ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, name) VALUES ('delete', old.id, old.name);
    INSERT INTO files_fts(rowid, name) VALUES (new.id, new.name);
END;
CREATE TABLE IF NOT EXISTS folders (
    path  TEXT PRIMARY KEY,
    added INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS ignore_patterns (
    pattern TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Index {
    /// Opens, creating if needed, the index at path.
    pub fn open(path: &Path) -> Result<Index> {
        let mut conn = Connection::open(path).with_context(|| format!("open index {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(60))?;
        let _write = writer();
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        super::migrate::migrate(&mut conn)?;
        Ok(Index { conn })
    }

    /// Inserts or replaces records, all in one transaction.
    pub fn upsert(&mut self, records: &[FileRecord], seen: i64) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode, seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(path) DO UPDATE SET root = excluded.root, name = excluded.name,
                     ext = excluded.ext, is_dir = excluded.is_dir, size = excluded.size,
                     modified = excluded.modified, inode = excluded.inode, seen = excluded.seen",
            )?;
            for r in records {
                stmt.execute(params![r.path, r.root, r.name, r.ext, r.is_dir, r.size, r.modified, r.inode, seen])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes path and, if it is a folder, everything under it.
    pub fn remove_tree(&mut self, path: &str) -> Result<usize> {
        let (low, high) = subtree_bounds(path);
        let _write = writer();
        let n = self.conn.execute(
            "DELETE FROM files WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
            params![path, low, high],
        )?;
        Ok(n)
    }

    /// Removes every row of root not seen by the scan numbered seen.
    pub fn remove_unseen(&mut self, root: &str, seen: i64) -> Result<usize> {
        let _write = writer();
        Ok(self.conn.execute("DELETE FROM files WHERE root = ?1 AND seen < ?2", params![root, seen])?)
    }

    /// Removes everything indexed under root.
    pub fn remove_root(&mut self, root: &str) -> Result<usize> {
        let _write = writer();
        Ok(self.conn.execute("DELETE FROM files WHERE root = ?1", params![root])?)
    }

    /// Forgets a folder the user removed: its row goes at once, and it is
    /// marked for removal of everything indexed under it, which
    /// remove_root_in_batches does. A mark left when SIFS stopped part way
    /// is in removals, so the next start can finish. A path that is not a
    /// folder (one taken into a folder added around it, say) is left alone:
    /// false.
    pub fn forget_folder(&mut self, path: &str) -> Result<bool> {
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = tx.execute("DELETE FROM folders WHERE path = ?1", params![path])? == 1;
        if removed {
            tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')", params![format!("{REMOVING}{path}")])?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Adds a folder, taking in the folders inside it, so folders never
    /// nest: their rows are forgotten and their files' rows become the new
    /// folder's, kept with what was read from them. Returns the folders taken in.
    pub fn add_folder_around(&mut self, path: &str, now: i64) -> Result<Vec<String>> {
        let (low, high) = subtree_bounds(path);
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inside: Vec<String> = {
            let mut stmt = tx.prepare("SELECT path FROM folders WHERE path >= ?1 AND path < ?2 ORDER BY path")?;
            let rows = stmt.query_map(params![low, high], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        for child in &inside {
            tx.execute("DELETE FROM folders WHERE path = ?1", params![child])?;
        }
        tx.execute("INSERT OR IGNORE INTO folders (path, added) VALUES (?1, ?2)", params![path, now])?;
        for child in &inside {
            give_to_folders(&tx, child, None)?;
        }
        tx.commit()?;
        Ok(inside)
    }

    /// Inserts or replaces records as upsert does, but each under the folder
    /// that contains it, read in the same transaction; a record no folder
    /// contains is skipped. For the watcher, whose list of folders may be
    /// older than a folder added around others or removed. Returns how many
    /// were written.
    pub fn upsert_into_folders(&mut self, records: &[FileRecord], seen: i64) -> Result<usize> {
        if records.is_empty() {
            return Ok(0);
        }
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut written = 0;
        {
            let mut stmt = tx.prepare_cached(&format!(
                "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode, seen)
                 SELECT ?1, owner, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM (SELECT {} AS owner) WHERE owner IS NOT NULL
                 ON CONFLICT(path) DO UPDATE SET root = excluded.root, name = excluded.name,
                     ext = excluded.ext, is_dir = excluded.is_dir, size = excluded.size,
                     modified = excluded.modified, inode = excluded.inode, seen = excluded.seen",
                folder_of("?1")
            ))?;
            for r in records {
                written += stmt.execute(params![r.path, r.name, r.ext, r.is_dir, r.size, r.modified, r.inode, seen])?;
            }
        }
        tx.commit()?;
        Ok(written)
    }

    /// Folders forgotten whose files are not all removed yet.
    pub fn removals(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT key FROM meta WHERE key >= ?1 AND key < ?2 ORDER BY key")?;
        // ';' follows ':' in byte order: every key starting "removing:".
        let rows = stmt.query_map(params![REMOVING, "removing;"], |r| r.get::<_, String>(0))?;
        let keys: Vec<String> = rows.collect::<std::result::Result<_, _>>()?;
        Ok(keys.into_iter().map(|k| k[REMOVING.len()..].to_string()).collect())
    }

    /// At startup, so every row has a folder and folders never nest: a
    /// folder inside another (both added in phase 1, which allowed that) is
    /// taken into the outer one, rows under a root that is not a folder but
    /// lies inside one become that folder's, and every other root that is
    /// not a folder is marked for removal (rows a scan or the watcher wrote
    /// after their folder was removed, or left by an older SIFS).
    pub fn tidy_roots(&mut self) -> Result<Tidy> {
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut tidy = Tidy::default();
        let folders: Vec<String> = {
            let mut stmt = tx.prepare("SELECT path FROM folders ORDER BY path")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        // In path order, a folder's outermost folder comes before it.
        let mut outer: Vec<String> = Vec::new();
        for folder in folders {
            if outer.iter().any(|o| inside(&folder, o)) {
                tx.execute("DELETE FROM folders WHERE path = ?1", params![folder])?;
                tidy.absorbed.push(folder);
            } else {
                outer.push(folder);
            }
        }
        // The folders taken in are roots that are not folders now, too.
        let roots: Vec<String> = {
            let mut stmt = tx.prepare("SELECT DISTINCT root FROM files")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        for root in roots.iter().filter(|r| !outer.contains(r)) {
            if give_to_folders(&tx, root, None)? > 0 {
                tidy.reassigned += 1;
            }
            let left: i64 = tx.query_row("SELECT COUNT(*) FROM files WHERE root = ?1", params![root], |r| r.get(0))?;
            if left > 0 {
                tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')", params![format!("{REMOVING}{root}")])?;
                tidy.orphans += 1;
            }
        }
        tx.commit()?;
        Ok(tidy)
    }

    /// Removes everything indexed under root, batch rows at a time, each in
    /// its own short transaction, so a large folder and the text read from
    /// its files don't hold up searches and other writes; then clears
    /// root's removal mark. A row that lies inside a folder (root itself,
    /// added again since, or a folder added around it or inside it) is
    /// given to that folder instead, as tidy_roots does, checked in the same
    /// transaction, so no add can slip in between. progress is called after
    /// each batch with the number removed so far.
    pub fn remove_root_in_batches(&mut self, root: &str, batch: usize, mut progress: impl FnMut(usize)) -> Result<usize> {
        let mut removed = 0;
        loop {
            let (given, deleted) = {
                let _write = writer();
                let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let given = give_to_folders(&tx, root, Some(batch))?;
                let deleted = tx.execute(
                    &format!(
                        "DELETE FROM files WHERE id IN
                         (SELECT g.id FROM files g WHERE g.root = ?1 AND {} IS NULL LIMIT ?2)",
                        folder_of("g.path")
                    ),
                    params![root, batch as i64],
                )?;
                tx.commit()?;
                (given, deleted)
            };
            if given + deleted == 0 {
                break;
            }
            removed += deleted;
            progress(removed);
        }
        let _write = writer();
        self.conn.execute("DELETE FROM meta WHERE key = ?1", params![format!("{REMOVING}{root}")])?;
        Ok(removed)
    }

    pub fn get(&self, path: &str) -> Result<Option<FileRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT path, root, name, ext, is_dir, size, modified, inode FROM files WHERE path = ?1",
                params![path],
                row_to_record,
            )
            .optional()?)
    }

    /// The connection, for measuring and inspecting the index; writes go
    /// through the methods here, which take the writer lock.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?)
    }

    pub fn count_root(&self, root: &str) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM files WHERE root = ?1", params![root], |r| r.get(0))?)
    }

    // --- settings ---

    pub fn folders(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM folders ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn add_folder(&mut self, path: &str, now: i64) -> Result<bool> {
        let _write = writer();
        Ok(self.conn.execute("INSERT OR IGNORE INTO folders (path, added) VALUES (?1, ?2)", params![path, now])? == 1)
    }

    pub fn remove_folder(&mut self, path: &str) -> Result<bool> {
        let _write = writer();
        Ok(self.conn.execute("DELETE FROM folders WHERE path = ?1", params![path])? == 1)
    }

    /// The ignore patterns, or None if the user never set them.
    pub fn ignore_patterns(&self) -> Result<Option<Vec<String>>> {
        let set: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'ignore_set'", [], |r| r.get(0))
            .optional()?;
        if set.is_none() {
            return Ok(None);
        }
        let mut stmt = self.conn.prepare("SELECT pattern FROM ignore_patterns ORDER BY pattern")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(Some(rows.collect::<std::result::Result<_, _>>()?))
    }

    pub fn set_ignore_patterns(&mut self, patterns: &[String]) -> Result<()> {
        let _write = writer();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM ignore_patterns", [])?;
        for p in patterns {
            tx.execute("INSERT OR IGNORE INTO ignore_patterns (pattern) VALUES (?1)", params![p])?;
        }
        tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('ignore_set', '1')", [])?;
        tx.commit()?;
        Ok(())
    }

    /// The number of the newest scan started, 0 before any.
    pub fn current_scan(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'scan'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn next_scan(&mut self) -> Result<i64> {
        let _write = writer();
        let current: i64 = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'scan'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let next = current + 1;
        self.conn
            .execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('scan', ?1)", params![next.to_string()])?;
        Ok(next)
    }
}

/// What tidy_roots did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tidy {
    /// Folders inside other folders, taken into them.
    pub absorbed: Vec<String>,
    /// Roots that are not folders but lie inside one, given to it.
    pub reassigned: usize,
    /// Roots marked for removal.
    pub orphans: usize,
}

/// Whether path lies strictly inside folder.
fn inside(path: &str, folder: &str) -> bool {
    path.starts_with(&format!("{}/", folder.trim_end_matches('/')))
}

/// SQL for the folder that contains the path expr (a column or a
/// parameter): the folder itself, or the one it lies inside; NULL if none.
/// Folders never nest, so there is at most one (the outermost, if they do).
/// Every write of a row's root that may race with folders changing reads
/// it from here, in its own transaction, never from a copy.
fn folder_of(expr: &str) -> String {
    format!(
        "(SELECT f.path FROM folders f WHERE {expr} = f.path
          OR ({expr} >= rtrim(f.path, '/') || '/' AND {expr} < rtrim(f.path, '/') || '0')
          ORDER BY length(f.path) LIMIT 1)"
    )
}

/// Gives the rows under root that lie inside a folder other than root to
/// that folder, at most limit of them. They are marked seen by the newest
/// scan, so a scan of that folder already under way doesn't remove them as
/// unseen: they may have been written after it walked their paths (a late
/// batch of another folder's scan). Its next scan drops any that are gone.
fn give_to_folders(conn: &Connection, root: &str, limit: Option<usize>) -> Result<usize> {
    let limit = limit.map_or(-1, |n| n as i64);
    Ok(conn.execute(
        &format!(
            "UPDATE files SET root = {owner},
                 seen = max(seen, coalesce((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'scan'), 0))
             WHERE id IN (SELECT g.id FROM files g WHERE g.root = ?1
                 AND {g_owner} IS NOT NULL AND {g_owner} != ?1 LIMIT ?2)",
            owner = folder_of("files.path"),
            g_owner = folder_of("g.path"),
        ),
        params![root, limit],
    )?)
}

pub(crate) fn row_to_record(r: &rusqlite::Row) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        path: r.get(0)?,
        root: r.get(1)?,
        name: r.get(2)?,
        ext: r.get(3)?,
        is_dir: r.get(4)?,
        size: r.get(5)?,
        modified: r.get(6)?,
        inode: r.get(7)?,
    })
}

/// The range of paths strictly under a folder: "/a/b/" up to, but not
/// including, "/a/b0" ('0' follows '/' in byte order). A range compare uses
/// the path index, and unlike LIKE it has no wildcard characters to escape.
fn subtree_bounds(path: &str) -> (String, String) {
    let base = path.trim_end_matches('/');
    (format!("{base}/"), format!("{base}0"))
}
