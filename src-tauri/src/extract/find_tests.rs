//! Search over names and contents. Most of these come from the step 3
//! reviews (plans/phase3-reading-inside-files.md, "Step 3 review, round 1"),
//! each failing on the code they reviewed.

use super::*;
use crate::extract::{store, worker, Registry, WorkerOptions};
use crate::index::crawl::record_of;
use crate::index::query::fold;
use rusqlite::params;

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn db() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(&dir.path().join("find.db")).unwrap();
    (dir, index)
}

/// A file under /home, with its parts read as the worker would have read
/// them (its extract matching its time and size).
fn file(index: &mut Index, path: &str, modified: i64, parts: &[(&str, &str)]) -> String {
    let name = path.rsplit('/').next().unwrap();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let size: i64 = parts.iter().map(|p| p.1.len() as i64).sum();
    index
        .upsert(
            &[FileRecord {
                path: path.into(),
                root: "/home".into(),
                name: name.into(),
                ext: ext.into(),
                is_dir: false,
                size,
                modified,
                inode: 1,
            }],
            1,
        )
        .unwrap();
    let id = file_id(index, path);
    for (n, (label, text)) in parts.iter().enumerate() {
        part(index, id, n as i64, label, text);
    }
    if !parts.is_empty() {
        index
            .conn
            .execute(
                "INSERT INTO extracts (file_id, reader, version, modified, size, status, millis, read_at)
                 VALUES (?1, 'text', 1, ?2, ?3, 'done', 0, ?2)",
                params![id, modified, size],
            )
            .unwrap();
    }
    path.into()
}

fn file_id(index: &Index, path: &str) -> i64 {
    index.conn.query_row("SELECT id FROM files WHERE path = ?1", [path], |r| r.get(0)).unwrap()
}

fn part(index: &Index, file_id: i64, n: i64, label: &str, text: &str) {
    index
        .conn
        .execute("INSERT INTO contents (file_id, part, label, text) VALUES (?1, ?2, ?3, ?4)", params![file_id, n, label, text])
        .unwrap();
}

fn all(index: &Index, query: &str, limit: usize) -> Results {
    search_all(index, query, limit, NOW, Duration::from_secs(10)).unwrap()
}

fn paths(results: &Results) -> Vec<&str> {
    results.hits.iter().map(|r| r.file.path.as_str()).collect()
}

fn names(results: &Results) -> Vec<&str> {
    results.hits.iter().map(|r| r.file.name.as_str()).collect()
}

fn marked(found: &Found) -> Vec<&str> {
    found.snippet.iter().filter(|s| s.hit).map(|s| s.text.as_str()).collect()
}

fn rebuilt(found: &Found) -> String {
    found.snippet.iter().map(|s| s.text.as_str()).collect()
}

/// SQL that runs long enough to be stopped by a progress handler left behind.
fn plain_sql_runs(index: &Index) {
    let total: i64 = index
        .conn
        .query_row(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT sum(x) FROM n",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total, 50_005_000);
}

// --- the query, as typed ---

#[test]
fn a_control_character_typed_separates_words_and_never_errors() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "tapir zebra okapi")]);
    for q in ["\0", "tapir\0", "tapir\0zebra", "tapir \0 zebra", "tapir\0okapi", "tapir \0", "\u{1}\u{2}\u{3}\u{7f}"] {
        let got = search_all(&index, q, 60, NOW, CONTENT_TIME);
        assert!(got.is_ok(), "{q:?}: {got:?}");
    }
    assert_eq!(paths(&all(&index, "tapir\0okapi", 60)), ["/home/notes.txt"]);
}

#[test]
fn the_prefix_rule_counts_a_words_letters_not_its_punctuation() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "invoice alphabet")]);
    for q in ["in,", "(in)", "\"a\"", "in:"] {
        assert!(all(&index, q, 60).hits.is_empty(), "{q:?}: one or two letters match whole words only");
    }
    assert_eq!(paths(&all(&index, "inv,", 60)), ["/home/notes.txt"]);
}

