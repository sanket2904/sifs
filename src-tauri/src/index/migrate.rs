//! Schema changes, numbered by SQLite's `user_version`. The schema in db.rs
//! is version 0, created if missing at every open; each migration after it
//! runs once, in one transaction with its version number, so a database is
//! never left half migrated. Phase 1 databases upgrade in place.

use anyhow::{Context, Result};
use rusqlite::{Connection, TransactionBehavior};

/// Migration n + 1 is MIGRATIONS[n].
pub(crate) const MIGRATIONS: &[&str] = &[CONTENTS, CONTENT_OWNERS, NANOSECONDS];

/// Version 1: reading inside files. What was read from each file, its text
/// by page or part with a full-text index over it, the facts read alongside,
/// and the queue of files waiting to be read. Triggers on files keep the
/// queue right, so the crawler and the watcher need not know about it.
const CONTENTS: &str = concat!(
    r#"
CREATE TABLE extracts (
    file_id  INTEGER PRIMARY KEY,
    reader   TEXT NOT NULL,
    version  INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    size     INTEGER NOT NULL,
    status   TEXT NOT NULL CHECK (status IN ('done', 'empty', 'failed', 'skipped')),
    error    TEXT,
    millis   INTEGER NOT NULL,
    chars    INTEGER NOT NULL DEFAULT 0,
    read_at  INTEGER NOT NULL
);
CREATE INDEX extracts_status ON extracts(status);

CREATE TABLE contents (
    id      INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    part    INTEGER NOT NULL,
    label   TEXT NOT NULL,
    text    TEXT NOT NULL
);
CREATE INDEX contents_file ON contents(file_id, part);
-- Letters, numbers and combining marks make words. Without the marks (M*),
-- Hindi and other Indic words split at every vowel sign ("किराया" into
-- क / र / य); a decomposed accent is still folded away ("résumé" is
-- "resume"). The emoji variation selectors U+FE0E and U+FE0F and the
-- keycap U+20E3 are marks too, but separate words: "I❤️you" has "you", and
-- "❤️" is no word, not a selector that every emoji with one matches.
CREATE VIRTUAL TABLE contents_fts USING fts5(
    label UNINDEXED, text, content='contents', content_rowid='id',
    tokenize="unicode61 remove_diacritics 2 categories 'L* N* Co M*' separators '"#,
    "\u{fe0e}\u{fe0f}\u{20e3}",
    r#"'"
);
CREATE TRIGGER contents_ai AFTER INSERT ON contents BEGIN
    INSERT INTO contents_fts(rowid, label, text) VALUES (new.id, new.label, new.text);
END;
CREATE TRIGGER contents_ad AFTER DELETE ON contents BEGIN
    INSERT INTO contents_fts(contents_fts, rowid, label, text) VALUES ('delete', old.id, old.label, old.text);
END;
CREATE TRIGGER contents_au AFTER UPDATE ON contents BEGIN
    INSERT INTO contents_fts(contents_fts, rowid, label, text) VALUES ('delete', old.id, old.label, old.text);
    INSERT INTO contents_fts(rowid, label, text) VALUES (new.id, new.label, new.text);
END;

CREATE TABLE facts (
    file_id INTEGER NOT NULL,
    key     TEXT NOT NULL,
    value   TEXT NOT NULL,
    PRIMARY KEY (file_id, key, value)
) WITHOUT ROWID;
CREATE INDEX facts_key ON facts(key, value);

-- priority: -1 opened in SIFS, 0 changed in the last week, then the
-- reader's: 1 documents, 2 images, 3 the rest. attempts: reads started and
-- never finished, so a file that stops SIFS is not read again forever.
CREATE TABLE extract_queue (
    file_id  INTEGER PRIMARY KEY,
    priority INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    size     INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX extract_queue_order ON extract_queue(priority, modified DESC);
CREATE INDEX extract_queue_retry ON extract_queue(file_id) WHERE attempts > 0;

-- Which extensions have a reader on this machine. Rewritten from the
-- readers at startup, so each platform registers its own.
CREATE TABLE extract_kinds (
    ext      TEXT PRIMARY KEY,
    reader   TEXT NOT NULL,
    version  INTEGER NOT NULL,
    priority INTEGER NOT NULL,
    packages INTEGER NOT NULL DEFAULT 0
);

CREATE TRIGGER files_ai_extract AFTER INSERT ON files BEGIN
    INSERT OR REPLACE INTO extract_queue (file_id, priority, modified, size, attempts)
    SELECT new.id, CASE WHEN new.modified >= unixepoch() - 604800 THEN 0 ELSE k.priority END,
           new.modified, new.size, 0
    FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1);
END;
-- The crawler's upsert rewrites every column of every row it sees, so only
-- a real change of time or size queues the file again.
CREATE TRIGGER files_au_extract AFTER UPDATE OF modified, size ON files
WHEN new.modified IS NOT old.modified OR new.size IS NOT old.size BEGIN
    INSERT OR REPLACE INTO extract_queue (file_id, priority, modified, size, attempts)
    SELECT new.id, 0, new.modified, new.size, 0
    FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1);
