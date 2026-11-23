//! Identifiers: each scheme's shape, the label that must stand beside it
//! where the shape alone is too common, and its check where it has one.

use super::Line;
use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($s:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($s).unwrap());
        &*R
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Iban,
    Bic,
    Card,
    Ifsc,
    InPan,
    Mrz,
    GbSortCode,
    FrBankCode,
    FrBranchCode,
    FrRibKey,
    Account,
    PassportNumber,
}

impl Kind {
    /// Holds money: what a statement is about and what "an account of its
    /// own" means in the paired-code rule.
    pub fn is_account(self) -> bool {
        matches!(self, Kind::Iban | Kind::Account | Kind::Card)
    }
    /// Carries its institution in its first four letters.
    pub fn is_bank_code(self) -> bool {
        matches!(self, Kind::Ifsc | Kind::Bic)
    }
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub line: usize,
    pub page: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: Kind,
    pub value: String,
    pub valid: Option<bool>,
}

impl Ident {
    /// The organisation a code names by itself, as a raw key.
    pub fn institution_key(&self) -> Option<String> {
        if self.valid == Some(false) {
            return None;
        }
        match self.kind {
            Kind::Ifsc => Some(format!("ifsc:{}", &self.value[..4])),
            Kind::Bic => Some(format!("bic:{}", &self.value[..6])),
            Kind::Iban => {
                let l = iban_layout(&self.value[..2])?;
                Some(format!(
                    "iban:{}:{}",
                    &self.value[..2],
                    &self.value[l.bank.0..l.bank.1]
                ))
            }
            _ => None,
        }
    }
}

/// Where the parts of a country's IBAN sit, as ranges of the value.
pub struct IbanLayout {
    pub len: usize,
    pub bank: (usize, usize),
    pub branch: Option<(usize, usize)>,
    pub account: Option<(usize, usize)>,
}

pub fn iban_layout(cc: &str) -> Option<IbanLayout> {
    let l = |len, bank, branch, account| IbanLayout {
        len,
        bank,
        branch,
        account,
    };
    Some(match cc {
        "GB" | "IE" => l(22, (4, 8), Some((8, 14)), Some((14, 22))),
        "NL" => l(18, (4, 8), None, Some((8, 18))),
        "DE" => l(22, (4, 12), None, Some((12, 22))),
        "FR" | "MC" => l(27, (4, 9), Some((9, 14)), Some((14, 25))),
        "ES" => l(24, (4, 8), Some((8, 12)), Some((14, 24))),
        "IT" | "SM" => l(27, (5, 10), Some((10, 15)), Some((15, 27))),
        "AT" => l(20, (4, 9), None, Some((9, 20))),
        "CH" | "LI" => l(21, (4, 9), None, Some((9, 21))),
        "BE" => l(16, (4, 7), None, None),
        "LU" => l(20, (4, 7), None, None),
        _ => return None,
    })
}

pub fn iban_valid(v: &str) -> bool {
    let Some(layout) = iban_layout(v.get(..2).unwrap_or("")) else {
        return false;
    };
    if v.len() != layout.len
        || !v
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return false;
    }
    let mut m = 0u32;
    for c in v[4..].chars().chain(v[..4].chars()) {
        let d = c.to_digit(36).unwrap();
        m = (m * if d >= 10 { 100 } else { 10 } + d) % 97;
    }
    m == 1
}

pub fn luhn(digits: &str) -> bool {
    let sum: u32 = digits
        .chars()
        .rev()
        .filter_map(|c| c.to_digit(10))
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                if d * 2 > 9 {
                    d * 2 - 9
                } else {
                    d * 2
                }
            } else {
                d
            }
        })
        .sum();
    digits.len() >= 12 && sum.is_multiple_of(10)
}

/// Shape and holder status (the fourth letter); the Department issues
/// serials 0001 to 9999, so 0000 fails the strict check.
pub fn pan_valid(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 10 && b"PCHFATBLJG".contains(&b[3]) && &v[5..9] != "0000"
}

fn mrz_value(c: char) -> u32 {
    match c {
        '0'..='9' => c as u32 - '0' as u32,
        'A'..='Z' => c as u32 - 'A' as u32 + 10,
        _ => 0,
    }
}

