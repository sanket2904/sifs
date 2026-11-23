//! Calendar dates as documents print them, in the sample's five languages.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Date {
    pub fn new(y: i32, m: u32, d: u32) -> Option<Date> {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        (1..=12).contains(&m).then_some(())?;
        (1..=days[m as usize - 1])
            .contains(&d)
            .then_some(Date { y, m, d })
    }
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

const MONTHS: &[(&str, u32)] = &[
    ("jan", 1),
    ("january", 1),
    ("januar", 1),
    ("janvier", 1),
    ("जनवरी", 1),
    ("feb", 2),
    ("february", 2),
    ("februar", 2),
    ("février", 2),
    ("फ़रवरी", 2),
    ("फरवरी", 2),
    ("mar", 3),
    ("march", 3),
    ("märz", 3),
    ("mars", 3),
    ("मार्च", 3),
    ("apr", 4),
    ("april", 4),
    ("avril", 4),
    ("अप्रैल", 4),
    ("may", 5),
    ("mai", 5),
    ("मई", 5),
    ("jun", 6),
    ("june", 6),
    ("juni", 6),
    ("juin", 6),
    ("जून", 6),
    ("jul", 7),
    ("july", 7),
    ("juli", 7),
    ("juillet", 7),
    ("जुलाई", 7),
    ("aug", 8),
    ("august", 8),
    ("août", 8),
    ("अगस्त", 8),
    ("sep", 9),
    ("sept", 9),
    ("september", 9),
    ("septembre", 9),
    ("सितंबर", 9),
    ("सितम्बर", 9),
    ("oct", 10),
    ("october", 10),
    ("oktober", 10),
    ("octobre", 10),
    ("अक्टूबर", 10),
    ("nov", 11),
    ("november", 11),
    ("novembre", 11),
    ("नवंबर", 11),
    ("नवम्बर", 11),
    ("dec", 12),
    ("december", 12),
    ("dezember", 12),
    ("décembre", 12),
    ("दिसंबर", 12),
    ("दिसम्बर", 12),
];

/// A date found in text: byte range, and either a date or a numeric
/// day-month pair whose order the text alone does not settle.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    date: Option<Date>,
    numeric: Option<(u32, u32, i32)>,
}

static WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})\.?\s+([\p{L}\p{M}]+)\.?,?\s+(\d{4})\b").unwrap());
static NUMERIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})([./-])(\d{1,2})([./-])(\d{4})\b").unwrap());
static ISO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})\b").unwrap());
static KANJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日").unwrap());

pub fn find(s: &str) -> Vec<Found> {
    let n = |c: &regex::Captures, i: usize| c[i].parse::<u32>().unwrap();
    let mut out: Vec<Found> = Vec::new();
    let mut add = |f: Found| {
        if !out.iter().any(|o| o.start < f.end && f.start < o.end) {
            out.push(f);
        }
    };
    for c in KANJI.captures_iter(s) {
        let m = c.get(0).unwrap();
        add(Found {
            start: m.start(),
            end: m.end(),
            date: Date::new(n(&c, 1) as i32, n(&c, 2), n(&c, 3)),
            numeric: None,
        });
    }
    for c in ISO.captures_iter(s) {
        let m = c.get(0).unwrap();
        add(Found {
            start: m.start(),
            end: m.end(),
            date: Date::new(n(&c, 1) as i32, n(&c, 2), n(&c, 3)),
            numeric: None,
        });
    }
    for c in WORDS.captures_iter(s) {
        let word = c[2].to_lowercase();
        if let Some(&(_, month)) = MONTHS.iter().find(|(w, _)| *w == word) {
            let m = c.get(0).unwrap();
            add(Found {
                start: m.start(),
                end: m.end(),
                date: Date::new(n(&c, 3) as i32, month, n(&c, 1)),
                numeric: None,
            });
        }
    }
    for c in NUMERIC.captures_iter(s) {
        if c[2] != c[4] {
            continue;
        }
        let m = c.get(0).unwrap();
        add(Found {
            start: m.start(),
            end: m.end(),
            date: None,
            numeric: Some((n(&c, 1), n(&c, 3), n(&c, 5) as i32)),
        });
    }
    out.retain(|f| f.date.is_some() || f.numeric.is_some());
    out.sort_by_key(|f| f.start);
    out
}

