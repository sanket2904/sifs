//! What reading inside files keeps in the index: which extensions have a
//! reader, the queue, and what was read. Every write takes the index's
//! writer lock, as the crawler and the watcher do.

use super::{platform, Part, Registry};
use crate::index::db::{writer, Index, NOT_REMOVING};
use anyhow::Result;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::time::{Duration, Instant};

/// Reconcile looks at this many file ids per transaction, so it never
/// holds the writer lock for long.
const RECONCILE_CHUNK: i64 = 50_000;
/// Priority of a file opened in SIFS: ahead of everything.
const OPENED: i64 = -1;

/// A file claimed from the queue, as the queue had it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub file_id: i64,
    pub path: String,
    pub ext: String,
    pub modified: i64,
    pub size: i64,
    /// Including this claim.
    pub attempts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Done,
    Empty,
    Failed,
    Skipped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Done => "done",
            Status::Empty => "empty",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
        }
    }
}

/// What reading one file came to, with the file's time and size as read.
#[derive(Debug, Clone)]
pub struct Extract {
    pub reader: &'static str,
    pub version: u32,
    pub modified: i64,
    pub size: i64,
    pub status: Status,
    pub error: Option<String>,
    pub millis: i64,
    pub parts: Vec<Part>,
    pub facts: Vec<(String, String)>,
}

impl Extract {
    pub fn chars(&self) -> usize {
        self.parts.iter().map(|p| p.text.chars().count()).sum()
    }

    pub fn text_bytes(&self) -> usize {
        self.parts.iter().map(|p| p.text.len()).sum()
    }
}

/// How a claimed job ended.
#[derive(Debug, Clone)]
pub enum Finished {
    /// Nothing to write: the file is gone, has no reader, or what was read
    /// from it is current. It leaves the queue.
    Drop(Job),
    /// Not read after all (it changed while it was read, or the worker
    /// stopped first): it stays queued, and the claim doesn't count.
    Unclaim(Job),
    /// The path is no longer something its reader reads (a file replaced
    /// by a link): what was read from it goes, and it leaves the queue.
    Clear(Job),
    Read(Job, Extract),
}

/// Counts for settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub read: i64,
    pub empty: i64,
    pub failed: i64,
    pub skipped: i64,
    pub waiting: i64,
}

/// A file that couldn't be read, or was skipped, and why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Failure {
    pub path: String,
    pub status: String,
    pub error: String,
}

/// Where a word was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMatch {
    pub path: String,
    pub part: u32,
    pub label: String,
}

