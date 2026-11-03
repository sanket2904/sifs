//! Reading inside files, end to end: the migration, the queue's triggers,
//! the worker, and the sample files in tests/fixtures/contents.

use super::store::{self, ContentMatch};
use super::worker::{run_until_empty, STOPPED_TWICE};
use super::*;
use crate::index::crawl::{record_of, refresh, scan};
use crate::index::db::Index;
use crate::index::ignore::Rules;
use crate::index::tests::eventually;
use crate::index::watch::Watch;
use rusqlite::params;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

// --- helpers ---

fn now() -> i64 {
    SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

struct Home {
    _dir: tempfile::TempDir,
    root: String,
    db: PathBuf,
    rules: Rules,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap().to_str().unwrap().to_string();
        let root = format!("{base}/home");
        fs::create_dir_all(&root).unwrap();
        let rules = Rules::new(&Rules::default_patterns(), &root).unwrap();
        Home { db: dir.path().join("sifs.db"), _dir: dir, root, rules }
    }

    /// A home with the sample files in samples/.
    fn with_samples() -> Home {
        let home = Home::new();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contents");
        for entry in fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            home.write(&format!("samples/{}", entry.file_name().to_str().unwrap()), &fs::read(entry.path()).unwrap());
        }
        home
    }

    fn at(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }

    fn write(&self, rel: &str, body: &[u8]) -> String {
        let path = self.at(rel);
        fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    fn open(&self) -> Index {
        Index::open(&self.db).unwrap()
    }

    /// Opens the index with registry's kinds, adds the home folder and scans it.
    fn scanned(&self, registry: &Registry) -> Index {
        let mut index = self.open();
        store::register_kinds(&mut index, registry).unwrap();
        index.add_folder(&self.root, 0).unwrap();
        scan(&mut index, &self.root, &self.rules, |_| {}).unwrap();
        index
    }

    fn refresh(&self, index: &mut Index, rel: &str) {
        refresh(index, Path::new(&self.at(rel)), std::slice::from_ref(&self.root), &self.rules).unwrap();
    }
}

fn set_age(path: &str, days: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
}

fn options() -> WorkerOptions {
    WorkerOptions { idle_poll: Duration::from_millis(20), grace: Duration::from_millis(50), ..WorkerOptions::default() }
}

fn run(index: &mut Index, registry: &Registry) -> RunStats {
    run_until_empty(index, registry, &options()).unwrap()
}

fn found(index: &Index, query: &str) -> Vec<String> {
    store::match_contents(index, query, 100).unwrap().into_iter().map(|ContentMatch { path, .. }| path).collect()
}

fn count(index: &Index, sql: &str) -> i64 {
    index.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn queued(index: &Index) -> i64 {
    count(index, "SELECT COUNT(*) FROM extract_queue")
}

fn id_of(index: &Index, path: &str) -> i64 {
    index.conn.query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0)).unwrap()
}

