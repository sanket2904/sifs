//! One document: what ties each identifier to an organisation, its
//! statement and its passport.

use super::dates::{self, Date};
use super::ids::{self, iban_layout, Ident, Kind};
use super::orgs::{self, Mention};
use super::{institution, Directory, Document, Line};

#[derive(Clone, Debug)]
pub struct Link {
    pub ident: usize,
    pub key: String,
    pub reasons: Vec<&'static str>,
    pub via: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Statement {
    pub from: Date,
    pub to: Date,
    pub accounts: Vec<usize>,
    pub issuer: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expiry {
    Date(Date),
    CannotTell,
}

#[derive(Clone, Debug)]
pub struct Passport {
    pub surname: String,
    pub given: String,
    pub number: Option<usize>,
    pub mrz: Option<usize>,
    pub issued: Option<Date>,
    pub expiry: Option<Expiry>,
    /// (page, start, end, text) of the printed expiry.
    pub expiry_at: Option<(usize, usize, usize, String)>,
    /// Every expiry the page prints, the zone's included when its check
    /// fails: a passport is current when all of them are.
    pub candidates: Vec<Date>,
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub struct Findings {
    pub id: String,
    pub mentions: Vec<Mention>,
    pub idents: Vec<Ident>,
    pub links: Vec<Link>,
    /// Pairs of raw keys the document shows are one organisation.
    pub ties: Vec<(String, String)>,
    pub statement: Option<Statement>,
    pub passport: Option<Passport>,
}

pub fn extract(doc: &Document, dir: Option<&Directory>) -> Findings {
    let lines = super::split_lines(&doc.pages);
    let idents = ids::find(&lines);
    let mentions = orgs::find(&lines, &idents, dir);
    let mut f = Findings {
        id: doc.id.clone(),
        mentions,
        idents,
        links: Vec::new(),
        ties: Vec::new(),
        statement: None,
        passport: None,
    };
    code_links(&lines, &mut f, dir);
    let day_first = (doc.lang != "en").then_some(true);
    f.statement = statement(&lines, &mut f, day_first);
    f.passport = passport(&lines, &f.idents);
    f
}

fn link(f: &mut Findings, ident: usize, key: String, reason: &'static str, via: Option<usize>) {
    f.links.push(Link {
        ident,
        key,
        reasons: vec![reason],
        via,
    });
}

/// The code an account or IBAN is paired with: on its line when the line
/// holds one code and one account, else on the line above or below when that
/// line holds a code and no account and the account's own line holds no code.
fn paired_code(lines: &[Line], idents: &[Ident], i: usize) -> Option<usize> {
    let li = idents[i].line;
    let on = |l: usize, p: fn(Kind) -> bool| {
        idents
            .iter()
            .enumerate()
            .filter(|(_, x)| x.line == l && x.valid != Some(false) && p(x.kind))
            .map(|(j, _)| j)
            .collect::<Vec<_>>()
    };
    let codes = on(li, Kind::is_bank_code);
    if !codes.is_empty() {
        let accounts = idents
            .iter()
            .filter(|x| x.line == li && x.kind.is_account())
            .count();
        return (codes.len() == 1 && accounts == 1).then(|| codes[0]);
    }
    let near: Vec<usize> = [li.checked_sub(1), Some(li + 1)]
        .into_iter()
        .flatten()
        .filter(|&l| l < lines.len() && lines[l].page == lines[li].page)
        .filter(|&l| !idents.iter().any(|x| x.line == l && x.kind.is_account()))
        .filter_map(|l| {
            let c = on(l, Kind::is_bank_code);
            (c.len() == 1).then(|| c[0])
        })
        .collect();
    (near.len() == 1).then(|| near[0])
}

fn code_links(lines: &[Line], f: &mut Findings, dir: Option<&Directory>) {
    for i in 0..f.idents.len() {
        let x = f.idents[i].clone();
        if x.valid == Some(false) {
            continue;
        }
        let reason = match x.kind {
            Kind::Ifsc => Some("ifsc-prefix"),
            Kind::Bic => Some("bic-institution"),
            Kind::Iban => Some("iban-bank-code"),
            _ => None,
        };
        if let (Some(r), Some(k)) = (reason, institution(dir, &x)) {
            link(f, i, k, r, None);
        }
        if matches!(x.kind, Kind::Account | Kind::Iban) {
            if let Some((c, ck)) = paired_code(lines, &f.idents, i)
                .and_then(|c| Some((c, institution(dir, &f.idents[c])?)))
            {
                if let Some(own) = x.institution_key() {
                    f.ties.push((own, ck.clone()));
                }
                link(f, i, ck, "paired-code", Some(c));
            }
        }
        let parts = |range: fn(&ids::IbanLayout) -> Option<(usize, usize)>| -> Option<String> {
            f.idents
                .iter()
                .filter(|y| y.kind == Kind::Iban && y.valid == Some(true))
                .find_map(|y| {
                    let r = range(&iban_layout(&y.value[..2])?)?;
                    (y.value[r.0..r.1] == x.value)
                        .then(|| y.institution_key())
                        .flatten()
                })
        };
        let inside = match x.kind {
            Kind::GbSortCode | Kind::FrBranchCode => {
                parts(|l| l.branch).map(|k| (k, "inside-iban"))
            }
            Kind::FrBankCode => parts(|l| Some(l.bank)).map(|k| (k, "inside-iban")),
            Kind::Account => parts(|l| l.account).map(|k| (k, "same-account")),
            _ => None,
        };
        if let Some((k, r)) = inside {
            link(f, i, k, r, None);
        }
    }
}

/// A field at the start of a line: the label, then a colon. On a scanned
/// page one misread character in the label and its colon is allowed.
fn field<'a>(line: &'a Line, labels: &[&str]) -> Option<(usize, &'a str)> {
    let t = line.text.as_str();
    let lead = t.len() - t.trim_start().len();
    let body: Vec<(usize, char)> = t[lead..].char_indices().collect();
    for label in labels {
        let want: Vec<char> = label.chars().flat_map(char::to_lowercase).collect();
        if body.len() < want.len() {
            continue;
        }
        let got: Vec<char> = body[..want.len()]
            .iter()
            .flat_map(|(_, c)| c.to_lowercase())
            .collect();
        if got.len() != want.len() {
            continue;
        }
        let after = lead + body.get(want.len()).map_or(t.len() - lead, |(b, _)| *b);
        let rest = &t[after..];
        let colon = rest.trim_start().strip_prefix([':', '：']);
        if got == want {
            if let Some(v) = colon {
                return Some((t.len() - v.len(), v));
            }
        }
        // Scanned: the label and its colon with at most one character changed.
        let fuzzy_want: Vec<char> = want.iter().copied().chain([':']).collect();
        if line.scanned && want.len() >= 6 && body.len() > want.len() {
            let got: Vec<char> = body[..=want.len()]
                .iter()
                .flat_map(|(_, c)| c.to_lowercase())
                .collect();
            if got.len() == fuzzy_want.len()
                && got.iter().zip(&fuzzy_want).filter(|(a, b)| a != b).count() <= 1
            {
                let b = body.get(want.len() + 1).map_or(t.len() - lead, |(b, _)| *b);
                return Some((lead + b, &t[lead + b..]));
            }
        }
    }
    None
}

fn statement(lines: &[Line], f: &mut Findings, day_first: Option<bool>) -> Option<Statement> {
    const PERIOD: &[&str] = &[
        "statement period",
        "period",
        "zeitraum",
        "période",
        "periode",
        "अवधि",
        "期間",
    ];
    let (pl, (from, to)) = lines
        .iter()
        .enumerate()
        .find_map(|(i, l)| Some((i, dates::range(field(l, PERIOD)?.1, day_first)?)))?;
    let first = f.mentions.first();
    let issuer = first.and_then(|m| m.key.clone());
    let header: Vec<usize> = (0..f.idents.len())
        .filter(|&i| {
            f.idents[i].line < pl
                && f.idents[i].kind.is_account()
                && f.idents[i].valid != Some(false)
        })
        .collect();
    let mut values: Vec<&str> = header.iter().map(|&i| f.idents[i].value.as_str()).collect();
    values.sort();
    values.dedup();
    let one_account = match values.as_slice() {
        [_] => true,
        [a, b] => f.links.iter().any(|l| {
            l.reasons.contains(&"same-account")
                && [*a, *b].contains(&f.idents[l.ident].value.as_str())
        }),
        _ => false,
    };
    let accounts: Vec<usize> = if one_account {
        (0..f.idents.len())
            .filter(|&i| {
                f.idents[i].kind.is_account() && values.contains(&f.idents[i].value.as_str())
            })
            .collect()
    } else {
        Vec::new()
    };
    if let Some(k) = &issuer {
        for &a in &accounts {
            link(f, a, k.clone(), "statement-header", None);
        }
        let codes: Vec<String> = f
            .links
            .iter()
            .filter(|l| accounts.contains(&l.ident) && l.key != *k)
            .map(|l| l.key.clone())
            .collect();
        for c in codes {
            f.ties.push((k.clone(), c));
        }
    }
    Some(Statement {
        from,
        to,
        accounts,
        issuer,
    })
}

fn passport(lines: &[Line], idents: &[Ident]) -> Option<Passport> {
    let mrz = idents
        .iter()
        .position(|x| x.kind == Kind::Mrz && x.value.starts_with('P'));
    let number = idents.iter().position(|x| x.kind == Kind::PassportNumber);
    let get = |labels: &[&str]| {
        lines
            .iter()
            .find_map(|l| field(l, labels).map(|(b, v)| (l, b, v)))
    };
    let printed_expiry = get(&["date of expiry", "expiry date", "date d'expiration"]);
    if mrz.is_none() && (number.is_none() || printed_expiry.is_none()) {
        return None;
    }
    let date_in = |hit: Option<(&Line, usize, &str)>| {
        let (l, b, v) = hit?;
        let (s, e, d) = dates::first(v, None)?;
        let (bs, be) = (b + s, b + e);
        Some((
            d,
            (
                l.page,
                l.start + super::chars(&l.text, bs),
                l.start + super::chars(&l.text, be),
                l.text[bs..be].to_string(),
            ),
        ))
    };
    let printed = date_in(printed_expiry);
    let issued = date_in(get(&["date of issue", "issue date"])).map(|x| x.0);

    let zone = mrz.map(|i| &idents[i]);
    let zone_date = zone.and_then(|z| {
        let l2 = z.value.split('\n').nth(1)?;
        let n = |r: std::ops::Range<usize>| l2[r].parse::<u32>().ok();
        Date::new(2000 + n(21..23)? as i32, n(23..25)?, n(25..27)?)
    });
    let zone_expiry = zone_date.filter(|_| zone.is_some_and(|z| z.valid == Some(true)));
    let candidates: Vec<Date> = printed.iter().map(|p| p.0).chain(zone_date).collect();
    let expiry = match (zone, zone_expiry, &printed) {
        (Some(_), None, _) => Some(Expiry::CannotTell),
        (Some(_), Some(z), Some((p, _))) if *p != z => Some(Expiry::CannotTell),
        (Some(_), Some(z), _) => Some(Expiry::Date(z)),
        (None, _, Some((p, _))) => Some(Expiry::Date(*p)),
        (None, _, None) => None,
    };

    let name = |labels: &[&str]| {
        get(labels)
            .map(|(_, _, v)| {
                v.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_uppercase()
            })
            .filter(|v| !v.is_empty())
    };
    let (mut surname, mut given) = (
        name(&["surname", "nom"]),
        name(&["given names", "given name", "prénoms"]),
    );
    if let (Some(z), true) = (zone, surname.is_none() || given.is_none()) {
        let l1 = z.value.split('\n').next().unwrap_or("");
        let mut parts = l1.get(5..).unwrap_or("").splitn(2, "<<");
        let clean = |s: &str| {
            s.split('<')
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        surname = surname.or(parts.next().map(clean).filter(|s| !s.is_empty()));
        given = given.or(parts.next().map(clean).filter(|s| !s.is_empty()));
    }
    let cancelled = lines
        .iter()
        .any(|l| l.text.trim().eq_ignore_ascii_case("cancelled"));
    Some(Passport {
        surname: surname?,
        given: given?,
        number,
        mrz,
        issued,
        expiry,
        expiry_at: printed.map(|p| p.1),
        candidates,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::Page;

    fn run(text: &str) -> Findings {
        extract(
            &Document {
                id: "t".into(),
                lang: "en".into(),
                pages: vec![Page {
                    text: text.into(),
                    scanned: false,
                }],
            },
            None,
        )
    }

    fn links(f: &Findings) -> Vec<(String, String, Vec<&'static str>)> {
        f.links
            .iter()
            .map(|l| {
                (
                    f.idents[l.ident].value.clone(),
                    l.key.clone(),
                    l.reasons.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn same_line_code_ties_each_row() {
        let f = run("Debit: account 11122233344 IFSC: AAAA0XYZ001\nCredit: account 55566677788 IFSC: BBBB0XYZ002");
        let l = links(&f);
        assert!(l.contains(&(
            "11122233344".into(),
            "ifsc:AAAA".into(),
            vec!["paired-code"]
        )));
        assert!(l.contains(&(
            "55566677788".into(),
            "ifsc:BBBB".into(),
            vec!["paired-code"]
        )));
        assert!(!l.iter().any(|x| x.0 == "11122233344" && x.1 == "ifsc:BBBB"));
    }

    #[test]
    fn next_line_code_pairs_only_when_neither_line_holds_another() {
        let f = run("Account number: 11122233344\nIFSC: AAAA0XYZ001");
        assert!(links(&f)
            .iter()
            .any(|x| x.0 == "11122233344" && x.2 == ["paired-code"]));
        let f = run("Account number: 11122233344\nAccount number: 99988877766 IFSC: AAAA0XYZ001");
        assert!(
            !links(&f).iter().any(|x| x.0 == "11122233344"),
            "the code's line holds an account of its own"
        );
    }

    #[test]
    fn two_banks_and_two_numbers_with_nothing_tying_them_is_no_link() {
        let f = run(
            "Zed Alpha Bank and Yew Beta Bank\nFrom account: 11122233344\nTo account: 55566677788",
        );
        assert!(f.links.is_empty());
    }

    #[test]
    fn iban_parts_link_inside_the_iban() {
        let f =
            run("Sort code: 11-22-33 Account number: 44556677\nIBAN: GB40 ABCD 1122 3344 5566 77");
        let l = links(&f);
        assert!(l.contains(&("112233".into(), "iban:GB:ABCD".into(), vec!["inside-iban"])));
        assert!(l.contains(&(
            "44556677".into(),
            "iban:GB:ABCD".into(),
            vec!["same-account"]
        )));
        assert!(l.contains(&(
            "GB40ABCD11223344556677".into(),
            "iban:GB:ABCD".into(),
            vec!["iban-bank-code"]
        )));
    }

    #[test]
    fn invalid_identifier_is_linked_nowhere() {
        let f = run("IBAN: GB41 ABCD 1122 3344 5566 77\nBIC: ABCDGB2L");
        assert!(!links(&f).iter().any(|x| x.0.starts_with("GB41")));
    }

    #[test]
    fn statement_header_ties_the_one_account_to_the_first_organisation() {
        let f = run("Zed Alpha Bank Ltd\nAccount number: 11122233344\nIFSC: ZEDA0XYZ001\nPeriod: 01 Mar 2025 to 31 Mar 2025\n02 Mar 2025 Yew Beta Bank transfer 100.00");
        let s = f.statement.as_ref().unwrap();
        assert_eq!(
            (s.from.iso(), s.to.iso()),
            ("2025-03-01".into(), "2025-03-31".into())
        );
        assert_eq!(s.issuer.as_deref(), Some("name:zedalphabank"));
        assert!(links(&f).contains(&(
            "11122233344".into(),
            "name:zedalphabank".into(),
            vec!["statement-header"]
        )));
        assert!(f
            .ties
            .contains(&("name:zedalphabank".into(), "ifsc:ZEDA".into())));
    }

    #[test]
    fn statement_with_two_accounts_names_none() {
        let f = run("Zed Bank\nAccount number: 11122233344\nAccount number: 55566677788\nPeriod: 01 Mar 2025 to 31 Mar 2025");
        assert!(f.statement.unwrap().accounts.is_empty());
        assert!(f.links.is_empty());
    }

    #[test]
    fn passport_expiry_agrees_or_cannot_be_told() {
        let l1 = "P<ZZZDOE<<JOHN<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<";
        let l2 = "XY12345671ZZZ9001158M3006014<<<<<<<<<<<<<<<4";
        let f = run(&format!("Passport No.: XY1234567\nSurname: DOE\nGiven names: JOHN\nDate of expiry: 01 JUN 2030\n{l1}\n{l2}"));
        let p = f.passport.unwrap();
        assert_eq!((p.surname.as_str(), p.given.as_str()), ("DOE", "JOHN"));
        assert_eq!(p.expiry, Some(Expiry::Date(Date::new(2030, 6, 1).unwrap())));
        let f = run(&format!(
            "Passport No.: XY1234567\nDate of expiry: 02 JUN 2030\n{l1}\n{l2}"
        ));
        assert_eq!(
            f.passport.unwrap().expiry,
            Some(Expiry::CannotTell),
            "printed and zone disagree"
        );
    }

    #[test]
    fn scanned_label_allows_one_misread() {
        let page = Page { text: "SURHAME: DOE\nGIVEN NAMESE JOHN\nPASSPORT NO: XY1234567\nDATE OF EXPIRY: 01 JUN 2030".into(), scanned: true };
        let f = extract(
            &Document {
                id: "t".into(),
                lang: "en".into(),
                pages: vec![page],
            },
            None,
        );
        let p = f.passport.unwrap();
        assert_eq!((p.surname.as_str(), p.given.as_str()), ("DOE", "JOHN"));
    }
}
