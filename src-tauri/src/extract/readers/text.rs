//! Plain text in its many forms: notes and logs, markdown, tables (CSV and
//! TSV), source code, and files without an extension (README, Makefile).

use crate::extract::decode::{decode, read_capped};
use crate::extract::{fact, Class, Limits, ReadError, Reader, Sink, MB};
use std::path::Path;
use std::sync::OnceLock;

/// Reads path as text into sink, recording its encoding; returns the text
/// for readers that look for facts in it.
pub(crate) fn read_text(path: &Path, limits: Limits, sink: &mut Sink) -> Result<String, ReadError> {
    let decoded = decode(&read_capped(path, limits.max_bytes)?, None)?;
    if let Some(encoding) = decoded.encoding {
        sink.fact(fact::ENCODING, encoding);
    }
    sink.text(&decoded.text)?;
    Ok(decoded.text)
}

/// Notes, logs, markdown and tables.
pub struct Text;

impl Reader for Text {
    fn id(&self) -> &'static str {
        "text"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["txt", "text", "log", "md", "markdown", "rst", "org", "tex", "csv", "tsv"]
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let text = read_text(path, self.limits(), sink)?;
        match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
            Some("md" | "markdown") => markdown_facts(&text, sink),
            Some("csv") => table_facts(&text, ',', sink),
            Some("tsv") => table_facts(&text, '\t', sink),
            _ => {}
        }
        Ok(())
    }
}

/// Title and author from YAML front matter, or the title from the first
/// "# " heading.
fn markdown_facts(text: &str, sink: &mut Sink) {
    let mut title = None;
    let mut lines = text.lines();
    if text.starts_with("---") && lines.next().map(str::trim_end) == Some("---") {
        for line in lines.by_ref() {
            if matches!(line.trim_end(), "---" | "...") {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
                match key.trim().to_ascii_lowercase().as_str() {
                    "title" => title = Some(value.to_string()),
                    "author" => sink.fact(fact::AUTHOR, value),
                    _ => {}
                }
            }
        }
    }
    let title = title.or_else(|| {
        lines.find_map(|l| l.strip_prefix("# ").map(|t| t.trim().trim_end_matches('#').trim().to_string()))
    });
    if let Some(title) = title {
        sink.fact(fact::TITLE, title);
    }
}

/// Column names from the header row, and the number of rows below it.
fn table_facts(text: &str, separator: char, sink: &mut Sink) {
    let mut rows = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = rows.next() else { return };
    let columns: Vec<String> = split_row(header, separator).into_iter().filter(|c| !c.is_empty()).collect();
    sink.fact(fact::COLUMNS, columns.join(", "));
    sink.fact(fact::LINES, rows.count().to_string());
}

/// The cells of one row, with quotes around a cell removed (a doubled quote
/// inside one is a quote).
fn split_row(row: &str, separator: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == separator && !quoted => cells.push(std::mem::take(&mut cell).trim().to_string()),
            c => cell.push(c),
        }
    }
    cells.push(cell.trim().to_string());
    cells
}

/// Source code and configuration, read last.
pub struct Code;

impl Reader for Code {
    fn id(&self) -> &'static str {
        "code"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        static EXTS: OnceLock<Vec<&'static str>> = OnceLock::new();
        // JSON has its own reader.
        EXTS.get_or_init(|| crate::index::search::CODE.iter().copied().filter(|e| *e != "json").collect())
    }
    fn class(&self) -> Class {
        Class::Other
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        read_text(path, self.limits(), sink).map(|_| ())
    }
}

/// Files without an extension: README, Makefile, LICENSE, and many that
/// aren't text at all, which the binary check skips.
pub struct Plain;

