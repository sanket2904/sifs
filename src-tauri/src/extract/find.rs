//! Search over contents, and names and contents ranked together (phase 3
//! step 3 in plans/phase3-reading-inside-files.md).

use crate::index::db::{row_to_record, FileRecord, Index, NOT_REMOVING};
use crate::index::query::{fold_into, Query, WORD};
use crate::index::search::{self, kind_prior, place_prior, recency};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a search over names and contents may take: past it, the
/// content query stops and snippets are left empty.
pub const CONTENT_TIME: Duration = Duration::from_millis(400);
/// Matching parts looked at, at most (a file may match in many). Past it
/// the results are marked incomplete.
pub const CONTENT_PARTS: usize = 10_000;
/// A snippet's length in characters, at most, and how many of them come
/// before its first word.
const SNIPPET_CHARS: usize = 200;
const SNIPPET_BEFORE: usize = 40;
/// The deadline is checked every this many words while looking for the
/// first one in a part.
const CHECK_EVERY: usize = 1024;
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
    /// Empty if the time ran out before it was made.
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
    /// The content search stopped short, at its time limit or its cap on
    /// parts: files matching by contents may be missing, or lack a snippet.
    pub contents_timed_out: bool,
    /// Words typed past the ones searched (index::query::MAX_WORDS).
    pub ignored_words: usize,
}

/// How a file matched. With kind and place equal, a name match ranks
/// first, then contents, then a typo-tolerant name (a guess ranks below
/// words that are really there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum By {
    Name,
    Contents,
    CloseName,
}

/// A part's id in contents.
type PartId = i64;

/// The files whose contents match, best first, each once with its best part
/// (by bm25).
#[derive(Default)]
struct ContentMatches {
    files: Vec<(FileRecord, PartId)>,
    /// The search stopped before it saw every matching part.
    incomplete: bool,
}

struct Candidate {
    file: FileRecord,
    prior: f64,
    by: By,
    score: f64,
    part: Option<PartId>,
}

/// Names and contents ranked together (owner's decision 5: a name match
/// ranks above a content match only when kind and place are equal). Every
/// candidate is ranked before the limit applies: by kind and place priors,
/// then By, then score (match quality or content rank, and recency). Only
/// the results kept get snippets.
pub fn search_all(index: &Index, typed: &str, limit: usize, now: i64, time_limit: Duration) -> Result<Results> {
    let deadline = Instant::now() + time_limit;
    let query = Query::parse(typed);
    let names = search::matches(index, &query, limit, now)?;
    let contents = match query.fts() {
        None => ContentMatches::default(),
        Some(fts) => matching_parts(&index.conn, &fts, deadline)?
            .unwrap_or(ContentMatches { files: Vec::new(), incomplete: true }),
    };
    let mut incomplete = contents.incomplete;
    let total = contents.files.len().max(1) as f64;
    let mut by_contents: HashMap<String, (FileRecord, f64, PartId)> = contents
        .files
        .into_iter()
        .enumerate()
        .map(|(position, (file, part))| {
            let score = CONTENT_BASE + CONTENT_RANK * (1.0 - position as f64 / total) + recency(file.modified, now);
            (file.path.clone(), (file, score, part))
        })
        .collect();
    let prior = |file: &FileRecord| kind_prior(file) + place_prior(&file.path);
    let mut candidates: Vec<Candidate> = Vec::with_capacity(names.len() + by_contents.len());
    for hit in names {
        let (by, score, part) = match (by_contents.remove(&hit.file.path), hit.fuzzy) {
            (Some((_, score, part)), true) => (By::Contents, score, Some(part)),
            (Some((_, _, part)), false) => (By::Name, hit.score + BOTH, Some(part)),
            (None, true) => (By::CloseName, hit.score, None),
            (None, false) => (By::Name, hit.score, None),
        };
        candidates.push(Candidate { prior: prior(&hit.file), file: hit.file, by, score, part });
    }
    for (file, score, part) in by_contents.into_values() {
        candidates.push(Candidate { prior: prior(&file), file, by: By::Contents, score, part: Some(part) });
    }
    candidates.sort_by(|a, b| {
        b.prior
            .total_cmp(&a.prior)
            .then(a.by.cmp(&b.by))
            .then(b.score.total_cmp(&a.score))
            .then_with(|| a.file.path.cmp(&b.file.path))
    });
    candidates.truncate(limit);
    let mut hits = Vec::with_capacity(candidates.len());
    for c in candidates {
        let found = match c.part {
            None => None,
            Some(part) => {
                let found = found_in(index, part, &query, deadline)?;
                incomplete |= found.snippet.is_empty();
                Some(found)
            }
        };
        hits.push(Ranked { file: c.file, score: c.score, fuzzy: c.by == By::CloseName, found });
    }
    Ok(Results { hits, contents_timed_out: incomplete, ignored_words: query.ignored })
}

