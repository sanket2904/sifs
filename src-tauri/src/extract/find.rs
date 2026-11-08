//! Search over contents, and names and contents ranked together (phase 3
//! step 3 in plans/phase3-reading-inside-files.md).

use crate::index::db::{row_to_record, FileRecord, Index, NOT_REMOVING};
use crate::index::query::Query;
use crate::index::search::{self, kind_prior, place_prior, recency, Order};
use crate::index::stop::{stopped, Stop, Until};
use crate::index::tokenize::Tokenizer;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::time::Duration;

/// How long a search over names and contents may take, from when it is
/// asked for: past it, the search stops and snippets are left empty.
pub const CONTENT_TIME: Duration = Duration::from_millis(400);
/// Matching parts looked at, at most (a file may match in many): the first
/// ones in storage order, not the best, since finding the best means ranking
/// every match. Past it the results are marked capped.
pub const CONTENT_PARTS: usize = 10_000;
/// A snippet's length in characters, at most, ellipses included, and how
/// many of them come before its first word.
const SNIPPET_CHARS: usize = 200;
const SNIPPET_BEFORE: usize = 40;
/// The time is checked every this many words while looking for the first
/// one in a part.
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
#[derive(Debug, Clone, Default, serde::Serialize)]
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
    /// The time ran out while names were compared: files matching by name
    /// may be missing.
    pub names_timed_out: bool,
    /// More than search::CANDIDATES names matched: the best of them were
    /// ranked with the contents.
    pub names_capped: bool,
    /// The time ran out: files matching by contents may be missing, or
    /// lack a snippet.
    pub contents_timed_out: bool,
    /// CONTENT_PARTS parts matched and no more were looked at: files
    /// matching only in later parts are missing.
    pub contents_capped: bool,
    /// Some of what was typed was not searched (index::query::MAX_BYTES,
    /// MAX_WORDS).
    pub words_left_out: bool,
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

/// A part in contents, and the file it is of.
#[derive(Debug, Clone, Copy)]
struct Part {
    id: i64,
    file: i64,
}

/// The files whose contents match, best first, each once with its best part
/// (by bm25).
#[derive(Default)]
struct ContentMatches {
    files: Vec<(FileRecord, Part)>,
    /// CONTENT_PARTS parts matched and no more were looked at.
    capped: bool,
}

struct Candidate {
    file: FileRecord,
    prior: f64,
    by: By,
    score: f64,
    part: Option<Part>,
}

