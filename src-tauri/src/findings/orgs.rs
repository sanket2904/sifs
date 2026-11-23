//! Organisation mentions. A name is an organisation because of the words it
//! is built from (Bank, Ltd, GmbH, 銀行, बैंक) or the field it fills
//! (`Bank: …`), never because a list holds it. A short form takes the key of
//! the full name or code it spells in the same document.

use super::ids::Ident;
use super::{institution, Directory, Line};
use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($s:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($s).unwrap());
        &*R
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    Name,
    Mention,
    Code,
}

impl Settled {
    pub fn as_str(self) -> &'static str {
        match self {
            Settled::Name => "name",
            Settled::Mention => "mention",
            Settled::Code => "code",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Mention {
    pub page: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// Raw key: `name:<match key>`, or a code's institution key.
    pub key: Option<String>,
    pub settled: Option<Settled>,
}

/// The directory's match key (README, "Bank directory"). NFKC is folded only
/// for full-width ASCII, the one compatibility form these scripts print.
pub fn match_key(s: &str) -> String {
    const DROP: &[&str] = &["the", "ltd", "limited", "plc", "inc", "ag", "sa", "nv"];
    let folded: String = s
        .chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap(),
            '\u{3000}' => ' ',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
        .replace("株式会社", "");
    folded
        .split(|c: char| !(c.is_alphanumeric() || is_mark(c)))
        .filter(|w| !w.is_empty() && !DROP.contains(w))
        .collect()
}

fn is_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}' | '\u{0951}'..='\u{0957}' | '\u{0962}'..='\u{0963}' | '\u{3099}'..='\u{309A}')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Word {
    /// Names a kind of organisation: Bank, Insurance, Office.
    Org,
    /// A Romance kind word, whose qualifiers follow it: La Banque Postale.
    OrgRomance,
    Legal,
    /// One word ending in `bank`: Zedbank.
    Compound,
    Connector,
    Other,
}

fn word(t: &str) -> Word {
    let l = t.to_lowercase();
    match l.as_str() {
        "bank" | "union" | "insurance" | "assurance" | "versicherung" | "corporation"
        | "company" | "office" | "school" | "university" | "college" | "sparkasse" => Word::Org,
        "banque" | "banco" | "banca" | "compagnie" | "société" | "caisse" => Word::OrgRomance,
        "ltd" | "limited" | "plc" | "gmbh" | "ag" | "inc" | "llc" | "llp" | "n.v." | "nv"
        | "s.a." | "sa" | "sas" | "sarl" | "kg" | "pvt" | "b.v." | "bv" | "se" | "co" | "corp" => {
            Word::Legal
        }
        "of" | "and" | "&" | "de" | "du" | "des" | "et" | "für" => Word::Connector,
        _ if l.ends_with("bank") && l.chars().count() > 4 => Word::Compound,
        _ => Word::Other,
    }
}

#[derive(Clone, Copy)]
struct Tok {
    s: usize,
    e: usize,
}

fn latin_tokens(t: &str) -> Vec<Tok> {
    re!(r"\p{L}[\p{L}\p{M}\p{N}]*(?:(?:\.\p{L}[\p{L}\p{M}]*)+\.)?|&")
        .find_iter(t)
        .map(|m| Tok {
            s: m.start(),
            e: m.end(),
        })
        .collect()
}

fn capitalised(t: &str) -> bool {
    t.chars().next().is_some_and(char::is_uppercase)
}

/// Runs of capitalised words (and connectors between them) separated only by
/// spaces.
fn runs(t: &str) -> Vec<Vec<Tok>> {
    let mut out: Vec<Vec<Tok>> = Vec::new();
    let mut cur: Vec<Tok> = Vec::new();
    for tok in latin_tokens(t) {
        let w = &t[tok.s..tok.e];
        let joins = cur.last().is_some_and(|p| {
            let gap = &t[p.e..tok.s];
            !gap.is_empty() && gap.chars().all(|c| c == ' ' || c == '\t')
        });
        // A token with digits in it is a code, never part of a name.
        let fits = (capitalised(w) || word(w) == Word::Connector)
            && !w.chars().any(|c| c.is_ascii_digit());
        if !(joins && fits) {
            out.push(std::mem::take(&mut cur));
        }
        if fits {
            cur.push(tok);
        }
    }
    out.push(cur);
    for r in &mut out {
        while r
            .last()
            .is_some_and(|x| word(&t[x.s..x.e]) == Word::Connector)
        {
            r.pop();
        }
        while r
            .first()
            .is_some_and(|x| word(&t[x.s..x.e]) == Word::Connector)
        {
            r.remove(0);
        }
    }
    out.retain(|r| !r.is_empty());
    out
}

