use super::*;
use crate::index::db::Index;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

struct Db {
    dir: tempfile::TempDir,
    path: PathBuf,
}

/// An index with the reading-inside-files schema and nothing in it.
fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sifs.db");
    drop(Index::open(&path).unwrap());
    Db { dir, path }
}

impl Db {
    fn out(&self, name: &str) -> PathBuf {
        self.dir.path().join(name).join("set")
    }

    /// Records a read file as the worker would have written it.
    fn add(&self, path: &str, reader: &str, status: &str, parts: &[(&str, &str)], facts: &[(&str, &str)]) {
        let conn = rusqlite::Connection::open(&self.path).unwrap();
        let name = path.rsplit('/').next().unwrap();
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e).to_lowercase();
        conn.execute(
            "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode) VALUES (?1, '/r', ?2, ?3, 0, 1, 1700000000, 1)",
            params![path, name, ext],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO extracts (file_id, reader, version, modified, size, status, millis, read_at) VALUES (?1, ?2, 1, 1700000000, 1, ?3, 0, 0)",
            params![id, reader, status],
        )
        .unwrap();
        for (i, (label, text)) in parts.iter().enumerate() {
            conn.execute("INSERT INTO contents (file_id, part, label, text) VALUES (?1, ?2, ?3, ?4)", params![id, i, label, text])
                .unwrap();
        }
        for (key, value) in facts {
            conn.execute("INSERT INTO facts (file_id, key, value) VALUES (?1, ?2, ?3)", params![id, key, value]).unwrap();
        }
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// The exported documents, by their `file`.
fn by_file(out: &Path) -> BTreeMap<String, serde_json::Value> {
    let index = read_json(&out.join("index.json"));
    index["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            let doc = read_json(&out.join(format!("{}.json", id.as_str().unwrap())));
            (doc["file"].as_str().unwrap().to_string(), doc)
        })
        .collect()
}

fn pages_of(doc: &serde_json::Value) -> Vec<(String, bool)> {
    doc["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["text"].as_str().unwrap().to_string(), p["scanned"].as_bool().unwrap()))
        .collect()
}

fn files_in(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().to_str().unwrap().to_string(), fs::read(e.path()).unwrap())
        })
        .collect()
}

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

// --- T1: read-only, a new folder, the same bytes ---

#[test]
fn the_index_is_opened_read_only() {
    let db = db();
    db.add("/r/a.txt", "text", "done", &[("", "alpha")], &[]);
    let before = fs::read(&db.path).unwrap();
    let conn = open_read_only(&db.path).unwrap();
    assert!(conn.is_readonly(rusqlite::DatabaseName::Main).unwrap());
    let write = conn.execute("CREATE TABLE written (x INTEGER)", []);
    assert!(matches!(write, Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ReadOnly), "{write:?}");
    drop(conn);
    export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    assert_eq!(fs::read(&db.path).unwrap(), before, "the index file is unchanged");
}

#[test]
fn a_read_only_index_file_still_exports() {
    use std::os::unix::fs::PermissionsExt;
    let db = db();
    db.add("/r/a.txt", "text", "done", &[("", "alpha")], &[]);
    fs::set_permissions(&db.path, fs::Permissions::from_mode(0o444)).unwrap();
    let exported = export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    assert_eq!(exported.documents.len(), 1);
}

#[test]
fn the_out_folder_must_be_new_or_empty() {
    let db = db();
    db.add("/r/a.txt", "text", "done", &[("", "alpha")], &[]);
    let full = db.out("full");
    fs::create_dir_all(&full).unwrap();
    fs::write(full.join("keep.txt"), "mine").unwrap();
    let refused = export(&db.path, &full, "2026-11-22").unwrap_err().to_string();
    assert!(refused.contains("is not empty"), "{refused}");
    assert_eq!(files_in(&full), BTreeMap::from([("keep.txt".to_string(), b"mine".to_vec())]));

    let empty = db.out("empty");
    fs::create_dir_all(&empty).unwrap();
    assert_eq!(export(&db.path, &empty, "2026-11-22").unwrap().documents.len(), 1);
    assert_eq!(export(&db.path, &db.out("absent"), "2026-11-22").unwrap().documents.len(), 1);
    assert!(export(&db.path, &empty, "2026-11-22").is_err(), "a second export into the same folder");
}