/// Names and contents ranked together (owner's decision 5: a name match
/// ranks above a content match only when kind and place are equal). Every
/// candidate is ranked before the limit applies: by kind and place priors,
/// then By, then score (match quality or content rank, and recency). Only
/// the results kept get snippets. Everything is read from one snapshot of
/// the index, and stops when until passes.
pub fn search_all(index: &Index, typed: &str, limit: usize, now: i64, until: &Until) -> Result<Results> {
    let conn = &index.conn;
    let mut tokenizer = Tokenizer::contents(conn)?;
    let query = Query::parse(typed, &mut tokenizer)?;
    // A caller already in a transaction has its snapshot.
    let _snapshot = if conn.is_autocommit() { Some(conn.unchecked_transaction()?) } else { None };
    let _stop = Stop::at(conn, until);
    let names = search::matches(index, &query, now, until, Order::Tiers)?;
    let mut timed_out = false;
    let contents = match query.fts() {
        None => ContentMatches::default(),
        Some(_) if until.passed() => {
            timed_out = true;
            ContentMatches::default()
        }
        Some(fts) => matching_parts(conn, &fts)?.unwrap_or_else(|| {
            timed_out = true;
            ContentMatches::default()
        }),
    };
    let capped = contents.capped;
    let total = contents.files.len().max(1) as f64;
    let mut by_contents: HashMap<String, (FileRecord, f64, Part)> = contents
        .files
        .into_iter()
        .enumerate()
        .map(|(position, (file, part))| {
            let score = CONTENT_BASE + CONTENT_RANK * (1.0 - position as f64 / total) + recency(file.modified, now);
            (file.path.clone(), (file, score, part))
        })
        .collect();
    let mut candidates: Vec<Candidate> = Vec::with_capacity(names.hits.len() + by_contents.len());
    for hit in names.hits {
        let (by, score, part) = match (by_contents.remove(&hit.file.path), hit.fuzzy) {
            (Some((_, score, part)), true) => (By::Contents, score, Some(part)),
            (Some((_, _, part)), false) => (By::Name, hit.score + BOTH, Some(part)),
            (None, true) => (By::CloseName, hit.score, None),
            (None, false) => (By::Name, hit.score, None),
        };
        candidates.push(Candidate { prior: hit.prior, file: hit.file, by, score, part });
    }
    // A name that matches too but was not among the names kept keeps its
    // tier and the bonus.
    for (file, score, part) in by_contents.into_values() {
        let prior = kind_prior(file.is_dir, &file.ext) + place_prior(&file.path);
        let (by, score) = match search::name_score(&file, &query.terms, prior, now) {
            Some(name) => (By::Name, name + BOTH),
            None => (By::Contents, score),
        };
        candidates.push(Candidate { prior, file, by, score, part: Some(part) });
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
                let (found, late) = found_in(conn, &mut tokenizer, part, &query, until)?;
                timed_out |= late;
                Some(found)
            }
        };
        hits.push(Ranked { file: c.file, score: c.score, fuzzy: c.by == By::CloseName, found });
    }
    Ok(Results {
        hits,
        names_timed_out: names.timed_out,
        names_capped: names.capped,
        contents_timed_out: timed_out,
        contents_capped: capped,
        words_left_out: query.left_out,
    })
}

