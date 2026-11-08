//! Name search: case-insensitive substring matching through the trigram
//! index, a typo-tolerant fallback, and ranking by match quality and
//! recency.

use super::db::{row_to_record, FileRecord, Index, NOT_REMOVING};
use super::query::Query;
use super::stop::{stopped, Stop, Until};
use super::tokenize::Tokenizer;
use anyhow::Result;
use regex::Regex;
use rusqlite::Row;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;

/// One result: the file and why it ranked where it did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub file: FileRecord,
    pub score: f64,
    pub fuzzy: bool,
    /// kind_prior plus place_prior, part of score.
    #[serde(skip)]
    pub(crate) prior: f64,
}

const COLUMNS: &str = "f.path, f.root, f.name, f.ext, f.is_dir, f.size, f.modified, f.inode, f.modified_ns";
/// Name matches kept, the best by the caller's order; past it the search
/// is marked capped.
pub const CANDIDATES: usize = 5000;
/// How long a name search may take: past it, it stops with what it found.
pub const NAME_TIME: Duration = Duration::from_millis(400);

/// The name matches a search kept.
#[derive(Debug, Default)]
pub(crate) struct Names {
    /// Best first.
    pub hits: Vec<Hit>,
    /// More than CANDIDATES names matched: only the best were kept.
    pub capped: bool,
    /// The search stopped before every name was looked at.
    pub timed_out: bool,
}

/// Searches names. now is the current time in Unix seconds, for recency.
pub fn search(index: &Index, query: &str, limit: usize, now: i64) -> Result<Vec<Hit>> {
    search_until(index, query, limit, now, &Until::after(NAME_TIME))
}

/// search, stopping when until passes, with what it found.
pub fn search_until(index: &Index, query: &str, limit: usize, now: i64, until: &Until) -> Result<Vec<Hit>> {
    let query = Query::parse(query, &mut Tokenizer::contents(&index.conn)?)?;
    let _stop = Stop::at(&index.conn, until);
    let mut hits = matches(index, &query, now, until, Order::Score)?.hits;
    hits.truncate(limit);
    Ok(hits)
}

/// The order name matches are kept and returned in, best first; ties go by path.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Order {
    /// By score: the names-only search.
    Score,
    /// By kind and place, then exact before close names, then score: as
    /// search over names and contents ranks.
    Tiers,
}

/// What a match is ordered by, compared from the first number; higher is better.
type Key = [f64; 3];

impl Order {
    fn key(self, prior: f64, fuzzy: bool, score: f64) -> Key {
        match self {
            Order::Score => [score, 0.0, 0.0],
            Order::Tiers => [prior, if fuzzy { -1.0 } else { 0.0 }, score],
        }
    }

    /// The best key an exact match of this kind and recency can have:
    /// place_prior and the length penalties only lower it, and match
    /// quality adds at most 100.
    fn best_key(self, kind: f64, recency: f64) -> Key {
        match self {
            Order::Score => [kind + recency + 100.0, 0.0, 0.0],
            Order::Tiers => [kind, 0.0, f64::INFINITY],
        }
    }
}

fn better(a: (&Key, &str), b: (&Key, &str)) -> Ordering {
    let by_key = (0..3).map(|i| b.0[i].total_cmp(&a.0[i])).find(|o| o.is_ne()).unwrap_or(Ordering::Equal);
    by_key.then_with(|| a.1.cmp(b.1))
}

