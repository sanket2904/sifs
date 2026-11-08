//! What was typed into the search box, parsed once for both searches: terms
//! for names (substrings) and words for contents (split and folded by the
//! contents index's own tokenizer). Bounded: at most MAX_BYTES of what was
//! typed are read, so no input makes a long parse or an over-long query.

use super::tokenize::Tokenizer;
use anyhow::Result;
use std::ops::ControlFlow;

/// Bytes of the typed text read; the rest is left out, and reported.
pub const MAX_BYTES: usize = 4096;
/// Terms and words searched, at most; the rest are left out, and reported.
pub const MAX_WORDS: usize = 16;
/// Characters kept of a term or word. A word cut short matches as a prefix.
pub const MAX_CHARS: usize = 64;
/// The last word matches as a prefix from this many characters.
const PREFIX_FROM: usize = 3;

/// A word of the query, as the contents index keeps words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub prefix: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// For names: split at spaces, lowercased, each once.
    pub terms: Vec<String>,
    /// For contents: words as the index has them, each once.
    pub words: Vec<Word>,
    /// Some of what was typed is not searched: past MAX_BYTES, or terms or
    /// words past MAX_WORDS.
    pub left_out: bool,
}

impl Query {
    pub fn parse(typed: &str, tokenizer: &Tokenizer) -> Result<Query> {
        let mut cut = MAX_BYTES.min(typed.len());
        while !typed.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut query = Query { left_out: cut < typed.len(), ..Query::default() };
        let typed = &typed[..cut];
        // A pasted tab or NUL separates, as a space does.
        let spaced: String = typed.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        for term in spaced.split_whitespace() {
            let term = first_chars(&term.to_lowercase()).0.to_string();
            if !query.terms.contains(&term) {
                if query.terms.len() < MAX_WORDS {
                    query.terms.push(term);
                } else {
                    query.left_out = true;
                }
            }
        }
        // The word the last one typed is, if kept: it may match as a prefix.
        let mut last = None;
        tokenizer.words(typed, |folded, _| {
            if folded.is_empty() {
                return ControlFlow::Continue(());
            }
            let (text, was_cut) = first_chars(folded);
            last = match query.words.iter().position(|w| w.text == text) {
                Some(same) => Some(same),
                None if query.words.len() < MAX_WORDS => {
                    query.words.push(Word { text: text.to_string(), prefix: false });
                    Some(query.words.len() - 1)
                }
                None => {
                    query.left_out = true;
                    None
                }
            };
            if let Some(i) = last {
                query.words[i].prefix |= was_cut;
            }
            ControlFlow::Continue(())
        })?;
        if let Some(i) = last {
            query.words[i].prefix |= query.words[i].text.chars().count() >= PREFIX_FROM;
        }
        Ok(query)
    }

    /// The FTS5 query over contents: every word, each quoted (a word as the
    /// index keeps it has no quote or space in it), prefixes marked. None
    /// without words.
    pub fn fts(&self) -> Option<String> {
        if self.words.is_empty() {
            return None;
        }
        let quoted: Vec<String> =
            self.words.iter().map(|w| format!("\"{}\"{}", w.text, if w.prefix { "*" } else { "" })).collect();
        Some(quoted.join(" "))
    }

    /// Whether a word of some text, as the index keeps it, is one the query
    /// looks for.
    pub(crate) fn matches(&self, word: &str) -> bool {
        self.words.iter().any(|w| if w.prefix { word.starts_with(&w.text) } else { word == w.text })
    }
}

/// At most MAX_CHARS characters of s, and whether it was cut.
fn first_chars(s: &str) -> (&str, bool) {
    match s.char_indices().nth(MAX_CHARS) {
        Some((i, _)) => (&s[..i], true),
        None => (s, false),
    }
}
