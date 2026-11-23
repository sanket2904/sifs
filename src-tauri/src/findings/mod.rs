//! Rule-based findings for the M1a sample (sample/FORMAT.md): organisation
//! mentions, identifiers, "identifier held at organisation" links with their
//! reasons, statement and passport facts, and the answers across documents.
//! No model and no list of organisation names; the bank directory is an
//! optional hint.

pub mod dates;
mod doc;
mod ids;
mod orgs;

use dates::Date;
use doc::{Expiry, Findings};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Deserialize)]
pub struct Page {
    pub text: String,
    pub scanned: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Document {
    pub id: String,
    pub lang: String,
    pub pages: Vec<Page>,
}

#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub page: usize,
    pub start: usize,
    pub text: String,
    pub scanned: bool,
}

pub(crate) fn split_lines(pages: &[Page]) -> Vec<Line> {
    let mut out = Vec::new();
    for (p, page) in pages.iter().enumerate() {
        let mut start = 0;
        for l in page.text.split('\n') {
            out.push(Line {
                page: p + 1,
                start,
                text: l.to_string(),
                scanned: page.scanned,
            });
            start += l.chars().count() + 1;
        }
    }
    out
}

/// The organisation a code names inside itself, as a raw key. With the
/// directory on, only a code some entry holds names one: the README links a
/// code only when its code names a directory entry.
pub(crate) fn institution(dir: Option<&Directory>, x: &ids::Ident) -> Option<String> {
    x.institution_key()
        .filter(|k| dir.is_none_or(|d| d.lookup(k).is_some()))
}

/// Unicode scalar values before byte `b` of `s`.
pub(crate) fn chars(s: &str, b: usize) -> usize {
    s[..b].chars().count()
}

/// The bank directory (sample/directory/banks.json), reduced to what a
/// lookup needs.
#[derive(Debug, Default)]
pub struct Directory {
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    id: String,
    names: Vec<EntryName>,
    codes: Codes,
}

#[derive(Debug, Deserialize)]
struct EntryName {
    key: String,
}

#[derive(Debug, Deserialize)]
struct Codes {
    bic: Vec<String>,
    ifsc_prefix: Vec<String>,
    iban_bank_code: Vec<IbanCode>,
}

#[derive(Debug, Deserialize)]
struct IbanCode {
    country: String,
    code: String,
}

impl Directory {
    pub fn parse(json: &str) -> serde_json::Result<Directory> {
        #[derive(Deserialize)]
        struct File {
            entries: Vec<Entry>,
        }
        Ok(Directory {
            entries: serde_json::from_str::<File>(json)?.entries,
        })
    }

    /// The one entry a raw key names exactly; none when two entries share it.
    fn lookup(&self, key: &str) -> Option<&str> {
        let (scheme, rest) = key.split_once(':')?;
        let hit = |e: &Entry| match scheme {
            "name" => e.names.iter().any(|n| n.key == rest),
            "ifsc" => e.codes.ifsc_prefix.iter().any(|p| p == rest),
            "bic" => e.codes.bic.iter().any(|b| b.get(..6) == Some(rest)),
            "iban" => rest.split_once(':').is_some_and(|(cc, code)| {
                e.codes
                    .iban_bank_code
                    .iter()
                    .any(|c| c.country == cc && c.code == code)
            }),
            _ => false,
        };
        let mut found = self.entries.iter().filter(|e| hit(e));
        let first = found.next()?;
        found.next().is_none().then_some(first.id.as_str())
    }
}

/// Raw keys joined into organisations. Two codes of one scheme that differ
/// (two IFSC prefixes, two BIC institutions) never join: that would be two
/// banks on one card.
#[derive(Default)]
struct Groups {
    parent: HashMap<String, String>,
}

impl Groups {
    fn root(&mut self, k: &str) -> String {
        let p = self
            .parent
            .entry(k.to_string())
            .or_insert_with(|| k.to_string())
            .clone();
        if p == k {
            return p;
        }
        let r = self.root(&p);
        self.parent.insert(k.to_string(), r.clone());
        r
    }

    fn members(&mut self, root: &str) -> Vec<String> {
        let keys: Vec<String> = self.parent.keys().cloned().collect();
        keys.into_iter().filter(|k| self.root(k) == root).collect()
    }

    fn join(&mut self, a: &str, b: &str) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return;
        }
        let codes = |m: Vec<String>| {
            m.into_iter()
                .filter(|k| !k.starts_with("name:"))
                .collect::<Vec<_>>()
        };
        let (ca, cb) = (codes(self.members(&ra)), codes(self.members(&rb)));
        let scheme = |k: &String| {
            k.rsplit_once(':')
                .map(|x| x.0.to_string())
                .unwrap_or_default()
        };
        if ca
            .iter()
            .any(|x| cb.iter().any(|y| scheme(x) == scheme(y) && x != y))
        {
            return;
        }
        self.parent.insert(rb, ra);
    }
}