pub fn mrz_digit(s: &str) -> u32 {
    s.chars()
        .zip([7, 3, 1].iter().cycle())
        .map(|(c, w)| mrz_value(c) * w)
        .sum::<u32>()
        % 10
}

fn mrz_field_ok(field: &str, check: char) -> bool {
    check.to_digit(10) == Some(mrz_digit(field))
        || (check == '<' && field.chars().all(|c| c == '<'))
}

/// A passport (TD3, line one starts with `P`) or visa (MRV-A, `V`) zone with
/// every check digit right.
pub fn mrz_valid(l1: &str, l2: &str) -> bool {
    if l1.len() != 44 || l2.len() != 44 || !l2.is_ascii() {
        return false;
    }
    let c = |i: usize| l2.as_bytes()[i] as char;
    let core = mrz_field_ok(&l2[0..9], c(9))
        && mrz_field_ok(&l2[13..19], c(19))
        && mrz_field_ok(&l2[21..27], c(27));
    match &l1[..1] {
        "P" => {
            let composite = format!("{}{}{}", &l2[0..10], &l2[13..20], &l2[21..43]);
            core && mrz_field_ok(&l2[28..42], c(42)) && mrz_field_ok(&composite, c(43))
        }
        "V" => core,
        _ => false,
    }
}

/// The clé RIB from bank code, branch code and account (letters by the
/// French table).
pub fn rib_key(bank: &str, branch: &str, account: &str) -> Option<String> {
    let digits: String = account
        .chars()
        .map(|c| match c {
            '0'..='9' => Some(c),
            'A'..='Z' => Some(b"123456789123456789234567890"[(c as u8 - b'A') as usize] as char),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let (b, g, a): (u64, u64, u64) = (
        bank.parse().ok()?,
        branch.parse().ok()?,
        digits.parse().ok()?,
    );
    Some(format!("{:02}", 97 - (89 * b + 15 * g + 3 * a) % 97))
}

struct Finder<'a> {
    lines: &'a [Line],
    out: Vec<Ident>,
}

impl Finder<'_> {
    fn taken(&self, li: usize, s: usize, e: usize) -> bool {
        let l = &self.lines[li];
        self.out
            .iter()
            .any(|x| x.page == l.page && x.start < l.start + e && l.start + s < x.end)
    }

    /// `bs..be` in bytes of the line.
    fn push(
        &mut self,
        li: usize,
        bs: usize,
        be: usize,
        kind: Kind,
        value: String,
        valid: Option<bool>,
    ) {
        let l = &self.lines[li];
        let (s, e) = (super::chars(&l.text, bs), super::chars(&l.text, be));
        if self.taken(li, s, e) {
            return;
        }
        self.out.push(Ident {
            line: li,
            page: l.page,
            start: l.start + s,
            end: l.start + e,
            text: l.text[bs..be].to_string(),
            kind,
            value,
            valid,
        });
    }
}

pub fn find(lines: &[Line]) -> Vec<Ident> {
    let mut f = Finder {
        lines,
        out: Vec::new(),
    };
    find_mrz(&mut f);
    for li in 0..lines.len() {
        find_on_line(&mut f, li);
    }
    find_table_accounts(&mut f);
    check_rib_keys(&mut f);
    f.out.sort_by_key(|x| (x.page, x.start));
    f.out
}

fn find_mrz(f: &mut Finder) {
    let zone = re!(r"^[A-Z0-9<]{44}$");
    for li in 0..f.lines.len().saturating_sub(1) {
        let (a, b) = (&f.lines[li], &f.lines[li + 1]);
        if a.page != b.page
            || !zone.is_match(&a.text)
            || !zone.is_match(&b.text)
            || !(a.text.starts_with('P') || a.text.starts_with('V'))
        {
            continue;
        }
        let value = format!("{}\n{}", a.text, b.text);
        f.out.push(Ident {
            line: li,
            page: a.page,
            start: a.start,
            end: b.start + 44,
            text: value.clone(),
            kind: Kind::Mrz,
            valid: Some(mrz_valid(&a.text, &b.text)),
            value,
        });
    }
}