#[test]
fn punctuation_separates_words_and_asks_for_no_phrase() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/a.txt", NOW, &[("", "beta then alpha")]);
    file(&mut index, "/home/b.txt", NOW, &[("", "rent monthly invoice")]);
    assert_eq!(names(&all(&index, "alpha-beta", 60)), ["a.txt"]);
    assert_eq!(names(&all(&index, "rent,invoice", 60)), ["b.txt"]);
    assert_eq!(names(&all(&index, "tapir *", 60)), Vec::<&str>::new());
    file(&mut index, "/home/c.txt", NOW, &[("", "tapir")]);
    assert_eq!(names(&all(&index, "tapir *", 60)), ["c.txt"], "a term of punctuation alone hides no word");
}

#[test]
fn many_words_never_error_and_those_past_the_limit_are_reported() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "a tapir")]);
    let same = "a ".repeat(1100);
    let got = search_all(&index, &same, 60, NOW, CONTENT_TIME).unwrap();
    assert_eq!(paths(&got), ["/home/notes.txt"]);
    assert_eq!(got.ignored_words, 0, "the same word repeated is one word");
    let distinct: Vec<String> = (0..1100).map(|i| format!("w{i}")).collect();
    let got = search_all(&index, &distinct.join(" "), 60, NOW, CONTENT_TIME).unwrap();
    assert_eq!(got.ignored_words, 1100 - crate::index::query::MAX_WORDS);
    // Long words of varied letters: the typo-tolerant name search ORs
    // their trigrams.
    let mut seed: u32 = 1;
    let long: Vec<String> = (0..1100)
        .map(|_| {
            (0..70)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    char::from(b'a' + (seed >> 16) as u8 % 26)
                })
                .collect()
        })
        .collect();
    assert!(search_all(&index, &long.join(" "), 60, NOW, CONTENT_TIME).is_ok());
}

#[test]
fn a_word_longer_than_kept_matches_by_its_start() {
    let (_dir, mut index) = db();
    let word = "x".repeat(100);
    file(&mut index, "/home/notes.txt", NOW, &[("", &format!("{word} tapir"))]);
    assert_eq!(paths(&all(&index, &format!("{word} tapir"), 60)), ["/home/notes.txt"]);
}

#[test]
fn words_are_split_and_folded_as_the_index_splits_and_folds_them() {
    let (_dir, index) = db();
    let text = "Résumé re\u{301}sume\u{301} Việt ẞtraße İstanbul λόγος ſ µm 𐐀 ＡＢＣ ﬁle Ⓐbc I❤️you 1️⃣ \u{e000}x \
                किराया शर्मा 東京 alpha-beta_gamma o'brien naïve CAFÉ";
    index.conn.execute_batch(concat!(
        "CREATE VIRTUAL TABLE temp.t USING fts5(text, tokenize=\"unicode61 remove_diacritics 2 categories 'L* N* Co M*' separators '",
        "\u{fe0e}\u{fe0f}\u{20e3}",
        "'\"); CREATE VIRTUAL TABLE temp.v USING fts5vocab(temp, t, 'instance');"
    ))
    .unwrap();
    index.conn.execute("INSERT INTO temp.t (text) VALUES (?1)", [text]).unwrap();
    let mut stmt = index.conn.prepare("SELECT term FROM temp.v ORDER BY offset").unwrap();
    let sqlite: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let ours: Vec<String> = WORD.find_iter(text).map(|m| fold(m.as_str())).filter(|w| !w.is_empty()).collect();
    assert_eq!(ours, sqlite);
}

// --- what is found ---

