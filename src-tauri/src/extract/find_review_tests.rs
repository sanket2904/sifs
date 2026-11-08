//! The step 3 review, round 2 (plans/phase3-reading-inside-files.md, "Step
//! 3 review, round 2"): the reviewers' tests, adapted, and one per fix.
//! Each failed on 5054213.

use super::*;
use crate::index::migrate;
use crate::index::search::CANDIDATES;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Files with no contents, written in one transaction.
fn named(index: &mut Index, paths: impl IntoIterator<Item = String>) {
    let records: Vec<FileRecord> = paths
        .into_iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap().to_string();
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
            FileRecord { path, root: "/home".into(), name, ext, is_dir: false, size: 1, modified: NOW, inode: 1, modified_ns: Some(0) }
        })
        .collect();
    index.upsert(&records, 1).unwrap();
}

type OnSql<'a> = Box<dyn FnMut(&str) + 'a>;

/// Calls on_sql with the text of each statement the connection starts,
/// for the life of the guard.
struct Trace<'a> {
    conn: &'a rusqlite::Connection,
    _hook: Box<OnSql<'a>>,
}

impl<'a> Trace<'a> {
    fn new(conn: &'a rusqlite::Connection, on_sql: impl FnMut(&str) + 'a) -> Trace<'a> {
        unsafe extern "C" fn trace(_: u32, ctx: *mut std::ffi::c_void, _: *mut std::ffi::c_void, sql: *mut std::ffi::c_void) -> i32 {
            let hook = &mut *(ctx as *mut OnSql);
            hook(&std::ffi::CStr::from_ptr(sql as *const std::ffi::c_char).to_string_lossy());
            0
        }
        let mut hook: Box<OnSql<'a>> = Box::new(Box::new(on_sql));
        let ctx = (&mut *hook) as *mut OnSql<'a> as *mut std::ffi::c_void;
        let rc = unsafe { rusqlite::ffi::sqlite3_trace_v2(conn.handle(), rusqlite::ffi::SQLITE_TRACE_STMT as u32, Some(trace), ctx) };
        assert_eq!(rc, 0);
        Trace { conn, _hook: hook }
    }
}

impl Drop for Trace<'_> {
    fn drop(&mut self) {
        unsafe { rusqlite::ffi::sqlite3_trace_v2(self.conn.handle(), 0, None, std::ptr::null_mut()) };
    }
}

const SNIPPET_READ: &str = "SELECT label, text FROM contents";

// --- 1. one snapshot ---

#[test]
fn a_worker_commit_during_a_search_attaches_no_other_files_label_or_text() {
    let (dir, mut writer) = db();
    file(&mut writer, "/home/a.txt", NOW, &[("A LABEL", "tapir")]);
    file(&mut writer, "/home/b.txt", NOW, &[]);
    let reader = Index::open(&dir.path().join("find.db")).unwrap();
    let mut committed = 0;
    let got = {
        let _trace = Trace::new(&reader.conn, |sql| {
            if committed == 0 && sql.starts_with(SNIPPET_READ) {
                committed += 1;
                // Part 1 is deleted and its id reused by the other file.
                writer
                    .conn
                    .execute_batch(
                        "BEGIN IMMEDIATE; DELETE FROM contents WHERE file_id = 1;
                         INSERT INTO contents (file_id, part, label, text) VALUES (2, 0, 'B PRIVATE LABEL', 'B private tapir');
                         COMMIT;",
                    )
                    .unwrap();
            }
        });
        all(&reader, "tapir", 60)
    };
    assert_eq!(committed, 1, "the commit ran during the search");
    assert_eq!(paths(&got), ["/home/a.txt"]);
    let found = got.hits[0].found.as_ref().unwrap();
    assert_eq!((found.label.as_str(), rebuilt(found).as_str()), ("A LABEL", "tapir"));
    assert!(!got.contents_timed_out);
}

#[test]
fn a_part_deleted_during_a_search_is_not_a_timeout() {
    let (dir, mut writer) = db();
    file(&mut writer, "/home/a.txt", NOW, &[("page 1", "tapir")]);
    let reader = Index::open(&dir.path().join("find.db")).unwrap();
    let mut deleted = false;
    let got = {
        let _trace = Trace::new(&reader.conn, |sql| {
            if !deleted && sql.starts_with(SNIPPET_READ) {
                deleted = true;
                writer.conn.execute("DELETE FROM contents", []).unwrap();
            }
        });
        all(&reader, "tapir", 60)
    };
    assert!(deleted);
    assert!(!got.contents_timed_out, "a deleted part is not a timeout");
    assert_eq!(rebuilt(got.hits[0].found.as_ref().unwrap()), "tapir", "read from the search's own snapshot");
}