#[test]
fn two_exports_give_the_same_bytes() {
    let db = db();
    db.add("/r/b.pdf", "pdf", "done", &[("page 2", "two")], &[("pages", "2")]);
    db.add("/r/a.txt", "text", "done", &[("", "alpha")], &[]);
    export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    export(&db.path, &db.out("two"), "2026-11-22").unwrap();
    let (one, two) = (files_in(&db.out("one")), files_in(&db.out("two")));
    assert_eq!(one.len(), 3);
    assert_eq!(one, two);
    let index = read_json(&db.out("one").join("index.json"));
    assert_eq!(index["format"], 1);
    assert_eq!(index["text_set"], "set");
    assert_eq!(index["reference_date"], "2026-11-22");
}

#[test]
fn documents_are_listed_in_path_order() {
    let db = db();
    db.add("/r/z.txt", "text", "done", &[("", "z")], &[]);
    db.add("/r/a.txt", "text", "done", &[("", "a")], &[]);
    db.add("/r/m.txt", "text", "done", &[("", "m")], &[]);
    let exported = export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    let want: Vec<String> = ["/r/a.txt", "/r/m.txt", "/r/z.txt"].iter().map(|p| doc_id(&p[3..], p)).collect();
    assert_eq!(exported.documents, want);
    assert_eq!(read_json(&db.out("one").join("index.json"))["documents"], serde_json::json!(want));
}

#[test]
fn the_reference_date_is_checked() {
    let db = db();
    for bad in ["2026-02-29", "2026-13-01", "2026-1-01", "22-11-2026", "2026-11-22T00", "+026-11-22", ""] {
        assert!(export(&db.path, &db.out(&format!("d{}", bad.len())), bad).is_err(), "{bad:?}");
    }
    assert!(is_date("2024-02-29") && is_date("2000-02-29") && !is_date("1900-02-29"));
}

#[test]
fn an_index_that_never_read_inside_files_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE files (id INTEGER)").unwrap();
    let refused = export(&path, &dir.path().join("set"), "2026-11-22").unwrap_err().to_string();
    assert!(refused.contains("not read inside any file"), "{refused}");
}

// --- T3: ids ---

#[test]
fn ids_are_stable_distinct_and_plain() {
    let names = [
        "/r/Statement Sep 2025.PDF",
        "/r/other/Statement Sep 2025.PDF",
        "/r/取引明細.pdf",
        "/r/.hidden",
        "/r/a..b--c.tar.gz",
        "/r/very long name that goes on and on and on past forty characters.txt",
    ];
    let first = db();
    for path in names {
        first.add(path, "text", "done", &[("", "x")], &[]);
    }
    // The same files, indexed in the other order after another file that
    // sorts first: other row ids, other places in the list.
    let second = db();
    second.add("/a/elsewhere.txt", "text", "done", &[("", "x")], &[]);
    for path in names.iter().rev() {
        second.add(path, "text", "done", &[("", "x")], &[]);
    }
    // Path to id, as exported: the documents are listed in path order.
    let mut sorted = names.to_vec();
    sorted.sort();
    let ids = |db: &Db, out: &str, extra: usize| -> BTreeMap<String, String> {
        export(&db.path, &db.out(out), "2026-11-22").unwrap();
        let listed = read_json(&db.out(out).join("index.json"))["documents"].clone();
        let listed: Vec<String> = listed.as_array().unwrap().iter().map(|id| id.as_str().unwrap().to_string()).collect();
        for id in &listed {
            assert_eq!(read_json(&db.out(out).join(format!("{id}.json")))["id"], id.as_str());
        }
        assert_eq!(listed.len(), names.len() + extra);
        sorted.iter().map(|p| p.to_string()).zip(listed.into_iter().skip(extra)).collect()
    };
    let (one, two) = (ids(&first, "one", 0), ids(&second, "two", 1));
    assert_eq!(one, two, "each file keeps its id in another index");
    assert_eq!(one.values().collect::<HashSet<_>>().len(), names.len(), "distinct: {one:?}");
    assert!(one.values().all(|id| plain_id(id)), "{one:?}");
    assert!(doc_id("Statement Sep 2025.PDF", "/r/x").starts_with("statement-sep-2025-"));
    assert!(doc_id("取引明細.pdf", "/r/x").starts_with("file-"));
}

