//! Search over contents, and names and contents ranked together (phase 3
//! step 3 in plans/phase3-reading-inside-files.md).

use crate::index::db::{row_to_record, FileRecord, Index, NOT_REMOVING};
use crate::index::search::{self, kind_prior, place_prior, recency, Hit};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Files content results come from, at most.
pub const CONTENT_FILES: usize = 200;
/// Parts looked at to find them (a file may match in many).
const CONTENT_PARTS: usize = 2000;
/// How long the content query may take before it is stopped.
pub const CONTENT_TIME: Duration = Duration::from_millis(400);
/// The last word matches as a prefix from this many characters.
const PREFIX_FROM: usize = 3;
/// A content match's base score (shown with results; ranking is by tier
/// first, see search_all).
const CONTENT_BASE: f64 = 20.0;
/// Added for the best content match, less for each one after.
const CONTENT_RANK: f64 = 10.0;
/// Added to a name match whose file's contents match too.
const BOTH: f64 = 5.0;

/// A piece of a snippet: text, and whether it is a matched word.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

/// Where a file's words were found: its best part.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Found {
    /// The part's label: "page 3", a sheet's name, "slide 2 notes", or
    /// empty for a file of one part.
    pub label: String,
    pub snippet: Vec<Segment>,
}

/// One result of a search over names and contents.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Ranked {
    #[serde(flatten)]
    pub file: FileRecord,
    pub score: f64,
    pub fuzzy: bool,
    /// Set when the file's contents matched.
    pub found: Option<Found>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Results {
    pub hits: Vec<Ranked>,
    /// The content query was stopped at its time limit: only names were
    /// searched.
    pub contents_timed_out: bool,
}

/// The FTS5 query for what was typed: each word quoted (so nothing in it
/// is query syntax), all of them required, the last as a prefix when it
/// is long enough. None if there are no words.
pub fn fts_query(query: &str) -> Option<String> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let last = words.len().checked_sub(1)?;
    let terms: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let quoted = format!("\"{}\"", w.replace('"', "\"\""));
            if i == last && w.chars().count() >= PREFIX_FROM {
                quoted + "*"
            } else {
                quoted
            }
        })
        .collect();
    Some(terms.join(" "))
}

/// The files whose contents match, best first, each with its best part and
/// a snippet. Stopped after time_limit (Ok(None)).
pub fn search_contents(index: &Index, query: &str, time_limit: Duration) -> Result<Option<Vec<(FileRecord, Found)>>> {
    let Some(fts) = fts_query(query) else { return Ok(Some(Vec::new())) };
    let sql = format!(
        "SELECT f.path, f.root, f.name, f.ext, f.is_dir, f.size, f.modified, f.inode, c.label,
                snippet(contents_fts, 1, char(2), char(3), '…', 12)
         FROM contents_fts JOIN contents c ON c.id = contents_fts.rowid JOIN files f ON f.id = c.file_id
         WHERE contents_fts MATCH ?1 AND {NOT_REMOVING} ORDER BY rank LIMIT {CONTENT_PARTS}"
    );
    // SQLite asks every 64 steps whether to stop: past the deadline, the
    // query ends with SQLITE_INTERRUPT. (A query too small to reach a check
    // can't run long either.)
    let deadline = Instant::now() + time_limit;
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    index.conn.progress_handler(
        64,
        Some(move || {
            let stop = Instant::now() >= deadline;
            if stop {
                flag.store(true, Ordering::Relaxed);
            }
            stop
        }),
    );
    let found = (|| -> rusqlite::Result<Vec<(FileRecord, Found)>> {
        let mut stmt = index.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query([&fts])?;
        let mut out: Vec<(FileRecord, Found)> = Vec::new();
        let mut seen: HashMap<String, ()> = HashMap::new();
        while let Some(row) = rows.next()? {
            let file = row_to_record(row)?;
            if seen.insert(file.path.clone(), ()).is_some() {
                continue;
            }
            let found = Found { label: row.get(8)?, snippet: segments(&row.get::<_, String>(9)?) };
            out.push((file, found));
            if out.len() >= CONTENT_FILES {
                break;
            }
        }
        Ok(out)
    })();
    index.conn.progress_handler(0, None::<fn() -> bool>);
    let interrupted = stopped.load(Ordering::Relaxed);
    match found {
        Ok(found) => Ok(Some(found)),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::OperationInterrupted || interrupted => {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// A snippet with char(2) and char(3) around each matched word, as pieces.
fn segments(snippet: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut hit = false;
    let mut text = String::new();
    let flush = |text: &mut String, hit: bool, out: &mut Vec<Segment>| {
        if !text.is_empty() {
            out.push(Segment { text: std::mem::take(text), hit });
        }
    };
    for c in snippet.chars() {
        match c {
            '\u{2}' => {
                flush(&mut text, hit, &mut out);
                hit = true;
            }
            '\u{3}' => {
                flush(&mut text, hit, &mut out);
                hit = false;
            }
            // Line breaks inside a snippet read as spaces.
            '\n' | '\r' | '\t' => text.push(' '),
            c => text.push(c),
        }
    }
    flush(&mut text, hit, &mut out);
    out
}

/// Names and contents ranked together (owner's decision 5: a name match
/// ranks above a content match only when kind and place are equal). Files
/// are ordered by their kind and place priors first; within equal priors,
/// name matches, then content matches, then typo-tolerant name matches;
/// then by score (match quality or content rank, and recency).
pub fn search_all(index: &Index, query: &str, limit: usize, now: i64, time_limit: Duration) -> Result<Results> {
    let names: Vec<Hit> = search::search(index, query, limit, now)?;
    let contents = search_contents(index, query, time_limit)?;
    let contents_timed_out = contents.is_none();
    let contents = contents.unwrap_or_default();
    let total = contents.len().max(1) as f64;
    let mut by_path: HashMap<String, (Found, f64)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut files: HashMap<String, FileRecord> = HashMap::new();
    for (position, (file, found)) in contents.into_iter().enumerate() {
        let score = CONTENT_RANK * (1.0 - position as f64 / total) + recency(file.modified, now);
        order.push(file.path.clone());
        by_path.insert(file.path.clone(), (found, score));
        files.insert(file.path.clone(), file);
    }
    let mut ranked: Vec<(f64, u8, Ranked)> = Vec::new();
    let prior = |file: &FileRecord| kind_prior(file) + place_prior(&file.path);
    for hit in names {
        let found = by_path.remove(&hit.file.path).map(|(found, _)| found);
        let tier = if hit.fuzzy { 2 } else { 0 };
        let score = if found.is_some() { hit.score + BOTH } else { hit.score };
        ranked.push((prior(&hit.file), tier, Ranked { file: hit.file, score, fuzzy: hit.fuzzy, found }));
    }
    for path in order {
        if let (Some((found, score)), Some(file)) = (by_path.remove(&path), files.remove(&path)) {
            ranked.push((prior(&file), 1, Ranked { file, score: CONTENT_BASE + score, fuzzy: false, found: Some(found) }));
        }
    }
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(b.2.score.total_cmp(&a.2.score))
            .then_with(|| a.2.file.path.cmp(&b.2.file.path))
    });
    let hits = ranked.into_iter().take(limit).map(|(_, _, hit)| hit).collect();
    Ok(Results { hits, contents_timed_out })
}