/// (identifier, card) to the union of its reasons and its `via`.
type LinkRows<'a> = BTreeMap<(usize, String), (Vec<&'a str>, Option<usize>)>;

/// An account's spellings, and (period end, document) for each statement.
type Account = (Vec<String>, Vec<(Date, String)>);

fn holder(p: &doc::Passport) -> String {
    format!("{}, {}", p.surname, p.given)
}

/// The whole run: every document's findings, the organisation cards and the
/// answers, as FORMAT.md's output object.
pub fn run(
    docs: &[Document],
    directory: Option<&Directory>,
    text_set: &str,
    reference: Date,
) -> Value {
    let found: Vec<Findings> = docs.iter().map(|d| doc::extract(d, directory)).collect();

    let mut groups = Groups::default();
    let mut order: Vec<String> = Vec::new();
    for f in &found {
        let keys = f
            .mentions
            .iter()
            .filter_map(|m| m.key.clone())
            .chain(f.links.iter().map(|l| l.key.clone()));
        for k in keys {
            if !order.contains(&k) {
                groups.root(&k);
                order.push(k);
            }
        }
    }
    for f in &found {
        for (a, b) in &f.ties {
            groups.join(a, b);
        }
    }

    // With the directory on, a group takes the one entry its keys name; two
    // groups that name the same entry are one organisation.
    let mut dir_of: HashMap<String, Option<String>> = HashMap::new();
    if let Some(dir) = directory {
        let mut by_root: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for k in &order {
            if let Some(id) = dir.lookup(k) {
                by_root
                    .entry(groups.root(k))
                    .or_default()
                    .push(id.to_string());
            }
        }
        let mut first_root: HashMap<String, String> = HashMap::new();
        for (root, mut ids) in by_root {
            ids.sort();
            ids.dedup();
            if let [id] = ids.as_slice() {
                match first_root.get(id) {
                    Some(r) => groups.join(r, &root),
                    None => {
                        first_root.insert(id.clone(), root.clone());
                    }
                }
            }
        }
        for k in &order {
            let root = groups.root(k);
            if dir_of.contains_key(&root) {
                continue;
            }
            // Each spelling the group's mentions print is looked up too.
            let spelled: Vec<String> = found
                .iter()
                .flat_map(|f| &f.mentions)
                .filter(|m| m.key.as_ref().is_some_and(|k| groups.root(k) == root))
                .map(|m| format!("name:{}", orgs::match_key(&m.text)))
                .collect();
            let mut ids: Vec<String> = groups
                .members(&root)
                .iter()
                .chain(&spelled)
                .filter_map(|m| dir.lookup(m))
                .map(String::from)
                .collect();
            ids.sort();
            ids.dedup();
            dir_of.insert(root, (ids.len() == 1).then(|| ids[0].clone()));
        }
    }

    let mut card_of: HashMap<String, String> = HashMap::new();
    let mut organisations = Map::new();
    for k in &order {
        let root = groups.root(k);
        if card_of.contains_key(&root) {
            continue;
        }
        let card = format!("org-{}", card_of.len() + 1);
        // A name read from a clean page, settled by the name itself, first.
        let mut named: Vec<(bool, &str)> = Vec::new();
        for (f, d) in found.iter().zip(docs) {
            for m in &f.mentions {
                if m.key.as_ref().is_some_and(|mk| groups.root(mk) == root) {
                    named.push((
                        m.settled == Some(orgs::Settled::Name) && !d.pages[m.page - 1].scanned,
                        &m.text,
                    ));
                }
            }
        }
        let name = named
            .iter()
            .find(|n| n.0)
            .or(named.first())
            .map(|n| n.1.to_string());
        organisations.insert(
            card.clone(),
            json!({"name": name, "directory": dir_of.get(&root).cloned().flatten()}),
        );
        card_of.insert(root, card);
    }
    let card = |groups: &mut Groups, k: &str| card_of[&groups.root(k)].clone();

    let mut documents = Vec::new();
    let mut statements: Vec<(String, Vec<String>, Date)> = Vec::new();
    let mut passports: Vec<(String, String)> = Vec::new();
    for f in &found {
        let iid = |i: usize| format!("i{}", i + 1);
        let span = |page, start, end, text: &str| json!({"page": page, "start": start, "end": end, "text": text});
        let mentions: Vec<Value> = f
            .mentions
            .iter()
            .map(|m| {
                let mut v = span(m.page, m.start, m.end, &m.text);
                v["org"] = json!(m.key.as_ref().map(|k| card(&mut groups, k)));
                v["settled_by"] = json!(m.settled.map(orgs::Settled::as_str));
                v
            })
            .collect();
        let identifiers: Vec<Value> = f
            .idents
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut v = span(x.page, x.start, x.end, &x.text);
                v["id"] = json!(iid(i));
                v["kind"] = json!(x.kind);
                v["value"] = json!(x.value);
                v["valid"] = json!(x.valid);
                v
            })
            .collect();
        // Rows for one identifier and one organisation are one link.
        let mut merged: LinkRows = BTreeMap::new();
        for l in &f.links {
            let e = merged
                .entry((l.ident, card(&mut groups, &l.key)))
                .or_default();
            for r in &l.reasons {
                if !e.0.contains(r) {
                    e.0.push(r);
                }
            }
            e.1 = e.1.or(l.via);
        }
        let links: Vec<Value> = merged
            .into_iter()
            .map(|((i, org), (reasons, via))| {
                let mut v = json!({"identifier": iid(i), "org": org, "reasons": reasons});
                if let Some(c) = via {
                    v["via"] = json!(iid(c));
                }
                v
            })
            .collect();
        let mut entry =
            json!({"id": f.id, "orgs": mentions, "identifiers": identifiers, "links": links});
        if let Some(s) = &f.statement {
            entry["statement"] = json!({
                "from": s.from.iso(), "to": s.to.iso(),
                "accounts": s.accounts.iter().map(|&i| iid(i)).collect::<Vec<_>>(),
                "issuer": s.issuer.as_ref().map(|k| card(&mut groups, k)),
            });
            if !s.accounts.is_empty() {
                let mut values: Vec<String> = s
                    .accounts
                    .iter()
                    .map(|&i| f.idents[i].value.clone())
                    .collect();
                values.sort_by_key(|v| {
                    f.idents
                        .iter()
                        .all(|x| x.value != *v || x.kind != ids::Kind::Iban)
                });
                values.dedup();
                statements.push((f.id.clone(), values, s.to));
            }
        }
        if let Some(p) = &f.passport {
            let expiry = match &p.expiry {
                Some(Expiry::Date(d)) => json!(d.iso()),
                Some(Expiry::CannotTell) => json!("cannot-tell"),
                None => Value::Null,
            };
            let current = match (p.cancelled, p.candidates.as_slice()) {
                (true, _) => Some(false),
                (false, []) => None,
                (false, c) if c.iter().all(|d| *d >= reference) => Some(true),
                (false, c) if c.iter().all(|d| *d < reference) => Some(false),
                _ => None,
            };
            if current == Some(true) {
                passports.push((holder(p), f.id.clone()));
            }
            entry["passport"] = json!({
                "holder": {"surname": p.surname, "given": p.given},
                "number": p.number.map(iid), "mrz": p.mrz.map(iid),
                "issued": p.issued.map(Date::iso), "expiry": expiry,
                "expiry_at": p.expiry_at.as_ref().map(|(pg, s, e, t)| span(*pg, *s, *e, t)),
                "current": current,
            });
        }
        documents.push(entry);
    }

    json!({
        "format": 1,
        "extractor": "rust-m1a",
        "directory": if directory.is_some() { "on" } else { "off" },
        "text_set": text_set,
        "organisations": organisations,
        "documents": documents,
        "answers": {"latest_statement": latest(&statements), "current_passport": current(&passports)},
    })
}