/// The files whose contents match, incomplete past CONTENT_PARTS. Only
/// contents read from the file as it is now count (an edited file's old
/// words are not found), and not under a folder being removed. None if the
/// deadline passed first.
fn matching_parts(conn: &Connection, fts: &str, deadline: Instant) -> Result<Option<ContentMatches>> {
    // Parts are found through contents_owner, never their rows: a part's
    // row holds its text, so reading thousands of them is thousands of
    // pages read from disk.
    let sql = format!(
        "SELECT f.path, f.root, f.name, f.ext, f.is_dir, f.size, f.modified, f.inode, g.part, g.parts,
                e.modified IS f.modified AND e.size IS f.size AND {NOT_REMOVING}
         FROM (SELECT c.file_id, c.id AS part, MIN(m.rank) AS rank, COUNT(*) AS parts
               FROM (SELECT rowid, rank FROM contents_fts WHERE contents_fts MATCH ?1 LIMIT {CONTENT_PARTS}) m
               JOIN contents c INDEXED BY contents_owner ON c.id = m.rowid GROUP BY c.file_id) g
         JOIN files f ON f.id = g.file_id LEFT JOIN extracts e ON e.file_id = f.id
         ORDER BY g.rank, f.path"
    );
    let _stop = Stop::at(conn, deadline);
    let found = (|| -> rusqlite::Result<_> {
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = stmt.query([fts])?;
        let mut files = Vec::new();
        let mut scanned = 0;
        while let Some(row) = rows.next()? {
            scanned += row.get::<_, usize>(9)?;
            if row.get::<_, bool>(10)? {
                files.push((row_to_record(row)?, row.get(8)?));
            }
        }
        Ok(ContentMatches { files, incomplete: scanned >= CONTENT_PARTS })
    })();
    match found {
        Ok(found) => Ok(Some(found)),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::OperationInterrupted => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Stops the connection's statements once the deadline has passed (SQLite
/// asks every 64 steps), until dropped.
struct Stop<'a>(&'a Connection);

impl<'a> Stop<'a> {
    fn at(conn: &'a Connection, deadline: Instant) -> Self {
        conn.progress_handler(64, Some(move || Instant::now() >= deadline));
        Stop(conn)
    }
}

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

/// A shown part's label and snippet. The snippet is left empty if the
/// deadline passed first, or the part was deleted since it matched.
fn found_in(index: &Index, part: PartId, query: &Query, deadline: Instant) -> Result<Found> {
    let column = |name: &str| -> Result<Option<String>> {
        let sql = format!("SELECT {name} FROM contents WHERE id = ?1");
        Ok(index.conn.prepare_cached(&sql)?.query_row([part], |r| r.get(0)).optional()?)
    };
    let label = column("label")?.unwrap_or_default();
    let mut snippet = Vec::new();
    if Instant::now() < deadline {
        if let Some(text) = column("text")? {
            snippet = self::snippet(&text, query, deadline).unwrap_or_default();
        }
    }
    Ok(Found { label, snippet })
}

/// At most SNIPPET_CHARS characters of text around the first word the query
/// looks for, from a whole word at most SNIPPET_BEFORE characters before it
/// to a whole word (or into the word found, if it is that long), with each
/// such word marked and an ellipsis where text is left out. Hits are
/// positions found here, never marks inside the text, so the file's own
/// characters can't pose as one. None if the deadline passed while looking.
fn snippet(text: &str, query: &Query, deadline: Instant) -> Option<Vec<Segment>> {
    let mut folded = String::new();
    let mut is_hit = |word: &str| {
        fold_into(word, &mut folded);
        query.matches(&folded)
    };
    let mut first = None;
    for (n, word) in WORD.find_iter(text).enumerate() {
        if n % CHECK_EVERY == 0 && Instant::now() >= deadline {
            return None;
        }
        if is_hit(word.as_str()) {
            first = Some(word.range());
            break;
        }
    }
    let first = first.unwrap_or(0..0);
    let mut start = text[..first.start].char_indices().rev().nth(SNIPPET_BEFORE - 1).map_or(0, |(i, _)| i);
    if start > 0 {
        if in_word(text, start) {
            start = WORD.find_at(text, start).map_or(start, |w| w.end());
        }
        start = WORD.find_at(text, start).map_or(first.start, |w| w.start().min(first.start));
    }
    let mut end = text[start..].char_indices().nth(SNIPPET_CHARS).map_or(text.len(), |(i, _)| start + i);
    if end < text.len() && in_word(text, end) {
        let cut = text[..end].char_indices().rev().take_while(|&(_, c)| is_word_char(c)).last().map_or(end, |(i, _)| i);
        if cut >= first.end {
            end = text[..cut].trim_end().len().max(first.end);
        }
    }
    let mut segments = Vec::new();
    let mut plain = String::from(if start > 0 { "…" } else { "" });
    let mut at = start;
    for word in WORD.find_iter(&text[start..end]) {
        if is_hit(word.as_str()) {
            plain.push_str(&text[at..start + word.start()]);
            if !plain.is_empty() {
                segments.push(Segment { text: spaced(&std::mem::take(&mut plain)), hit: false });
            }
            segments.push(Segment { text: word.as_str().to_string(), hit: true });
            at = start + word.end();
        }
    }
    plain.push_str(&text[at..end]);
    if end < text.len() {
        plain.push('…');
    }
    if !plain.is_empty() {
        segments.push(Segment { text: spaced(&plain), hit: false });
    }
    Some(segments)
}

/// Line breaks and tabs in a snippet read as spaces.
fn spaced(text: &str) -> String {
    text.replace(['\n', '\r', '\t'], " ")
}

fn is_word_char(c: char) -> bool {
    WORD.is_match(c.encode_utf8(&mut [0; 4]))
}

/// Whether byte offset at is inside a word: word characters on both sides.
fn in_word(text: &str, at: usize) -> bool {
    text[at..].chars().next().is_some_and(is_word_char) && text[..at].chars().next_back().is_some_and(is_word_char)
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