fn find_on_line(f: &mut Finder, li: usize) {
    let line = &f.lines[li];
    let t = line.text.as_str();

    for m in re!(r"\b[A-Z]{2}\d{2}[ A-Z0-9]{11,45}").find_iter(t) {
        let Some(layout) = iban_layout(&m.as_str()[..2]) else {
            continue;
        };
        let (mut value, mut end, mut gap) = (String::new(), m.start(), false);
        for (b, c) in t[m.start()..].char_indices() {
            if value.len() == layout.len {
                break;
            }
            if c == ' ' && !gap {
                gap = true;
                continue;
            }
            if !c.is_ascii_alphanumeric() {
                break;
            }
            gap = false;
            value.push(c);
            end = m.start() + b + 1;
        }
        if value.len() == layout.len && !t[end..].starts_with(|c: char| c.is_alphanumeric()) {
            let valid = iban_valid(&value);
            f.push(li, m.start(), end, Kind::Iban, value, Some(valid));
        }
    }

    for m in re!(r"\b\d{4}(?: \d{4}){3}\b").find_iter(t) {
        let v = m.as_str().replace(' ', "");
        let ok = luhn(&v);
        f.push(li, m.start(), m.end(), Kind::Card, v, Some(ok));
    }
    if re!(r"(?i)\bcard\b").is_match(t) {
        for m in re!(r"\b\d{13,19}\b").find_iter(t) {
            let ok = luhn(m.as_str());
            f.push(
                li,
                m.start(),
                m.end(),
                Kind::Card,
                m.as_str().to_string(),
                Some(ok),
            );
        }
    }

    let ifsc_label = re!(r"\bIFSC\b").is_match(t);
    let ifsc = if line.scanned && ifsc_label {
        re!(r"\b[A-Z]{4}[0OØ][A-Z0-9]{6}\b")
    } else {
        re!(r"\b[A-Z]{4}0[A-Z0-9]{6}\b")
    };
    for m in ifsc.find_iter(t) {
        // On a scanned page the fixed fifth character is read back as zero.
        let v: String = m
            .as_str()
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 4 { '0' } else { c })
            .collect();
        f.push(li, m.start(), m.end(), Kind::Ifsc, v, Some(true));
    }

    if re!(r"(?i)\b(?:bic|swift)").is_match(t) {
        for m in re!(r"\b[A-Z]{6}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b").find_iter(t) {
            f.push(
                li,
                m.start(),
                m.end(),
                Kind::Bic,
                m.as_str().to_string(),
                Some(true),
            );
        }
    }

    if re!(r"(?i)\bPAN\b|पैन").is_match(t) {
        for m in re!(r"\b[A-Z]{5}\d{4}[A-Z]\b").find_iter(t) {
            let ok = pan_valid(m.as_str());
            f.push(
                li,
                m.start(),
                m.end(),
                Kind::InPan,
                m.as_str().to_string(),
                Some(ok),
            );
        }
    }

    if re!(r"(?i)\bsort\s*code\b").is_match(t) {
        for m in re!(r"\b\d{2}-\d{2}-\d{2}\b").find_iter(t) {
            f.push(
                li,
                m.start(),
                m.end(),
                Kind::GbSortCode,
                m.as_str().replace('-', ""),
                Some(true),
            );
        }
    }

    for (r, kind) in [
        (
            re!(r"(?i:\bcode\s+banque)\s*:\s*(\d{5})\b"),
            Kind::FrBankCode,
        ),
        (
            re!(r"(?i:\bcode\s+guichet)\s*:\s*(\d{5})\b"),
            Kind::FrBranchCode,
        ),
        (re!(r"(?i:\bcl[ée]\s+rib)\s*:\s*(\d{2})\b"), Kind::FrRibKey),
    ] {
        for c in r.captures_iter(t) {
            let g = c.get(1).unwrap();
            let valid = if kind == Kind::FrRibKey {
                None
            } else {
                Some(true)
            };
            f.push(li, g.start(), g.end(), kind, g.as_str().to_string(), valid);
        }
    }

    for c in
        re!(r"(?i:\bpassport\s+(?:no\.?|number))\s*[:：]?\s*([A-Z0-9]{6,9})\b").captures_iter(t)
    {
        let g = c.get(1).unwrap();
        f.push(
            li,
            g.start(),
            g.end(),
            Kind::PassportNumber,
            g.as_str().to_string(),
            None,
        );
    }

    let account = re!(
        r"(?i:\baccount(?:\s+(?:no\.?|number|nr\.?))?|\ba/c(?:\s+no\.?)?|\bkonto(?:nummer)?|\bnum[ée]ro\s+de\s+compte|口座(?:番号)?|खाता(?:\s+संख्या)?)\s*[:：.]?\s*(?:[^\s\d:：]{1,4}\s+)?([0-9][0-9A-Z]{5,19})\b"
    );
    for c in account.captures_iter(t) {
        let g = c.get(1).unwrap();
        if g.as_str().bytes().filter(u8::is_ascii_digit).count() >= 6 {
            f.push(
                li,
                g.start(),
                g.end(),
                Kind::Account,
                g.as_str().to_string(),
                None,
            );
        }
    }
}

