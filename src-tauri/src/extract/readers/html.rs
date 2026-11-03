//! HTML: the text a browser would show. Tags, scripts, styles and comments
//! are dropped, block tags become line breaks, entities are decoded, and
//! the title and author are kept as facts. A `<meta charset>` near the top
//! is honoured before the encoding is guessed.

use crate::extract::decode::{decode, read_capped};
use crate::extract::{fact, ReadError, Reader, Sink};
use encoding_rs::Encoding;
use std::path::Path;

pub struct Html;

impl Reader for Html {
    fn id(&self) -> &'static str {
        "html"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["html", "htm", "xhtml"]
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let bytes = read_capped(path, self.limits().max_bytes)?;
        let decoded = decode(&bytes, meta_charset(&bytes))?;
        if let Some(encoding) = decoded.encoding {
            sink.fact(fact::ENCODING, encoding);
        }
        let page = strip(&decoded.text);
        if let Some(title) = page.title {
            sink.fact(fact::TITLE, title);
        }
        if let Some(author) = page.author {
            sink.fact(fact::AUTHOR, author);
        }
        sink.text(&page.text)?;
        Ok(())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub text: String,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// Their text is not shown.
const HIDDEN: &[&str] = &["script", "style", "noscript", "template"];
/// Each starts a new line.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "ol",
    "option",
    "p",
    "pre",
    "section",
    "table",
    "tr",
    "ul",
];
/// Table cells: kept apart by a space.
const CELL: &[&str] = &["td", "th"];

pub fn strip(html: &str) -> Page {
    // ASCII lowercasing keeps every byte offset, so positions found in
    // lower are positions in html.
    let lower = html.to_ascii_lowercase();
    let n = html.len();
    let mut page = Page::default();
    let mut out = String::with_capacity(n / 2);
    let mut i = 0;
    while i < n {
        let Some(lt) = html[i..].find('<').map(|p| i + p) else {
            push_text(&mut out, &html[i..]);
            break;
        };
        push_text(&mut out, &html[i..lt]);
        i = lt;
        let rest = &lower[i..];
        if rest.starts_with("<!--") {
            i = lower[i + 4..].find("-->").map(|p| i + 4 + p + 3).unwrap_or(n);
            continue;
        }
        if rest.starts_with("<![cdata[") {
            let start = i + 9;
            let end = lower[start..].find("]]>").map(|p| start + p).unwrap_or(n);
            push_literal(&mut out, &html[start..end]);
            i = (end + 3).min(n);
            continue;
        }
        let closing = rest[1..].starts_with('/');
        let name_at = i + 1 + closing as usize;
        let name_len = lower[name_at..].bytes().take_while(|b| b.is_ascii_alphanumeric()).count();
        if name_len == 0 {
            if rest[1..].starts_with(['!', '?']) {
                // A doctype or processing instruction.
                i = tag_end(html, i);
            } else {
                // A "<" that starts no tag is text.
                out.push('<');
                i += 1;
            }
            continue;
        }
        let name = &lower[name_at..name_at + name_len];
        let end = tag_end(html, i);
        if !closing {
            if HIDDEN.contains(&name) {
                i = close_of(html, &lower, name, end).1;
                continue;
            }
            if name == "title" {
                let (close, after) = close_of(html, &lower, name, end);
                let title = collapse(&html_escape::decode_html_entities(&html[end..close]));
                if !title.is_empty() {
                    out.push('\n');
                    out.push_str(&title);
                    out.push('\n');
                    page.title.get_or_insert(title);
                }
                i = after;
                continue;
            }
            if name == "meta"
                && attr(&html[i..end], "name").is_some_and(|v| v.eq_ignore_ascii_case("author"))
                && page.author.is_none()
            {
                page.author = attr(&html[i..end], "content")
                    .map(|v| collapse(&html_escape::decode_html_entities(&v)))
                    .filter(|v| !v.is_empty());
            }
        }
        if BLOCK.contains(&name) {
            out.push('\n');
        } else if CELL.contains(&name) && !out.ends_with([' ', '\n']) {
            out.push(' ');
        }
        i = end;
    }
    page.text = tidy(&out);
    page
}

/// The text between tags: entities decoded, runs of white space made one
/// space, as a browser shows them.
fn push_text(out: &mut String, chunk: &str) {
    if !chunk.is_empty() {
        push_literal(out, &html_escape::decode_html_entities(chunk));
    }
}