/// The organisation name inside a run, as a token range: up to its last
/// kind or legal word, with `of …` after a kind word, legal forms after it,
/// and everything after a Romance kind word.
fn name_in_run(t: &str, run: &[Tok]) -> Option<(usize, usize)> {
    let w: Vec<Word> = run.iter().map(|x| word(&t[x.s..x.e])).collect();
    let anchor = w.iter().rposition(|k| {
        matches!(
            k,
            Word::Org | Word::OrgRomance | Word::Legal | Word::Compound
        )
    })?;
    let mut end = anchor;
    match w[anchor] {
        Word::OrgRomance => end = run.len() - 1,
        Word::Org if anchor + 2 < run.len() && w[anchor + 1] == Word::Connector => {
            end = run.len() - 1
        }
        _ => {}
    }
    while end + 1 < run.len() && w[end + 1] == Word::Legal {
        end += 1;
    }
    // A title-case word before a name set in capitals is not part of it:
    // "The ZED BANK app".
    let caps = |x: &Tok| t[x.s..x.e].chars().all(|c| !c.is_lowercase()) && x.e - x.s >= 2;
    let rest_in_caps = run[1..=end]
        .iter()
        .zip(&w[1..=end])
        .all(|(x, k)| *k == Word::Connector || caps(x))
        && w[1..=end].contains(&Word::Other);
    let start = usize::from(end > 0 && w[0] == Word::Other && !caps(&run[0]) && rest_in_caps);
    w[start..=end]
        .iter()
        .any(|k| matches!(k, Word::Other | Word::Compound))
        .then_some((start, end))
}

pub fn find(lines: &[Line], idents: &[Ident], dir: Option<&Directory>) -> Vec<Mention> {
    let mut out: Vec<Mention> = Vec::new();
    for line in lines {
        let t = line.text.as_str();
        let mut add = |bs: usize, be: usize| {
            let text = t[bs..be].to_string();
            let (s, e) = (
                line.start + super::chars(t, bs),
                line.start + super::chars(t, be),
            );
            if out
                .iter()
                .any(|m| m.page == line.page && m.start < e && s < m.end)
            {
                return;
            }
            out.push(Mention {
                page: line.page,
                start: s,
                end: e,
                key: Some(format!("name:{}", match_key(&text))),
                text,
                settled: Some(Settled::Name),
            });
        };
        for run in runs(t) {
            if let Some((a, b)) = name_in_run(t, &run) {
                add(run[a].s, run[b].e);
            }
        }
        for tok in latin_tokens(t) {
            let w = &t[tok.s..tok.e];
            if !capitalised(w) && word(w) == Word::Compound {
                add(tok.s, tok.e);
            }
        }
        for (s, e) in devanagari_names(t) {
            add(s, e);
        }
        for (s, e) in cjk_names(t) {
            add(s, e);
        }
        for (s, e) in field_names(t) {
            add(s, e);
        }
    }
    short_forms(lines, idents, dir, &mut out);
    out.sort_by_key(|m| (m.page, m.start));
    out
}