// --- 2. edits within the same second ---

pub(super) fn set_time(path: &std::path::Path, secs: i64, nanos: u32) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(UNIX_EPOCH + Duration::new(secs as u64, nanos)).unwrap();
}

pub(super) fn queue_len(index: &Index) -> i64 {
    index.conn.query_row("SELECT count(*) FROM extract_queue", [], |r| r.get(0)).unwrap()
}

#[test]
fn an_edit_within_the_same_second_keeping_the_size_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let (path, root) = (dir.path().join("notes.txt"), dir.path().to_str().unwrap());
    let mut index = Index::open(&dir.path().join("same-second.db")).unwrap();
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    std::fs::write(&path, "tapir").unwrap();
    set_time(&path, NOW, 100_000_000);
    index.upsert(&[record_of(&path, root).unwrap()], 1).unwrap();
    assert!(worker::run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap().errors.is_empty());
    assert_eq!(all(&index, "tapir", 60).hits.len(), 1);
    let old = record_of(&path, root).unwrap();
    std::fs::write(&path, "zebra").unwrap();
    set_time(&path, NOW, 900_000_000);
    let new = record_of(&path, root).unwrap();
    assert_eq!((old.modified, old.size), (new.modified, new.size), "the same second and size");
    index.upsert(&[new], 2).unwrap();
    assert_eq!(queue_len(&index), 1, "queued again");
    assert!(all(&index, "tapir", 60).hits.is_empty(), "its old words are not found meanwhile");
    assert!(worker::run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap().errors.is_empty());
    assert!(all(&index, "tapir", 60).hits.is_empty());
    assert_eq!(all(&index, "zebra", 60).hits.len(), 1);
}