#[test]
fn an_edited_file_is_not_found_by_its_old_words_before_it_is_read_again() {
    let (_dir, mut index) = db();
    store::register_kinds(&mut index, &Registry::platform()).unwrap();
    let path = file(&mut index, "/home/notes.txt", NOW, &[("", "okafor old tenant")]);
    let id = file_id(&index, &path);
    index.conn.execute("DELETE FROM extract_queue WHERE file_id = ?1", [id]).unwrap();
    assert_eq!(names(&all(&index, "okafor", 60)), ["notes.txt"]);
    index.conn.execute("UPDATE files SET modified = modified + 1, size = 30 WHERE id = ?1", [id]).unwrap();
    let queued: i64 = index.conn.query_row("SELECT count(*) FROM extract_queue WHERE file_id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(queued, 1);
    assert!(all(&index, "okafor", 60).hits.is_empty());
}

#[test]
fn one_file_with_many_matching_pages_hides_no_other() {
    let (_dir, mut index) = db();
    let pages: Vec<(&str, &str)> = (0..2000).map(|_| ("page", "tapir")).collect();
    file(&mut index, "/home/Documents/atlas.docx", NOW, &pages);
    file(&mut index, "/home/Documents/notes.txt", NOW, &[("", "a tapir in a considerably longer note about the zoo")]);
    let got = all(&index, "tapir", 60);
    assert_eq!(names(&got).len(), 2, "{:?}", names(&got));
    assert!(names(&got).contains(&"notes.txt"));
    assert!(!got.contents_timed_out && !got.contents_capped);
}

#[test]
fn a_fast_search_past_the_cap_reports_the_cap_not_the_time() {
    let (_dir, mut index) = db();
    let atlas = file(&mut index, "/home/atlas.docx", NOW, &[("page 0", "tapir")]);
    index
        .conn
        .execute(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x < ?2)
             INSERT INTO contents (file_id, part, label, text) SELECT ?1, x, 'page ' || x, 'tapir' FROM n",
            params![file_id(&index, &atlas), CONTENT_PARTS as i64],
        )
        .unwrap();
    file(&mut index, "/home/later.txt", NOW, &[("", "tapir")]);
    let got = all(&index, "tapir", 60);
    assert_eq!(names(&got), ["atlas.docx"]);
    assert!(got.contents_capped, "a file past the cap may be missing");
    assert!(!got.contents_timed_out, "it finished well within its time");
}

// --- ranking: every candidate, then the limit ---

#[test]
fn the_limit_applies_after_every_name_match_is_ranked() {
    let (_dir, mut index) = db();
    for i in 0..60 {
        file(&mut index, &format!("/home/code/{i}/tapir.rs"), NOW, &[]);
    }
    let wanted = file(&mut index, "/home/Documents/mytapirnotes.txt", NOW, &[]);
    file(&mut index, "/home/Documents/observations.txt", NOW, &[("", "tapir")]);
    assert_eq!(all(&index, "tapir", 60).hits[0].file.path, wanted);
    let (_dir, mut index) = db();
    file(&mut index, "/home/tapir.py", NOW, &[("", "nothing")]);
    file(&mut index, "/home/xxtapirxx.txt", NOW, &[("", "nothing")]);
    assert_eq!(names(&all(&index, "tapir", 1)), ["xxtapirxx.txt"]);
    for n in 1..60 {
        file(&mut index, &format!("/home/code/{n}/tapir.py"), NOW, &[("", "nothing")]);
    }
    assert_eq!(names(&all(&index, "tapir", 60)).first(), Some(&"xxtapirxx.txt"));
}

#[test]
fn the_limit_applies_after_every_content_match_is_ranked() {
    let (_dir, mut index) = db();
    for i in 0..200 {
        file(&mut index, &format!("/home/src/item{i}.rs"), NOW, &[("", "tapir")]);
    }
    file(&mut index, "/home/notes.txt", NOW, &[("", &format!("tapir {}", "filler ".repeat(100)))]);
    assert_eq!(names(&all(&index, "tapir", 1)), ["notes.txt"]);
}

#[test]
fn a_typo_tolerant_name_does_not_demote_a_real_content_match() {
    let (_dir, mut index) = db();
    let wanted = file(&mut index, "/home/Documents/tapirx.txt", NOW, &[("", "tapirs")]);
    file(&mut index, "/home/Documents/zoo.txt", NOW - 400 * DAY, &[("", "tapirs")]);
    let got = all(&index, "tapirs", 60);
    assert_eq!(got.hits[0].file.path, wanted, "ranked by its contents and recency");
    assert!(!got.hits[0].fuzzy);
    assert!(got.hits[0].found.is_some());
}

#[test]
fn a_zero_limit_returns_nothing() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/tapir.txt", NOW, &[("", "tapir")]);
    assert!(all(&index, "tapir", 0).hits.is_empty());
}

// --- snippets ---