// --- T5: a document gives its file's full path ---

/// Paths as the index holds them, each beside the `file` its document must
/// give. The names are kept in the Unicode form they were stored in: one
/// folder is decomposed (e + U+0301, as HFS+ stores names), one composed.
const PATHS: [(&str, &str); 11] = [
    ("/Users/ana/Tax Papers/2031/return.pdf", "/Users/ana/Tax Papers/2031/return.pdf"),
    ("/Users/ana/Gene\u{301}ve/bail.pdf", "/Users/ana/Gene\u{301}ve/bail.pdf"),
    ("/Users/ana/Gen\u{e8}ve/bail.pdf", "/Users/ana/Gen\u{e8}ve/bail.pdf"),
    ("/Users/ana/Receipts #12/till.txt", "/Users/ana/Receipts #12/till.txt"),
    ("/Users/ana/100% done/notes.txt", "/Users/ana/100% done/notes.txt"),
    ("/Users/ana/Ana's \"best\" files/list.txt", "/Users/ana/Ana's \"best\" files/list.txt"),
    ("/Users/ana/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/deep.txt", "/Users/ana/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/deep.txt"),
    ("/Volumes/Backup/scan 2031.pdf", "/Volumes/Backup/scan 2031.pdf"),
    ("/Users/ana/Inbox/back\\slash.txt", "/Users/ana/Inbox/back\\slash.txt"),
    ("/top.txt", "/top.txt"),
    ("/Users/ana/Banks/statement.pdf", "/Users/ana/Banks/statement.pdf"),
];

/// Each exported document's `file`, by its id.
fn files_by_id(out: &Path) -> BTreeMap<String, String> {
    let index = read_json(&out.join("index.json"));
    index["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            let id = id.as_str().unwrap();
            let doc = read_json(&out.join(format!("{id}.json")));
            (id.to_string(), doc["file"].as_str().unwrap().to_string())
        })
        .collect()
}