/// A table whose header names an account column: each row's long digit run
/// is that row's account.
fn find_table_accounts(f: &mut Finder) {
    let header = re!(r"(?i)\baccount\b");
    let mut li = 0;
    while li < f.lines.len() {
        let h = &f.lines[li];
        let is_header = !h.text.contains(':')
            && header.is_match(&h.text)
            && h.text.split_whitespace().count() >= 3;
        li += 1;
        if !is_header {
            continue;
        }
        while li < f.lines.len()
            && f.lines[li].page == h.page
            && !f.lines[li].text.contains(':')
            && !f.lines[li].text.trim().is_empty()
        {
            let text = f.lines[li].text.clone();
            for m in re!(r"\b\d{8,18}\b").find_iter(&text) {
                f.push(
                    li,
                    m.start(),
                    m.end(),
                    Kind::Account,
                    m.as_str().to_string(),
                    None,
                );
            }
            li += 1;
        }
    }
}

/// The clé RIB is checked against the bank code, branch code and account on
/// its own line; with any of them missing it stays unchecked.
fn check_rib_keys(f: &mut Finder) {
    for i in 0..f.out.len() {
        if f.out[i].kind != Kind::FrRibKey {
            continue;
        }
        let on_line = |k: Kind| {
            f.out
                .iter()
                .find(|x| x.line == f.out[i].line && x.kind == k)
                .map(|x| x.value.clone())
        };
        if let (Some(b), Some(g), Some(a)) = (
            on_line(Kind::FrBankCode),
            on_line(Kind::FrBranchCode),
            on_line(Kind::Account),
        ) {
            f.out[i].valid = rib_key(&b, &g, &a).map(|k| k == f.out[i].value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::split_lines;
    use crate::findings::Page;

    fn idents(text: &str) -> Vec<Ident> {
        find(&split_lines(&[Page {
            text: text.into(),
            scanned: false,
        }]))
    }

    #[test]
    fn iban_check_digits() {
        assert!(iban_valid("DE21123456780000012345"));
        assert!(!iban_valid("DE22123456780000012345"));
        assert!(iban_valid("GB40ABCD11223344556677"));
        assert!(
            !iban_valid("GB40ABCD1122334455667"),
            "wrong length for the country"
        );
        assert!(!iban_valid("XX40ABCD11223344556677"), "unknown country");
    }

    #[test]
    fn iban_found_with_spaces_and_stops_at_its_length() {
        let v = idents("IBAN: GB40 ABCD 1122 3344 5566 77 BIC: ABCDGB2L");
        let iban = v.iter().find(|x| x.kind == Kind::Iban).unwrap();
        assert_eq!(iban.value, "GB40ABCD11223344556677");
        assert_eq!(iban.text, "GB40 ABCD 1122 3344 5566 77");
        assert_eq!(iban.valid, Some(true));
        assert_eq!(iban.institution_key().as_deref(), Some("iban:GB:ABCD"));
        assert!(v
            .iter()
            .any(|x| x.kind == Kind::Bic && x.value == "ABCDGB2L"));
    }

    #[test]
    fn card_luhn() {
        assert!(luhn("4000123456789017"));
        assert!(!luhn("4000123456789018"));
        let v = idents("Card: 4000 1234 5678 9018");
        assert_eq!(v[0].kind, Kind::Card);
        assert_eq!(v[0].valid, Some(false));
    }

    #[test]
    fn pan_status_and_serial() {
        assert!(pan_valid("ABCPZ1234Q"));
        assert!(!pan_valid("ABCDZ1234Q"), "D is no holder status");
        assert!(!pan_valid("ABCPZ0000Q"), "serial 0000 is never issued");
        assert!(
            idents("TAN: ABCPZ1234Q").is_empty(),
            "a PAN needs its label"
        );
        assert_eq!(idents("PAN: ABCPZ1234Q")[0].kind, Kind::InPan);
    }

    #[test]
    fn mrz_check_digits() {
        let l1 = "P<ZZZDOE<<JOHN<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<";
        let l2 = "XY12345671ZZZ9001158M3006014<<<<<<<<<<<<<<<4";
        assert!(mrz_valid(l1, l2));
        assert!(
            !mrz_valid(l1, &l2.replace("3006014", "3006024")),
            "expiry check digit"
        );
        assert!(
            !mrz_valid(l1, &format!("{}5", &l2[..43])),
            "composite check digit"
        );
        let v = idents(&format!("Passport\n{l1}\n{l2}"));
        assert_eq!(v[0].kind, Kind::Mrz);
        assert_eq!(v[0].value, format!("{l1}\n{l2}"));
        assert_eq!(v[0].valid, Some(true));
    }

    #[test]
    fn ifsc_shape_and_scanned_zero() {
        let v = idents("IFSC: ABCD0XYZ123");
        assert_eq!(
            (v[0].kind, v[0].value.as_str()),
            (Kind::Ifsc, "ABCD0XYZ123")
        );
        assert!(
            idents("IFSC: ABCDOXYZ123").is_empty(),
            "letter O in clean text is no IFSC"
        );
        let scanned = find(&split_lines(&[Page {
            text: "IFSC: ABCDØXYZ123".into(),
            scanned: true,
        }]));
        assert_eq!(scanned[0].value, "ABCD0XYZ123");
    }

    #[test]
    fn bic_needs_its_label() {
        assert!(idents("DOCUMENT").is_empty());
        assert_eq!(idents("SWIFT code: ABCDJPJT")[0].kind, Kind::Bic);
    }

    #[test]
    fn sort_code_value_drops_hyphens() {
        let v = idents("Sort code: 11-22-33");
        assert_eq!(
            (v[0].kind, v[0].value.as_str()),
            (Kind::GbSortCode, "112233")
        );
    }

    #[test]
    fn rib_key_recomputed_from_its_line() {
        assert_eq!(
            rib_key("12345", "67890", "0000012345A").as_deref(),
            Some("60")
        );
        let v = idents(
            "Code banque : 12345 Code guichet : 67890 Numéro de compte : 0000012345A Clé RIB : 60",
        );
        let key = v.iter().find(|x| x.kind == Kind::FrRibKey).unwrap();
        assert_eq!(key.valid, Some(true));
        let v = idents(
            "Code banque : 12345 Code guichet : 67890 Numéro de compte : 0000012345A Clé RIB : 61",
        );
        assert_eq!(
            v.iter().find(|x| x.kind == Kind::FrRibKey).unwrap().valid,
            Some(false)
        );
    }

    #[test]
    fn account_needs_an_account_label() {
        assert!(idents("Invoice number: 99887766554433").is_empty());
        assert!(idents("Phone: 020 1234 5678").is_empty());
        assert_eq!(idents("Account No. 99887766")[0].value, "99887766");
        assert_eq!(idents("受取口座: 普通 7654321")[0].value, "7654321");
        assert_eq!(idents("खाता संख्या: 99887766554")[0].value, "99887766554");
    }

    #[test]
    fn table_rows_under_an_account_column() {
        let v = idents("No Name Code Account\n1 Some One ABCD0XYZ123 99887766554433\nTotal: 3");
        assert!(v
            .iter()
            .any(|x| x.kind == Kind::Account && x.value == "99887766554433"));
        assert!(v.iter().any(|x| x.kind == Kind::Ifsc));
    }
}