#[test]
fn file_text_cannot_pose_as_a_matched_word() {
    let (dir, mut index) = db();
    let source = dir.path().join("source.json");
    std::fs::write(&source, serde_json::to_string("tapir \u{2}innocent\u{3} ending").unwrap()).unwrap();
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    index.upsert(&[record_of(&source, dir.path().to_str().unwrap()).unwrap()], 1).unwrap();
    let stats = worker::run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    let stored: String = index.conn.query_row("SELECT text FROM contents", [], |r| r.get(0)).unwrap();
    assert!(stored.contains('\u{2}') && stored.contains('\u{3}'), "the JSON reader keeps them: {stored:?}");
    let got = all(&index, "tapir", 60);
    let found = got.hits[0].found.as_ref().unwrap();
    assert_eq!(marked(found), ["tapir"]);
    assert_eq!(rebuilt(found), stored.replace('\n', " "));
}

#[test]
fn a_snippet_is_short_in_bytes_not_only_in_words() {
    let (_dir, mut index) = db();
    let emoji = format!("needle {} tail", "\u{1f4a5}".repeat(999_980));
    assert!(emoji.chars().count() < 1_000_000 && emoji.len() < 4 * 1024 * 1024);
    file(&mut index, "/home/emoji.txt", NOW, &[("", &emoji)]);
    file(&mut index, "/home/xs.txt", NOW - DAY, &[("", &format!("needle {}", "x".repeat(999_993)))]);
    let got = all(&index, "needle", 60);
    assert_eq!(got.hits.len(), 2);
    for hit in &got.hits {
        let found = hit.found.as_ref().unwrap();
        assert_eq!(marked(found), ["needle"]);
        let bytes: usize = found.snippet.iter().map(|s| s.text.len()).sum();
        assert!(bytes <= 4096, "{}: {bytes} bytes", hit.file.name);
    }
}

#[test]
fn a_snippet_is_around_the_first_word_with_every_word_marked() {
    let (_dir, mut index) = db();
    let text = format!("{}the Résumé of a tapir, résumé again{}", "lead in words ".repeat(50), " and more".repeat(50));
    file(&mut index, "/home/cv.txt", NOW, &[("page 4", &text)]);
    let found = all(&index, "resume", 60).hits[0].found.clone().unwrap();
    assert_eq!(found.label, "page 4");
    assert_eq!(marked(&found), ["Résumé", "résumé"]);
    let shown = rebuilt(&found);
    assert!(shown.starts_with('…') && shown.ends_with('…'), "{shown}");
    assert!(shown.chars().count() <= SNIPPET_CHARS + 2, "{shown}");
    assert!(shown.starts_with("…words ") || shown.starts_with("…in ") || shown.starts_with("…lead "), "starts at a whole word: {shown}");
}

#[test]
fn only_the_results_kept_are_read_for_snippets() {
    let (_dir, mut index) = db();
    // Making all 300 snippets takes longer than CONTENT_TIME.
    let text = format!("{}tapir", "filler ".repeat(5_000));
    for i in 0..300 {
        file(&mut index, &format!("/home/n{i:03}.txt"), NOW, &[("", &text)]);
    }
    let got = search_all(&index, "tapir", 5, NOW, CONTENT_TIME).unwrap();
    assert!(!got.contents_timed_out);
    assert_eq!(got.hits.len(), 5);
    assert!(got.hits.iter().all(|h| h.found.as_ref().is_some_and(|f| marked(f) == ["tapir"])));
}

#[test]
fn matching_parts_are_found_without_reading_their_text() {
    let (dir, mut index) = db();
    let text = format!("tapir {}", "word ".repeat(1_000));
    for i in 0..2_000 {
        file(&mut index, &format!("/home/n{i:04}.txt"), NOW, &[("", &text)]);
    }
    drop(index);
    let index = Index::open(&dir.path().join("find.db")).unwrap();
    let pages_read = || {
        let (mut now, mut high) = (0, 0);
        let status = unsafe {
            rusqlite::ffi::sqlite3_db_status(index.conn.handle(), rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS, &mut now, &mut high, 0)
        };
        assert_eq!(status, 0);
        now
    };
    let before = pages_read();
    let got = search_all(&index, "tapir", 1, NOW, Duration::from_secs(10)).unwrap();
    assert_eq!(got.hits.len(), 1);
    let read = pages_read() - before;
    // Reading the 2,000 parts' own rows would be over 500 pages.
    assert!(read < 250, "{read} pages read");
}

#[test]
fn looking_for_a_snippet_stops_at_the_deadline() {
    let query = Query::parse("tapir");
    let text = format!("{}tapir", "word ".repeat(100_000));
    assert!(snippet(&text, &query, Instant::now() + Duration::from_secs(60)).is_some());
    assert_eq!(snippet(&text, &query, Instant::now()), None);
}