impl Reader for Plain {
    fn id(&self) -> &'static str {
        "plain"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &[""]
    }
    fn class(&self) -> Class {
        Class::Other
    }
    fn limits(&self) -> Limits {
        Limits { max_bytes: MB, ..Limits::default() }
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        read_text(path, self.limits(), sink).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn sink() -> Sink {
        Sink::new(1_000_000, Instant::now() + Duration::from_secs(10))
    }

    fn facts_of(reader: &dyn Reader, name: &str, body: &[u8]) -> (String, Vec<(String, String)>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        let mut s = sink();
        reader.read(&path, &mut s).unwrap();
        let (parts, facts) = s.finish();
        (parts.into_iter().map(|p| p.text).collect(), facts)
    }

    fn fact<'a>(facts: &'a [(String, String)], key: &str) -> Option<&'a str> {
        facts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn markdown_title_and_author_come_from_front_matter() {
        let (text, facts) = facts_of(
            &Text,
            "a.md",
            b"---\ntitle: \"My R\xc3\xa9sum\xc3\xa9\"\nauthor: Sanket\n---\n# Other heading\nbody\n",
        );
        assert_eq!(fact(&facts, "title"), Some("My Résumé"));
        assert_eq!(fact(&facts, "author"), Some("Sanket"));
        assert!(text.contains("body"));
    }

    #[test]
    fn markdown_title_falls_back_to_the_first_heading() {
        let (_, facts) = facts_of(&Text, "b.markdown", b"intro\n\n# Trip to Goa #\n\n## Day 1\n");
        assert_eq!(fact(&facts, "title"), Some("Trip to Goa"));
        assert_eq!(fact(&facts, "author"), None);
    }

    #[test]
    fn csv_and_tsv_record_their_columns_and_rows() {
        let (text, facts) =
            facts_of(&Text, "rent.csv", b"month,\"rent, INR\",city\nJan,25000,Bengaluru\nFeb,25000,Bengaluru\n");
        assert_eq!(fact(&facts, "columns"), Some("month, rent, INR, city"));
        assert_eq!(fact(&facts, "lines"), Some("2"));
        assert!(text.contains("Bengaluru"));
        let (_, facts) = facts_of(&Text, "t.tsv", b"name\tcity\nAsha\tPune\n");
        assert_eq!(fact(&facts, "columns"), Some("name, city"));
        assert_eq!(fact(&facts, "lines"), Some("1"));
    }

    #[test]
    fn a_table_with_old_mac_line_endings_has_its_rows() {
        let (text, facts) = facts_of(&Text, "mac.csv", b"name,city\rAsha,Pune\rRavi,Goa\r");
        assert_eq!(fact(&facts, "columns"), Some("name, city"));
        assert_eq!(fact(&facts, "lines"), Some("2"));
        assert_eq!(text, "name,city\nAsha,Pune\nRavi,Goa\n");
    }

    #[test]
    fn a_huge_header_makes_a_capped_fact() {
        let (_, facts) = facts_of(&Text, "wide.csv", &b"a,".repeat(2 * 1024 * 1024 - 10));
        let columns = fact(&facts, "columns").unwrap();
        assert_eq!(columns.chars().count(), crate::extract::MAX_FACT_CHARS);
    }

    #[test]
    fn files_without_an_extension_are_read_unless_binary() {
        let (text, _) = facts_of(&Plain, "README", b"Build with make.\n");
        assert_eq!(text, "Build with make.\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        std::fs::write(&path, b"\x7fELF\x02\x01\x01\0\0\0\0\0").unwrap();
        assert!(matches!(Plain.read(&path, &mut sink()), Err(ReadError::Binary)));
    }

    #[test]
    fn code_files_are_read_except_json() {
        assert!(Code.exts().contains(&"rs") && Code.exts().contains(&"py"));
        assert!(!Code.exts().contains(&"json"));
        let (text, _) = facts_of(&Code, "main.rs", b"fn main() { quokka(); }\n");
        assert!(text.contains("quokka"));
    }

    #[test]
    fn the_limits_are_the_ones_the_owner_chose() {
        for reader in super::super::all() {
            let limits = reader.limits();
            let max = match reader.id() {
                "json" | "plain" => MB,
                // Only an Office file's XML is unpacked, within caps of its own.
                "docx" | "xlsx" | "pptx" => 100 * MB,
                _ => 4 * MB,
            };
            assert_eq!(limits.max_bytes, max, "{}", reader.id());
            assert_eq!(limits.max_chars, 1_000_000, "{}", reader.id());
            assert_eq!(limits.time, Duration::from_secs(10), "{}", reader.id());
        }
        assert_eq!(Code.class(), Class::Other, "code is read last");
    }
}