END;
-- A row that changed into something no reader takes (a file replaced by a
-- folder of the same name, which keeps the row and its id): what was read
-- from the file goes.
CREATE TRIGGER files_au_unread AFTER UPDATE OF is_dir, modified, size ON files
WHEN (new.is_dir IS NOT old.is_dir OR new.modified IS NOT old.modified OR new.size IS NOT old.size)
    AND NOT EXISTS (SELECT 1 FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1)) BEGIN
    DELETE FROM contents WHERE file_id = new.id;
    DELETE FROM facts WHERE file_id = new.id;
    DELETE FROM extracts WHERE file_id = new.id;
    DELETE FROM extract_queue WHERE file_id = new.id;
END;
CREATE TRIGGER files_ad_extract AFTER DELETE ON files BEGIN
    DELETE FROM contents WHERE file_id = old.id;
    DELETE FROM facts WHERE file_id = old.id;
    DELETE FROM extracts WHERE file_id = old.id;
    DELETE FROM extract_queue WHERE file_id = old.id;
END;
"#
);

/// Version 2: which file each part is from, without its text. A part's row
/// holds its text, so most rows fill a page: search finds the files of
/// thousands of matching parts through this index, reading far fewer pages.
const CONTENT_OWNERS: &str = "CREATE INDEX contents_owner ON contents(id, file_id);";

/// Version 3: file times to the nanosecond, so an edit within the same
/// second that keeps the size is a change: the file is read again, and its
/// old words are not found meanwhile. NULL is a time written before this
/// version, not known to the nanosecond. Every file read before it is read
/// once more (no released build has read contents), and until then its
/// contents count as current only while the file's time is unknown too.
/// So filling in the files' times (the first scan after the upgrade does it
/// for every file) queues nothing more.
const NANOSECONDS: &str = r#"
ALTER TABLE files ADD COLUMN modified_ns INTEGER;
ALTER TABLE extracts ADD COLUMN modified_ns INTEGER;
ALTER TABLE extract_queue ADD COLUMN modified_ns INTEGER;

INSERT OR IGNORE INTO extract_queue (file_id, priority, modified, size, attempts, modified_ns)
SELECT e.file_id, COALESCE(k.priority, 3), f.modified, f.size, 0, NULL
FROM extracts e JOIN files f ON f.id = e.file_id LEFT JOIN extract_kinds k ON k.ext = f.ext;

DROP TRIGGER files_ai_extract;
CREATE TRIGGER files_ai_extract AFTER INSERT ON files BEGIN
    INSERT OR REPLACE INTO extract_queue (file_id, priority, modified, size, attempts, modified_ns)
    SELECT new.id, CASE WHEN new.modified >= unixepoch() - 604800 THEN 0 ELSE k.priority END,
           new.modified, new.size, 0, new.modified_ns
    FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1);
END;
DROP TRIGGER files_au_extract;
-- An upsert of files (the crawler's and the watcher's) runs its update
-- with ABORT, which overrides a trigger's OR REPLACE: a file still queued
-- would fail to update. ON CONFLICT here is not overridden.
CREATE TRIGGER files_au_extract AFTER UPDATE OF modified, size, modified_ns ON files
WHEN new.modified IS NOT old.modified OR new.size IS NOT old.size
    OR (old.modified_ns IS NOT NULL AND new.modified_ns IS NOT old.modified_ns) BEGIN
    INSERT INTO extract_queue (file_id, priority, modified, size, attempts, modified_ns)
    SELECT new.id, 0, new.modified, new.size, 0, new.modified_ns
    FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1)
    ON CONFLICT (file_id) DO UPDATE SET priority = 0, modified = excluded.modified, size = excluded.size,
        attempts = 0, modified_ns = excluded.modified_ns;
END;
DROP TRIGGER files_au_unread;
CREATE TRIGGER files_au_unread AFTER UPDATE OF is_dir, modified, size, modified_ns ON files
WHEN (new.is_dir IS NOT old.is_dir OR new.modified IS NOT old.modified OR new.size IS NOT old.size
        OR (old.modified_ns IS NOT NULL AND new.modified_ns IS NOT old.modified_ns))
    AND NOT EXISTS (SELECT 1 FROM extract_kinds k WHERE k.ext = new.ext AND (new.is_dir = 0 OR k.packages = 1)) BEGIN
    DELETE FROM contents WHERE file_id = new.id;
    DELETE FROM facts WHERE file_id = new.id;
    DELETE FROM extracts WHERE file_id = new.id;
    DELETE FROM extract_queue WHERE file_id = new.id;
END;
"#;

/// The schema version this build knows.
pub const VERSION: i64 = MIGRATIONS.len() as i64;

/// Brings the database up to VERSION; the caller holds the writer lock.
/// A database from a newer SIFS is left as it is: its additions only touch
/// tables this build doesn't use.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let Some(sql) = usize::try_from(version).ok().and_then(|v| MIGRATIONS.get(v)) else {
            return Ok(());
        };
        tx.execute_batch(sql).with_context(|| format!("migrate the index to version {}", version + 1))?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
        log::info!("index migrated to version {}", version + 1);
    }
}
