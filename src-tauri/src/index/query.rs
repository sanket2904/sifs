//! What was typed into the search box, parsed once for both searches: terms
//! for names (substrings) and words for contents (as the contents index
//! splits and folds them). Bounded, so no input makes an over-long query.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Terms and words searched, at most; the rest are ignored, and counted.
pub const MAX_WORDS: usize = 16;
/// Characters kept of a term or word. A word cut short matches as a prefix.
pub const MAX_CHARS: usize = 64;
/// The last word matches as a prefix from this many characters.
const PREFIX_FROM: usize = 3;

/// A run of the characters contents_fts's tokenizer (migrate.rs) keeps in
/// words: letters, numbers, marks and private-use characters, except the
/// emoji variation selectors and the keycap mark.
pub(crate) static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[[\p{L}\p{N}\p{Co}\p{M}]--[\x{FE0E}\x{FE0F}\x{20E3}]]+").unwrap());

/// A word of the query, folded.
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
    /// Terms or words past MAX_WORDS, not searched.
    pub ignored: usize,
}

impl Query {
    pub fn parse(typed: &str) -> Query {
        // A pasted tab or NUL separates, as a space does.
        let typed: String = typed.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let (terms, ignored_terms) = first_unique(typed.split_whitespace().map(|t| cut(&t.to_lowercase()).0));
        let folded: Vec<String> = WORD.find_iter(&typed).map(|m| fold(m.as_str())).filter(|w| !w.is_empty()).collect();
        let last = folded.len().saturating_sub(1);
        let mut words: Vec<Word> = Vec::new();
        let mut seen = HashSet::new();
        let mut ignored_words = 0;
        for (i, word) in folded.iter().enumerate() {
            let (text, was_cut) = cut(word);
            let prefix = was_cut || (i == last && text.chars().count() >= PREFIX_FROM);
            if let Some(same) = words.iter_mut().find(|w| w.text == text) {
                same.prefix |= prefix;
            } else if seen.insert(text.clone()) {
                if words.len() < MAX_WORDS {
                    words.push(Word { text, prefix });
                } else {
                    ignored_words += 1;
                }
            }
        }
        Query { terms, words, ignored: ignored_terms.max(ignored_words) }
    }

    /// The FTS5 query over contents: every word, each quoted (a word has no
    /// quote or query syntax in it), prefixes marked. None without words.
    pub fn fts(&self) -> Option<String> {
        if self.words.is_empty() {
            return None;
        }
        let quoted: Vec<String> =
            self.words.iter().map(|w| format!("\"{}\"{}", w.text, if w.prefix { "*" } else { "" })).collect();
        Some(quoted.join(" "))
    }

    /// Whether a folded word of some text is one the query looks for.
    pub(crate) fn matches(&self, folded: &str) -> bool {
        self.words.iter().any(|w| if w.prefix { folded.starts_with(&w.text) } else { folded == w.text })
    }
}

/// The first MAX_WORDS distinct items, and how many distinct ones are left out.
fn first_unique(items: impl Iterator<Item = String>) -> (Vec<String>, usize) {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut ignored = 0;
    for item in items {
        if seen.insert(item.clone()) {
            if kept.len() < MAX_WORDS {
                kept.push(item);
            } else {
                ignored += 1;
            }
        }
    }
    (kept, ignored)
}

/// At most MAX_CHARS characters of s, and whether it was cut.
fn cut(s: &str) -> (String, bool) {
    match s.char_indices().nth(MAX_CHARS) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// A word as contents_fts keeps it: SQLite's unicode61 tokenizer with
/// remove_diacritics 2 lowercases and drops diacritics ("Résumé" and
/// "re\u{301}sume\u{301}" are "resume").
pub fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    fold_into(word, &mut out);
    out
}

/// fold, into a buffer that is reused.
pub(crate) fn fold_into(word: &str, out: &mut String) {
    out.clear();
    for c in word.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else if (c as u32) < 0x10000 {
            match c {
                // SQLite folds case by Unicode's case folding, which
                // differs from lowercasing in these.
                'ς' => out.push('σ'),
                'ſ' => out.push('s'),
                'µ' => out.push('μ'),
                c => out.extend(c.to_lowercase().filter_map(without_diacritic)),
            }
        } else {
            // Beyond the first plane SQLite folds only Deseret.
            let c = if ('\u{10400}'..'\u{10428}').contains(&c) { char::from_u32(c as u32 + 40).unwrap_or(c) } else { c };
            out.push(c);
        }
    }
}

/// SQLite's fts5_remove_diacritic (sqlite3.c 3.46) with bComplex set: a
/// lowercase letter with a diacritic becomes its ASCII letter, and a
/// combining diacritic nothing. Each DIACRITICS entry is a first code point
/// shifted left by 3 plus how many follow it, all mapping to the BASE letter
/// at the same index (0 drops them).
fn without_diacritic(c: char) -> Option<char> {
    let key = ((c as u32) << 3) | 7;
    let i = DIACRITICS.partition_point(|&d| u32::from(d) <= key) - 1;
    let (first, more) = (u32::from(DIACRITICS[i]) >> 3, u32::from(DIACRITICS[i]) & 7);
    match BASE[i] {
        _ if c as u32 > first + more => Some(c),
        0 => None,
        b => Some(char::from(b)),
    }
}

const DIACRITICS: [u16; 126] = [
    0, 1797, 1848, 1859, 1891, 1928, 1940, 1995, 2024, 2040, 2060, 2110, 2168, 2206, 2264, 2286,
    2344, 2383, 2472, 2488, 2516, 2596, 2668, 2732, 2782, 2842, 2894, 2954, 2984, 3000, 3028, 3336,
    3456, 3696, 3712, 3728, 3744, 3766, 3832, 3896, 3912, 3928, 3944, 3968, 4008, 4040, 4056, 4106,
    4138, 4170, 4202, 4234, 4266, 4296, 4312, 4344, 4408, 4424, 4442, 4472, 4488, 4504, 6148, 6198,
    6264, 6280, 6360, 6429, 6505, 6529, 61448, 61468, 61512, 61534, 61592, 61610, 61642, 61672, 61688, 61704,
    61726, 61784, 61800, 61816, 61836, 61880, 61896, 61914, 61948, 61998, 62062, 62122, 62154, 62184, 62200, 62218,
    62252, 62302, 62364, 62410, 62442, 62478, 62536, 62554, 62584, 62604, 62640, 62648, 62656, 62664, 62730, 62766,
    62830, 62890, 62924, 62974, 63032, 63050, 63082, 63118, 63182, 63242, 63274, 63310, 63368, 63390,
];
const BASE: &[u8; 126] =
    b"\0aceinouyyacdeeghijklnorstuuwyzouaiouuagkoojgnaaeiorusthaeoooy\0\0\0\0\0\0\0\0abcddeeefghhiiklllmnoprrrsstuuvwwxyzhtwyaaaaeeeioooouuuy";