/// Devanagari has no capitals: a name is the words before a kind word back
/// to the last delimiter, at most four, with `ऑफ़ …` after it.
fn devanagari_names(t: &str) -> Vec<(usize, usize)> {
    let deva = |w: &str| {
        w.chars()
            .all(|c| ('\u{0900}'..='\u{097F}').contains(&c) && c != '।' && c != '॥')
    };
    let mut out = Vec::new();
    let mut seg_start = 0;
    for seg in t.split_inclusive([':', '：', ',', '।', '(', ')', ';']) {
        let toks: Vec<(usize, &str)> = seg
            .split_whitespace()
            .map(|w| w.trim_end_matches([':', '：', ',', '।', '(', ')', ';']))
            .filter(|w| !w.is_empty())
            .map(|w| (seg_start + (w.as_ptr() as usize - seg.as_ptr() as usize), w))
            .collect();
        seg_start += seg.len();
        let is_kind = |w: &str| w == "बैंक" || w == "लिमिटेड";
        let Some(anchor) = toks.iter().rposition(|(_, w)| is_kind(w)) else {
            continue;
        };
        let mut first = anchor;
        while first > 0 && anchor - first < 4 && deva(toks[first - 1].1) {
            first -= 1;
        }
        let mut last = anchor;
        if toks[anchor].1 == "बैंक"
            && toks
                .get(anchor + 1)
                .is_some_and(|(_, w)| w.starts_with("ऑफ") && w.chars().count() <= 4)
            && toks.get(anchor + 2).is_some_and(|(_, w)| deva(w))
        {
            last = anchor + 2;
        }
        while toks.get(last + 1).is_some_and(|(_, w)| *w == "लिमिटेड") {
            last += 1;
        }
        if first < anchor {
            out.push((toks[first].0, toks[last].0 + toks[last].1.len()));
        }
    }
    out
}

/// Japanese: the kanji, katakana and Latin capitals before 銀行, and a
/// leading kana run when it starts at a delimiter. A 銀行 followed by a colon
/// is a field label.
fn cjk_names(t: &str) -> Vec<(usize, usize)> {
    let core = |c: char| {
        ('\u{4E00}'..='\u{9FFF}').contains(&c)
            || ('\u{30A0}'..='\u{30FF}').contains(&c)
            || c.is_ascii_uppercase()
            || c.is_ascii_digit()
            || ('\u{FF21}'..='\u{FF3A}').contains(&c)
    };
    let kana = |c: char| ('\u{3041}'..='\u{309F}').contains(&c);
    let mut out = Vec::new();
    for m in re!(r"銀行|信用金庫|信用組合").find_iter(t) {
        if t[m.end()..].starts_with([':', '：']) {
            continue;
        }
        let before: Vec<(usize, char)> = t[..m.start()].char_indices().collect();
        let mut i = before.len();
        while i > 0 && core(before[i - 1].1) {
            i -= 1;
        }
        let mut j = i;
        while j > 0 && kana(before[j - 1].1) {
            j -= 1;
        }
        if j < i && (j == 0 || !(core(before[j - 1].1) || before[j - 1].1.is_alphanumeric())) {
            i = j;
        }
        if i < before.len() {
            out.push((before[i].0, m.end()));
        }
    }
    out
}

/// The value of a field whose label is a bank word: `Bank: X`,
/// `Banks involved: X, Y`, `送金銀行: X`.
fn field_names(t: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let labels = [
        re!(r"(?i:\b(?:bank|banks|banque|kreditinstitut)\b)[^:：]{0,40}[:：]\s*"),
        re!(r"(?:銀行|बैंक)\s*[:：]\s*"),
    ];
    for label in labels {
        for m in label.find_iter(t) {
            let rest = &t[m.end()..];
            let rest = &rest[..rest
                .find([':', '：'])
                .map_or(rest.len(), |i| rest[..i].rfind(' ').unwrap_or(0))];
            let mut off = m.end();
            for piece in rest.split([',', '、']) {
                let lead = piece.len() - piece.trim_start().len();
                let p = piece.trim();
                let s = off + lead;
                off += piece.len() + 1;
                let first = p.chars().next();
                let looks = first
                    .is_some_and(|c| c.is_uppercase() || ('\u{3040}'..='\u{9FFF}').contains(&c));
                let words: Vec<&str> = p.split_whitespace().collect();
                // One bare word (`Bank: ZB`) could be any of several banks: it
                // is left to a code or a full name in the document.
                let bare = words.len() == 1 && word(words[0]) == Word::Other;
                if looks && !bare && words.len() <= 6 && !p.chars().any(|c| c.is_ascii_digit()) {
                    out.push((s, s + p.len()));
                }
            }
        }
    }
    out
}

/// The one code key whose first four letters a capitalised word spells.
fn code_key(idents: &[Ident], dir: Option<&Directory>, w: &str) -> Option<String> {
    if w.len() != 4 || !w.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let mut keys: Vec<String> = idents
        .iter()
        .filter(|x| x.kind.is_bank_code() && x.value.starts_with(w))
        .filter_map(|x| institution(dir, x))
        .collect();
    keys.sort();
    keys.dedup();
    (keys.len() == 1).then(|| keys.remove(0))
}