#[test]
fn a_part_repeating_the_word_is_searched_within_the_time_limit() {
    for (word, times) in [("needle", 30_000), ("tapir", 100_000)] {
        let (dir, mut index) = db();
        file(&mut index, "/home/notes.txt", NOW, &[("", &format!("{word} ").repeat(times))]);
        // A fresh connection, so SQLite's step count starts at zero.
        drop(index);
        let index = Index::open(&dir.path().join("find.db")).unwrap();
        let started = Instant::now();
        let got = search_all(&index, word, 60, NOW, CONTENT_TIME).unwrap();
        let took = started.elapsed();
        assert!(took < CONTENT_TIME + Duration::from_millis(100), "{word} x{times}: {took:?}");
        assert_eq!(got.hits.len(), 1);
    }
}

#[test]
fn labels_scripts_and_query_syntax() {
    let (_dir, mut index) = db();
    let a = file(
        &mut index,
        "/home/Documents/report.docx",
        NOW,
        &[("page 2", "resume résumé re\u{301}sume\u{301} किराया 東京 alpha"), ("page 3", "beta")],
    );
    let b = file(&mut index, "/home/Documents/budget.xlsx", NOW, &[("Q2 (hidden)", "capybara")]);
    let c = file(&mut index, "/home/Documents/deck.pptx", NOW, &[("slide 3 notes", "capybara")]);
    for q in ["", " \n\t \u{2003}"] {
        assert!(all(&index, q, 60).hits.is_empty());
    }
    for q in ["resume", "résumé", "re\u{301}sume\u{301}", "किराया", "東京"] {
        let got = all(&index, q, 60);
        assert_eq!(paths(&got), [a.as_str()]);
        assert_eq!(got.hits[0].found.as_ref().unwrap().label, "page 2");
    }
    assert!(all(&index, "alpha beta", 60).hits.is_empty());
    for hit in all(&index, "capybara", 60).hits {
        let want = if hit.file.path == b { "Q2 (hidden)" } else { "slide 3 notes" };
        assert!(hit.file.path == b || hit.file.path == c);
        assert_eq!(hit.found.as_ref().unwrap().label, want);
    }
    for q in ["*", "^", "\"", "\u{1}", "\u{2}", "\u{3}", "\u{7f}", "AND", "NEAR(alpha beta)", "text:alpha", "alpha OR beta"] {
        let got = search_all(&index, q, 60, NOW, CONTENT_TIME);
        assert!(got.is_ok(), "{q:?}: {got:?}");
    }
}

#[test]
fn each_file_is_found_once_with_the_label_of_the_part_its_words_are_in() {
    let (_dir, mut index) = db();
    let path = file(&mut index, "/home/notes.txt", NOW, &[("", "tapir")]);
    let id = file_id(&index, &path);
    part(&index, id, 1, "Q2", "okapi spreadsheet");
    part(&index, id, 2, "slide 2 notes", "capybara notes");
    part(&index, id, 3, "page 4", "wombat page");
    for (query, label) in [("okapi", "Q2"), ("capybara", "slide 2 notes"), ("wombat", "page 4")] {
        let got = all(&index, query, 60);
        assert_eq!(got.hits.len(), 1);
        assert_eq!(got.hits[0].found.as_ref().unwrap().label, label);
    }
    assert!(all(&index, "tapir okapi", 60).hits.is_empty());
    part(&index, id, 4, "page 5", "tapir again");
    assert_eq!(names(&all(&index, "tapir", 60)), ["notes.txt"]);
}

// --- the time limit and the connection ---

#[test]
fn a_search_stopped_at_its_time_returns_names_only_and_leaves_the_connection_clean() {
    let (_dir, mut index) = db();
    let named = file(&mut index, "/home/Documents/capybara.txt", NOW, &[]);
    for i in 0..205 {
        file(&mut index, &format!("/home/Documents/n{i}.txt"), NOW, &[("", "capybara")]);
    }
    let stopped = search_all(&index, "capybara", 60, NOW, Duration::ZERO).unwrap();
    assert!(stopped.contents_timed_out);
    assert!(!stopped.contents_capped, "205 parts are under the cap");
    assert_eq!(paths(&stopped), [named.as_str()]);
    plain_sql_runs(&index);
    let got = all(&index, "capybara", 500);
    assert_eq!(got.hits.len(), 206, "no cap on files matching by contents");
    assert!(!got.contents_timed_out);
    let got = all(&index, "capybara", 60);
    assert_eq!(got.hits.len(), 60);
    let unique: std::collections::HashSet<_> = paths(&got).into_iter().collect();
    assert_eq!(unique.len(), 60);
}