/// The files whose contents match, capped at CONTENT_PARTS. Only
/// contents read from the file as it is now count (an edited file's old
/// words are not found), and not under a folder being removed. None if the
/// statement was stopped first.
fn matching_parts(conn: &Connection, fts: &str) -> Result<Option<ContentMatches>> {
    // Parts are found through contents_owner, never their rows: a part's
    // row holds its text, so reading thousands of them is thousands of
    // pages read from disk.
    let sql = format!(
        "SELECT f.path, f.root, f.name, f.ext, f.is_dir, f.size, f.modified, f.inode, f.modified_ns, g.part, g.file_id, g.parts,
                e.modified IS f.modified AND e.size IS f.size AND e.modified_ns IS f.modified_ns AND {NOT_REMOVING}
         FROM (SELECT c.file_id, c.id AS part, MIN(m.rank) AS rank, COUNT(*) AS parts
               FROM (SELECT rowid, rank FROM contents_fts WHERE contents_fts MATCH ?1 LIMIT {CONTENT_PARTS}) m
               JOIN contents c INDEXED BY contents_owner ON c.id = m.rowid GROUP BY c.file_id) g
         JOIN files f ON f.id = g.file_id LEFT JOIN extracts e ON e.file_id = f.id
         ORDER BY g.rank, f.path"
    );
    let found = (|| -> rusqlite::Result<_> {
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = stmt.query([fts])?;
        let mut files = Vec::new();
        let mut scanned = 0;
        while let Some(row) = rows.next()? {
            scanned += row.get::<_, usize>(11)?;
            if row.get::<_, bool>(12)? {
                files.push((row_to_record(row)?, Part { id: row.get(9)?, file: row.get(10)? }));
            }
        }
        Ok(ContentMatches { files, capped: scanned >= CONTENT_PARTS })
    })();
    match found {
        Ok(found) => Ok(Some(found)),
        Err(e) if stopped(&e) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// A shown part's label and snippet, read together, and whether the time
/// ran out first. Read by part and file: a part id is reused once deleted.
/// A part not there gives neither, and is no timeout.
fn found_in(conn: &Connection, tokenizer: &mut Tokenizer, part: Part, query: &Query, until: &Until) -> Result<(Found, bool)> {
    if until.passed() {
        return Ok((Found::default(), true));
    }
    let read = conn
        .prepare_cached("SELECT label, text FROM contents WHERE id = ?1 AND file_id = ?2")?
        .query_row(params![part.id, part.file], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .optional();
    let (label, text) = match read {
        Ok(Some(read)) => read,
        Ok(None) => return Ok((Found::default(), false)),
        Err(e) if stopped(&e) => return Ok((Found::default(), true)),
        Err(e) => return Err(e.into()),
    };
    Ok(match snippet(tokenizer, &text, query, until)? {
        Some(snippet) => (Found { label, snippet }, false),
        None => (Found { label, snippet: Vec::new() }, true),
    })
}

/// At most SNIPPET_CHARS characters of text, ellipses included, around the
/// first word the query looks for: from a whole word at most SNIPPET_BEFORE
/// characters before it to a whole word (or into the word found, if it is
/// that long), with each such word marked and an ellipsis where text is
/// left out. Words are the index's own (tokenizer), and hits are positions
/// found here, never marks inside the text, so the file's own characters
/// can't pose as one. None if until passed while looking.
fn snippet(tokenizer: &mut Tokenizer, text: &str, query: &Query, until: &Until) -> Result<Option<Vec<Segment>>> {
    let mut first = None;
    let mut late = false;
    let mut n = 0;
    tokenizer.words(text, |word, at| {
        n += 1;
        if n % CHECK_EVERY == 0 && until.passed() {
            late = true;
            return ControlFlow::Break(());
        }
        if query.matches(word) {
            first = Some(at);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    })?;
    if late {
        return Ok(None);
    }
    let first = first.unwrap_or(0..0);
    let start = match text[..first.start].char_indices().rev().nth(SNIPPET_BEFORE - 1) {
        None => 0,
        // The first word starting there or after: one starting before it,
        // or right at it after a word character, would be cut.
        Some((back, _)) => {
            let from = text[..back].char_indices().next_back().map_or(back, |(i, _)| i);
            let words = words_in(tokenizer, query, text, from..first.start)?;
            words.into_iter().map(|(word, _)| word.start).find(|&at| at >= back).unwrap_or(first.start)
        }
    };
    let room = SNIPPET_CHARS - usize::from(start > 0);
    let mut end = text.len();
    if text[start..].chars().nth(room).is_some() {
        // Too long: one character less, for the closing ellipsis.
        end = start + text[start..].char_indices().nth(room - 1).map_or(0, |(i, _)| i);
        let next = end + text[end..].chars().next().map_or(0, char::len_utf8);
        let words = words_in(tokenizer, query, text, start..next)?;
        if let Some((word, _)) = words.into_iter().find(|(w, _)| w.start < end && w.end > end) {
            if word.start >= first.end {
                end = text[..word.start].trim_end().len();
            }
        }
    }
    let mut segments = Vec::new();
    let mut plain = String::from(if start > 0 { "…" } else { "" });
    let mut at = start;
    for (word, hit) in words_in(tokenizer, query, text, start..end)? {
        if hit {
            plain.push_str(&text[at..word.start]);
            if !plain.is_empty() {
                segments.push(Segment { text: spaced(&std::mem::take(&mut plain)), hit: false });
            }
            segments.push(Segment { text: text[word.clone()].to_string(), hit: true });
            at = word.end;
        }
    }
    plain.push_str(&text[at..end]);
    if end < text.len() {
        plain.push('…');
    }
    if !plain.is_empty() {
        segments.push(Segment { text: spaced(&plain), hit: false });
    }
    Ok(Some(segments))
}

/// Where the words of text[within] are, in text, and whether each is one
/// the query looks for.
fn words_in(tokenizer: &mut Tokenizer, query: &Query, text: &str, within: Range<usize>) -> Result<Vec<(Range<usize>, bool)>> {
    let mut words = Vec::new();
    let offset = within.start;
    tokenizer.words(&text[within], |word, at| {
        words.push((offset + at.start..offset + at.end, query.matches(word)));
        ControlFlow::Continue(())
    })?;
    Ok(words)
}

/// Line breaks and tabs in a snippet read as spaces.
fn spaced(text: &str) -> String {
    text.replace(['\n', '\r', '\t'], " ")
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
