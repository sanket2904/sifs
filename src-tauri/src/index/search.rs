//! Name search: case-insensitive substring matching through the trigram
//! index, a typo-tolerant fallback, and ranking by match quality and
//! recency.

use super::db::{row_to_record, FileRecord, Index};
use anyhow::Result;
use std::collections::HashSet;

/// One result: the file and why it ranked where it did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub file: FileRecord,
    pub score: f64,
    pub fuzzy: bool,
}

const COLUMNS: &str = "f.path, f.root, f.name, f.ext, f.is_dir, f.size, f.modified, f.inode";
/// Candidates fetched before ranking.
const CANDIDATES: usize = 5000;

/// Searches names. now is the current time in Unix seconds, for recency.
pub fn search(index: &Index, query: &str, limit: usize, now: i64) -> Result<Vec<Hit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    let mut hits: Vec<Hit> = candidates(index, &terms)?
        .into_iter()
        .map(|file| {
            let score = rank(&file, &terms, now);
            Hit { file, score, fuzzy: false }
        })
        .collect();
    if hits.len() < limit && query.chars().count() >= 4 {
        let seen: HashSet<String> = hits.iter().map(|h| h.file.path.clone()).collect();
        for (file, shared) in fuzzy_candidates(index, &query.to_lowercase())? {
            if !seen.contains(&file.path) {
                let score = fuzzy_rank(&file, shared, now);
                hits.push(Hit { file, score, fuzzy: true });
            }
        }
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.file.path.cmp(&b.file.path)));
    hits.truncate(limit);
    Ok(hits)
}

/// Files whose name contains every term.
fn candidates(index: &Index, terms: &[String]) -> Result<Vec<FileRecord>> {
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
    for term in &short {
        params.push(format!("%{}%", escape_like(term)));
        sql.push_str(&format!(" AND f.name LIKE ?{} ESCAPE '\\'", params.len()));
    }
    sql.push_str(&format!(" LIMIT {CANDIDATES}"));
    let mut stmt = index.conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_record)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Files sharing trigrams with the query, for a misspelt one, with how many
/// of its trigrams each shares.
fn fuzzy_candidates(index: &Index, query: &str) -> Result<Vec<(FileRecord, usize)>> {
    let grams = trigrams(query);
    if grams.is_empty() {
        return Ok(Vec::new());
    }
    let phrase = grams.iter().map(|g| format!("\"{}\"", g.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ");
    let sql = format!("SELECT {COLUMNS} FROM files_fts JOIN files f ON f.id = files_fts.rowid WHERE files_fts MATCH ?1 LIMIT {CANDIDATES}");
    let mut stmt = index.conn.prepare_cached(&sql)?;
    let rows = stmt.query_map([phrase], row_to_record)?;
    let mut out = Vec::new();
    for row in rows {
        let file = row?;
        let name = file.name.to_lowercase();
        let shared = grams.iter().filter(|g| name.contains(g.as_str())).count();
        // At least half the query's trigrams, so a few common ones don't match.
        if shared * 2 >= grams.len() {
            out.push((file, shared));
        }
    }
    Ok(out)
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
fn rank(file: &FileRecord, terms: &[String], now: i64) -> f64 {
    let name = file.name.to_lowercase();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 && !file.is_dir => &name[..i],
        _ => name.as_str(),
    };
    let whole = terms.join(" ");
    let mut score = if stem == whole || name == whole {
        100.0
    } else if name.starts_with(&whole) {
        70.0
    } else if terms.iter().all(|t| starts_word(&name, t)) {
        50.0
    } else {
        25.0
    };
    score += kind_prior(file) + place_prior(&file.path);
    score += recency(file.modified, now);
    score -= (name.chars().count() as f64) * 0.05;
    score -= (file.path.matches('/').count() as f64) * 0.3;
    score
}

/// How likely a person searching is to want this kind of file.
pub(crate) fn kind_prior(file: &FileRecord) -> f64 {
    if file.is_dir {
        return 5.0;
    }
    match file.ext.as_str() {
        "pdf" | "doc" | "docx" | "pages" | "rtf" | "odt" | "txt" | "md" | "epub" | "xls" | "xlsx" | "numbers" | "csv"
        | "ods" | "ppt" | "pptx" | "key" | "odp" | "eml" | "msg" => 40.0,
        "jpg" | "jpeg" | "png" | "heic" | "gif" | "webp" | "tiff" | "raw" | "mp4" | "mov" | "mkv" | "avi" | "mp3"
        | "m4a" | "wav" | "flac" | "zip" | "dmg" | "app" => 15.0,
        "o" | "d" | "dia" | "swiftdeps" | "swiftmodule" | "pyc" | "class" | "obj" | "a" | "dylib" | "so" | "map"
        | "lock" | "pcm" | "hmap" => -50.0,
        "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "swift" | "go" | "rs" | "java" | "kt" | "c" | "h" | "cc"
        | "cpp" | "hpp" | "m" | "mm" | "rb" | "php" | "cs" | "dart" | "yaml" | "yml" | "json" | "xml" | "plist"
        | "toml" | "ini" | "cfg" | "sh" | "sql" | "proto" | "gradle" | "css" | "scss" => -30.0,
        _ => 0.0,
    }
}

/// Paths inside SDKs, dependencies and build products: rarely what a
/// person is looking for, though they may live in a folder they chose.
pub(crate) fn place_prior(path: &str) -> f64 {
    const NOISE: &[&str] = &[
        "/build/", "/dist/", "/out/", "/vendor/", "/third_party/", "/third-party/", "/site-packages/", "/checkouts/",
        "/intermediates", ".noindex/", "/deriveddata", "/sourcepackages/", "-sdk/", "/sdk/", "/sdks/", "/lib/",
        "/libs/", "/cache/", "/caches/", "/tmp/", "/target/", "/pods/", "/bin/",
    ];
    let lower = path.to_lowercase();
    if NOISE.iter().any(|marker| lower.contains(marker)) {
        -40.0
    } else {
        0.0
    }
}

fn fuzzy_rank(file: &FileRecord, shared: usize, now: i64) -> f64 {
    10.0 + shared as f64 * 2.0 + recency(file.modified, now) * 0.5 - (file.name.chars().count() as f64) * 0.05
}

fn recency(modified: i64, now: i64) -> f64 {
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