/// An index as version 2 left it, with one file read: notes.txt, "tapir",
/// changed at second NOW.
pub(super) fn version_2_index(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let (db, path) = (dir.join("v2.db"), dir.join("notes.txt"));
    std::fs::write(&path, "tapir").unwrap();
    set_time(&path, NOW, 100_000_000);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(crate::index::db::SCHEMA).unwrap();
    for sql in &migrate::MIGRATIONS[..2] {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch(&format!(
        "PRAGMA user_version = 2;
         INSERT INTO files (id, path, root, name, ext, is_dir, size, modified, inode, seen)
             VALUES (1, '{path}', '{root}', 'notes.txt', 'txt', 0, 5, {NOW}, 1, 1);
         INSERT INTO contents (file_id, part, label, text) VALUES (1, 0, '', 'tapir');
         INSERT INTO extracts (file_id, reader, version, modified, size, status, millis, read_at)
             VALUES (1, 'text', 1, {NOW}, 5, 'done', 0, {NOW});",
        path = path.display(),
        root = dir.display()
    ))
    .unwrap();
    (db, path)
}

#[test]
fn migration_3_stopped_at_any_point_leaves_version_2_and_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let steps = Arc::new(AtomicUsize::new(0));
    let try_at = |stop_at: usize| -> bool {
        let case = tempfile::tempdir_in(dir.path()).unwrap();
        let (db, _) = version_2_index(case.path());
        let mut conn = rusqlite::Connection::open(&db).unwrap();
        let counter = steps.clone();
        counter.store(0, Ordering::SeqCst);
        conn.progress_handler(1, Some(move || counter.fetch_add(1, Ordering::SeqCst) + 1 == stop_at));
        let migrated = migrate::migrate(&mut conn).is_ok();
        conn.progress_handler(0, None::<fn() -> bool>);
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let columns: i64 = conn
            .query_row(
                "SELECT (SELECT count(*) FROM pragma_table_info('files') WHERE name = 'modified_ns')
                      + (SELECT count(*) FROM pragma_table_info('extracts') WHERE name = 'modified_ns')
                      + (SELECT count(*) FROM pragma_table_info('extract_queue') WHERE name = 'modified_ns')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if migrated {
            assert_eq!((version, columns), (3, 3));
        } else {
            // Stopped after the commit, it is wholly migrated.
            assert!([(2, 0), (3, 3)].contains(&(version, columns)), "stopped at step {stop_at}: never half migrated, {version} {columns}");
            drop(conn);
            let index = Index::open(&db).unwrap();
            assert_eq!(index.conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
            assert_eq!(queue_len(&index), 1, "opened again, what was read is queued once more");
            assert_eq!(all(&index, "tapir", 60).hits.len(), 1, "opened again, it is migrated and searchable");
        }
        migrated
    };
    assert!(try_at(usize::MAX));
    let total = steps.load(Ordering::SeqCst);
    assert!(total > 20, "{total} steps");
    for stop_at in (1..total).step_by((total / 25).max(1)) {
        assert!(!try_at(stop_at), "stopped at step {stop_at} of {total}");
    }
}

// --- 3. SQLite's own tokenizer ---

#[test]
fn text_finds_itself_whatever_its_script() {
    let (_dir, mut index) = db();
    let texts = ["alpha🦄beta", "alpha🥰beta", "Ꭰbc", "ᎠᎡᎢ", "ᲐᲑᲒ", "ꟀꟂꟄ", "\u{1c90}ab", "\u{1e9b}ab", "alpha\u{1ab0}beta", "alpha\u{200c}beta", "alpha\u{061c}beta", "a\u{1f170}b", "\u{301}tapir"];
    for (n, text) in texts.iter().enumerate() {
        let path = file(&mut index, &format!("/home/n{n}.txt"), NOW, &[("", text)]);
        let got = all(&index, text, 60);
        assert!(paths(&got).contains(&path.as_str()), "{text:?} finds itself: {:?}", paths(&got));
        let found = got.hits.iter().find(|h| h.file.path == path).unwrap().found.as_ref().unwrap();
        assert!(!marked(found).is_empty(), "{text:?} is marked: {:?}", found.snippet);
    }
}

#[test]
fn a_word_matched_by_its_fold_is_marked() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/theta.txt", NOW, &[("page 1", "ϑeta")]);
    file(&mut index, "/home/beta.txt", NOW, &[("page 1", "ϐeta")]);
    for (q, want) in [("θeta", "ϑeta"), ("βeta", "ϐeta")] {
        let got = all(&index, q, 60);
        let found: Vec<&Found> = got.hits.iter().filter_map(|h| h.found.as_ref()).collect();
        assert_eq!(found.len(), 1, "{q}");
        assert_eq!(marked(found[0]), [want], "{q}");
    }
}

// --- 4. bounded input ---

#[test]
fn a_huge_query_is_cut_and_searched_within_the_time_limit() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "word0 word1")]);
    let typed: String = (0..3_000_000).map(|n| format!("word{n} ")).collect();
    let started = Instant::now();
    let got = search_all(&index, &typed, 60, NOW, &Until::after(CONTENT_TIME)).unwrap();
    let took = started.elapsed();
    assert!(took < CONTENT_TIME, "{} bytes typed: {took:?}", typed.len());
    assert!(got.words_left_out);
    let mut tokenizer = Tokenizer::contents(&index.conn).unwrap();
    let cut = format!("{}tapir", "é ".repeat(2000));
    let query = Query::parse(&cut, &mut tokenizer).unwrap();
    assert!(query.left_out, "past {} bytes", crate::index::query::MAX_BYTES);
    assert!(query.words.iter().all(|w| w.text != "tapir"), "words past the cut are not searched");
    let query = Query::parse(&"é ".repeat(1000), &mut tokenizer).unwrap();
    assert!(!query.left_out);
}

// --- 5. candidates in ranking order ---

#[test]
fn a_document_named_for_the_query_beats_5000_code_files_named_so() {
    let (_dir, mut index) = db();
    named(&mut index, (0..CANDIDATES).map(|i| format!("/home/code/{i}/tapir.rs")));
    let wanted = file(&mut index, "/home/Documents/mytapirnotes.txt", NOW, &[]);
    let got = all(&index, "tapir", 60);
    assert_eq!(got.hits[0].file.path, wanted);
    assert!(got.names_capped, "{} names match", CANDIDATES + 1);
    let (_dir, mut index) = db();
    named(&mut index, (0..CANDIDATES).map(|i| format!("/home/code/{i}/tapir.rs")));
    assert!(!all(&index, "tapir", 60).names_capped, "exactly {CANDIDATES} names match");
}

