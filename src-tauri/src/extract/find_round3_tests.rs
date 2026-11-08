//! The step 3 review, round 3 (plans/phase3-reading-inside-files.md, "Step
//! 3 review, round 3"): the reviewers' tests, adapted. Each failed on
//! a4668b4.

use super::review_round2::{queue_len, set_time, version_2_index};
use super::*;
use crate::extract::{store, worker, Registry, WorkerOptions};
use crate::index::crawl::record_of;
use crate::index::search::CANDIDATES;
use std::ops::ControlFlow;

// --- 2. what was read before migration 3 ---

#[test]
fn after_the_upgrade_each_file_read_before_it_is_read_once_more_and_no_edit_is_missed() {
    let dir = tempfile::tempdir().unwrap();
    let (db, path) = version_2_index(dir.path());
    let root = dir.path().to_str().unwrap();
    let mut index = Index::open(&db).unwrap();
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    assert_eq!(queue_len(&index), 1, "queued by the migration");
    assert_eq!(all(&index, "tapir", 60).hits.len(), 1, "times unknown on both sides compare to the second");
    index.upsert(&[record_of(&path, root).unwrap()], 2).unwrap();
    assert!(all(&index, "tapir", 60).hits.is_empty(), "the file's nanoseconds are known and the contents' are not");
    std::fs::write(&path, "zebra").unwrap();
    set_time(&path, NOW, 900_000_000);
    index.upsert(&[record_of(&path, root).unwrap()], 3).unwrap();
    assert_eq!(queue_len(&index), 1);
    assert!(all(&index, "tapir", 60).hits.is_empty(), "old words hidden while the edit waits");
    assert!(worker::run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap().errors.is_empty());
    assert!(all(&index, "tapir", 60).hits.is_empty());
    assert_eq!(all(&index, "zebra", 60).hits.len(), 1);
}

#[test]
fn an_edit_before_the_first_scan_after_the_upgrade_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let (db, path) = version_2_index(dir.path());
    let root = dir.path().to_str().unwrap();
    let mut index = Index::open(&db).unwrap();
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    std::fs::write(&path, "zebra").unwrap();
    set_time(&path, NOW, 900_000_000);
    index.upsert(&[record_of(&path, root).unwrap()], 2).unwrap();
    let stats = worker::run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    assert_eq!(stats.reads, 1);
    assert!(all(&index, "tapir", 60).hits.is_empty());
    assert_eq!(all(&index, "zebra", 60).hits.len(), 1);
    assert_eq!(queue_len(&index), 0);
}

// --- 3. the name shortlist ---

#[test]
fn a_file_matching_by_name_and_contents_wins_past_the_name_shortlist() {
    let (_dir, mut index) = db();
    let records: Vec<FileRecord> = (0..CANDIDATES as i64)
        .map(|n| FileRecord {
            path: format!("/home/Documents/{n:04}/tapir.txt"),
            root: "/home".into(),
            name: "tapir.txt".into(),
            ext: "txt".into(),
            is_dir: false,
            size: 0,
            modified: NOW,
            inode: n,
            modified_ns: Some(0),
        })
        .collect();
    index.upsert(&records, 1).unwrap();
    let wanted = file(&mut index, "/home/Documents/zzzz/tapir.txt", NOW, &[("", "tapir")]);
    let got = all(&index, "tapir", 60);
    assert!(got.names_capped, "{} names match", CANDIDATES + 1);
    assert_eq!(got.hits[0].file.path, wanted, "the both-match bonus puts it first");
    assert!(!got.hits[0].fuzzy);
    assert!(got.hits[0].found.is_some());
    index.conn.execute("DELETE FROM files WHERE path = '/home/Documents/0000/tapir.txt'", []).unwrap();
    let shortlisted = all(&index, "tapir", 60);
    assert!(!shortlisted.names_capped);
    assert_eq!(shortlisted.hits[0].file.path, wanted);
    assert_eq!(got.hits[0].score, shortlisted.hits[0].score, "the same score in the shortlist or out of it");
}

#[test]
fn a_document_matching_by_name_and_contents_beats_5000_tied_names_in_other_folders() {
    let (_dir, mut index) = db();
    let records: Vec<FileRecord> = (0..CANDIDATES as i64)
        .map(|n| FileRecord {
            path: format!("/home/a{n:04}/tapir.txt"),
            root: "/home".into(),
            name: "tapir.txt".into(),
            ext: "txt".into(),
            is_dir: false,
            size: 1,
            modified: NOW,
            inode: n,
            modified_ns: Some(0),
        })
        .collect();
    index.upsert(&records, 1).unwrap();
    let wanted = file(&mut index, "/home/z9999/tapir.txt", NOW, &[("", "tapir")]);
    let got = all(&index, "tapir", 60);
    assert_eq!(got.hits[0].file.path, wanted);
}

// --- 4. the tokenizer ---

#[test]
fn nested_tokenizers_keep_each_ones_word() {
    let (_dir, index) = db();
    let mut outer = Tokenizer::contents(&index.conn).unwrap();
    let mut inner = Tokenizer::contents(&index.conn).unwrap();
    let long = "x".repeat(511);
    let mut seen = Vec::new();
    outer
        .words("tapir alpha", |word, _| {
            inner
                .words(&long, |w, _| {
                    assert_eq!(w.len(), 511);
                    ControlFlow::Continue(())
                })
                .unwrap();
            seen.push(word.to_string());
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(seen, ["tapir", "alpha"], "the outer word is intact after the inner call");
}