#[test]
fn each_document_gives_its_files_full_path() {
    let db = db();
    for (path, _) in PATHS {
        db.add(path, "text", "done", &[("", "x")], &[]);
    }
    export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    let got = files_by_id(&db.out("one"));
    let want: BTreeMap<String, String> = PATHS
        .iter()
        .map(|(path, file)| (doc_id(path.rsplit('/').next().unwrap(), path), file.to_string()))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn two_files_of_one_name_are_two_documents() {
    let db = db();
    db.add("/Users/ana/Banks/statement.pdf", "text", "done", &[("", "one")], &[]);
    db.add("/Users/ana/Old Banks/statement.pdf", "text", "done", &[("", "two")], &[]);
    export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    assert_eq!(
        files_by_id(&db.out("one")),
        BTreeMap::from([
            ("statement-de21a520cd1150b5".to_string(), "/Users/ana/Banks/statement.pdf".to_string()),
            ("statement-e1895903ccfebd18".to_string(), "/Users/ana/Old Banks/statement.pdf".to_string()),
        ])
    );
}

/// The ids written before a document gave its path, pinned: a person's
/// documents keep their ids.
#[test]
fn ids_are_as_before_the_path_was_given() {
    let db = db();
    for path in ["/Users/ana/Banks/statement.pdf", "/Users/ana/Old Banks/statement.pdf", "/Volumes/Backup/scan 2031.pdf"] {
        db.add(path, "text", "done", &[("", "x")], &[]);
    }
    let exported = export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    assert_eq!(exported.documents, ["statement-de21a520cd1150b5", "statement-e1895903ccfebd18", "scan-2031-aec8d8326d234c02"]);
}

// --- T2: parts to pages ---

#[test]
fn each_kind_of_part_maps_to_pages() {
    let db = db();
    db.add("/r/a-mixed.pdf", "pdf", "done", &[("page 1", "one"), ("page 3", "three")], &[("pages", "3"), ("textless_pages", "1")]);
    db.add("/r/b-scan.pdf", "pdf", "empty", &[], &[("pages", "2"), ("textless_pages", "2")]);
    db.add(
        "/r/c-cut.pdf",
        "pdf",
        "done",
        &[("page 1", "one"), ("page 2", "tw")],
        &[("pages", "5"), ("truncated", "1"), ("stopped", "took longer than 60 s, after page 2")],
    );
    db.add("/r/d-photo.jpg", "image", "done", &[("", "RECEIPT\nTOTAL 12.00")], &[]);
    db.add("/r/e-blank.png", "image", "empty", &[], &[]);
    db.add("/r/f-notes.txt", "text", "done", &[("", "line one\nline two\n")], &[]);
    db.add("/r/g-book.xlsx", "excel", "done", &[("Sheet1", "a\tb\n"), ("Rates (hidden)", "c\n")], &[]);
    db.add("/r/h-deck.pptx", "powerpoint", "done", &[("slide 1", "title\n"), ("slide 1 notes", "say hi\n")], &[]);
    db.add("/r/i-empty.txt", "text", "empty", &[], &[]);
    db.add("/r/j-broken.pdf", "pdf", "failed", &[], &[]);
    db.add("/r/k-big.txt", "text", "skipped", &[], &[]);
    export(&db.path, &db.out("one"), "2026-11-22").unwrap();
    let docs = by_file(&db.out("one"));
    let page = |text: &str, scanned| (text.to_string(), scanned);
    let want: BTreeMap<&str, Vec<(String, bool)>> = BTreeMap::from([
        ("/r/a-mixed.pdf", vec![page("one", false), page("", true), page("three", false)]),
        ("/r/b-scan.pdf", vec![page("", true), page("", true)]),
        ("/r/c-cut.pdf", vec![page("one", false), page("tw", false)]),
        ("/r/d-photo.jpg", vec![page("RECEIPT\nTOTAL 12.00", true)]),
        ("/r/e-blank.png", vec![page("", true)]),
        ("/r/f-notes.txt", vec![page("line one\nline two\n", false)]),
        ("/r/g-book.xlsx", vec![page("a\tb\n", false), page("c\n", false)]),
        ("/r/h-deck.pptx", vec![page("title\n", false), page("say hi\n", false)]),
    ]);
    let got: BTreeMap<&str, Vec<(String, bool)>> = docs.iter().map(|(f, d)| (f.as_str(), pages_of(d))).collect();
    assert_eq!(got, want, "empty, failed and skipped files with no page are left out");

    let doc = &docs["/r/a-mixed.pdf"];
    assert_eq!(doc["id"], doc_id("a-mixed.pdf", "/r/a-mixed.pdf"));
    assert_eq!(doc["format"], "pdf");
    assert_eq!(doc["lang"], "");
    assert_eq!(doc["sha256"], serde_json::Value::Null);
    assert_eq!(doc["created"], "2023-11-14T22:13:20Z");
    let keys: Vec<&str> = doc.as_object().unwrap().keys().map(String::as_str).collect();
    let mut want_keys = vec!["id", "file", "sha256", "format", "lang", "created", "pages"];
    want_keys.sort();
    assert_eq!(keys, want_keys);
}

#[test]
fn a_pdf_part_not_labelled_as_a_page_is_an_error() {
    let db = db();
    db.add("/r/odd.pdf", "pdf", "done", &[("cover", "x")], &[("pages", "1")]);
    let error = format!("{:#}", export(&db.path, &db.out("one"), "2026-11-22").unwrap_err());
    assert!(error.contains("not \"page N\""), "{error}");
    assert!(!db.out("one").exists(), "nothing is written");
}

#[test]
fn utc_writes_the_file_date() {
    assert_eq!(utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(utc(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(utc(-1), "1969-12-31T23:59:59Z");
    assert_eq!(utc(4_102_444_799), "2099-12-31T23:59:59Z");
}

// --- T4: the sample's made-up files, read by the library's own readers ---

/// Why an exported document's pages differ from the sample's text set.
fn cause(sample: &[(String, bool)], exported: &[(String, bool)], at: usize) -> &'static str {
    match (sample.get(at), exported.get(at)) {
        (Some((_, true)), Some((text, true))) if text.is_empty() => "scanned PDF page, not read (no OCR of PDF pages)",
        (Some((want, _)), Some((got, _))) if got.strip_suffix('\n') == Some(want) => "the file's last line feed kept",
        (Some((want, _)), Some((got, _)))
            if got.split_once('\n').is_some_and(|(_, rest)| rest.strip_suffix('\n') == Some(want)) =>
        {
            "the HTML title read as a first line (and the last line feed kept)"
        }
        (Some(_), None) | (None, Some(_)) => "page count",
        _ => "text layer reads differently",
    }
}

/// The first difference: page (from 1), offset in characters, 40
/// characters of each from there.
fn first_difference(sample: &[(String, bool)], exported: &[(String, bool)]) -> Option<(usize, usize, String, String)> {
    let take = |s: &str, from: usize| s.chars().skip(from).take(40).collect::<String>();
    for at in 0..sample.len().max(exported.len()) {
        let (s, e) = (sample.get(at), exported.get(at));
        if s == e {
            continue;
        }
        let (st, et) = (s.map_or("", |p| p.0.as_str()), e.map_or("", |p| p.0.as_str()));
        let offset = st.chars().zip(et.chars()).take_while(|(a, b)| a == b).count();
        return Some((at + 1, offset, take(st, offset), take(et, offset)));
    }
    None
}

#[cfg(target_os = "macos")]
#[test]
fn the_sample_files_export_like_their_text_set() {
    use crate::extract::{store, worker::run_until_empty, Registry, WorkerOptions};
    use crate::index::{crawl::scan, ignore::Rules};
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sample");
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    let folder = home.join("files");
    fs::create_dir_all(&folder).unwrap();
    for entry in fs::read_dir(sample.join("files")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    let db = base.join("sifs.db");
    {
        let mut index = Index::open(&db).unwrap();
        let registry = Registry::platform();
        store::register_kinds(&mut index, &registry).unwrap();
        let root = folder.to_str().unwrap();
        index.add_folder(root, 0).unwrap();
        let rules = Rules::new(&Rules::default_patterns(), home.to_str().unwrap()).unwrap();
        scan(&mut index, root, &rules, |_| {}).unwrap();
        let options = WorkerOptions {
            idle_poll: std::time::Duration::from_millis(20),
            grace: std::time::Duration::from_millis(50),
            ..WorkerOptions::default()
        };
        run_until_empty(&mut index, &registry, &options).unwrap();
    }
    let read: HashSet<String> = {
        let conn = open_read_only(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT f.name FROM files f JOIN extracts e ON e.file_id = f.id WHERE e.status IN ('done', 'empty')")
            .unwrap();
        let names = stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        names
    };

    let sample_index = read_json(&sample.join("text/index.json"));
    let reference_date = sample_index["reference_date"].as_str().unwrap();
    let out = base.join("text-export");
    let exported = export(&db, &out, reference_date).unwrap();
    assert_eq!(read_json(&out.join("index.json"))["documents"], serde_json::json!(exported.documents));
    assert_eq!(exported.documents.iter().collect::<HashSet<_>>().len(), exported.documents.len(), "distinct ids");
    assert!(exported.documents.iter().all(|id| plain_id(id)), "{:?}", exported.documents);
    let docs = by_file(&out);

    let mut table = vec!["| Sample document | File | Pages (sample, export) | Result |".to_string(), "| --- | --- | --- | --- |".into()];
    let mut causes: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut compared, mut equal, mut unread) = (0, 0, Vec::new());
    for id in sample_index["documents"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        let want = read_json(&sample.join(format!("text/{id}.json")));
        let file = want["file"].as_str().unwrap();
        if !read.contains(file) {
            unread.push(id);
            continue;
        }
        let path = folder.join(file);
        let got = docs.get(path.to_str().unwrap()).unwrap_or_else(|| panic!("{file} was read but not exported"));
        let (want, got) = (pages_of(&want), pages_of(got));
        compared += 1;
        let result = match first_difference(&want, &got) {
            None => {
                equal += 1;
                "equal".to_string()
            }
            Some((page, offset, s, e)) => {
                let why = cause(&want, &got, page - 1);
                *causes.entry(why).or_default() += 1;
                format!("{why}: page {page}, offset {offset}, sample {s:?}, export {e:?}")
            }
        };
        table.push(format!("| {id} | {file} | {}, {} | {} |", want.len(), got.len(), result.replace('|', "\\|")));
    }
    println!("{}", table.join("\n"));
    println!("\n{compared} compared, {equal} equal, {} different, {} not read: {unread:?}", compared - equal, unread.len());
    for (why, n) in &causes {
        println!("  {n} {why}");
    }
    assert!(compared > 0);
    assert_eq!(compared + unread.len(), sample_index["documents"].as_array().unwrap().len());
}