#[test]
fn the_typo_tolerant_names_do_not_depend_on_the_limit() {
    let (_dir, mut index) = db();
    named(&mut index, (0..60).map(|i| format!("/home/code/{i}/tapirs.rs")));
    let wanted = file(&mut index, "/home/Documents/tapirx.txt", NOW, &[]);
    for limit in [1, 60, 61, 500] {
        let got = all(&index, "tapirs", limit);
        assert_eq!(got.hits[0].file.path, wanted, "limit {limit}");
        assert!(got.hits[0].fuzzy);
    }
}

// --- 6. the whole call within its time ---

#[test]
fn a_search_stops_as_soon_as_a_newer_one_starts() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/tapir.txt", NOW, &[("", "tapir")]);
    let newest = Arc::new(AtomicU64::new(0));
    let older = Until::after(Duration::from_secs(60)).or_newer(&newest);
    assert!(!search_all(&index, "tapir", 60, NOW, &older).unwrap().names_timed_out);
    let _newer = Until::after(Duration::from_secs(60)).or_newer(&newest);
    let got = search_all(&index, "tapir", 60, NOW, &older).unwrap();
    assert!(got.names_timed_out && got.contents_timed_out, "an older search stops");
    assert!(got.hits.is_empty());
    plain_sql_runs(&index);
}

#[test]
fn a_long_name_scan_stops_at_the_time_limit() {
    let (_dir, index) = db();
    let name = format!("aa ab ac ad ae af ag ah ai aj ak al am an ao {}.txt", "_".repeat(185));
    index
        .conn
        .execute(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x < 100000)
             INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode, seen)
             SELECT '/home/' || x || '/' || ?1, '/home', ?1, 'txt', 0, 1, ?2, x, 1 FROM n",
            params![name, NOW],
        )
        .unwrap();
    let started = Instant::now();
    // Two letters: no trigram index, every name is checked; and too short
    // for the typo-tolerant pass.
    let got = search_all(&index, "aa", 60, NOW, &Until::after(Duration::from_millis(50))).unwrap();
    let took = started.elapsed();
    assert!(took < Duration::from_millis(350), "{took:?}");
    assert!(got.names_timed_out, "100,000 names are not compared in 50 ms");
    plain_sql_runs(&index);
}

// --- 7. snippets ---

#[test]
fn a_snippet_is_at_most_200_characters_with_its_ellipses() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/a.txt", NOW, &[("", &format!("{}needle {}", "lead ".repeat(50), "💥".repeat(400)))]);
    file(&mut index, "/home/b.txt", NOW, &[("", &format!("{}needle {}", "lead ".repeat(100), "💥".repeat(1000)))]);
    let got = all(&index, "needle", 60);
    assert_eq!(got.hits.len(), 2);
    for hit in &got.hits {
        let shown = rebuilt(hit.found.as_ref().unwrap());
        assert!(shown.starts_with('…') && shown.ends_with('…'), "{shown}");
        assert!(shown.chars().count() <= SNIPPET_CHARS, "{}: {} characters", hit.file.name, shown.chars().count());
    }
}

#[test]
fn snippets_of_mixed_text_never_panic_and_stay_within_200_characters() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "needle")]);
    let alphabet = ['é', 'Ꭰ', '🥰', '💥', 'ः', '\u{301}', '\u{fe0f}', '\u{20e3}', '\0', '\u{e000}', '\u{10fffd}', '\u{2028}', '"', ' ', 'x', 'a', '\n', '\t'];
    let mut seed = 1u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        alphabet[seed as usize % alphabet.len()]
    };
    for case in 0..600 {
        let mut text: String = (0..case % 401).map(|_| next()).collect();
        text.push_str(" needle. ");
        text.extend((0..case % 309).map(|_| next()));
        index.conn.execute("UPDATE contents SET text = ?1", [&text]).unwrap();
        let got = all(&index, "needle", 1);
        let found = got.hits[0].found.as_ref().unwrap();
        assert!(rebuilt(found).chars().count() <= SNIPPET_CHARS, "case {case}");
        assert_eq!(marked(found), ["needle"], "case {case}");
    }
}