/// Writes the registry's extensions into extract_kinds, which the queue's
/// triggers read. Run at startup, before anything is scanned.
pub fn register_kinds(index: &mut Index, registry: &Registry) -> Result<()> {
    let _write = writer();
    let tx = index.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM extract_kinds", [])?;
    for (ext, reader) in registry.kinds() {
        tx.execute(
            "INSERT INTO extract_kinds (ext, reader, version, priority, packages) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ext, reader.id(), reader.version(), reader.class().priority(), reader.packages()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Whether the readers changed since the files indexed were last checked
/// against them (or never were: a new or just migrated index).
pub fn needs_reconcile(index: &Index, registry: &Registry) -> Result<bool> {
    let stored: Option<String> =
        index.conn.query_row("SELECT value FROM meta WHERE key = 'extract_signature'", [], |r| r.get(0)).optional()?;
    Ok(stored.as_deref() != Some(registry.signature().as_str()))
}

/// Queues every file whose reading is missing or out of date: never read,
/// changed since, or read by another reader or an older version of it. A
/// file that failed or was skipped, and is as it was then, is not queued
/// again. Then records the registry's signature. Returns how many it queued.
pub fn reconcile(index: &mut Index, registry: &Registry, now: i64) -> Result<usize> {
    let max_id: i64 = index.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM files", [], |r| r.get(0))?;
    let mut queued = 0;
    let mut lo = 0;
    while lo < max_id {
        let hi = lo + RECONCILE_CHUNK;
        let _write = writer();
        queued += index.conn.execute(
            "INSERT OR IGNORE INTO extract_queue (file_id, priority, modified, size, attempts)
             SELECT f.id, CASE WHEN f.modified >= ?1 - 604800 THEN 0 ELSE k.priority END, f.modified, f.size, 0
             FROM files f JOIN extract_kinds k ON k.ext = f.ext LEFT JOIN extracts e ON e.file_id = f.id
             WHERE f.id > ?2 AND f.id <= ?3 AND (f.is_dir = 0 OR k.packages = 1)
               AND (e.file_id IS NULL OR e.modified != f.modified OR e.size != f.size
                    OR e.reader != k.reader OR e.version < k.version)",
            params![now, lo, hi],
        )?;
        lo = hi;
    }
    let _write = writer();
    index.conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('extract_signature', ?1)",
        params![registry.signature()],
    )?;
    Ok(queued)
}

/// Queues again the files skipped because they were in iCloud and not
/// downloaded: downloading one changes neither its time nor its size, so
/// nothing else would. Run at startup.
pub fn requeue_not_downloaded(index: &mut Index) -> Result<usize> {
    let _write = writer();
    let tx = index.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let n = tx.execute(
        "INSERT OR IGNORE INTO extract_queue (file_id, priority, modified, size, attempts)
         SELECT f.id, k.priority, f.modified, f.size, 0
         FROM extracts e JOIN files f ON f.id = e.file_id JOIN extract_kinds k ON k.ext = f.ext
         WHERE e.status = 'skipped' AND e.error = ?1",
        params![platform::NOT_DOWNLOADED],
    )?;
    tx.execute("DELETE FROM extracts WHERE status = 'skipped' AND error = ?1", params![platform::NOT_DOWNLOADED])?;
    tx.commit()?;
    Ok(n)
}

/// Claims up to limit files from the front of the queue, counting an
/// attempt on each before anything is read, and commits that. While any
/// file has an attempt that never finished (SIFS stopped while reading it),
/// those come first and one at a time, so a file that stops SIFS is known
/// to be the one.
pub fn claim(index: &mut Index, limit: usize) -> Result<Vec<Job>> {
    let _write = writer();
    let tx = index.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let select = "SELECT q.file_id, f.path, f.ext, q.modified, q.size, q.attempts + 1
                  FROM extract_queue q JOIN files f ON f.id = q.file_id";
    let to_job = |r: &rusqlite::Row| {
        Ok(Job {
            file_id: r.get(0)?,
            path: r.get(1)?,
            ext: r.get(2)?,
            modified: r.get(3)?,
            size: r.get(4)?,
            attempts: r.get(5)?,
        })
    };
    let mut jobs: Vec<Job> = {
        let mut stmt = tx.prepare_cached(&format!("{select} WHERE q.attempts > 0 LIMIT 1"))?;
        let rows = stmt.query_map([], to_job)?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    if jobs.is_empty() {
        let mut stmt = tx.prepare_cached(&format!("{select} ORDER BY q.priority, q.modified DESC LIMIT {limit}"))?;
        let rows = stmt.query_map([], to_job)?;
        jobs = rows.collect::<std::result::Result<_, _>>()?;
    }
    {
        let mut stmt = tx.prepare_cached("UPDATE extract_queue SET attempts = attempts + 1 WHERE file_id = ?1")?;
        for job in &jobs {
            stmt.execute(params![job.file_id])?;
        }
    }
    tx.commit()?;
    Ok(jobs)
}

/// Writes what the worker finished, in one transaction. Returns how long it
/// held the writer lock.
pub fn write_results(index: &mut Index, results: &[Finished], now: i64) -> Result<Duration> {
    if results.is_empty() {
        return Ok(Duration::ZERO);
    }
    let _write = writer();
    let held = Instant::now();
    let tx = index.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        // Only the queue row as claimed: if the file changed meanwhile, the
        // trigger has replaced it with the new time and size, and it stays.
        let mut dequeue =
            tx.prepare_cached("DELETE FROM extract_queue WHERE file_id = ?1 AND modified = ?2 AND size = ?3")?;
        let mut unclaim = tx.prepare_cached(
            "UPDATE extract_queue SET attempts = MAX(attempts - 1, 0) WHERE file_id = ?1 AND modified = ?2 AND size = ?3",
        )?;
        let mut path_of = tx.prepare_cached("SELECT path FROM files WHERE id = ?1")?;
        let mut clear_contents = tx.prepare_cached("DELETE FROM contents WHERE file_id = ?1")?;
        let mut clear_facts = tx.prepare_cached("DELETE FROM facts WHERE file_id = ?1")?;
        let mut clear_extract = tx.prepare_cached("DELETE FROM extracts WHERE file_id = ?1")?;
        let mut add_part =
            tx.prepare_cached("INSERT INTO contents (file_id, part, label, text) VALUES (?1, ?2, ?3, ?4)")?;
        let mut add_fact =
            tx.prepare_cached("INSERT OR IGNORE INTO facts (file_id, key, value) VALUES (?1, ?2, ?3)")?;
        let mut record = tx.prepare_cached(
            "INSERT OR REPLACE INTO extracts (file_id, reader, version, modified, size, status, error, millis, chars, read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for result in results {
            let job = match result {
                Finished::Drop(job) | Finished::Unclaim(job) | Finished::Clear(job) | Finished::Read(job, _) => job,
            };
            // Ids are reused after a delete (a rename is a delete and an
            // insert): the row must still be the file that was claimed, or
            // what is written, or the queue row removed, is another file's.
            let path: Option<String> = path_of.query_row(params![job.file_id], |r| r.get(0)).optional()?;
            if path.as_deref() != Some(job.path.as_str()) {
                continue;
            }
            match result {
                Finished::Drop(job) => {
                    dequeue.execute(params![job.file_id, job.modified, job.size])?;
                }
                Finished::Unclaim(job) => {
                    unclaim.execute(params![job.file_id, job.modified, job.size])?;
                }
                Finished::Clear(job) => {
                    clear_contents.execute(params![job.file_id])?;
                    clear_facts.execute(params![job.file_id])?;
                    clear_extract.execute(params![job.file_id])?;
                    dequeue.execute(params![job.file_id, job.modified, job.size])?;
                }
                Finished::Read(job, extract) => {
                    clear_contents.execute(params![job.file_id])?;
                    clear_facts.execute(params![job.file_id])?;
                    for part in &extract.parts {
                        add_part.execute(params![job.file_id, part.part, part.label, part.text])?;
                    }
                    for (key, value) in &extract.facts {
                        add_fact.execute(params![job.file_id, key, value])?;
                    }
                    record.execute(params![
                        job.file_id,
                        extract.reader,
                        extract.version,
                        extract.modified,
                        extract.size,
                        extract.status.as_str(),
                        extract.error,
                        extract.millis,
                        extract.chars() as i64,
                        now
                    ])?;
                    dequeue.execute(params![job.file_id, job.modified, job.size])?;
                }
            }
        }
    }
    tx.commit()?;
    Ok(held.elapsed())
}

pub fn progress(index: &Index) -> Result<Progress> {
    let mut progress = Progress::default();
    let mut stmt = index.conn.prepare_cached("SELECT status, COUNT(*) FROM extracts GROUP BY status")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let n: i64 = row.get(1)?;
        match row.get_ref(0)?.as_str()? {
            "done" => progress.read = n,
            "empty" => progress.empty = n,
            "failed" => progress.failed = n,
            "skipped" => progress.skipped = n,
            _ => {}
        }
    }
    progress.waiting = index.conn.query_row("SELECT COUNT(*) FROM extract_queue", [], |r| r.get(0))?;
    Ok(progress)
}

/// A file opened in SIFS is read next, if it is waiting.
pub fn bump_opened(index: &mut Index, path: &str) -> Result<bool> {
    let _write = writer();
    let n = index.conn.execute(
        "UPDATE extract_queue SET priority = ?2 WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
        params![path, OPENED],
    )?;
    Ok(n > 0)
}

/// The files that couldn't be read, then those skipped, newest first.
pub fn failures(index: &Index, limit: usize) -> Result<Vec<Failure>> {
    let mut stmt = index.conn.prepare_cached(
        "SELECT f.path, e.status, COALESCE(e.error, '') FROM extracts e JOIN files f ON f.id = e.file_id
         WHERE e.status IN ('failed', 'skipped') ORDER BY e.status = 'skipped', e.read_at DESC, f.path LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], |r| Ok(Failure { path: r.get(0)?, status: r.get(1)?, error: r.get(2)? }))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The files whose text has every word of query, best first, each once with
/// the first part it was found in. A plain helper for tests and the bench;
/// search over contents proper comes with snippets and ranking.
pub fn match_contents(index: &Index, query: &str, limit: usize) -> Result<Vec<ContentMatch>> {
    let phrase =
        query.split_whitespace().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" ");
    if phrase.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = index.conn.prepare_cached(&format!(
        "SELECT f.path, c.part, c.label FROM contents_fts JOIN contents c ON c.id = contents_fts.rowid
         JOIN files f ON f.id = c.file_id WHERE contents_fts MATCH ?1 AND {NOT_REMOVING} ORDER BY rank"
    ))?;
    let rows =
        stmt.query_map(params![phrase], |r| Ok(ContentMatch { path: r.get(0)?, part: r.get(1)?, label: r.get(2)? }))?;
    let mut out: Vec<ContentMatch> = Vec::new();
    for row in rows {
        let m = row?;
        if !out.iter().any(|o| o.path == m.path) {
            out.push(m);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}