/// Day first unless the numbers force month first; `None` when both orders
/// make a date and `day_first` is not the document's convention.
fn resolve(f: &Found, day_first: Option<bool>) -> Option<Date> {
    if let Some(d) = f.date {
        return Some(d);
    }
    let (a, b, y) = f.numeric?;
    match (Date::new(y, b, a), Date::new(y, a, b), day_first) {
        (Some(dm), Some(_), Some(true)) => Some(dm),
        (Some(_), Some(md), Some(false)) => Some(md),
        (Some(_), Some(_), None) if a == b => Date::new(y, a, a),
        (Some(_), Some(_), None) => None,
        (Some(dm), None, _) => Some(dm),
        (None, Some(md), _) => Some(md),
        (None, None, _) => None,
    }
}

/// The first date at the start of `s` (after spaces).
pub fn first(s: &str, day_first: Option<bool>) -> Option<(usize, usize, Date)> {
    let f = find(s).into_iter().next()?;
    s[..f.start].trim().is_empty().then_some(())?;
    Some((f.start, f.end, resolve(&f, day_first)?))
}

/// `from <sep> to`, the whole of `s`.
pub fn range(s: &str, day_first: Option<bool>) -> Option<(Date, Date)> {
    static SEP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*(?:to|-|–|bis|au|से|～|~|〜)\s*$").unwrap());
    let found = find(s);
    let [a, b] = found.get(..2)? else { return None };
    if !s[..a.start].trim().is_empty() || !SEP.is_match(&s[a.end..b.start]) {
        return None;
    }
    // The two ends share one order: a number above twelve in either decides it.
    let forced = [a, b].iter().find_map(|f| {
        f.numeric.and_then(|(x, y, _)| {
            if x > 12 {
                Some(true)
            } else if y > 12 {
                Some(false)
            } else {
                None
            }
        })
    });
    let order = forced.or(day_first);
    let (from, to) = (resolve(a, order)?, resolve(b, order)?);
    (from <= to).then_some((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, dd: u32) -> Date {
        Date::new(y, m, dd).unwrap()
    }

    #[test]
    fn calendar() {
        assert!(Date::new(2024, 2, 29).is_some());
        assert!(Date::new(2023, 2, 29).is_none());
        assert!(Date::new(2023, 13, 1).is_none());
    }

    #[test]
    fn ranges_in_each_language() {
        assert_eq!(
            range("02 Mar 2025 to 01 Apr 2025", None),
            Some((d(2025, 3, 2), d(2025, 4, 1)))
        );
        assert_eq!(
            range("3 Februar 2025 bis 2. März 2025", None),
            Some((d(2025, 2, 3), d(2025, 3, 2)))
        );
        assert_eq!(
            range("05.01.2025 bis 04.02.2025", Some(true)),
            Some((d(2025, 1, 5), d(2025, 2, 4)))
        );
        assert_eq!(
            range("01 मार्च 2025 से 31 मई 2025", None),
            Some((d(2025, 3, 1), d(2025, 5, 31)))
        );
        assert_eq!(
            range("2025年3月1日～2025年3月31日", None),
            Some((d(2025, 3, 1), d(2025, 3, 31)))
        );
        assert_eq!(
            range("1 janvier 2025 au 31 janvier 2025", None),
            Some((d(2025, 1, 1), d(2025, 1, 31)))
        );
    }

    #[test]
    fn numeric_order_forced_by_either_end() {
        assert_eq!(
            range("02-03-2025 to 30-03-2025", None),
            Some((d(2025, 3, 2), d(2025, 3, 30)))
        );
        assert_eq!(
            range("02-03-2025 to 04-03-2025", None),
            None,
            "both orders make dates"
        );
    }

    #[test]
    fn a_range_must_be_the_whole_field() {
        assert_eq!(range("cover 01 Mar 2025 to 31 Mar 2025", None), None);
        assert_eq!(
            range("31 Mar 2025 to 01 Mar 2025", None),
            None,
            "ends out of order"
        );
    }

    #[test]
    fn upper_case_months() {
        assert_eq!(
            first(" 14 FEB 2031", None).map(|x| x.2),
            Some(d(2031, 2, 14))
        );
    }
}