/// The CANDIDATES best files in order whose name matches: those containing
/// every term, and, unless more than CANDIDATES do, those whose names are
/// close. Stops when until passes, with what it found (the caller holds a
/// Stop for it).
pub(crate) fn matches(index: &Index, query: &Query, now: i64, until: &Until, order: Order) -> Result<Names> {
    let terms = &query.terms;
    if terms.is_empty() {
        return Ok(Names::default());
    }
    // A statement that finishes within SQLite's first check is never stopped.
    if until.passed() {
        return Ok(Names { timed_out: true, ..Names::default() });
    }
    let whole = terms.join(" ");
    let mut best = Best { keep: CANDIDATES, items: Vec::new(), worst: None, dropped: false };
    let finished = each_candidate(index, terms, |row| {
        let file = Fields::of(row)?;
        let kind = kind_prior(file.is_dir, file.ext);
        if best.beats(&order.best_key(kind, recency(file.modified, now))) {
            best.dropped = true;
            return Ok(());
        }
        let prior = kind + place_prior(file.path);
        let score = rank(file, terms, &whole, prior, now);
        best.offer(order.key(prior, false, score), file.path, || Ok(Hit { file: row_to_record(row)?, score, fuzzy: false, prior }))
    })?;
    let mut timed_out = !finished;
    if finished && !best.dropped && whole.chars().count() >= 4 {
        // Every exact match is kept so far, so every one is in best.
        let exact: HashSet<String> = best.items.iter().map(|(_, h)| h.file.path.clone()).collect();
        timed_out = until.passed()
            || !each_close_name(index, &whole, |row, shared| {
                let file = Fields::of(row)?;
                if exact.contains(file.path) {
                    return Ok(());
                }
                let prior = kind_prior(file.is_dir, file.ext) + place_prior(file.path);
                let score = fuzzy_rank(file, shared, now);
                best.offer(order.key(prior, true, score), file.path, || Ok(Hit { file: row_to_record(row)?, score, fuzzy: true, prior }))
            })?;
    }
    let (hits, capped) = best.into_sorted();
    Ok(Names { hits, capped, timed_out })
}

/// The score a file whose name contains every term has in the name search;
/// None if it doesn't. For a file found another way, which may be one of
/// many names left out of the CANDIDATES best.
pub(crate) fn name_score(file: &FileRecord, terms: &[String], prior: f64, now: i64) -> Option<f64> {
    let name = file.name.to_lowercase();
    if terms.is_empty() || !terms.iter().all(|t| name.contains(t.as_str())) {
        return None;
    }
    let fields = Fields { path: &file.path, name: &file.name, ext: &file.ext, is_dir: file.is_dir, modified: file.modified };
    Some(rank(fields, terms, &terms.join(" "), prior, now))
}

/// The keep best matches offered, in memory for at most twice as many.
struct Best {
    keep: usize,
    items: Vec<(Key, Hit)>,
    /// The last one kept at the last trim: one no better is not kept.
    worst: Option<(Key, String)>,
    /// A match was not kept, for keep better ones.
    dropped: bool,
}

impl Best {
    /// Whether every match kept now is better than one with this key.
    fn beats(&self, key: &Key) -> bool {
        self.worst.as_ref().is_some_and(|(worst, _)| better((worst, ""), (key, "")).is_lt())
    }

    /// Keeps the match make makes if it may be among the best, without
    /// making it otherwise.
    fn offer(&mut self, key: Key, path: &str, make: impl FnOnce() -> Result<Hit>) -> Result<()> {
        if self.worst.as_ref().is_some_and(|(k, p)| better((&key, path), (k, p)).is_ge()) {
            self.dropped = true;
            return Ok(());
        }
        self.items.push((key, make()?));
        if self.items.len() >= 2 * self.keep {
            self.trim();
        }
        Ok(())
    }

    fn trim(&mut self) {
        if self.items.len() > self.keep {
            let last = self.keep - 1;
            self.items.select_nth_unstable_by(last, |a, b| better((&a.0, &a.1.file.path), (&b.0, &b.1.file.path)));
            self.items.truncate(self.keep);
            self.worst = Some((self.items[last].0, self.items[last].1.file.path.clone()));
            self.dropped = true;
        }
    }

    /// Best first, and whether any were dropped.
    fn into_sorted(mut self) -> (Vec<Hit>, bool) {
        self.trim();
        self.items.sort_by(|a, b| better((&a.0, &a.1.file.path), (&b.0, &b.1.file.path)));
        (self.items.into_iter().map(|(_, hit)| hit).collect(), self.dropped)
    }
}