fn push_literal(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.ends_with([' ', '\n']) && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lines trimmed, blank ones dropped.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Just past the ">" closing the tag starting at i, skipping quoted values.
fn tag_end(html: &str, i: usize) -> usize {
    let mut quote = None;
    for (j, b) in html.bytes().enumerate().skip(i + 1) {
        match (quote, b) {
            (None, b'"' | b'\'') => quote = Some(b),
            (Some(q), b) if b == q => quote = None,
            (None, b'>') => return j + 1,
            _ => {}
        }
    }
    html.len()
}

/// Where the element named name, whose start tag ends at from, closes:
/// the start of its end tag, and just past it.
fn close_of(html: &str, lower: &str, name: &str, from: usize) -> (usize, usize) {
    match lower[from..].find(&format!("</{name}")) {
        Some(p) => (from + p, tag_end(html, from + p)),
        None => (html.len(), html.len()),
    }
}

/// The value of attribute name in tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let b = lower.as_bytes();
    let skip_space = |mut j: usize| {
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        j
    };
    let mut from = 0;
    while let Some(p) = lower[from..].find(name) {
        let at = from + p;
        from = at + name.len();
        if at == 0 || !b[at - 1].is_ascii_whitespace() {
            continue;
        }
        let j = skip_space(at + name.len());
        if b.get(j) != Some(&b'=') {
            continue;
        }
        let j = skip_space(j + 1);
        let (start, end) = match b.get(j) {
            Some(q @ (b'"' | b'\'')) => {
                let start = j + 1;
                (start, lower[start..].find(*q as char).map_or(b.len(), |e| start + e))
            }
            Some(_) => (j, lower[j..].find(|c: char| c.is_ascii_whitespace() || c == '>').map_or(b.len(), |e| j + e)),
            None => return None,
        };
        return Some(tag[start..end].trim_end_matches('/').to_string());
    }
    None
}

/// The encoding a `<meta charset=…>` or `<meta http-equiv … content="…;
/// charset=…">` in the first 1 KB declares.
pub fn meta_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let head: String = bytes[..bytes.len().min(1024)].iter().map(|b| (*b as char).to_ascii_lowercase()).collect();
    let mut from = 0;
    while let Some(p) = head[from..].find("<meta") {
        let start = from + p;
        let end = head[start..].find('>').map_or(head.len(), |e| start + e);
        from = end;
        let tag = &head[start..end];
        let Some(c) = tag.find("charset") else { continue };
        let value = tag[c + 7..].trim_start().strip_prefix('=').map(|v| v.trim_start().trim_start_matches(['"', '\'']));
        let Some(value) = value else { continue };
        let label: String =
            value.chars().take_while(|c| !matches!(c, '"' | '\'' | ';' | '/' | '>') && !c.is_whitespace()).collect();
        if let Some(encoding) = Encoding::for_label(label.as_bytes()) {
            return Some(encoding);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_scripts_styles_and_comments_are_dropped_and_entities_decoded() {
        let page = strip(
            "<!DOCTYPE html><html><head><title>Rent &amp; Deposit</title>\
             <meta name=\"author\" content=\"Asha Rao\">\
             <style>.secretstyle { color: red }</style>\
             <script>var secretscript = '<p>';</script></head>\
             <body><!-- secretcomment --><h1>Montr&eacute;al</h1><p>It&#8217;s <b>bold</b>ly   spaced\n text.</p>\
             <ul><li>one</li><li>two</li></ul><table><tr><td>a</td><td>b</td></tr></table>3 < 4 &nbsp;ok</body></html>",
        );
        assert_eq!(page.title.as_deref(), Some("Rent & Deposit"));
        assert_eq!(page.author.as_deref(), Some("Asha Rao"));
        assert_eq!(page.text, "Rent & Deposit\nMontréal\nIt\u{2019}s boldly spaced text.\none\ntwo\na b\n3 < 4 ok\n");
        for hidden in ["secretstyle", "secretscript", "secretcomment", "<p>"] {
            assert!(!page.text.contains(hidden), "{hidden} in {:?}", page.text);
        }
    }

    #[test]
    fn attributes_in_quotes_may_hold_a_greater_than_sign() {
        let page = strip("<a title=\"x > y\" href='z'>link</a> after");
        assert_eq!(page.text, "link after\n");
    }

    #[test]
    fn the_declared_charset_is_used_rather_than_a_guess() {
        // Guessing takes this KOI8-R text for KOI8-U.
        let (body, _, _) = encoding_rs::KOI8_R.encode("<meta charset=\"koi8-r\"><p>Аренда квартиры</p>");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rent.html");
        std::fs::write(&path, &body).unwrap();
        let mut sink = Sink::new(1000, std::time::Instant::now() + std::time::Duration::from_secs(10));
        Html.read(&path, &mut sink).unwrap();
        let (parts, facts) = sink.finish();
        assert_eq!(parts[0].text, "Аренда квартиры\n");
        assert_eq!(facts, vec![("encoding".to_string(), "KOI8-R".to_string())]);
    }

    #[test]
    fn the_declared_charset_is_found() {
        assert_eq!(meta_charset(b"<html><head><meta charset=\"windows-1252\">"), Some(encoding_rs::WINDOWS_1252));
        assert_eq!(
            meta_charset(b"<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=ISO-8859-1\">"),
            Some(encoding_rs::WINDOWS_1252)
        );
        assert_eq!(meta_charset(b"<meta charset=utf-8>"), Some(encoding_rs::UTF_8));
        assert_eq!(meta_charset(b"<meta name=\"viewport\" content=\"width=device-width\">"), None);
    }
}