#[test]
fn a_failed_search_leaves_the_connection_clean() {
    let (_dir, mut index) = db();
    file(&mut index, "/home/notes.txt", NOW, &[("", "tapir")]);
    index.conn.execute_batch("ALTER TABLE extracts RENAME COLUMN size TO missing_size").unwrap();
    assert!(search_all(&index, "tapir", 60, NOW, Duration::ZERO).is_err());
    plain_sql_runs(&index);
}

// --- connections ---

#[test]
fn a_long_lived_reader_sees_what_the_worker_read() {
    let (dir, mut index) = db();
    let path = dir.path().join("unrelated.txt");
    std::fs::write(&path, "alpha").unwrap();
    let registry = std::sync::Arc::new(Registry::platform());
    store::register_kinds(&mut index, &registry).unwrap();
    index.upsert(&[record_of(&path, dir.path().to_str().unwrap()).unwrap()], 1).unwrap();
    let reader = Index::open(&dir.path().join("find.db")).unwrap();
    assert!(all(&reader, "alpha", 60).hits.is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = worker::Worker::start(
        dir.path().join("find.db"),
        registry,
        WorkerOptions {
            on_progress: std::sync::Arc::new(move |p: &store::Progress| {
                let _ = tx.send(p.clone());
            }),
            ..WorkerOptions::default()
        },
    )
    .unwrap();
    loop {
        let p = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        if p.read == 1 && p.waiting == 0 {
            break;
        }
    }
    assert!(worker.stop().errors.is_empty());
    assert_eq!(all(&reader, "alpha", 60).hits.len(), 1);
}

#[test]
fn a_content_reader_blocks_neither_the_writer_nor_name_searches() {
    let (dir, mut writer) = db();
    file(&mut writer, "/home/first.txt", NOW, &[("", "alpha")]);
    let contents = Index::open(&dir.path().join("find.db")).unwrap();
    let names = Index::open(&dir.path().join("find.db")).unwrap();
    contents.conn.execute_batch("BEGIN").unwrap();
    assert_eq!(contents.count().unwrap(), 1);
    file(&mut writer, "/home/second.txt", NOW, &[("", "beta")]);
    assert_eq!(contents.count().unwrap(), 1);
    assert_eq!(names.count().unwrap(), 2);
    assert_eq!(search::search(&names, "second", 60, NOW).unwrap().len(), 1);
    contents.conn.execute_batch("COMMIT").unwrap();
    assert_eq!(all(&contents, "beta", 60).hits.len(), 1);
}

#[test]
fn a_reader_sees_each_commit_and_the_writer_commits_during_its_reads() {
    let (dir, mut writer) = db();
    let path = file(&mut writer, "/home/notes.txt", NOW, &[("", "oldword")]);
    let id = file_id(&writer, &path);
    let reader = Index::open(&dir.path().join("find.db")).unwrap();
    assert_eq!(names(&all(&reader, "oldword", 60)), ["notes.txt"]);
    writer.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer.conn.execute("UPDATE contents SET text = 'newword' WHERE file_id = ?1", [id]).unwrap();
    assert_eq!(names(&all(&reader, "oldword", 60)), ["notes.txt"]);
    writer.conn.execute_batch("COMMIT").unwrap();
    assert!(all(&reader, "oldword", 60).hits.is_empty());
    assert_eq!(names(&all(&reader, "newword", 60)), ["notes.txt"]);
    reader.conn.execute_batch("BEGIN").unwrap();
    assert_eq!(names(&all(&reader, "newword", 60)), ["notes.txt"]);
    writer.conn.execute("UPDATE contents SET text = 'latestword' WHERE file_id = ?1", [id]).unwrap();
    reader.conn.execute_batch("COMMIT").unwrap();
    assert_eq!(names(&all(&reader, "latestword", 60)), ["notes.txt"]);
}