/// What ranking reads of a file, borrowed from its row (COLUMNS).
#[derive(Debug, Clone, Copy)]
struct Fields<'a> {
    path: &'a str,
    name: &'a str,
    ext: &'a str,
    is_dir: bool,
    modified: i64,
}

impl<'a> Fields<'a> {
    fn of(row: &'a Row) -> Result<Fields<'a>> {
        Ok(Fields {
            path: row.get_ref(0)?.as_str()?,
            name: row.get_ref(2)?.as_str()?,
            ext: row.get_ref(3)?.as_str()?,
            is_dir: row.get(4)?,
            modified: row.get(6)?,
        })
    }
}

/// Calls each with every row the statement returns. Ok(false) if the
/// statement was stopped first.
fn each_row(index: &Index, sql: &str, params: &[String], mut each: impl FnMut(&Row) -> Result<()>) -> Result<bool> {
    let mut stmt = index.conn.prepare_cached(sql)?;
    let mut rows = match stmt.query(rusqlite::params_from_iter(params)) {
        Err(e) if stopped(&e) => return Ok(false),
        rows => rows?,
    };
    loop {
        match rows.next() {
            Ok(Some(row)) => each(row)?,
            Ok(None) => return Ok(true),
            Err(e) if stopped(&e) => return Ok(false),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Calls each with every file (COLUMNS) whose name contains every term.
/// Ok(false) if stopped first.
fn each_candidate(index: &Index, terms: &[String], each: impl FnMut(&Row) -> Result<()>) -> Result<bool> {
    let (long, short): (Vec<&String>, Vec<&String>) = terms.iter().partition(|t| t.chars().count() >= 3);
    let mut params: Vec<String> = Vec::new();
    let mut sql = if long.is_empty() {
        format!("SELECT {COLUMNS} FROM files f WHERE 1")
    } else {
        // Each term a quoted phrase: the trigram tokenizer matches it as a
        // case-insensitive substring of the name.
        let phrase = long.iter().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" ");
        params.push(phrase);
        format!("SELECT {COLUMNS} FROM files_fts JOIN files f ON f.id = files_fts.rowid WHERE files_fts MATCH ?1")
    };
    sql.push_str(&format!(" AND {NOT_REMOVING}"));
    for term in &short {
        params.push(format!("%{}%", escape_like(term)));
        sql.push_str(&format!(" AND f.name LIKE ?{} ESCAPE '\\'", params.len()));
    }
    each_row(index, &sql, &params, each)
}

/// Calls each with every file (COLUMNS) sharing at least half of the
/// query's trigrams, for a misspelt one, with how many it shares.
/// Ok(false) if stopped first.
fn each_close_name(index: &Index, query: &str, mut each: impl FnMut(&Row, usize) -> Result<()>) -> Result<bool> {
    let grams = trigrams(query);
    if grams.is_empty() {
        return Ok(true);
    }
    let phrase = grams.iter().map(|g| format!("\"{}\"", g.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ");
    let sql = format!("SELECT {COLUMNS} FROM files_fts JOIN files f ON f.id = files_fts.rowid WHERE files_fts MATCH ?1 AND {NOT_REMOVING}");
    each_row(index, &sql, &[phrase], |row| {
        let name = Fields::of(row)?.name.to_lowercase();
        let shared = grams.iter().filter(|g| name.contains(g.as_str())).count();
        // At least half the query's trigrams, so a few common ones don't match.
        if shared * 2 >= grams.len() {
            each(row, shared)?;
        }
        Ok(())
    })
}

fn trigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for w in chars.windows(3) {
        let g: String = w.iter().collect();
        if !g.trim().is_empty() && seen.insert(g.clone()) {
            out.push(g);
        }
    }
    out
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Match quality, the kind of file and where it lives, then recency, then
/// shorter names and paths. A person searching their files means their
/// documents far more often than a same-named source file in an SDK or a
/// build product: "resume" should find a resume, not resume.py.
fn rank(file: Fields, terms: &[String], whole: &str, prior: f64, now: i64) -> f64 {
    let name = file.name.to_lowercase();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 && !file.is_dir => &name[..i],
        _ => name.as_str(),
    };
    let mut score = if stem == whole || name == whole {
        100.0
    } else if name.starts_with(whole) {
        70.0
    } else if terms.iter().all(|t| starts_word(&name, t)) {
        50.0
    } else {
        25.0
    };
    score += prior;
    score += recency(file.modified, now);
    score -= (name.chars().count() as f64) * 0.05;
    score -= (file.path.matches('/').count() as f64) * 0.3;
    score
}

/// Source code and configuration: ranked below documents, and read last.
pub const CODE: &[&str] = &[
    "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "swift", "go", "rs", "java", "kt", "c", "h", "cc", "cpp", "hpp", "m",
    "mm", "rb", "php", "cs", "dart", "yaml", "yml", "json", "xml", "plist", "toml", "ini", "cfg", "sh", "sql", "proto",
    "gradle", "css", "scss",
];

/// How likely a person searching is to want this kind of file.
pub(crate) fn kind_prior(is_dir: bool, ext: &str) -> f64 {
    if is_dir {
        return 5.0;
    }
    match ext {
        "pdf" | "doc" | "docx" | "pages" | "rtf" | "odt" | "txt" | "md" | "epub" | "xls" | "xlsx" | "numbers" | "csv"
        | "ods" | "ppt" | "pptx" | "key" | "odp" | "eml" | "msg" => 40.0,
        "jpg" | "jpeg" | "png" | "heic" | "gif" | "webp" | "tiff" | "raw" | "mp4" | "mov" | "mkv" | "avi" | "mp3"
        | "m4a" | "wav" | "flac" | "zip" | "dmg" | "app" => 15.0,
        "o" | "d" | "dia" | "swiftdeps" | "swiftmodule" | "pyc" | "class" | "obj" | "a" | "dylib" | "so" | "map"
        | "lock" | "pcm" | "hmap" => -50.0,
        ext if CODE.contains(&ext) => -30.0,
        _ => 0.0,
    }
}

/// Paths inside SDKs, dependencies and build products: rarely what a
/// person is looking for, though they may live in a folder they chose.
pub(crate) fn place_prior(path: &str) -> f64 {
    if NOISE.is_match(path) {
        -40.0
    } else {
        0.0
    }
}

/// Any of these, ignoring case, in one pass: place_prior runs on every name
/// that matches.
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    const MARKERS: &[&str] = &[
        "/build/", "/dist/", "/out/", "/vendor/", "/third_party/", "/third-party/", "/site-packages/", "/checkouts/",
        "/intermediates", ".noindex/", "/deriveddata", "/sourcepackages/", "-sdk/", "/sdk/", "/sdks/", "/lib/",
        "/libs/", "/cache/", "/caches/", "/tmp/", "/target/", "/pods/", "/bin/",
    ];
    let any: Vec<String> = MARKERS.iter().map(|m| regex::escape(m)).collect();
    Regex::new(&format!("(?i){}", any.join("|"))).unwrap()
});

fn fuzzy_rank(file: Fields, shared: usize, now: i64) -> f64 {
    10.0 + shared as f64 * 2.0 + recency(file.modified, now) * 0.5 - (file.name.chars().count() as f64) * 0.05
}

pub(crate) fn recency(modified: i64, now: i64) -> f64 {
    let days = (now - modified).max(0) as f64 / 86_400.0;
    if days < 7.0 {
        15.0
    } else if days < 30.0 {
        10.0
    } else if days < 365.0 {
        5.0
    } else {
        0.0
    }
}

/// Whether term starts a word of name: at its start or after a separator.
fn starts_word(name: &str, term: &str) -> bool {
    let mut from = 0;
    while let Some(i) = name[from..].find(term) {
        let at = from + i;
        if at == 0 || name[..at].ends_with(|c: char| !c.is_alphanumeric()) {
            return true;
        }
        from = at + name[at..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    false
}
