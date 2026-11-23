//! Writes a text set (sample/FORMAT.md, "Input") from what an index has read
//! inside its files, so an extractor can read real indexed text. The index
//! is opened read-only and read in one transaction; the out folder is new
//! or empty, and no file in it is ever replaced.

use super::fact;
use crate::index::db::NOT_REMOVING;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub text: String,
    pub scanned: bool,
}

/// One file of the text set, in FORMAT.md's field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    pub id: String,
    pub file: String,
    /// The index holds no hash, and the exporter reads nothing but the index.
    pub sha256: Option<String>,
    pub format: String,
    pub lang: String,
    pub created: String,
    pub pages: Vec<Page>,
}

#[derive(Serialize)]
struct IndexFile<'a> {
    format: u8,
    text_set: &'a str,
    reference_date: &'a str,
    documents: &'a [String],
}

/// How a reader's parts become pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Parts labelled "page N", only the pages with text; the rest were
    /// scans, counted in the pages fact.
    PdfPages,
    /// One picture, read by OCR.
    Picture,
    /// Every other reader: each part (a sheet, a slide, a section) is a page
    /// with a text layer.
    Parts,
}

impl Layout {
    fn of(reader: &str) -> Layout {
        match reader {
            "pdf" => Layout::PdfPages,
            "image" => Layout::Picture,
            _ => Layout::Parts,
        }
    }
}

/// What the index holds for one file.
#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub modified: i64,
    pub reader: String,
    /// (label, text), in part order.
    pub parts: Vec<(String, String)>,
    /// The PDF's page count, as its reader read it.
    pub page_count: Option<u32>,
    /// The reader stopped taking text part way.
    pub truncated: bool,
}

/// The pages of a file, or none when it has nothing to give.
pub(crate) fn pages(stored: &Stored) -> Result<Vec<Page>> {
    let text_page = |text: &str| Page { text: text.to_string(), scanned: false };
    Ok(match Layout::of(&stored.reader) {
        Layout::Parts => stored.parts.iter().map(|(_, text)| text_page(text)).collect(),
        Layout::Picture if stored.parts.is_empty() => vec![Page { text: String::new(), scanned: true }],
        Layout::Picture => stored.parts.iter().map(|(_, text)| Page { text: text.clone(), scanned: true }).collect(),
        Layout::PdfPages => {
            let mut numbered = Vec::with_capacity(stored.parts.len());
            for (label, text) in &stored.parts {
                let n: u32 = label
                    .strip_prefix("page ")
                    .and_then(|n| n.parse().ok())
                    .filter(|&n| n > 0)
                    .with_context(|| format!("{}: a PDF part labelled {label:?}, not \"page N\"", stored.path))?;
                numbered.push((n, text.as_str()));
            }
            let last_stored = numbered.iter().map(|&(n, _)| n).max().unwrap_or(0);
            // Past a cut, pages were not read for text: they are left out
            // rather than called scans.
            let count = match stored.page_count {
                Some(count) if !stored.truncated => count.max(last_stored),
                _ => last_stored,
            };
            let mut pages = vec![Page { text: String::new(), scanned: true }; count as usize];
            for (n, text) in numbered {
                pages[n as usize - 1] = text_page(text);
            }
            pages
        }
    })
}

/// A file's document id: its name in `[a-z0-9-]`, then a hash of its path.
/// The path is the file's identity in the index (unique, and kept by every
/// rescan and by a rebuilt index), unlike the row id, which is given in
/// crawl order and reused after a delete.
pub(crate) fn doc_id(name: &str, path: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let mut slug = String::new();
    for c in stem.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "file" } else { slug };
    format!("{slug}-{:016x}", fnv1a(path.as_bytes()))
}

/// FNV-1a, 64 bits: fixed by its definition, so ids never change with the
/// Rust release (std's hasher may).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn utc(seconds: i64) -> String {
    let (days, secs) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Whether s is a calendar date written `YYYY-MM-DD`.
pub(crate) fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let number = |r: std::ops::Range<usize>| -> Option<u32> {
        let digits = &s[r];
        digits.bytes().all(|c| c.is_ascii_digit()).then(|| digits.parse().ok())?
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// The index, read-only: nothing this connection does can write it, migrate
/// it or take its write lock.
pub(crate) fn open_read_only(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("open {} read-only", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(60))?;
    Ok(conn)
}

/// Every file the index holds what was read from, in path order.
pub(crate) fn read_stored(conn: &Connection) -> Result<Vec<Stored>> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        bail!("this index has not read inside any file (schema version {version})");
    }
    let mut files = conn.prepare(&format!(
        "SELECT f.id, f.path, f.name, f.ext, f.modified, e.reader FROM files f JOIN extracts e ON e.file_id = f.id
         WHERE e.status IN ('done', 'empty') AND {NOT_REMOVING} ORDER BY f.path"
    ))?;
    let mut parts = conn.prepare("SELECT label, text FROM contents WHERE file_id = ?1 ORDER BY part")?;
    let mut facts = conn.prepare("SELECT key, value FROM facts WHERE file_id = ?1 AND key IN (?2, ?3)")?;
    let rows = files.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
    })?;
    let mut stored = Vec::new();
    for row in rows {
        let (file_id, path, name, ext, modified, reader) = row?;
        let parts = parts
            .query_map(params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<(String, String)>>>()?;
        let (mut page_count, mut truncated) = (None, false);
        for fact in facts.query_map(params![file_id, fact::PAGES, fact::TRUNCATED], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (key, value) = fact?;
            match key.as_str() {
                fact::PAGES => page_count = value.parse().ok(),
                _ => truncated = true,
            }
        }
        stored.push(Stored { path, name, ext, modified, reader, parts, page_count, truncated });
    }
    Ok(stored)
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// Document ids, in index.json's order.
    pub documents: Vec<String>,
}

/// Writes the text set of the index at db into out, which must be absent
/// or an empty folder.
pub fn export(db: &Path, out: &Path, reference_date: &str) -> Result<Exported> {
    if !is_date(reference_date) {
        bail!("the reference date {reference_date:?} is not a date written YYYY-MM-DD");
    }
    if out.exists() {
        let mut entries = std::fs::read_dir(out).with_context(|| format!("{} is not a folder", out.display()))?;
        if entries.next().is_some() {
            bail!("{} is not empty: the text set goes in a new or empty folder", out.display());
        }
    }
    let text_set = out
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("{} has no folder name", out.display()))?
        .to_string();
    let conn = open_read_only(db)?;
    // One read transaction: a snapshot, however the app writes meanwhile.
    let snapshot = conn.unchecked_transaction()?;
    let stored = read_stored(&snapshot)?;
    drop(snapshot);
    drop(conn);
    let mut docs = Vec::with_capacity(stored.len());
    for file in stored {
        let pages = pages(&file)?;
        if !pages.is_empty() {
            docs.push(Document {
                id: doc_id(&file.name, &file.path),
                file: file.name,
                sha256: None,
                format: file.ext,
                lang: String::new(),
                created: utc(file.modified),
                pages,
            });
        }
    }
    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let mut documents = Vec::with_capacity(docs.len());
    for doc in docs {
        write_new(&out.join(format!("{}.json", doc.id)), &doc)?;
        documents.push(doc.id);
    }
    let index = IndexFile { format: 1, text_set: &text_set, reference_date, documents: &documents };
    write_new(&out.join("index.json"), &index)?;
    Ok(Exported { documents })
}

/// Writes value as JSON to a file that must not exist yet: two documents
/// with one id fail here rather than one replacing the other.
fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(value)?;
    json.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(&json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "textset_tests.rs"]
mod tests;