fn words_of(text: &str) -> Vec<String> {
    latin_tokens(text)
        .iter()
        .map(|x| &text[x.s..x.e])
        .filter(|w| !matches!(word(w), Word::Legal | Word::Connector))
        .map(str::to_lowercase)
        .collect()
}

fn initials(words: &[String]) -> String {
    words.iter().filter_map(|w| w.chars().next()).collect()
}

/// `ZEDB` beside `ZEDB0…` takes the code's key; `Zed Life` before
/// `Zed Life Assurance Company Limited`, and `ZQC Ltd` before the three
/// words it abbreviates, take the full name's key. A form that fits names of
/// two organisations stays unsettled.
fn short_forms(lines: &[Line], idents: &[Ident], dir: Option<&Directory>, out: &mut Vec<Mention>) {
    let full: Vec<(Vec<String>, String)> = out
        .iter()
        .filter_map(|m| Some((words_of(&m.text), m.key.clone()?)))
        .filter(|(w, _)| w.len() >= 2)
        .collect();
    let only = |keys: Vec<&String>| -> Option<String> {
        let mut k = keys;
        k.sort();
        k.dedup();
        (k.len() == 1).then(|| k[0].clone())
    };
    let by_initials = |w: &str| {
        only(
            full.iter()
                .filter(|(f, _)| initials(f) == w)
                .map(|(_, k)| k)
                .collect(),
        )
    };

    for m in out.iter_mut() {
        let w = words_of(&m.text);
        if w.len() == 1 && w[0].chars().count() >= 2 {
            if let Some(k) = by_initials(&w[0]).filter(|k| Some(k) != m.key.as_ref()) {
                m.key = Some(k);
                m.settled = Some(Settled::Mention);
            }
        }
    }

    let mut extra = Vec::new();
    for line in lines {
        let t = line.text.as_str();
        let free = |s: usize, e: usize| {
            let (s, e) = (
                line.start + super::chars(t, s),
                line.start + super::chars(t, e),
            );
            !out.iter()
                .any(|m| m.page == line.page && m.start < e && s < m.end)
                && !idents
                    .iter()
                    .any(|x| x.page == line.page && x.start < e && s < x.end)
        };
        let mut push = |s: usize, e: usize, key: String, by: Settled| {
            extra.push(Mention {
                page: line.page,
                start: line.start + super::chars(t, s),
                end: line.start + super::chars(t, e),
                text: t[s..e].to_string(),
                key: Some(key),
                settled: Some(by),
            })
        };
        for run in runs(t) {
            let (s, e) = (run[0].s, run[run.len() - 1].e);
            if !free(s, e) {
                continue;
            }
            if run.len() > 1 {
                for x in &run {
                    if let Some(k) = code_key(idents, dir, &t[x.s..x.e]) {
                        push(x.s, x.e, k, Settled::Code);
                    }
                }
            }
            let text = &t[s..e];
            if run.len() == 1 && text.len() >= 2 && text.chars().all(|c| c.is_ascii_uppercase()) {
                if let Some(k) = code_key(idents, dir, text) {
                    push(s, e, k, Settled::Code);
                } else if let Some(k) = by_initials(&text.to_lowercase()) {
                    push(s, e, k, Settled::Mention);
                }
                continue;
            }
            let w = words_of(text);
            if w.len() >= 2 {
                let prefix = only(
                    full.iter()
                        .filter(|(f, _)| f.len() > w.len() && f[..w.len()] == w[..])
                        .map(|(_, k)| k)
                        .collect(),
                );
                if let Some(k) = prefix {
                    push(s, e, k, Settled::Mention);
                }
            }
        }
    }
    out.extend(extra);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::{ids, split_lines, Page};

    fn mentions(text: &str) -> Vec<(String, Option<String>, Option<Settled>)> {
        let lines = split_lines(&[Page {
            text: text.into(),
            scanned: false,
        }]);
        let idents = ids::find(&lines);
        find(&lines, &idents, None)
            .into_iter()
            .map(|m| (m.text, m.key, m.settled))
            .collect()
    }

    fn texts(text: &str) -> Vec<String> {
        mentions(text).into_iter().map(|m| m.0).collect()
    }

    #[test]
    fn key_folds_case_legal_words_and_width() {
        assert_eq!(match_key("Acme Bank Ltd"), "acmebank");
        assert_eq!(match_key("ACME BANK"), "acmebank");
        assert_eq!(match_key("acmebank"), "acmebank");
        assert_eq!(match_key("ＡＣＭＥ　銀行"), "acme銀行");
        assert_eq!(match_key("株式会社 例銀行"), "例銀行");
    }

    #[test]
    fn whole_names_by_kind_and_legal_words() {
        assert_eq!(
            texts("Zed Widget Bank Credit Card Statement"),
            ["Zed Widget Bank"]
        );
        assert_eq!(texts("Branch Manager, Zed Bank Ltd"), ["Zed Bank Ltd"]);
        assert_eq!(texts("Fees at Bank of Zedland today"), ["Bank of Zedland"]);
        assert_eq!(
            texts("Titulaire : La Banque Zédoise"),
            ["La Banque Zédoise"]
        );
        assert_eq!(texts("Zed Holdings N.V."), ["Zed Holdings N.V."]);
        assert_eq!(texts("Paid via Zedbank today"), ["Zedbank"]);
        assert_eq!(texts("The ZED BANK app"), ["ZED BANK"]);
        assert_eq!(texts("Zedbank AG"), ["Zedbank AG"]);
        assert_eq!(
            texts("IFSC: ZEDB0XYZ123 Bank: Zed Bank"),
            ["Zed Bank"],
            "a code is no name word"
        );
        assert!(texts("Bank: ").is_empty(), "a kind word alone is no name");
        assert!(texts("Pay at any branch of these banks").is_empty());
    }

    #[test]
    fn field_value_is_a_name() {
        assert_eq!(
            texts("Banks involved: Zedcorp Alpha, Beta Gamma"),
            ["Zedcorp Alpha", "Beta Gamma"]
        );
        assert_eq!(texts("送金銀行: ZED ALPHA"), ["ZED ALPHA"]);
        assert!(
            texts("Bank: ZB").is_empty(),
            "one bare word is not settled by its name"
        );
    }

    #[test]
    fn devanagari_and_japanese_names() {
        assert_eq!(texts("प्रबंधक, ज़ेड बैंक"), ["ज़ेड बैंक"]);
        assert_eq!(
            texts("नियोक्ता: ज़ेड टेक प्राइवेट लिमिटेड"),
            ["ज़ेड टेक प्राइवेट लिमिटेड"]
        );
        assert_eq!(
            texts("お問い合わせは例信託銀行ではなく、例銀行へ"),
            ["例信託銀行", "例銀行"]
        );
        assert_eq!(texts("ぜっと銀行"), ["ぜっと銀行"]);
        assert!(texts("受取銀行:").is_empty(), "a label");
    }

    #[test]
    fn code_settles_its_four_letters() {
        let m = mentions("ZEDB online\nAccount: 99887766 IFSC: ZEDB0XYZ123");
        assert_eq!(
            m[0],
            ("ZEDB".into(), Some("ifsc:ZEDB".into()), Some(Settled::Code))
        );
        let m = mentions("ZEDB NetPay receipt\nAccount: 99887766 IFSC: ZEDB0XYZ123");
        assert_eq!(m[0].0, "ZEDB", "inside a longer run of capitals");
    }

    #[test]
    fn short_form_and_initials_take_the_full_names_key() {
        let m = mentions("Zed Life Assurance Company Limited\nThanks from Zed Life.");
        assert_eq!(
            m[1],
            (
                "Zed Life".into(),
                Some("name:zedlifeassurancecompany".into()),
                Some(Settled::Mention)
            )
        );
        let m = mentions("ZQC Ltd\nZedPort Quay Corporation Limited");
        assert_eq!(m[0].1, Some("name:zedportquaycorporation".into()));
        assert_eq!(m[0].2, Some(Settled::Mention));
    }

    #[test]
    fn short_form_of_two_organisations_stays_unsettled() {
        let m = mentions("Zed Alpha Bank\nZed Alpha Insurance Company\nZed Alpha online.");
        assert_eq!(m.len(), 2, "{m:?}");
    }

    #[test]
    fn bare_acronym_without_tie_is_no_mention() {
        assert!(mentions("Moved money to ZB for the deposit.").is_empty());
    }
}