/// (status, error) of what was read from path.
fn status(index: &Index, path: &str) -> (String, Option<String>) {
    index
        .conn
        .query_row(
            "SELECT e.status, e.error FROM extracts e JOIN files f ON f.id = e.file_id WHERE f.path = ?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

fn fact_of(index: &Index, path: &str, key: &str) -> Option<String> {
    index
        .conn
        .query_row(
            "SELECT value FROM facts WHERE file_id = (SELECT id FROM files WHERE path = ?1) AND key = ?2",
            [path, key],
            |r| r.get(0),
        )
        .ok()
}

fn leak(exts: &[&'static str]) -> &'static [&'static str] {
    Box::leak(exts.to_vec().into_boxed_slice())
}

type Body = Arc<dyn Fn(&Path, &mut Sink, usize) -> Result<(), ReadError> + Send + Sync>;

/// A reader for tests: counts its reads by path, then does what body says
/// (given how many times this path has been read, this one included).
#[derive(Clone)]
struct Fake {
    id: &'static str,
    version: u32,
    exts: &'static [&'static str],
    class: Class,
    limits: Limits,
    reads: Arc<Mutex<HashMap<String, usize>>>,
    order: Arc<Mutex<Vec<String>>>,
    body: Body,
}

impl Fake {
    fn new(id: &'static str, exts: &[&'static str]) -> Fake {
        Fake {
            id,
            version: 1,
            exts: leak(exts),
            class: Class::Document,
            limits: Limits::default(),
            reads: Arc::default(),
            order: Arc::default(),
            body: Arc::new(|path, sink, _| {
                sink.text(&fs::read_to_string(path)?)?;
                Ok(())
            }),
        }
    }

    fn body(mut self, body: impl Fn(&Path, &mut Sink, usize) -> Result<(), ReadError> + Send + Sync + 'static) -> Fake {
        self.body = Arc::new(body);
        self
    }

    fn total(&self) -> usize {
        self.reads.lock().unwrap().values().sum()
    }

    fn of(&self, path: &str) -> usize {
        self.reads.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    fn registry(&self) -> Registry {
        Registry::with(vec![Arc::new(self.clone())])
    }
}

impl Reader for Fake {
    fn id(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> u32 {
        self.version
    }
    fn exts(&self) -> &'static [&'static str] {
        self.exts
    }
    fn class(&self) -> Class {
        self.class
    }
    fn limits(&self) -> Limits {
        self.limits
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let key = path.to_str().unwrap().to_string();
        let n = {
            let mut reads = self.reads.lock().unwrap();
            let n = reads.entry(key.clone()).or_default();
            *n += 1;
            *n
        };
        self.order.lock().unwrap().push(key);
        (self.body)(path, sink, n)
    }
}

// --- the sink ---

#[test]
fn the_sink_cuts_text_on_a_character_boundary_and_records_it() {
    let far = Instant::now() + Duration::from_secs(10);
    let mut sink = Sink::new(5, far);
    assert_eq!(sink.text("ab").unwrap(), Flow::More);
    assert_eq!(sink.text("cdéfg").unwrap(), Flow::Full);
    assert_eq!(sink.text("more").unwrap(), Flow::Full);
    assert_eq!(sink.chars(), 5);
    let (parts, facts) = sink.finish();
    assert_eq!(parts, vec![Part { part: 0, label: String::new(), text: "abcdé".into() }]);
    assert!(facts.contains(&("truncated".into(), "1".into())));
    // Multi-byte characters are counted as characters.
    let mut sink = Sink::new(3, far);
    sink.part("page 1");
    assert_eq!(sink.text("किराया").unwrap(), Flow::Full);
    let (parts, _) = sink.finish();
    assert_eq!((parts[0].label.as_str(), parts[0].text.as_str()), ("page 1", "किर"));
    // Text that fits exactly is not truncated.
    let mut sink = Sink::new(4, far);
    assert_eq!(sink.text("abcd").unwrap(), Flow::More);
    assert!(sink.finish().1.is_empty());
}

#[test]
fn the_sink_relabels_cuts_and_notes_what_was_left_out() {
    let mut sink = Sink::new(100, Instant::now() + Duration::from_secs(10));
    sink.part("");
    sink.text("one").unwrap();
    sink.relabel("page 1");
    sink.part("page 2");
    sink.text("two").unwrap();
    sink.note_truncated();
    assert_eq!(sink.text("more").unwrap(), Flow::More, "noting doesn't stop the text");
    assert_eq!(sink.cut(), Flow::Full);
    assert_eq!(sink.text("dropped").unwrap(), Flow::Full);
    let (parts, facts) = sink.finish();
    let parts: Vec<(&str, &str)> = parts.iter().map(|p| (p.label.as_str(), p.text.as_str())).collect();
    assert_eq!(parts, vec![("page 1", "one"), ("page 2", "twomore")]);
    assert_eq!(facts, vec![("truncated".to_string(), "1".to_string())]);
    let mut sink = Sink::new(100, Instant::now() + Duration::from_secs(10));
    sink.note_truncated();
    sink.text("kept").unwrap();
    assert_eq!(sink.finish().1, vec![("truncated".to_string(), "1".to_string())]);
}

#[test]
fn the_sink_stops_a_reader_past_its_time_limit() {
    let sink = Sink::new(100, Instant::now() - Duration::from_millis(1));
    assert!(matches!(sink.check(), Err(ReadError::Timeout)));
    let mut sink = sink;
    assert!(matches!(sink.text("late"), Err(ReadError::Timeout)));
}

// --- the migration ---

#[test]
fn a_new_index_reaches_version_1_with_every_table() {
    let home = Home::new();
    let index = home.open();
    assert_eq!(count(&index, "PRAGMA user_version"), 1);
    assert_eq!(crate::index::migrate::VERSION, 1);
    for table in ["extracts", "contents", "contents_fts", "facts", "extract_queue", "extract_kinds"] {
        let n = count(&index, &format!("SELECT COUNT(*) FROM sqlite_master WHERE name = '{table}'"));
        assert_eq!(n, 1, "{table}");
    }
    for trigger in
        ["files_ai_extract", "files_au_extract", "files_au_unread", "files_ad_extract", "contents_ai", "contents_ad"]
    {
        let n =
            count(&index, &format!("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = '{trigger}'"));
        assert_eq!(n, 1, "{trigger}");
    }
    drop(index);
    // Opening again changes nothing.
    assert_eq!(count(&home.open(), "PRAGMA user_version"), 1);
}

#[test]
fn a_phase_1_index_is_migrated_and_its_files_are_queued() {
    let home = Home::new();
    let notes = home.write("Documents/notes.txt", b"The ferry leaves at noon.");
    let page = home.write("Documents/page.html", b"<p>Ferry timetable</p>");
    let photo = home.write("Documents/photo.jpg", b"jpeg");
    {
        // The index as phase 1 left it: version 0, with files.
        let conn = rusqlite::Connection::open(&home.db).unwrap();
        conn.execute_batch(crate::index::db::SCHEMA).unwrap();
        for path in [&home.root, &format!("{}/Documents", home.root), &notes, &page, &photo] {
            let r = record_of(Path::new(path), &home.root).unwrap();
            conn.execute(
                "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode, seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                params![r.path, r.root, r.name, r.ext, r.is_dir, r.size, r.modified, r.inode],
            )
            .unwrap();
        }
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
    let mut index = home.open();
    assert_eq!(count(&index, "PRAGMA user_version"), 1);
    assert_eq!(index.count().unwrap(), 5, "the files are kept");
    assert_eq!(queued(&index), 0, "nothing is queued by the migration itself");
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    let stats = run(&mut index, &registry);
    assert_eq!(stats.reconciled, 2, "the text and HTML files; not the photo or the folders");
    let mut ferry = found(&index, "ferry");
    ferry.sort();
    assert_eq!(ferry, vec![notes.clone(), page.clone()]);
    assert_eq!(found(&index, "timetable"), vec![page]);
    // Name search still works over the migrated files.
    assert_eq!(crate::index::search::search(&index, "notes", 10, now()).unwrap()[0].file.path, notes);
}

#[test]
fn an_index_from_a_newer_sifs_still_opens() {
    let home = Home::new();
    home.open().conn.pragma_update(None, "user_version", 99).unwrap();
    let index = home.open();
    assert_eq!(count(&index, "PRAGMA user_version"), 99);
}

// --- the queue ---

#[test]
fn an_unchanged_rescan_queues_nothing() {
    let home = Home::new();
    for i in 0..5 {
        home.write(&format!("Documents/n{i}.txt"), format!("note {i}").as_bytes());
    }
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    assert_eq!(queued(&index), 5);
    run(&mut index, &registry);
    assert_eq!(queued(&index), 0);
    // The upsert rewrites every column of every row; nothing changed, so
    // nothing is queued.
    scan(&mut index, &home.root, &home.rules, |_| {}).unwrap();
    home.refresh(&mut index, "Documents/n0.txt");
    assert_eq!(queued(&index), 0);
}

#[test]
fn deleting_a_file_or_folder_removes_what_was_read_from_it_everywhere() {
    let home = Home::new();
    for rel in ["a/one.txt", "a/b/two.txt", "c/three.txt", "d/four.txt", "keep/five.txt"] {
        home.write(rel, format!("{rel} shared okapi words").as_bytes());
    }
    home.write("other/six.txt", b"okapi in another folder");
    home.write("a/page.html", b"<title>okapi page</title>");
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    let other = format!("{}/../other", home.root);
    fs::create_dir_all(&other).unwrap();
    fs::write(format!("{other}/seven.txt"), b"okapi outside home").unwrap();
    let other = fs::canonicalize(other).unwrap().to_str().unwrap().to_string();
    scan(&mut index, &other, &home.rules, |_| {}).unwrap();
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 8);
    assert!(count(&index, "SELECT COUNT(*) FROM facts") > 0, "the page's title");

    let orphans = |index: &Index| {
        ["contents", "facts", "extracts", "extract_queue"]
            .iter()
            .map(|t| count(index, &format!("SELECT COUNT(*) FROM {t} WHERE file_id NOT IN (SELECT id FROM files)")))
            .sum::<i64>()
    };
    // A file, through the watcher's refresh.
    fs::remove_file(home.at("a/one.txt")).unwrap();
    home.refresh(&mut index, "a/one.txt");
    assert!(!found(&index, "okapi").contains(&home.at("a/one.txt")));
    // A folder, with the page and its title inside it.
    fs::remove_dir_all(home.at("a")).unwrap();
    home.refresh(&mut index, "a");
    assert!(found(&index, "page").is_empty());
    assert_eq!(count(&index, "SELECT COUNT(*) FROM facts"), 0);
    // Gone when a rescan finds it missing.
    fs::remove_dir_all(home.at("c")).unwrap();
    scan(&mut index, &home.root, &home.rules, |_| {}).unwrap();
    // Newly ignored.
    let rules = Rules::new(&["**/d".to_string()], &home.root).unwrap();
    scan(&mut index, &home.root, &rules, |_| {}).unwrap();
    // A folder the user removed.
    index.remove_root(&other).unwrap();
    assert_eq!(orphans(&index), 0);
    let mut left = found(&index, "okapi");
    left.sort();
    assert_eq!(left, vec![home.at("keep/five.txt"), home.at("other/six.txt")]);
    assert_eq!(count(&index, "SELECT COUNT(*) FROM contents"), 2);
    // The full-text index agrees with its table.
    index.conn.execute("INSERT INTO contents_fts(contents_fts, rank) VALUES ('integrity-check', 1)", []).unwrap();
}

/// Files of every priority, each modified long enough ago to not count as
/// recent, but one.
fn ordered_home() -> (Home, Registry) {
    let home = Home::new();
    for (rel, days) in [("code.rs", 400), ("notes.txt", 300), ("older.md", 500), ("photo.png", 300), ("recent.rs", 0)] {
        let path = home.write(rel, b"x");
        if days > 0 {
            set_age(&path, days);
        }
    }
    let mut image = Fake::new("image", &["png"]);
    image.class = Class::Image;
    let mut registry = Registry::platform();
    registry.register(Arc::new(image));
    (home, registry)
}

fn claim_order(index: &mut Index) -> Vec<String> {
    let jobs = store::claim(index, 16).unwrap();
    jobs.into_iter().map(|j| j.path.rsplit('/').next().unwrap().to_string()).collect()
}

#[test]
fn the_queue_takes_recent_files_then_documents_then_images_then_code() {
    let (home, registry) = ordered_home();
    let mut index = home.scanned(&registry);
    // Within a priority, the most recently modified first.
    assert_eq!(claim_order(&mut index), vec!["recent.rs", "notes.txt", "older.md", "photo.png", "code.rs"]);
}

#[test]
fn opening_a_file_moves_it_to_the_front() {
    let (home, registry) = ordered_home();
    let mut index = home.scanned(&registry);
    assert!(store::bump_opened(&mut index, &home.at("code.rs")).unwrap());
    assert_eq!(claim_order(&mut index)[..2], ["code.rs", "recent.rs"]);
    assert!(!store::bump_opened(&mut index, &home.at("not-indexed.txt")).unwrap());
}

// --- reading again only when something changed ---

#[test]
fn a_second_run_reads_nothing() {
    let home = Home::new();
    for i in 0..5 {
        home.write(&format!("n{i}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(fake.total(), 5);
    run(&mut index, &registry);
    scan(&mut index, &home.root, &home.rules, |_| {}).unwrap();
    run(&mut index, &registry);
    assert_eq!(fake.total(), 5);
}

#[test]
fn a_changed_time_or_size_rereads_exactly_that_file() {
    let home = Home::new();
    for name in ["a", "b", "c"] {
        home.write(&format!("{name}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(fake.total(), 3);

    set_age(&home.at("a.txt"), 2);
    home.refresh(&mut index, "a.txt");
    run(&mut index, &registry);
    assert_eq!((fake.total(), fake.of(&home.at("a.txt"))), (4, 2));

    // A new size with the same time.
    let modified = fs::metadata(home.at("b.txt")).unwrap().modified().unwrap();
    home.write("b.txt", b"tapirs");
    fs::File::options().write(true).open(home.at("b.txt")).unwrap().set_modified(modified).unwrap();
    home.refresh(&mut index, "b.txt");
    run(&mut index, &registry);
    assert_eq!((fake.total(), fake.of(&home.at("b.txt"))), (5, 2));
    assert_eq!(found(&index, "tapirs"), vec![home.at("b.txt")]);
}

#[test]
fn a_new_reader_version_rereads_that_readers_files_and_no_others() {
    let home = Home::new();
    for rel in ["a.txt", "b.txt", "c.md"] {
        home.write(rel, b"tapir");
    }
    let (alpha, beta) = (Fake::new("alpha", &["txt"]), Fake::new("beta", &["md"]));
    let registry = Registry::with(vec![Arc::new(alpha.clone()), Arc::new(beta.clone())]);
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!((alpha.total(), beta.total()), (2, 1));

    let mut alpha2 = Fake::new("alpha", &["txt"]);
    alpha2.version = 2;
    let beta1 = Fake::new("beta", &["md"]);
    let registry = Registry::with(vec![Arc::new(alpha2.clone()), Arc::new(beta1.clone())]);
    store::register_kinds(&mut index, &registry).unwrap();
    let stats = run(&mut index, &registry);
    assert_eq!(stats.reconciled, 2);
    assert_eq!((alpha2.total(), beta1.total()), (2, 0));
    // And once is enough.
    run(&mut index, &registry);
    assert_eq!(alpha2.total(), 2);
}

// --- failures and limits ---

#[test]
fn a_failing_reader_is_recorded_and_not_retried_until_the_file_changes() {
    let home = Home::new();
    let path = home.write("bad.txt", b"tapir");
    let fake = Fake::new("fake", &["txt"]).body(|_, _, _| Err(ReadError::Other("cannot parse this".into())));
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(status(&index, &path), ("failed".into(), Some("cannot parse this".into())));
    let failures = store::failures(&index, 10).unwrap();
    assert_eq!(
        failures,
        vec![Failure { path: path.clone(), status: "failed".into(), error: "cannot parse this".into() }]
    );
    assert_eq!(store::progress(&index).unwrap(), Progress { failed: 1, ..Progress::default() });

    run(&mut index, &registry);
    assert_eq!(fake.total(), 1);
    // Checking every file against the readers again doesn't retry it either.
    index.conn.execute("DELETE FROM meta WHERE key = 'extract_signature'", []).unwrap();
    let stats = run(&mut index, &registry);
    assert_eq!((stats.reconciled, fake.total()), (0, 1));

    home.write("bad.txt", b"tapirs");
    home.refresh(&mut index, "bad.txt");
    run(&mut index, &registry);
    assert_eq!(fake.total(), 2);
}

#[test]
fn a_file_one_byte_over_the_cap_is_never_opened() {
    let home = Home::new();
    let over = home.write("over.txt", &[b'a'; 101]);
    let at = home.write("at.txt", &[b'a'; 100]);
    let mut fake = Fake::new("fake", &["txt"]);
    fake.limits.max_bytes = 100;
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!((fake.of(&over), fake.of(&at)), (0, 1));
    assert_eq!(status(&index, &over), ("skipped".into(), Some("larger than 100 bytes".into())));
    assert_eq!(status(&index, &at).0, "done");
}

#[test]
fn the_real_readers_skip_what_is_over_their_caps_and_keep_a_million_characters() {
    let home = Home::new();
    let text = home.write("big.txt", &b"ab ".repeat(4 * MB as usize / 3 + 1)[..4 * MB as usize]);
    let text_over = home.write("over.txt", &vec![b'a'; 4 * MB as usize + 1]);
    let json_over = home.write("over.json", &vec![b' '; MB as usize + 1]);
    let plain_over = home.write("Makefile", &vec![b'a'; MB as usize + 1]);
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(status(&index, &text).0, "done");
    assert_eq!(count(&index, "SELECT chars FROM extracts WHERE chars > 0"), 1_000_000);
    assert_eq!(fact_of(&index, &text, "truncated").as_deref(), Some("1"));
    assert_eq!(status(&index, &text_over), ("skipped".into(), Some("larger than 4 MB".into())));
    assert_eq!(status(&index, &json_over), ("skipped".into(), Some("larger than 1 MB".into())));
    assert_eq!(status(&index, &plain_over), ("skipped".into(), Some("larger than 1 MB".into())));
}

#[test]
fn a_slow_reader_is_recorded_failed_and_the_worker_carries_on() {
    let home = Home::new();
    let stuck = home.write("stuck.slow", b"x");
    let polite = home.write("polite.slow", b"x");
    for i in 0..4 {
        home.write(&format!("n{i}.txt"), b"tapir");
    }
    let mut slow = Fake::new("slow", &["slow"]).body(|path, sink, _| {
        if path.ends_with("stuck.slow") {
            // Ignores the time limit: abandoned.
            std::thread::sleep(Duration::from_secs(3));
            return Ok(());
        }
        // Stops itself at the limit.
        loop {
            sink.text("x")?;
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    slow.limits.time = Duration::from_millis(200);
    let text = Fake::new("fake", &["txt"]);
    let registry = Registry::with(vec![Arc::new(slow.clone()), Arc::new(text.clone())]);
    let mut index = home.scanned(&registry);
    // A full second's grace: the polite reader is never abandoned for being
    // scheduled late on a busy machine.
    let grace = WorkerOptions { grace: Duration::from_secs(1), ..options() };
    let stats = run_until_empty(&mut index, &registry, &grace).unwrap();
    let late = ("failed".to_string(), Some("took longer than 200 ms".to_string()));
    assert_eq!(status(&index, &stuck), late);
    assert_eq!(status(&index, &polite), late);
    assert_eq!(stats.abandoned, 1, "only the one that ignored the limit");
    assert_eq!(text.total(), 4);
    assert_eq!(store::progress(&index).unwrap(), Progress { read: 4, failed: 2, ..Progress::default() });
    // Not retried until it changes.
    run(&mut index, &registry);
    assert_eq!(slow.total(), 2);
}

// --- stopping and starting again ---

#[test]
fn a_restarted_worker_reads_every_file_exactly_once() {
    let home = Home::new();
    for i in 0..50 {
        home.write(&format!("n{i:02}.txt"), format!("tapir {i}").as_bytes());
    }
    let fake = Fake::new("fake", &["txt"]).body(|path, sink, _| {
        std::thread::sleep(Duration::from_millis(30));
        sink.text(&fs::read_to_string(path)?)?;
        Ok(())
    });
    let registry = Arc::new(fake.registry());
    let index = home.scanned(&registry);
    let read = |index: &Index| count(index, "SELECT COUNT(*) FROM extracts");

    let worker = Worker::start(home.db.clone(), registry.clone(), options()).unwrap();
    eventually("20 files read", || read(&index) >= 20);
    let stats = worker.stop();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    let done = read(&index);
    assert!(done < 50, "stopped part way: {done}");
    assert_eq!(done + queued(&index), 50);
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extract_queue WHERE attempts > 0"), 0, "nothing left claimed");

    let worker = Worker::start(home.db.clone(), registry, options()).unwrap();
    eventually("every file read", || queued(&index) == 0);
    assert!(worker.stop().errors.is_empty());
    assert_eq!(read(&index), 50);
    let reads = fake.reads.lock().unwrap();
    assert_eq!(reads.len(), 50);
    assert!(reads.values().all(|n| *n == 1), "{reads:?}");
}

#[test]
fn a_file_with_an_unfinished_attempt_is_read_alone_and_first() {
    let home = Home::new();
    for name in ["a", "b", "c", "d", "e"] {
        home.write(&format!("{name}.txt"), b"tapir");
    }
    // What the queue holds while each file is read: (file_id, attempts).
    type Claimed = Vec<(i64, i64)>;
    let seen: Arc<Mutex<Vec<Claimed>>> = Arc::default();
    let (db, probe) = (home.db.clone(), seen.clone());
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, _| {
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt =
            conn.prepare("SELECT file_id, attempts FROM extract_queue WHERE attempts > 0 ORDER BY file_id").unwrap();
        let claimed = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        probe.lock().unwrap().push(claimed);
        sink.text(&fs::read_to_string(path)?)?;
        Ok(())
    });
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    // SIFS stopped once while reading c.txt.
    let c = id_of(&index, &home.at("c.txt"));
    index.conn.execute("UPDATE extract_queue SET attempts = 1 WHERE file_id = ?1", [c]).unwrap();
    run(&mut index, &registry);
    assert_eq!(fake.order.lock().unwrap()[0], home.at("c.txt"));
    assert_eq!(seen.lock().unwrap()[0], vec![(c, 2)], "claimed alone, its claim committed before the read");
    assert_eq!(fake.total(), 5);
    assert_eq!(status(&index, &home.at("c.txt")).0, "done");
}

#[test]
fn a_file_that_stopped_sifs_twice_is_recorded_failed_without_being_read() {
    let home = Home::new();
    for name in ["a", "b"] {
        home.write(&format!("{name}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    let a = id_of(&index, &home.at("a.txt"));
    index.conn.execute("UPDATE extract_queue SET attempts = 2 WHERE file_id = ?1", [a]).unwrap();
    run(&mut index, &registry);
    assert_eq!(fake.of(&home.at("a.txt")), 0);
    assert_eq!(fake.of(&home.at("b.txt")), 1);
    assert_eq!(status(&index, &home.at("a.txt")), ("failed".into(), Some(STOPPED_TWICE.into())));
}

// --- the watcher ---

#[test]
fn the_watcher_keeps_what_was_read_current() {
    let home = Home::new();
    home.write("Documents/lease.txt", b"the tenant is Okafor");
    let registry = Arc::new(Registry::platform());
    let index = home.scanned(&registry);
    // It would look again only after a minute: the watcher's wake is what
    // gets each change read.
    let options = WorkerOptions { idle_poll: Duration::from_secs(60), ..options() };
    let worker = Worker::start(home.db.clone(), registry, options).unwrap();
    let waker = worker.waker();
    let _watch = Watch::start(
        home.db.clone(),
        vec![home.root.clone()],
        Arc::new(Mutex::new(Rules::new(&Rules::default_patterns(), &home.root).unwrap())),
        move |_| waker.wake(),
    )
    .unwrap();
    let lease = home.at("Documents/lease.txt");
    eventually("the first words found", || found(&index, "okafor") == vec![lease.clone()]);

    home.write("Documents/lease.txt", b"the new tenant is Haruki Sato");
    eventually("the new words found and the old gone", || {
        found(&index, "haruki") == vec![lease.clone()] && found(&index, "okafor").is_empty()
    });

    fs::remove_file(&lease).unwrap();
    eventually("its words gone", || {
        found(&index, "haruki").is_empty() && count(&index, "SELECT COUNT(*) FROM contents") == 0
    });
    assert!(worker.stop().errors.is_empty());
}

// --- races ---

#[test]
fn a_file_deleted_during_its_read_gets_nothing_written() {
    let home = Home::new();
    home.write("keep.txt", b"tapir");
    let gone = home.write("gone.txt", b"gone okapi");
    let db = home.db.clone();
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, _| {
        let text = fs::read_to_string(path)?;
        if path.ends_with("gone.txt") {
            // Deleted, and the watcher has removed it.
            fs::remove_file(path).unwrap();
            Index::open(&db).unwrap().remove_tree(path.to_str().unwrap()).unwrap();
        }
        sink.text(&text)?;
        Ok(())
    });
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(fake.of(&gone), 1);
    assert!(found(&index, "okapi").is_empty());
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extracts"), 1, "keep.txt only");
    assert_eq!(queued(&index), 0);
}

#[test]
fn a_file_removed_from_the_index_during_its_read_gets_nothing_written_even_if_its_id_is_reused() {
    let home = Home::new();
    home.write("keep.txt", b"tapir");
    let (db, root) = (home.db.clone(), home.root.clone());
    let replacement = home.at("replacement.txt");
    let new_file = replacement.clone();
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, _| {
        let text = fs::read_to_string(path)?;
        if path.ends_with("doomed.txt") {
            // Removed from the index (newly ignored, say) though still on
            // disk, and its id given to a new file.
            let mut index = Index::open(&db).unwrap();
            index.remove_tree(path.to_str().unwrap()).unwrap();
            // As like the old file as can be: the same size and time.
            fs::write(&new_file, b"fresh mangos").unwrap();
            let modified = fs::metadata(path).unwrap().modified().unwrap();
            fs::File::options().write(true).open(&new_file).unwrap().set_modified(modified).unwrap();
            index.upsert(&[record_of(Path::new(&new_file), &root).unwrap()], 1).unwrap();
        }
        sink.text(&text)?;
        Ok(())
    });
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    // Added last, so its id is the highest, and is given to the next file
    // added once it is removed.
    let doomed = home.write("doomed.txt", b"doomed okapi");
    home.refresh(&mut index, "doomed.txt");
    let doomed_id = id_of(&index, &doomed);
    run(&mut index, &registry);
    assert_eq!(id_of(&index, &replacement), doomed_id, "the id was reused");
    assert!(found(&index, "okapi").is_empty(), "the old file's words are not given to the new one");
    assert_eq!(found(&index, "mangos"), vec![replacement], "it has its own");
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extracts"), 2, "keep.txt and the replacement");
}

#[test]
fn a_file_changed_during_its_read_stays_queued_and_is_read_again() {
    let home = Home::new();
    let path = home.write("draft.txt", b"first");
    let (db, root, rules_root) = (home.db.clone(), home.root.clone(), home.root.clone());
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, n| {
        let text = fs::read_to_string(path)?;
        if n == 1 {
            // Saved again while being read; the watcher catches up.
            fs::write(path, b"first draft, then the second").unwrap();
            let mut index = Index::open(&db).unwrap();
            let rules = Rules::new(&Rules::default_patterns(), &rules_root).unwrap();
            refresh(&mut index, path, std::slice::from_ref(&root), &rules).unwrap();
        } else if n == 2 {
            // Saved again, and the watcher hasn't caught up yet.
            fs::write(path, b"first draft, then the second, then the third").unwrap();
        }
        sink.text(&text)?;
        Ok(())
    });
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(fake.of(&path), 3);
    assert_eq!(found(&index, "third"), vec![path.clone()]);
    assert_eq!(count(&index, "SELECT size FROM extracts"), fs::metadata(&path).unwrap().len() as i64);
    assert_eq!(queued(&index), 0);
}

// --- pausing ---

#[test]
fn a_paused_worker_reads_nothing_until_the_pause_ends() {
    let home = Home::new();
    for i in 0..5 {
        home.write(&format!("n{i}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = Arc::new(fake.registry());
    let index = home.scanned(&registry);
    let paused = Arc::new(AtomicBool::new(true));
    let asked = Arc::new(AtomicUsize::new(0));
    let (p, a) = (paused.clone(), asked.clone());
    let worker = Worker::start(
        home.db.clone(),
        registry,
        WorkerOptions {
            pause: Arc::new(move || {
                a.fetch_add(1, Ordering::SeqCst);
                p.load(Ordering::SeqCst)
            }),
            ..options()
        },
    )
    .unwrap();
    eventually("the worker to look several times", || asked.load(Ordering::SeqCst) >= 5);
    assert_eq!(fake.total(), 0);
    assert_eq!(queued(&index), 5);
    paused.store(false, Ordering::SeqCst);
    worker.wake();
    eventually("every file read", || queued(&index) == 0);
    assert_eq!(fake.total(), 5);
    drop(worker);
}

#[test]
fn progress_is_reported_when_the_work_runs_out() {
    let home = Home::new();
    for i in 0..3 {
        home.write(&format!("n{i}.txt"), b"tapir");
    }
    home.write("empty.txt", b"");
    let registry = Arc::new(Registry::platform());
    home.scanned(&registry);
    let last: Arc<Mutex<Option<Progress>>> = Arc::default();
    let l = last.clone();
    let worker = Worker::start(
        home.db.clone(),
        registry,
        WorkerOptions { on_progress: Arc::new(move |p| *l.lock().unwrap() = Some(p.clone())), ..options() },
    )
    .unwrap();
    let want = Progress { read: 3, empty: 1, ..Progress::default() };
    eventually("the final counts", || last.lock().unwrap().as_ref() == Some(&want));
    drop(worker);
}

// --- the samples ---

fn samples() -> (Home, Index) {
    let home = Home::with_samples();
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    let stats = run(&mut index, &registry);
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    (home, index)
}

#[test]
fn every_sample_is_found_by_a_word_inside_it() {
    let (home, index) = samples();
    let sample = |name: &str| home.at(&format!("samples/{name}"));
    for (word, name) in [
        ("lighthouse", "notes.txt"),
        ("backend", "résumé.md"),
        ("quokkas", "main.rs"),
        ("bengaluru", "data.csv"),
        ("mysuru", "data.tsv"),
        ("pelican", "config.json"),
        ("café", "config.json"),
        ("walrus", "broken.json"),
        ("okonkwo", "page.html"),
        ("montréal", "page.html"),
        ("zambezi", "cp1252.html"),
        ("fenwick", "latin1.txt"),
        ("brûlée", "latin1.txt"),
        ("alpaca", "utf16le-bom.txt"),
        ("narwhal", "utf16be-bom.txt"),
        ("pangolin", "utf16le-nobom.txt"),
        ("किराया", "hindi.txt"),
        ("zeppelin", "README"),
    ] {
        assert!(found(&index, word).contains(&sample(name)), "{word} should find {name}: {:?}", found(&index, word));
    }
    // What a browser doesn't show isn't indexed.
    for hidden in ["hiddenstyleword", "hiddenscriptword", "hiddencommentword", "binaryword", "elfword"] {
        assert!(found(&index, hidden).is_empty(), "{hidden}");
    }
    // An entity, and a Windows-1252 quote, decoded.
    assert_eq!(found(&index, "landlord’s"), vec![sample("page.html")]);
    assert_eq!(found(&index, "it’s"), vec![sample("cp1252.html")]);
    for name in ["binary.txt", "program"] {
        assert_eq!(status(&index, &sample(name)), ("skipped".into(), Some("not text".into())), "{name}");
    }
    assert_eq!(store::progress(&index).unwrap(), Progress { read: 15, skipped: 2, ..Progress::default() });
    // The facts.
    let fact = |name: &str, key: &str| fact_of(&index, &sample(name), key);
    assert_eq!(fact("résumé.md", "title").as_deref(), Some("Résumé of Asha Rao"));
    assert_eq!(fact("résumé.md", "author").as_deref(), Some("Asha Rao"));
    assert_eq!(fact("page.html", "title").as_deref(), Some("Flat in Montréal"));
    assert_eq!(fact("page.html", "author").as_deref(), Some("Asha Rao"));
    assert_eq!(fact("cp1252.html", "title").as_deref(), Some("Café"));
    assert_eq!(fact("data.csv", "columns").as_deref(), Some("month, rent, city"));
    assert_eq!(fact("data.csv", "lines").as_deref(), Some("2"));
    assert_eq!(fact("cp1252.html", "encoding").as_deref(), Some("windows-1252"));
    assert_eq!(fact("latin1.txt", "encoding").as_deref(), Some("windows-1252"));
    assert_eq!(fact("utf16le-bom.txt", "encoding").as_deref(), Some("UTF-16LE"));
    assert_eq!(fact("utf16be-bom.txt", "encoding").as_deref(), Some("UTF-16BE"));
    assert_eq!(fact("utf16le-nobom.txt", "encoding").as_deref(), Some("UTF-16LE"));
    assert_eq!(fact("notes.txt", "encoding"), None);
}

#[test]
fn office_files_are_found_by_their_words_with_the_page_sheet_or_slide() {
    use super::readers::office::sample::{self, paras, shape};
    let home = Home::new();
    let lease = home.write(
        "docs/lease.docx",
        &sample::docx(&format!(
            "{}{}",
            paras(&["Tenant pays rent"]),
            r#"<w:p><w:r><w:br w:type="page"/></w:r><w:r><w:t>Deposit refundable किराया</w:t></w:r></w:p>"#
        )),
    );
    let budget = home.write(
        "docs/budget.xlsx",
        &sample::xlsx(
            &[("Q1", r#"<row><c t="s"><v>0</v></c></row>"#), ("Q2", r#"<row><c t="inlineStr"><is><t>Groceries 東京</t></is></c><c><v>4200</v></c></row>"#)],
            &["<t>Electricity</t>"],
            "",
            false,
        ),
    );
    let pitch = home.write(
        "docs/pitch.pptx",
        &sample::pptx(&[(&shape(&["Seed round"]), Some("Thank the investors")), (&shape(&["Traction I❤️you"]), None)]),
    );
    let secret = home.write("docs/secret.docx", &sample::compound(&["EncryptionInfo", "EncryptedPackage"]));
    let whole = sample::docx(&paras(&["cut short"]));
    let cut = home.write("docs/cut.docx", &whole[..whole.len() / 2]);
    let fake = home.write("docs/fake.pptx", &sample::zip(&[("notes.txt", b"a plain zip")]));
    let big = home.write("docs/big.docx", b"");
    fs::File::options().write(true).open(&big).unwrap().set_len(100 * MB + 1).unwrap();
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    let stats = run(&mut index, &registry);
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    let at = |query: &str| -> Vec<(String, String)> {
        store::match_contents(&index, query, 10).unwrap().into_iter().map(|m| (m.path, m.label)).collect()
    };
    assert_eq!(at("tenant"), vec![(lease.clone(), "page 1".into())]);
    assert_eq!(at("deposit"), vec![(lease.clone(), "page 2".into())]);
    assert_eq!(at("किराया"), vec![(lease.clone(), "page 2".into())]);
    assert_eq!(at("electricity"), vec![(budget.clone(), "Q1".into())]);
    assert_eq!(at("groceries"), vec![(budget.clone(), "Q2".into())]);
    assert_eq!(at("東京"), vec![(budget.clone(), "Q2".into())]);
    assert_eq!(at("4200"), vec![(budget.clone(), "Q2".into())]);
    assert_eq!(at("seed"), vec![(pitch.clone(), "slide 1".into())]);
    assert_eq!(at("investors"), vec![(pitch.clone(), "slide 1 notes".into())]);
    assert_eq!(at("you"), vec![(pitch.clone(), "slide 2".into())]);
    assert_eq!(fact_of(&index, &lease, "title").as_deref(), Some("Rent agreement"));
    assert_eq!(fact_of(&index, &budget, "sheets").as_deref(), Some("Q1, Q2"));
    assert_eq!(fact_of(&index, &pitch, "slides").as_deref(), Some("2"));
    assert_eq!(status(&index, &secret), ("skipped".into(), Some("password-protected".into())));
    let (cut_status, why) = status(&index, &cut);
    assert_eq!(cut_status, "failed");
    assert!(why.unwrap().starts_with("damaged or incomplete"));
    assert_eq!(status(&index, &fake), ("skipped".into(), Some("not a PowerPoint presentation".into())));
    assert_eq!(status(&index, &big), ("skipped".into(), Some("larger than 100 MB".into())));
    // Nothing is read again, the failures included, until it changes.
    let again = run(&mut index, &registry);
    assert_eq!((again.reads, queued(&index)), (0, 0));
}

#[test]
fn resume_finds_résumé_composed_and_decomposed() {
    let home = Home::with_samples();
    let decomposed = home.write("samples/cv-nfd.md", "My re\u{301}sume\u{301}, decomposed.".as_bytes());
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    let composed = home.at("samples/résumé.md");
    for query in ["resume", "résumé", "RESUME", "re\u{301}sume\u{301}"] {
        let mut hits = found(&index, query);
        hits.sort();
        let mut want = vec![decomposed.clone(), composed.clone()];
        want.sort();
        assert_eq!(hits, want, "{query}");
    }
}

#[test]
fn the_hindi_sample_is_found_by_its_own_word() {
    let (home, index) = samples();
    assert_eq!(found(&index, "किराया"), vec![home.at("samples/hindi.txt")]);
    assert_eq!(found(&index, "रुपये"), vec![home.at("samples/hindi.txt")]);
}

#[test]
fn indic_words_are_kept_whole() {
    let home = Home::new();
    let rent = home.write("rent.txt", "मकान का किराया बाकी है".as_bytes());
    let work = home.write("work.txt", "काम कराया गया".as_bytes());
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    index.conn.execute_batch("CREATE VIRTUAL TABLE temp.terms USING fts5vocab(main, contents_fts, row)").unwrap();
    let mut stmt = index.conn.prepare("SELECT term FROM temp.terms ORDER BY term").unwrap();
    let terms: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    // Vowel signs are part of the word, not separators.
    assert!(terms.contains(&"किराया".to_string()), "{terms:?}");
    assert!(terms.contains(&"कराया".to_string()), "{terms:?}");
    assert_eq!(found(&index, "किराया"), vec![rent]);
    assert_eq!(found(&index, "कराया"), vec![work]);
    // A fragment of a word is not the word.
    assert!(found(&index, "कर").is_empty(), "{:?}", found(&index, "कर"));
    assert!(found(&index, "क").is_empty(), "{:?}", found(&index, "क"));
}

#[test]
fn the_migration_keeps_every_phase_1_row() {
    let home = Home::new();
    for i in 0..300 {
        home.write(&format!("d{}/f{i}.txt", i % 7), format!("body {i}").as_bytes());
    }
    let dump = |conn: &rusqlite::Connection| -> Vec<String> {
        let mut out = Vec::new();
        for table in ["files", "folders", "ignore_patterns", "meta"] {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table} ORDER BY 1")).unwrap();
            let n = stmt.column_count();
            let rows = stmt
                .query_map([], |r| {
                    Ok((0..n).map(|c| format!("{:?}", r.get_ref(c).unwrap())).collect::<Vec<_>>().join("|"))
                })
                .unwrap();
            out.extend(rows.map(|r| format!("{table}:{}", r.unwrap())));
        }
        let mut stmt = conn.prepare("SELECT rowid FROM files_fts WHERE files_fts MATCH 'f12' ORDER BY rowid").unwrap();
        out.extend(stmt.query_map([], |r| r.get::<_, i64>(0)).unwrap().map(|r| format!("fts:{}", r.unwrap())));
        out
    };
    let before = {
        // Written as phase 1 wrote them.
        let conn = rusqlite::Connection::open(&home.db).unwrap();
        conn.execute_batch(crate::index::db::SCHEMA).unwrap();
        for entry in walkdir::WalkDir::new(&home.root) {
            let r = record_of(entry.unwrap().path(), &home.root).unwrap();
            conn.execute(
                "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode, seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                params![r.path, r.root, r.name, r.ext, r.is_dir, r.size, r.modified, r.inode],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO folders VALUES (?1, 5)", [&home.root]).unwrap();
        conn.execute("INSERT INTO ignore_patterns VALUES ('**/x')", []).unwrap();
        conn.execute("INSERT INTO meta VALUES ('ignore_set', '1')", []).unwrap();
        conn.execute("INSERT INTO meta VALUES ('scan', '3')", []).unwrap();
        dump(&conn)
    };
    assert_eq!(dump(&home.open().conn), before);
    let index = home.open();
    assert_eq!(dump(&index.conn), before, "a second open changes nothing");
    assert_eq!(count(&index, "PRAGMA user_version"), 1);
}

#[test]
fn a_migration_that_fails_leaves_the_index_as_it_was() {
    let home = Home::new();
    {
        let conn = rusqlite::Connection::open(&home.db).unwrap();
        conn.execute_batch(crate::index::db::SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode) VALUES ('/a/x.txt', '/a', 'x.txt', 'txt', 0, 1, 1, 1)",
            [],
        )
        .unwrap();
        // Migration 1 fails part way, at CREATE TABLE facts.
        conn.execute_batch("CREATE TABLE facts (x)").unwrap();
    }
    assert!(Index::open(&home.db).is_err());
    let conn = rusqlite::Connection::open(&home.db).unwrap();
    assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    let left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('extracts', 'contents', 'contents_fts', 'files_ai_extract')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0, "nothing of the failed migration is left");
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn a_rename_moves_the_text_and_the_full_text_index_stays_consistent() {
    let home = Home::new();
    home.write("a/old.txt", b"okapi alpha");
    home.write("a/two.txt", b"okapi beta");
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    fs::rename(home.at("a/old.txt"), home.at("a/new.txt")).unwrap();
    home.refresh(&mut index, "a/new.txt");
    home.refresh(&mut index, "a/old.txt");
    home.refresh(&mut index, "a/two.txt");
    assert_eq!(queued(&index), 1, "only the renamed file");
    run(&mut index, &registry);
    assert_eq!(found(&index, "alpha"), vec![home.at("a/new.txt")]);
    assert_eq!(count(&index, "SELECT COUNT(*) FROM contents_fts WHERE contents_fts MATCH 'alpha'"), 1);
    let integrity = "INSERT INTO contents_fts(contents_fts, rank) VALUES ('integrity-check', 1)";
    index.conn.execute(integrity, []).unwrap();
    fs::remove_dir_all(home.at("a")).unwrap();
    home.refresh(&mut index, "a");
    assert_eq!(count(&index, "SELECT COUNT(*) FROM contents_fts WHERE contents_fts MATCH 'okapi'"), 0);
    index.conn.execute(integrity, []).unwrap();
}

#[test]
fn queries_with_full_text_syntax_in_them_are_safe() {
    let home = Home::new();
    home.write("d.txt", b"dont' \"quoted\" weird*chars OR AND NOT NEAR");
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    for q in ["\"", "*", "OR", "NEAR(", "a:b", "-x", "^", "\"quoted\"", "weird*", "(", ")", "{a}"] {
        let r = store::match_contents(&index, q, 10);
        assert!(r.is_ok(), "query {q:?}: {r:?}");
    }
    assert_eq!(found(&index, "OR"), vec![home.at("d.txt")], "a word, not an operator");
}

/// The crawler's upsert keeps a row's id when its path stays, so a file
/// replaced by a folder or a link of the same name is the same row.
#[test]
fn a_file_replaced_by_a_folder_of_the_same_name_loses_its_text() {
    let home = Home::new();
    home.write("notes.txt", b"zebu secret words");
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(found(&index, "zebu"), vec![home.at("notes.txt")]);
    fs::remove_file(home.at("notes.txt")).unwrap();
    fs::create_dir(home.at("notes.txt")).unwrap();
    home.refresh(&mut index, "notes.txt");
    run(&mut index, &registry);
    assert!(found(&index, "zebu").is_empty(), "{:?}", found(&index, "zebu"));
    let id = id_of(&index, &home.at("notes.txt"));
    for table in ["contents", "facts", "extracts", "extract_queue"] {
        assert_eq!(count(&index, &format!("SELECT COUNT(*) FROM {table} WHERE file_id = {id}")), 0, "{table}");
    }
}

#[test]
fn a_file_replaced_by_a_link_of_the_same_name_loses_its_text() {
    let home = Home::new();
    home.write("notes.txt", b"zebu secret words");
    home.write("other.bin", b"x");
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    fs::remove_file(home.at("notes.txt")).unwrap();
    std::os::unix::fs::symlink(home.at("other.bin"), home.at("notes.txt")).unwrap();
    home.refresh(&mut index, "notes.txt");
    run(&mut index, &registry);
    assert!(found(&index, "zebu").is_empty(), "{:?}", found(&index, "zebu"));
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extracts"), 0);
    assert_eq!(queued(&index), 0);
}

/// Renamed while being read, and the new path's row took the old id (the
/// old row had the highest), with the time and size a rename keeps.
#[test]
fn a_file_renamed_during_its_read_onto_its_own_id_is_still_read() {
    let home = Home::new();
    home.write("keep.txt", b"tapir");
    let (db, root) = (home.db.clone(), home.root.clone());
    let renamed = home.at("renamed.txt");
    let to = renamed.clone();
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, _| {
        let text = fs::read_to_string(path)?;
        if path.ends_with("doomed.txt") {
            fs::rename(path, &to).unwrap();
            // The watcher sees the old path gone, then the new one.
            let mut index = Index::open(&db).unwrap();
            index.remove_tree(path.to_str().unwrap()).unwrap();
            index.upsert(&[record_of(Path::new(&to), &root).unwrap()], 1).unwrap();
        }
        sink.text(&text)?;
        Ok(())
    });
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    let doomed = home.write("doomed.txt", b"doomed okapi");
    home.refresh(&mut index, "doomed.txt");
    let doomed_id = id_of(&index, &doomed);
    run(&mut index, &registry);
    assert_eq!(id_of(&index, &renamed), doomed_id, "the id was reused");
    assert_eq!(found(&index, "okapi"), vec![renamed.clone()], "read under its new name");
    assert_eq!(fake.of(&renamed), 1);
}

#[test]
fn a_fact_is_cut_at_a_thousand_characters_on_a_character_boundary() {
    let mut sink = Sink::new(100, Instant::now() + Duration::from_secs(10));
    sink.fact("title", "é".repeat(5000));
    sink.fact("author", "Asha Rao");
    let (_, facts) = sink.finish();
    assert_eq!(facts[0].1, "é".repeat(MAX_FACT_CHARS));
    assert_eq!(MAX_FACT_CHARS, 1000);
    assert_eq!(facts[1].1, "Asha Rao");
}

#[test]
fn an_html_title_without_its_end_tag_makes_a_capped_fact() {
    let home = Home::new();
    let page = home.write("page.html", format!("<title>Start {}", "word ".repeat(100_000)).as_bytes());
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(fact_of(&index, &page, "title").unwrap().chars().count(), MAX_FACT_CHARS);
}

#[test]
fn an_error_after_a_claim_puts_the_files_back_and_is_not_a_crash() {
    let home = Home::new();
    for name in ["a", "b", "c"] {
        home.write(&format!("{name}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    // Every write of what was read fails, as on a full disk.
    index
        .conn
        .execute_batch("CREATE TRIGGER fail BEFORE INSERT ON extracts BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .unwrap();
    for _ in 0..3 {
        assert!(run_until_empty(&mut index, &registry, &options()).is_err());
        assert_eq!(count(&index, "SELECT COUNT(*) FROM extract_queue WHERE attempts > 0"), 0, "the claims are undone");
    }
    index.conn.execute_batch("DROP TRIGGER fail").unwrap();
    run(&mut index, &registry);
    for name in ["a", "b", "c"] {
        assert_eq!(status(&index, &home.at(&format!("{name}.txt"))), ("done".into(), None), "{name}");
    }
}

#[test]
fn at_most_four_reads_are_abandoned_at_once_across_every_reader() {
    let home = Home::new();
    for name in ["a1.one", "a2.one", "a3.one", "b1.two", "b2.two", "b3.two"] {
        home.write(name, b"x");
    }
    let stuck = |id: &'static str, ext: &'static str| {
        let mut fake = Fake::new(id, &[ext]).body(|_, _, _| {
            std::thread::sleep(Duration::from_secs(2));
            Ok(())
        });
        fake.limits.time = Duration::from_millis(100);
        Arc::new(fake) as Arc<dyn Reader>
    };
    let registry = Registry::with(vec![stuck("one", "one"), stuck("two", "two")]);
    let mut index = home.scanned(&registry);
    let stats = run(&mut index, &registry);
    assert_eq!(stats.abandoned, 4);
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extracts WHERE status = 'failed'"), 4);
    assert_eq!(queued(&index), 2, "the rest wait until a stuck read ends");
    assert_eq!(count(&index, "SELECT COUNT(*) FROM extract_queue WHERE attempts > 0"), 0);
}

#[test]
fn a_removed_folder_goes_in_small_batches_and_its_removal_survives_a_restart() {
    let home = Home::new();
    for i in 0..25 {
        home.write(&format!("Docs/n{i}.txt"), b"okapi words");
    }
    home.write("Other/keep.txt", b"okapi kept");
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (docs, other) = (home.at("Docs"), home.at("Other"));
    for root in [&docs, &other] {
        index.add_folder(root, 1).unwrap();
        scan(&mut index, root, &home.rules, |_| {}).unwrap();
    }
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 26);

    assert!(index.forget_folder(&docs).unwrap());
    assert_eq!(index.folders().unwrap(), vec![other.clone()]);
    drop(index);
    // SIFS stopped before the files were removed: the next start knows.
    let mut index = home.open();
    assert_eq!(index.removals().unwrap(), vec![docs.clone()]);
    let mut batches = 0;
    let removed = index.remove_root_in_batches(&docs, 10, |_| batches += 1).unwrap();
    assert_eq!((removed, batches), (26, 3), "the folder and its 25 files, ten at a time");
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert!(index.removals().unwrap().is_empty());
    assert_eq!(found(&index, "okapi"), vec![home.at("Other/keep.txt")]);
    let orphans = "SELECT COUNT(*) FROM contents WHERE file_id NOT IN (SELECT id FROM files)";
    assert_eq!(count(&index, orphans), 0);
}

#[test]
fn a_file_opened_in_sifs_is_moved_to_the_front_by_the_worker() {
    let home = Home::new();
    for name in ["a", "b"] {
        let path = home.write(&format!("{name}.txt"), b"tapir");
        set_age(&path, 400);
    }
    let registry = Arc::new(Registry::platform());
    let index = home.scanned(&registry);
    let worker = Worker::start(
        home.db.clone(),
        registry,
        WorkerOptions { pause: Arc::new(|| true), idle_poll: Duration::from_secs(60), ..options() },
    )
    .unwrap();
    worker.opened(&home.at("b.txt"));
    let priority = |name: &str| {
        index
            .conn
            .query_row(
                "SELECT priority FROM extract_queue WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
                [home.at(name)],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    eventually("the opened file moved to the front, even while paused", || priority("b.txt") == -1);
    assert_eq!(priority("a.txt"), 1);
    drop(worker);
}

// --- files that are not regular files ---

#[cfg(unix)]
fn mkfifo(path: &str) {
    let c = std::ffi::CString::new(path).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0, "mkfifo {path}");
}

/// While it lives, keeps opening the writing end of each FIFO without
/// blocking, so a read stuck opening one ends at once rather than at its
/// time limit: a test that fails, fails fast, and leaves no thread behind.
#[cfg(unix)]
struct Unblocker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Unblocker {
    fn new(paths: Vec<String>) -> Unblocker {
        use std::os::unix::fs::OpenOptionsExt;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                for path in &paths {
                    let _ = fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(path);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Unblocker { stop, thread: Some(thread) }
    }
}

#[cfg(unix)]
impl Drop for Unblocker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A FIFO with a reader's extension, or none (the plain reader takes
/// those), blocks in open(); four of them used to stop every read.
#[cfg(unix)]
#[test]
fn fifos_are_not_indexed_and_never_stop_reading() {
    let home = Home::new();
    let fifos: Vec<String> = ["p1.txt", "p2.txt", "p3.txt", "p4.txt", "pipe"].iter().map(|n| home.at(n)).collect();
    for path in &fifos {
        mkfifo(path);
    }
    let real = home.write("real.txt", b"tapir real words");
    set_age(&real, 400); // read after the fifos, if they were queued
    let _unblock = Unblocker::new(fifos.clone());
    let mut fake = Fake::new("fake", &["txt", ""]);
    fake.limits.time = Duration::from_millis(200);
    let registry = fake.registry();
    let mut index = home.scanned(&registry);
    for path in &fifos {
        assert!(index.get(path).unwrap().is_none(), "{path} is indexed");
    }
    let stats = run(&mut index, &registry);
    assert_eq!((stats.reads, stats.abandoned), (1, 0));
    assert_eq!(found(&index, "tapir"), vec![real.clone()]);
    // Nor does the watcher index one.
    home.refresh(&mut index, "pipe");
    assert!(index.get(&home.at("pipe")).unwrap().is_none());
}

/// A file read, then replaced by a FIFO before the index knew: the worker
/// sees it just before opening it, and turns it away.
#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_fifo_is_cleared_not_opened() {
    let home = Home::new();
    let names = ["p1.txt", "p2.txt", "p3.txt", "p4.txt", "pipe"];
    for name in names {
        home.write(name, b"zebu words");
    }
    let real = home.write("real.txt", b"tapir real words");
    set_age(&real, 400);
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    assert_eq!(found(&index, "zebu").len(), 5);
    let fifos: Vec<String> = names.iter().map(|n| home.at(n)).collect();
    for path in &fifos {
        fs::remove_file(path).unwrap();
        mkfifo(path);
        // Queued again, as a change the watcher had not seen yet would be.
        index.conn.execute("UPDATE files SET size = size + 1 WHERE path = ?1", [path]).unwrap();
    }
    home.write("real.txt", b"tapir real words again");
    set_age(&real, 400);
    home.refresh(&mut index, "real.txt");
    assert_eq!(queued(&index), 6);
    let _unblock = Unblocker::new(fifos);
    let started = std::time::Instant::now();
    let stats = run(&mut index, &registry);
    assert_eq!((stats.reads, stats.abandoned), (1, 0), "only real.txt is opened");
    assert!(found(&index, "zebu").is_empty(), "what was read from them is cleared");
    assert_eq!(found(&index, "again"), vec![real]);
    assert!(started.elapsed() < Duration::from_secs(5));
}

// --- folders scanned and removed in the background ---

use crate::index::folders::{Event, Folders};

fn folders(home: &Home, on_event: impl Fn(Event) + Send + Sync + 'static) -> Folders {
    Folders::new(home.db.clone(), home.root.clone(), Duration::from_millis(20), on_event)
}

fn name_hits(index: &Index, query: &str) -> usize {
    crate::index::search::search(index, query, 100, now()).unwrap().len()
}

/// The scan of a folder still running when the folder is removed used to
/// write rows after the removal ended, and nothing ever removed them.
#[test]
fn a_folder_removed_during_its_scan_leaves_nothing_behind() {
    let home = Home::new();
    // More than one batch of the crawler's (2000), so the scan is caught
    // between two.
    for i in 0..2100 {
        home.write(&format!("Docs/d{}/n{i}.txt", i % 10), b"okapi words");
    }
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let docs = home.at("Docs");
    let (in_flight, waiting) = std::sync::mpsc::channel::<()>();
    let (go, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let first = AtomicBool::new(true);
    let scanned = Arc::new(Mutex::new(None));
    let stats = scanned.clone();
    let folders = folders(&home, move |event| {
        if let Event::Scanned(s) = &event {
            *stats.lock().unwrap() = Some(s.clone());
        }
        // The first batch is written: hold the scan there.
        if let Event::Progress { .. } = event {
            if first.swap(false, Ordering::SeqCst) {
                in_flight.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
            }
        }
    });
    folders.add(&mut index, &docs, now()).unwrap();
    folders.scan(docs.clone());
    waiting.recv().unwrap();
    assert!(index.count_root(&docs).unwrap() >= 2000);
    folders.remove(&mut index, &docs).unwrap();
    assert_eq!(name_hits(&index, "n12"), 0, "not found while it is being removed");
    // Time for a removal that didn't wait for the scan to run to its end.
    std::thread::sleep(Duration::from_millis(300));
    go.send(()).unwrap();
    eventually("the scan and the removal to end", || folders.idle());
    assert_eq!(index.count_root(&docs).unwrap(), 0, "left behind after the removal");
    let stats = scanned.lock().unwrap().clone().unwrap();
    assert!(stats.cancelled && stats.indexed == 2000, "the scan stopped before its next batch: {stats:?}");
    assert!(index.removals().unwrap().is_empty());
    assert!(index.folders().unwrap().is_empty());
}

/// Rows of a folder no longer searched, however they got there, are
/// removed at the next start.
#[test]
fn rows_no_folder_owns_are_removed_at_startup() {
    let home = Home::new();
    for i in 0..30 {
        home.write(&format!("Docs/n{i}.txt"), b"okapi words");
    }
    home.write("Other/keep.txt", b"okapi kept");
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (docs, other) = (home.at("Docs"), home.at("Other"));
    index.add_folder(&other, 1).unwrap();
    for root in [&docs, &other] {
        scan(&mut index, root, &home.rules, |_| {}).unwrap();
    }
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 31);
    let folders = folders(&home, |_| {});
    folders.resume(&mut index).unwrap();
    eventually("the leftover rows to be removed", || folders.idle() && index.count_root(&docs).unwrap() == 0);
    assert_eq!(found(&index, "okapi"), vec![home.at("Other/keep.txt")]);
    assert!(index.removals().unwrap().is_empty());
    assert_eq!(index.count_root(&other).unwrap(), 2);
}

/// Name and text searches leave out a folder being removed.
#[test]
fn a_folder_being_removed_is_not_found() {
    let home = Home::new();
    home.write("Docs/okapi-notes.txt", b"okapi words");
    home.write("Other/okapi-kept.txt", b"okapi kept");
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (docs, other) = (home.at("Docs"), home.at("Other"));
    for root in [&docs, &other] {
        index.add_folder(root, 1).unwrap();
        scan(&mut index, root, &home.rules, |_| {}).unwrap();
    }
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 2);
    assert_eq!(name_hits(&index, "okapi"), 2);
    assert_eq!(name_hits(&index, "okapy"), 2, "the typo-tolerant search finds both");
    // Marked, its rows not yet removed.
    index.forget_folder(&docs).unwrap();
    assert_eq!(found(&index, "okapi"), vec![home.at("Other/okapi-kept.txt")]);
    let names = |q: &str| -> Vec<String> {
        crate::index::search::search(&index, q, 100, now()).unwrap().into_iter().map(|h| h.file.path).collect()
    };
    assert_eq!(names("okapi"), vec![home.at("Other/okapi-kept.txt")]);
    assert_eq!(names("ok"), vec![home.at("Other/okapi-kept.txt")]);
    assert_eq!(names("okapy"), vec![home.at("Other/okapi-kept.txt")]);
}

/// A removal that fails is tried again, and adding the folder while its
/// mark is left with no removal running starts one.
#[test]
fn a_failed_removal_is_tried_again() {
    let home = Home::new();
    for i in 0..30 {
        home.write(&format!("Docs/n{i}.txt"), b"okapi words");
    }
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let docs = home.at("Docs");
    let folders = folders(&home, |_| {});
    folders.add(&mut index, &docs, now()).unwrap();
    scan(&mut index, &docs, &home.rules, |_| {}).unwrap();
    index
        .conn
        .execute_batch("CREATE TRIGGER fail BEFORE DELETE ON files BEGIN SELECT RAISE(ABORT, 'busy'); END")
        .unwrap();
    folders.remove(&mut index, &docs).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(index.count_root(&docs).unwrap(), 31);
    assert!(!folders.idle(), "still trying");
    assert!(folders.add(&mut index, &docs, now()).is_err(), "still being removed");
    index.conn.execute_batch("DROP TRIGGER fail").unwrap();
    eventually("the removal to succeed on a later try", || folders.idle());
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert!(index.removals().unwrap().is_empty());

    // A mark left with no removal running (as after an error, before this
    // fix): adding the folder again starts its removal.
    for i in 0..5 {
        home.write(&format!("Docs/m{i}.txt"), b"okapi words");
    }
    index.add_folder(&docs, 1).unwrap();
    scan(&mut index, &docs, &home.rules, |_| {}).unwrap();
    assert!(index.forget_folder(&docs).unwrap());
    let err = folders.add(&mut index, &docs, now()).unwrap_err().to_string();
    assert!(err.contains("still being removed"), "{err}");
    eventually("the removal add_folder started", || folders.idle() && index.removals().unwrap().is_empty());
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    folders.add(&mut index, &docs, now()).unwrap();
    assert_eq!(index.folders().unwrap(), vec![docs]);
}


/// The emoji variation selectors (U+FE0E, U+FE0F) and the keycap (U+20E3)
/// are marks, but they are not part of a word: "I❤️you" has the word "you",
/// and "❤️" is no word at all, rather than a lone selector every emoji with
/// one matches.
#[test]
fn emoji_selectors_and_keycaps_separate_words() {
    let home = Home::new();
    let love = home.write("a.txt", "I❤\u{fe0f}you".as_bytes());
    home.write("b.txt", "nothing ⭐\u{fe0f} here".as_bytes());
    let keycap = home.write("c.txt", "1\u{fe0f}\u{20e3} first, a\u{fe0e}b".as_bytes());
    let registry = Registry::platform();
    let mut index = home.scanned(&registry);
    run(&mut index, &registry);
    index.conn.execute_batch("CREATE VIRTUAL TABLE temp.terms USING fts5vocab(main, contents_fts, row)").unwrap();
    let mut stmt = index.conn.prepare("SELECT term FROM temp.terms ORDER BY term").unwrap();
    let terms: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(terms, ["1", "a", "b", "first", "here", "i", "nothing", "you"]);
    assert_eq!(found(&index, "you"), vec![love]);
    assert!(found(&index, "❤\u{fe0f}").is_empty(), "{:?}", found(&index, "❤\u{fe0f}"));
    assert!(found(&index, "\u{fe0f}").is_empty());
    assert_eq!(found(&index, "1\u{fe0f}\u{20e3}"), vec![keycap]);
}

// --- errors that keep happening ---

#[test]
fn the_wait_after_errors_doubles_up_to_ten_minutes() {
    use super::worker::backoff;
    let s = Duration::from_secs;
    assert_eq!([1, 2, 3, 4].map(|n| backoff(s(5), n)), [s(5), s(10), s(20), s(40)]);
    assert_eq!(backoff(s(5), 8), s(600));
    assert_eq!(backoff(s(5), 1000), s(600));
}

/// A write that fails after every claim (a full disk) used to have the same
/// files read again every idle_poll, without end.
#[test]
fn errors_in_a_row_are_waited_out_longer_each_time() {
    let home = Home::new();
    for name in ["a", "b", "c"] {
        home.write(&format!("{name}.txt"), b"tapir");
    }
    let fake = Fake::new("fake", &["txt"]);
    let registry = Arc::new(fake.registry());
    let index = home.scanned(&registry);
    index
        .conn
        .execute_batch("CREATE TRIGGER fail BEFORE INSERT ON extracts BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .unwrap();
    let worker = Worker::start(home.db.clone(), registry, options()).unwrap();
    // 20 ms, then 40, 80, 160, 320: five tries in 700 ms, not thirty.
    std::thread::sleep(Duration::from_millis(700));
    let stats = worker.stop();
    assert!((2..=7).contains(&stats.errors.len()), "{} tries", stats.errors.len());
    assert!(fake.total() <= 3 * 7);
}

/// Reads still running when writing a batch fails count as abandoned, so
/// the cap on stuck reads still holds.
#[test]
fn reads_still_running_when_a_write_fails_count_as_abandoned() {
    let home = Home::new();
    home.write("fast.txt", b"tapir");
    let slow = home.write("slow.txt", b"tapir");
    set_age(&slow, 400);
    let mut fake = Fake::new("fake", &["txt"]).body(|path, sink, _| {
        if path.ends_with("slow.txt") {
            std::thread::sleep(Duration::from_millis(600));
        }
        sink.text("tapir")?;
        Ok(())
    });
    fake.limits.time = Duration::from_secs(30);
    let registry = Arc::new(fake.registry());
    let index = home.scanned(&registry);
    index
        .conn
        .execute_batch("CREATE TRIGGER fail BEFORE INSERT ON extracts BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .unwrap();
    // Each read is written as soon as it ends, so fast.txt's write fails
    // while slow.txt is still being read.
    let opts = WorkerOptions { concurrency: 2, batch_bytes: 1, idle_poll: Duration::from_secs(60), ..options() };
    let worker = Worker::start(home.db.clone(), registry, opts).unwrap();
    eventually("the first write to fail", || fake.total() >= 2);
    std::thread::sleep(Duration::from_millis(100));
    let stats = worker.stop();
    assert_eq!(stats.errors.len(), 1, "{:?}", stats.errors);
    assert_eq!(stats.abandoned, 1);
}

// --- a folder added again, or asked to be scanned again, while it scans ---

/// A scan held after its first batch: the Folders, and the gate that lets it go on.
fn held_scan(home: &Home, n: usize) -> (Folders, std::sync::mpsc::Sender<()>) {
    for i in 0..n {
        home.write(&format!("Docs/d{}/n{i}.txt", i % 10), b"okapi words");
    }
    let (in_flight, waiting) = std::sync::mpsc::channel::<()>();
    let (go, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let first = AtomicBool::new(true);
    let folders = folders(home, move |event| {
        if let Event::Progress { .. } = event {
            if first.swap(false, Ordering::SeqCst) {
                in_flight.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
            }
        }
    });
    let mut index = home.open();
    let docs = home.at("Docs");
    folders.add(&mut index, &docs, now()).unwrap();
    folders.scan(docs);
    waiting.recv().unwrap();
    (folders, go)
}

/// The reviewer's rv3_readd_while_cancelled_scan_ends_is_never_scanned: a
/// folder removed during its scan and added again before that scan ended
/// was kept but never scanned.
#[test]
fn a_folder_added_again_before_its_cancelled_scan_ends_is_scanned() {
    let home = Home::new();
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (folders, go) = held_scan(&home, 2100);
    let docs = home.at("Docs");
    folders.remove(&mut index, &docs).unwrap();
    let err = folders.add(&mut index, &docs, now()).unwrap_err().to_string();
    assert!(err.contains("still being removed"), "{err}");
    eventually("the removal add started to end", || index.removals().unwrap().is_empty());
    // "add it again in a moment"
    folders.add(&mut index, &docs, now()).unwrap();
    folders.scan(docs.clone());
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    assert_eq!(index.folders().unwrap(), vec![docs.clone()]);
    assert_eq!(index.count_root(&docs).unwrap(), 2111, "the folder, 10 subfolders and 2100 files");
}

/// Asked to scan while it scans (the ignore rules changed): another scan
/// follows, with the new rules.
#[test]
fn a_scan_asked_for_during_a_scan_follows_it() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let docs = home.at("Docs");
    let mut patterns = Rules::default_patterns();
    patterns.push("**/d3".into());
    index.set_ignore_patterns(&patterns).unwrap();
    folders.scan(docs.clone());
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    let under_d3 = count(&index, &format!("SELECT COUNT(*) FROM files WHERE path LIKE '{docs}/d3%'"));
    assert_eq!(under_d3, 0, "the second scan used the new rules");
    assert_eq!(index.count_root(&docs).unwrap(), 2111 - 211);
}

/// A removal deletes nothing of a folder that is searched again, even when
/// it was asked for before the folder was added again.
#[test]
fn a_removal_never_deletes_the_rows_of_a_folder_searched_again() {
    let home = Home::new();
    for i in 0..30 {
        home.write(&format!("Docs/n{i}.txt"), b"okapi words");
    }
    let mut index = home.open();
    let docs = home.at("Docs");
    index.add_folder(&docs, 1).unwrap();
    scan(&mut index, &docs, &home.rules, |_| {}).unwrap();
    assert_eq!(index.remove_root_in_batches(&docs, 10, |_| {}).unwrap(), 0);
    assert_eq!(index.count_root(&docs).unwrap(), 31);
}

/// Removed, added again and removed again, all before its scan ends: no
/// scan follows, and nothing is left.
#[test]
fn a_folder_removed_added_and_removed_again_during_its_scan_leaves_nothing() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let docs = home.at("Docs");
    folders.remove(&mut index, &docs).unwrap();
    let _ = folders.add(&mut index, &docs, now());
    eventually("the removal add started to end", || index.removals().unwrap().is_empty());
    folders.add(&mut index, &docs, now()).unwrap();
    folders.scan(docs.clone());
    folders.remove(&mut index, &docs).unwrap();
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    assert!(index.folders().unwrap().is_empty());
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert!(index.removals().unwrap().is_empty());
}

// --- folders never nest ---

/// The reviewer's rv3_removing_a_nested_child_folder_keeps_the_parents_files:
/// with P/C added, then P, then P/C scanned again (as a restart or new
/// ignore rules do), removing P/C removed P's files under it. Now adding P
/// takes P/C in: P/C is no longer a folder, removing it does nothing, and
/// its files are P's.
#[test]
fn adding_a_folder_around_another_takes_it_in() {
    let home = Home::new();
    for i in 0..20 {
        home.write(&format!("P/C/n{i}.txt"), b"okapi words");
    }
    home.write("P/top.txt", b"okapi top");
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (p, c) = (home.at("P"), home.at("P/C"));
    let folders = folders(&home, |_| {});
    assert!(folders.add(&mut index, &c, now()).unwrap().is_empty());
    scan(&mut index, &c, &home.rules, |_| {}).unwrap();
    run(&mut index, &registry);
    assert_eq!(folders.add(&mut index, &p, now()).unwrap(), vec![c.clone()]);
    assert_eq!(index.folders().unwrap(), vec![p.clone()]);
    assert_eq!(index.count_root(&c).unwrap(), 0, "C's rows are P's at once");
    assert_eq!(queued(&index), 0, "and what was read from them is kept");
    scan(&mut index, &p, &home.rules, |_| {}).unwrap();
    // Adding C again is adding a folder inside P.
    let err = folders.add(&mut index, &c, now()).unwrap_err().to_string();
    assert!(err.contains("already searched"), "{err}");
    folders.remove(&mut index, &c).unwrap();
    eventually("settle", || folders.idle());
    let under_c = count(&index, &format!("SELECT COUNT(*) FROM files WHERE path LIKE '{c}/%' AND root = '{p}'"));
    assert_eq!(under_c, 20, "P is still a folder, its C files must stay");
    assert!(index.removals().unwrap().is_empty());
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 21);
}

/// A folder taken in while it is being scanned: its scan stops, and what it
/// wrote becomes the new folder's.
#[test]
fn a_folder_taken_in_during_its_scan_gives_its_files_to_the_new_one() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let (docs, root) = (home.at("Docs"), home.root.clone());
    home.write("top.txt", b"okapi top");
    assert_eq!(folders.add(&mut index, &root, now()).unwrap(), vec![docs.clone()]);
    assert_eq!(index.count_root(&docs).unwrap(), 0, "what Docs's scan wrote is the home folder's at once");
    // Docs's scan ends before the home folder is scanned: it stops, and
    // writes nothing more under Docs.
    go.send(()).unwrap();
    eventually("Docs's scan to end", || folders.idle());
    assert_eq!(index.count_root(&docs).unwrap(), 0, "Docs's scan stopped");
    assert_eq!(index.count_root(&root).unwrap(), 2000);
    folders.scan(root.clone());
    eventually("everything to settle", || folders.idle());
    assert_eq!(index.folders().unwrap(), vec![root.clone()]);
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert_eq!(index.count_root(&root).unwrap(), 2111 + 2, "Docs's rows, the home folder and top.txt");
    assert!(index.removals().unwrap().is_empty());
}

/// Folders inside other folders left by phase 1 are taken in at startup,
/// keeping their files and what was read from them.
#[test]
fn nested_folders_are_taken_in_at_startup() {
    let home = Home::new();
    for i in 0..20 {
        home.write(&format!("P/C/n{i}.txt"), b"okapi words");
    }
    home.write("P/C/D/deep.txt", b"okapi deep");
    home.write("P/top.txt", b"okapi top");
    home.write("Q/q.txt", b"okapi q");
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (p, c, d, q) = (home.at("P"), home.at("P/C"), home.at("P/C/D"), home.at("Q"));
    for folder in [&c, &p, &d, &q] {
        index.add_folder(folder, 1).unwrap();
    }
    // Scanned in any order, as at a phase 1 start: the last one wins the rows.
    for folder in [&p, &c, &d, &q] {
        scan(&mut index, folder, &home.rules, |_| {}).unwrap();
    }
    run(&mut index, &registry);
    assert_eq!(found(&index, "okapi").len(), 23);
    let folders = folders(&home, |_| {});
    folders.resume(&mut index).unwrap();
    assert_eq!(index.folders().unwrap(), vec![p.clone(), q.clone()]);
    eventually("the scans to end", || folders.idle());
    assert_eq!((index.count_root(&c).unwrap(), index.count_root(&d).unwrap()), (0, 0));
    assert_eq!(index.count_root(&p).unwrap(), 1 + 1 + 20 + 1 + 1 + 1, "P, C, C's files, D, deep.txt, top.txt");
    assert!(index.removals().unwrap().is_empty());
    assert_eq!(queued(&index), 0, "nothing read again");
    assert_eq!(found(&index, "okapi").len(), 23);
}

/// A batch the scan of a folder taken in was writing when it was taken in
/// lands after the new folder took its rows: it becomes the new folder's
/// too when that scan ends, rather than being left to a removal.
#[test]
fn a_batch_written_after_a_folder_was_taken_in_is_given_to_the_new_one() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let (docs, root) = (home.at("Docs"), home.root.clone());
    assert_eq!(folders.add(&mut index, &root, now()).unwrap(), vec![docs.clone()]);
    // As the scan's last upsert, waiting on the writer lock, would.
    let late = home.write("Docs/late.txt", b"okapi late");
    let record = record_of(Path::new(&late), &docs).unwrap();
    index.upsert(&[record], index.current_scan().unwrap()).unwrap();
    go.send(()).unwrap();
    eventually("the stopped scan to end", || folders.idle());
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert_eq!(index.get(&late).unwrap().map(|r| r.root), Some(root.clone()));
    assert!(index.removals().unwrap().is_empty());
}

/// The reviewer's rv3_stop_is_prompt_during_a_long_backoff: after errors in
/// a row the worker waits long; wakes (the watcher's, say) don't end that
/// wait, and stop still does at once.
#[test]
fn stop_is_prompt_and_wakes_are_ignored_during_a_long_backoff() {
    let home = Home::new();
    home.write("a.txt", b"tapir");
    let fake = Fake::new("fake", &["txt"]);
    let registry = Arc::new(fake.registry());
    let index = home.scanned(&registry);
    index
        .conn
        .execute_batch("CREATE TRIGGER fail BEFORE INSERT ON extracts BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .unwrap();
    let worker = Worker::start(home.db.clone(), registry, WorkerOptions { idle_poll: Duration::from_millis(300), ..options() })
        .unwrap();
    // Tries at 0, 300 and 900 ms: now waiting 1200 ms.
    std::thread::sleep(Duration::from_millis(1000));
    let waker = worker.waker();
    for _ in 0..20 {
        waker.wake();
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = std::time::Instant::now();
    let stats = worker.stop();
    assert!(started.elapsed() < Duration::from_millis(250), "stop took {:?}", started.elapsed());
    assert!(stats.errors.len() <= 3, "wakes ended the backoff early: {} tries", stats.errors.len());
}

/// An error, a success, then an error: each error is waited out idle_poll,
/// since a step that works starts the count again.
#[test]
fn a_step_that_works_resets_the_wait_after_errors() {
    let home = Home::new();
    home.write("a.txt", b"tapir");
    let reads: Arc<Mutex<Vec<(String, std::time::Instant)>>> = Arc::default();
    let log = reads.clone();
    let fake = Fake::new("fake", &["txt"]).body(move |path, sink, _| {
        log.lock().unwrap().push((path.file_name().unwrap().to_str().unwrap().to_string(), std::time::Instant::now()));
        sink.text("tapir")?;
        Ok(())
    });
    let registry = Arc::new(fake.registry());
    let mut index = home.scanned(&registry);
    let fail = |on: bool| {
        let sql = if on {
            "CREATE TRIGGER fail BEFORE INSERT ON extracts BEGIN SELECT RAISE(ABORT, 'disk full'); END"
        } else {
            "DROP TRIGGER fail"
        };
        Index::open(&home.db).unwrap().conn.execute_batch(sql).unwrap();
    };
    let reads_of = |name: &str| -> Vec<std::time::Instant> {
        reads.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, t)| *t).collect()
    };
    fail(true);
    let idle_poll = Duration::from_millis(300);
    let worker = Worker::start(home.db.clone(), registry, WorkerOptions { idle_poll, ..options() }).unwrap();
    // Two errors: the wait is now 600 ms, and would be 1200 after a third.
    eventually("two failed tries", || reads_of("a.txt").len() >= 2);
    fail(false);
    eventually("a.txt read", || count(&index, "SELECT COUNT(*) FROM extracts WHERE status = 'done'") == 1);
    // Then an error again, on a new file.
    fail(true);
    home.write("b.txt", b"tapir");
    home.refresh(&mut index, "b.txt");
    worker.wake();
    eventually("two failed tries of b.txt", || reads_of("b.txt").len() >= 2);
    let b = reads_of("b.txt");
    let gap = b[1] - b[0];
    drop(worker);
    assert!(gap < Duration::from_millis(800), "waited {gap:?} after the first error since a success, not {idle_poll:?}");
}

// --- a row's root comes from the folders when it is written (reviewer's round 4) ---

fn roots_of(index: &Index) -> Vec<(String, i64)> {
    let mut stmt = index.conn.prepare("SELECT root, COUNT(*) FROM files GROUP BY root ORDER BY root").unwrap();
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// Every row's root is a folder.
fn assert_roots_are_folders(index: &Index, what: &str) {
    let folders = index.folders().unwrap();
    let bad: Vec<_> = roots_of(index).into_iter().filter(|(r, _)| !folders.contains(r)).collect();
    assert!(bad.is_empty(), "{what}: rows whose root is not a folder: {bad:?} (folders {folders:?}, removals {:?})", index.removals().unwrap());
}

/// The reviewer's rv4_outer_folder_removed_before_a_taken_in_scan_ends.
/// A folder taken in while its scan is stuck (a slow disk, the writer
/// lock), the outer folder then removed and its removal finished, then the
/// stuck scan ends: its late rows used to go to the removed folder, kept
/// and searchable. Now they are removed, as no folder contains them.
#[test]
fn a_late_batch_of_a_folder_taken_in_goes_with_the_outer_folder_when_it_is_removed() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let (docs, root) = (home.at("Docs"), home.root.clone());
    home.write("top.txt", b"okapi top");
    assert_eq!(folders.add(&mut index, &root, now()).unwrap(), vec![docs.clone()]);
    // As the stuck scan's last upsert, waiting on the writer lock, would.
    let late = home.write("Docs/late.txt", b"okapi late");
    index.upsert(&[record_of(Path::new(&late), &docs).unwrap()], index.current_scan().unwrap()).unwrap();
    folders.scan(root.clone());
    folders.remove(&mut index, &root).unwrap();
    eventually("the outer folder's scan and removal to end", || {
        folders.scanning() == std::collections::HashSet::from([docs.clone()]) && index.removals().unwrap().is_empty()
    });
    std::thread::sleep(Duration::from_millis(200));
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    eprintln!("rv4 A: folders={:?} roots={:?} removals={:?} name hits for late: {}", index.folders().unwrap(), roots_of(&index), index.removals().unwrap(), name_hits(&index, "late"));
    assert!(index.folders().unwrap().is_empty());
    assert_roots_are_folders(&index, "after removing the only folder");
}

/// The reviewer's rv4_taken_in_twice_while_the_first_scan_is_stuck.
/// A folder taken in while its scan is stuck, its new folder itself taken
/// into one around it, then the stuck scan ends: its late rows go to the
/// folder that contains them now, not to the one it was first taken into.
#[test]
fn a_late_batch_of_a_folder_taken_in_twice_goes_to_the_outermost() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let (docs, root) = (home.at("Docs"), home.root.clone());
    let base = Path::new(&root).parent().unwrap().to_str().unwrap().to_string();
    assert_eq!(folders.add(&mut index, &root, now()).unwrap(), vec![docs.clone()]);
    folders.scan(root.clone());
    eventually("home's scan", || folders.scanning() == std::collections::HashSet::from([docs.clone()]));
    assert_eq!(folders.add(&mut index, &base, now()).unwrap(), vec![root.clone()]);
    folders.scan(base.clone());
    eventually("base's scan", || folders.scanning() == std::collections::HashSet::from([docs.clone()]));
    // As the stuck scan's last upsert, waiting on the writer lock, would.
    let late = home.write("Docs/late.txt", b"okapi late");
    index.upsert(&[record_of(Path::new(&late), &docs).unwrap()], index.current_scan().unwrap()).unwrap();
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    eprintln!("rv4 M: folders={:?} roots={:?}", index.folders().unwrap(), roots_of(&index));
    assert_eq!(index.folders().unwrap(), vec![base.clone()]);
    assert_roots_are_folders(&index, "taken in twice");
}

/// The reviewer's rv4_a_removed_folders_late_batch_after_an_outer_folder_scanned.
/// A removed folder's scan writes one last batch (its upsert waited on the
/// writer) after a folder added around it scanned those paths: the removal
/// of the old root used to delete them; now it gives them to the new folder.
#[test]
fn a_removed_folders_late_batch_inside_a_folder_added_around_it_is_kept() {
    let home = Home::new();
    let mut index = home.open();
    let (folders, go) = held_scan(&home, 2100);
    let (docs, root) = (home.at("Docs"), home.root.clone());
    folders.remove(&mut index, &docs).unwrap();
    assert!(folders.add(&mut index, &root, now()).unwrap().is_empty());
    folders.scan(root.clone());
    eventually("home's scan", || folders.scanning() == std::collections::HashSet::from([docs.clone()]));
    let late = home.at("Docs/d1/n1.txt");
    let record = record_of(Path::new(&late), &docs).unwrap();
    index.upsert(&[record], index.current_scan().unwrap()).unwrap();
    go.send(()).unwrap();
    eventually("everything to settle", || folders.idle());
    assert_roots_are_folders(&index, "after the late batch");
    assert!(index.get(&late).unwrap().is_some(), "{late} is under home, a folder, but is no longer indexed");
}

/// The reviewer's rv4_a_watcher_refresh_with_old_roots_after_a_take_in.
/// The watcher's batch uses the roots it read when the batch began: a
/// refresh with the old roots after a folder was taken in used to write rows
/// under the old root, which removing the outer folder left, searchable.
/// Now the root comes from the folders as the row is written.
#[test]
fn a_watcher_refresh_with_old_roots_writes_under_the_current_folder() {
    let home = Home::new();
    home.write("P/C/a.txt", b"okapi");
    let mut index = home.open();
    let (p, c) = (home.at("P"), home.at("P/C"));
    let folders = folders(&home, |_| {});
    folders.add(&mut index, &c, now()).unwrap();
    scan(&mut index, &c, &home.rules, |_| {}).unwrap();
    let old_roots = vec![c.clone()];
    folders.add(&mut index, &p, now()).unwrap();
    scan(&mut index, &p, &home.rules, |_| {}).unwrap();
    let x = home.write("P/C/rv4zebra.txt", b"okapi");
    refresh(&mut index, Path::new(&x), &old_roots, &home.rules).unwrap();
    folders.remove(&mut index, &p).unwrap();
    eventually("settle", || folders.idle());
    eprintln!("rv4 watcher: roots={:?} hits={}", roots_of(&index), name_hits(&index, "rv4zebra"));
    assert_roots_are_folders(&index, "after removing P");
}

/// The reviewer's rv4_restart_with_a_removed_folder_inside_an_added_one.
/// Restart with a folder marked for removal and a folder around it: its rows
/// are given to the outer folder and nothing is read again.
#[test]
fn a_removed_folder_inside_one_added_around_it_is_given_to_it_at_startup() {
    let home = Home::new();
    for i in 0..20 {
        home.write(&format!("P/C/n{i}.txt"), b"okapi words");
    }
    let registry = Registry::platform();
    let mut index = home.open();
    store::register_kinds(&mut index, &registry).unwrap();
    let (p, c) = (home.at("P"), home.at("P/C"));
    index.add_folder(&c, 1).unwrap();
    scan(&mut index, &c, &home.rules, |_| {}).unwrap();
    run(&mut index, &registry);
    assert!(index.forget_folder(&c).unwrap());
    index.add_folder_around(&p, 2).unwrap();
    drop(index);
    let mut index = home.open();
    let tidy = index.tidy_roots().unwrap();
    assert_eq!((tidy.reassigned, tidy.orphans), (1, 0), "given at once, not marked for removal");
    assert_eq!(index.count_root(&p).unwrap(), 21, "C and its 20 files");
    let folders = folders(&home, |_| {});
    folders.resume(&mut index).unwrap();
    eventually("settle", || folders.idle());
    assert_roots_are_folders(&index, "after restart");
    assert_eq!(index.count_root(&p).unwrap(), 22);
    assert!(index.removals().unwrap().is_empty());
    assert_eq!(queued(&index), 0, "nothing read again");
}


/// The reviewer's rv4_removing_a_taken_in_folder_keeps_the_outer_folder_watched
/// (inotify): removing a path taken into a folder around it unwatched it,
/// and notify then dropped the outer folder's watches on that subtree. As
/// app.rs's remove_folder now does, it is unwatched only if it was a folder.
#[cfg(target_os = "linux")]
#[test]
fn removing_a_folder_taken_in_keeps_the_outer_folder_watched() {
    let home = Home::new();
    home.write("P/C/a.txt", b"okapi");
    home.write("P/top.txt", b"okapi");
    let mut index = home.open();
    let (p, c) = (home.at("P"), home.at("P/C"));
    let folders = folders(&home, |_| {});
    folders.add(&mut index, &c, now()).unwrap();
    scan(&mut index, &c, &home.rules, |_| {}).unwrap();
    let rules = Arc::new(Mutex::new(Rules::new(&Rules::default_patterns(), &home.root).unwrap()));
    let mut watch = Watch::start(home.db.clone(), vec![c.clone()], rules, |_| {}).unwrap();
    // app.rs add_folder(P)
    let inside = folders.add(&mut index, &p, now()).unwrap();
    for child in &inside {
        watch.remove_root(child).unwrap();
    }
    watch.add_root(&p).unwrap();
    scan(&mut index, &p, &home.rules, |_| {}).unwrap();
    let before = home.write("P/C/before.txt", b"okapi");
    eventually("a file under C, watched through P", || index.get(&before).unwrap().is_some());
    // app.rs remove_folder(P/C)
    let forgot = folders.remove(&mut index, &c).unwrap();
    assert!(!forgot, "P/C is not a folder now");
    if forgot {
        watch.remove_root(&c).unwrap();
    }
    let after = home.write("P/C/after.txt", b"okapi");
    let control = home.write("P/control.txt", b"okapi");
    eventually("the control file", || index.get(&control).unwrap().is_some());
    eventually("P/C/after.txt, still watched through P", || index.get(&after).unwrap().is_some());
    assert_eq!(index.folders().unwrap(), vec![p.clone()]);
}

/// A folder that can't be watched doesn't stop the others being watched.
#[test]
fn the_watcher_watches_what_it_can() {
    let home = Home::new();
    let docs = home.at("Docs");
    home.write("Docs/a.txt", b"okapi");
    let mut index = home.open();
    index.add_folder(&docs, 0).unwrap();
    scan(&mut index, &docs, &home.rules, |_| {}).unwrap();
    let rules = Arc::new(Mutex::new(Rules::new(&Rules::default_patterns(), &home.root).unwrap()));
    let gone = home.at("not-there");
    let _watch = Watch::start(home.db.clone(), vec![gone, docs.clone()], rules, |_| {}).unwrap();
    let new = home.write("Docs/new.txt", b"okapi");
    eventually("a change in the folder that is watched", || index.get(&new).unwrap().is_some());
}

/// Rows given to a folder are marked seen by its newest scan: a late row
/// given to a folder whose scan has walked past it, and is about to remove
/// what it didn't see, stays.
#[test]
fn rows_given_to_a_folder_during_its_scan_are_not_removed_as_unseen() {
    let home = Home::new();
    let mut index = home.open();
    // 30 files: one batch, so the scan is held after all its upserts and
    // before it removes what it didn't see.
    let (folders, go) = held_scan(&home, 30);
    let docs = home.at("Docs");
    let stray = format!("{docs}/d0");
    let late = home.write("Docs/d0/late.txt", b"okapi late");
    // Written under a root that is not a folder, before this scan began.
    index.upsert(&[record_of(Path::new(&late), &stray).unwrap()], 0).unwrap();
    assert_eq!(index.remove_root_in_batches(&stray, 10, |_| {}).unwrap(), 0, "given, not removed");
    go.send(()).unwrap();
    eventually("the scan to end", || folders.idle());
    assert_eq!(index.get(&late).unwrap().map(|r| r.root), Some(docs.clone()));
}