/// The latest statement of each account, by its own period. Statements that
/// share any spelling are one account. Two latest statements ending the same
/// day leave the account out.
fn latest(statements: &[(String, Vec<String>, Date)]) -> Vec<Value> {
    let mut accounts: Vec<Account> = Vec::new();
    for (doc, values, to) in statements {
        match accounts
            .iter_mut()
            .find(|(v, _)| v.iter().any(|x| values.contains(x)))
        {
            Some((v, docs)) => {
                v.extend(values.iter().cloned());
                docs.push((*to, doc.clone()));
            }
            None => accounts.push((values.clone(), vec![(*to, doc.clone())])),
        }
    }
    accounts
        .into_iter()
        .filter_map(|(v, mut docs)| {
            docs.sort();
            let last = docs.last()?;
            (docs.iter().filter(|d| d.0 == last.0).count() == 1)
                .then(|| json!({"account": v[0], "document": last.1}))
        })
        .collect()
}

/// The current passport of each holder; two current ones leave the holder out.
fn current(passports: &[(String, String)]) -> Vec<Value> {
    let mut by: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for (h, doc) in passports {
        by.entry(h.clone()).or_default().push(doc);
    }
    by.into_iter()
        .filter(|(_, d)| d.len() == 1)
        .map(|(h, d)| json!({"holder": h, "document": d[0]}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, text: &str) -> Document {
        Document {
            id: id.into(),
            lang: "en".into(),
            pages: vec![Page {
                text: text.into(),
                scanned: false,
            }],
        }
    }

    fn reference() -> Date {
        Date::new(2025, 6, 1).unwrap()
    }

    #[test]
    fn statement_header_and_code_make_one_card_across_documents() {
        let docs = [
            doc("a", "Zed Alpha Bank Ltd\nAccount number: 11122233344\nIFSC: ZEDA0XYZ001\nPeriod: 01 Mar 2025 to 31 Mar 2025"),
            doc("b", "Zed Alpha Bank Limited\nDear customer"),
            doc("c", "Pay to account: 55566677788 IFSC: ZEDA0XYZ002"),
        ];
        let out = run(&docs, None, "text", reference());
        let orgs = out["organisations"].as_object().unwrap();
        assert_eq!(orgs.len(), 1, "{orgs:?}");
        assert_eq!(orgs["org-1"]["name"], "Zed Alpha Bank Ltd");
        assert_eq!(
            out["answers"]["latest_statement"][0]["account"],
            "11122233344"
        );
    }

    #[test]
    fn two_ifsc_prefixes_never_join() {
        let docs = [
            doc("a", "Zed Bank\nAccount number: 11122233344\nIFSC: AAAA0XYZ001\nPeriod: 01 Mar 2025 to 31 Mar 2025"),
            doc("b", "Zed Bank\nAccount number: 55566677788\nIFSC: BBBB0XYZ001\nPeriod: 01 Mar 2025 to 31 Mar 2025"),
        ];
        let out = run(&docs, None, "text", reference());
        let doc_b = &out["documents"][1];
        let ifsc_b = doc_b["links"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["identifier"] == "i2")
            .unwrap()["org"]
            .clone();
        let ifsc_a = out["documents"][0]["links"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["identifier"] == "i2")
            .unwrap()["org"]
            .clone();
        assert_ne!(ifsc_a, ifsc_b);
    }

    #[test]
    fn latest_statement_by_period_not_order() {
        let a = "Zed Bank\nAccount number: 11122233344\nPeriod: 01 Apr 2025 to 30 Apr 2025";
        let b = "Zed Bank\nAccount number: 11122233344\nPeriod: 01 Mar 2025 to 31 Mar 2025";
        let out = run(&[doc("apr", a), doc("mar", b)], None, "text", reference());
        assert_eq!(
            out["answers"]["latest_statement"],
            json!([{"account": "11122233344", "document": "apr"}])
        );
    }

    #[test]
    fn directory_lookup_needs_one_entry() {
        let dir = Directory::parse(r#"{"entries": [
            {"id": "Q1", "names": [{"key": "zedbank"}], "codes": {"bic": [], "ifsc_prefix": ["ZEDB"], "iban_bank_code": []}},
            {"id": "Q2", "names": [{"key": "zb"}], "codes": {"bic": [], "ifsc_prefix": [], "iban_bank_code": []}},
            {"id": "Q3", "names": [{"key": "zb"}], "codes": {"bic": [], "ifsc_prefix": [], "iban_bank_code": []}}
        ]}"#)
        .unwrap();
        assert_eq!(dir.lookup("ifsc:ZEDB"), Some("Q1"));
        assert_eq!(dir.lookup("name:zedbank"), Some("Q1"));
        assert_eq!(dir.lookup("name:zb"), None, "a clash");
        let out = run(
            &[
                doc("a", "Zed Bank\nDear customer"),
                doc("b", "Account: 11122233344 IFSC: ZEDB0XYZ001"),
            ],
            Some(&dir),
            "text",
            reference(),
        );
        let orgs = out["organisations"].as_object().unwrap();
        assert_eq!(orgs.len(), 1, "one entry, one card");
        assert_eq!(orgs["org-1"]["directory"], "Q1");

        let out = run(
            &[doc("c", "Account: 11122233344 IFSC: YYYY0XYZ001")],
            Some(&dir),
            "text",
            reference(),
        );
        assert_eq!(
            out["documents"][0]["links"],
            json!([]),
            "with the directory on, a code no entry holds names no bank"
        );
        let out = run(
            &[doc("c", "Account: 11122233344 IFSC: YYYY0XYZ001")],
            None,
            "text",
            reference(),
        );
        assert_eq!(
            out["documents"][0]["links"].as_array().unwrap().len(),
            2,
            "with it off, the code is the only evidence"
        );
    }
}
