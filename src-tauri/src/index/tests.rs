use super::crawl::{refresh, scan};
use super::db::Index;
use super::ignore::Rules;
use super::search::search;
use super::watch::Watch;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

/// A home folder with documents, an ignored dependency folder and an app bundle.
fn fixture() -> (tempfile::TempDir, String, Rules) {
    let dir = tempfile::tempdir().unwrap();
    // macOS temp paths go through a symlink; the watcher reports real paths.
    let home = fs::canonicalize(dir.path()).unwrap().to_str().unwrap().to_string();
    for p in [
        "Documents/Resume_2025.pdf",
        "Documents/Old/Resume_2023.docx",
        "Documents/Flat/LeaveLicense_2025.pdf",
        "Documents/Tax/Form16_FY2024.pdf",
        "Downloads/hdfc_statement_march.pdf",
        "code/app/node_modules/left-pad/index.js",
        "code/app/src/main.rs",
        "Applications/Calculator.app/Contents/MacOS/Calculator",
        "Library/Caches/thing.cache",
    ] {
        touch(&Path::new(&home).join(p));
    }
    let rules = Rules::new(&Rules::default_patterns(), &home).unwrap();
    (dir, home, rules)
}

fn open(dir: &tempfile::TempDir) -> Index {
    Index::open(&dir.path().join("sifs.db")).unwrap()
}

fn names(index: &Index, q: &str) -> Vec<String> {
    search(index, q, 50, now()).unwrap().into_iter().map(|h| h.file.name).collect()
}

#[test]
fn scan_indexes_what_the_rules_allow() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    let stats = scan(&mut index, &home, &rules, |_| {}).unwrap();
    assert!(stats.indexed > 0);
    assert!(index.get(&format!("{home}/Documents/Resume_2025.pdf")).unwrap().is_some());
    assert!(index.get(&format!("{home}/code/app/src/main.rs")).unwrap().is_some());
    // Ignored folders are skipped with everything in them.
    assert!(index.get(&format!("{home}/code/app/node_modules")).unwrap().is_none());
    assert!(index.get(&format!("{home}/code/app/node_modules/left-pad/index.js")).unwrap().is_none());
    assert!(index.get(&format!("{home}/Library/Caches/thing.cache")).unwrap().is_none());
    // A package is one item, without its insides.
    assert!(index.get(&format!("{home}/Applications/Calculator.app")).unwrap().is_some());
    assert!(index.get(&format!("{home}/Applications/Calculator.app/Contents")).unwrap().is_none());
    let record = index.get(&format!("{home}/Documents/Resume_2025.pdf")).unwrap().unwrap();
    assert_eq!((record.ext.as_str(), record.is_dir, record.size), ("pdf", false, 1));
}

#[test]
fn a_rescan_removes_what_is_gone_and_keeps_the_rest() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let before = index.count().unwrap();
    fs::remove_dir_all(format!("{home}/Documents/Old")).unwrap();
    let stats = scan(&mut index, &home, &rules, |_| {}).unwrap();
    assert_eq!(stats.removed, 2, "the folder and its file");
    assert_eq!(index.count().unwrap(), before - 2);
    assert!(index.get(&format!("{home}/Documents/Resume_2025.pdf")).unwrap().is_some());
}

#[test]
fn search_finds_substrings_in_any_case() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let found = names(&index, "resume");
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(names(&index, "LICENSE").contains(&"LeaveLicense_2025.pdf".to_string()), "substring, any case");
    assert_eq!(names(&index, "hdfc march"), vec!["hdfc_statement_march.pdf"], "every term must match");
    assert_eq!(names(&index, "16"), vec!["Form16_FY2024.pdf"], "short terms work too");
    assert!(names(&index, "zzzz").is_empty());
}

#[test]
fn search_tolerates_a_typo() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let hits = search(&index, "staement", 10, now()).unwrap();
    assert!(hits.iter().any(|h| h.file.name == "hdfc_statement_march.pdf" && h.fuzzy), "{hits:?}");
}

#[test]
fn ranking_prefers_name_starts_and_recent_files() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    // Both match "resume" at the start of the name; the newer one first.
    let old = fs::File::options().write(true).open(format!("{home}/Documents/Old/Resume_2023.docx")).unwrap();
    old.set_modified(SystemTime::now() - Duration::from_secs(400 * 86_400)).unwrap();
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    assert_eq!(names(&index, "resume"), vec!["Resume_2025.pdf", "Resume_2023.docx"]);
    // An exact name beats a name that only contains the query.
    touch(&Path::new(&home).join("Documents/statement"));
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    assert_eq!(names(&index, "statement")[0], "statement");
}

#[test]
fn a_chosen_folder_is_indexed_even_inside_a_hidden_one() {
    let (dir, home, rules) = fixture();
    let chosen = format!("{home}/.config/notes");
    touch(Path::new(&format!("{chosen}/todo.md")));
    touch(Path::new(&format!("{chosen}/.hidden/secret.md")));
    let mut index = open(&dir);
    scan(&mut index, &chosen, &rules, |_| {}).unwrap();
    assert!(index.get(&format!("{chosen}/todo.md")).unwrap().is_some(), "the chosen folder's own files");
    assert!(index.get(&format!("{chosen}/.hidden/secret.md")).unwrap().is_none(), "hidden folders below it are still skipped");
    // Scanning the home folder skips the hidden folder the other was chosen in.
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    assert!(index.get(&format!("{home}/.config")).unwrap().is_none());
}

#[test]
fn ranking_puts_documents_above_same_named_code_and_build_products() {
    let (dir, home, rules) = fixture();
    for p in [
        "Downloads/google-cloud-sdk/lib/surface/compute/resume.py",
        "code/app/build/Objects/ResumeButton.o",
        "code/app/Sources/ResumeButton.swift",
        "Documents/Sanket_Agarwal_Backend_Resume.pdf",
    ] {
        touch(&Path::new(&home).join(p));
    }
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let found = names(&index, "resume");
    let at = |name: &str| found.iter().position(|n| n == name).unwrap_or_else(|| panic!("{name} not found in {found:?}"));
    for document in ["Sanket_Agarwal_Backend_Resume.pdf", "Resume_2025.pdf", "Resume_2023.docx"] {
        for code in ["resume.py", "ResumeButton.swift", "ResumeButton.o"] {
            assert!(at(document) < at(code), "{document} should rank above {code}: {found:?}");
        }
    }
    assert!(at("ResumeButton.swift") < at("ResumeButton.o"), "source above a build product: {found:?}");
}

#[test]
fn xcode_build_folders_are_skipped() {
    let (dir, home, rules) = fixture();
    for p in ["code/ios/build/Index.noindex/DataStore/unit", "code/ios/DerivedDataDevice/x.o", "code/ios/build/SourcePackages/checkouts/lib.swift"] {
        touch(&Path::new(&home).join(p));
    }
    let mut index = open(&dir);
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    for p in ["code/ios/build/Index.noindex", "code/ios/DerivedDataDevice", "code/ios/build/SourcePackages"] {
        assert!(index.get(&format!("{home}/{p}")).unwrap().is_none(), "{p} is indexed");
    }
}

#[test]
fn settings_are_kept() {
    let (dir, _home, _rules) = fixture();
    let mut index = open(&dir);
    assert!(index.ignore_patterns().unwrap().is_none(), "unset until the user sets them");
    assert!(index.add_folder("/Users/me/Documents", 1).unwrap());
    assert!(!index.add_folder("/Users/me/Documents", 2).unwrap());
    index.set_ignore_patterns(&["**/build".to_string()]).unwrap();
    drop(index);
    let mut index = open(&dir);
    assert_eq!(index.folders().unwrap(), vec!["/Users/me/Documents"]);
    assert_eq!(index.ignore_patterns().unwrap(), Some(vec!["**/build".to_string()]));
    assert!(index.remove_folder("/Users/me/Documents").unwrap());
    assert!(index.folders().unwrap().is_empty());
}

#[test]
fn refresh_follows_creates_renames_moves_and_deletes() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    index.add_folder(&home, 0).unwrap();
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let roots = vec![home.clone()];
    let at = |p: &str| format!("{home}/{p}");

    touch(Path::new(&at("Documents/new.txt")));
    refresh(&mut index, Path::new(&at("Documents/new.txt")), &roots, &rules).unwrap();
    assert!(index.get(&at("Documents/new.txt")).unwrap().is_some());

    // A folder moved in brings what's inside it.
    fs::create_dir_all(at("elsewhere/Trip/Day1")).unwrap();
    touch(Path::new(&at("elsewhere/Trip/Day1/photo.jpg")));
    let outside = dir.path().join("outside");
    fs::rename(at("elsewhere/Trip"), &outside).unwrap();
    fs::rename(&outside, at("Documents/Trip")).unwrap();
    refresh(&mut index, Path::new(&at("Documents/Trip")), &roots, &rules).unwrap();
    assert!(index.get(&at("Documents/Trip/Day1/photo.jpg")).unwrap().is_some());

    // A rename: the old path is gone, the new one is there.
    fs::rename(at("Documents/Trip"), at("Documents/Goa")).unwrap();
    refresh(&mut index, Path::new(&at("Documents/Trip")), &roots, &rules).unwrap();
    refresh(&mut index, Path::new(&at("Documents/Goa")), &roots, &rules).unwrap();
    assert!(index.get(&at("Documents/Trip/Day1/photo.jpg")).unwrap().is_none());
    assert!(index.get(&at("Documents/Goa/Day1/photo.jpg")).unwrap().is_some());

    // A delete removes the folder and everything under it, and nothing else.
    fs::remove_dir_all(at("Documents/Goa")).unwrap();
    refresh(&mut index, Path::new(&at("Documents/Goa")), &roots, &rules).unwrap();
    assert!(index.get(&at("Documents/Goa/Day1")).unwrap().is_none());
    assert!(index.get(&at("Documents/Resume_2025.pdf")).unwrap().is_some());

    // Changes under ignored folders, inside packages or outside the roots are not indexed.
    touch(Path::new(&at("code/app/node_modules/x.js")));
    refresh(&mut index, Path::new(&at("code/app/node_modules/x.js")), &roots, &rules).unwrap();
    assert!(index.get(&at("code/app/node_modules/x.js")).unwrap().is_none());
    touch(Path::new(&at("Applications/Calculator.app/Contents/Info.plist")));
    refresh(&mut index, Path::new(&at("Applications/Calculator.app/Contents/Info.plist")), &roots, &rules).unwrap();
    assert!(index.get(&at("Applications/Calculator.app/Contents/Info.plist")).unwrap().is_none());
    refresh(&mut index, Path::new("/etc/hosts"), &roots, &rules).unwrap();
    assert!(index.get("/etc/hosts").unwrap().is_none());
}

#[test]
fn a_file_refreshed_during_a_scan_stays_indexed() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    index.add_folder(&home, 0).unwrap();
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    // A scan starts (its number is taken), the watcher then refreshes a new
    // file, and the scan ends without having walked it.
    let mut scanner = open(&dir);
    let running = scanner.next_scan().unwrap();
    let created = format!("{home}/Documents/created-during-scan.txt");
    touch(Path::new(&created));
    refresh(&mut index, Path::new(&created), std::slice::from_ref(&home), &rules).unwrap();
    scanner.remove_unseen(&home, running).unwrap();
    assert!(index.get(&created).unwrap().is_some(), "a refreshed row is not unseen by a scan running when it was refreshed");
}

#[test]
fn removing_a_folder_removes_its_files_only() {
    let (dir, home, rules) = fixture();
    let mut index = open(&dir);
    let docs = format!("{home}/Documents");
    let downloads = format!("{home}/Downloads");
    scan(&mut index, &docs, &rules, |_| {}).unwrap();
    scan(&mut index, &downloads, &rules, |_| {}).unwrap();
    assert!(index.count_root(&docs).unwrap() > 0);
    index.remove_root(&docs).unwrap();
    assert_eq!(index.count_root(&docs).unwrap(), 0);
    assert!(index.count_root(&downloads).unwrap() > 0);
}

/// Waits until check passes or fails after a deadline: the watcher reports
/// asynchronously, so the test waits for the outcome, not for a set time.
pub(crate) fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn the_watcher_keeps_the_index_current() {
    let (dir, home, rules) = fixture();
    let db = dir.path().join("sifs.db");
    let mut index = Index::open(&db).unwrap();
    index.add_folder(&home, 0).unwrap();
    scan(&mut index, &home, &rules, |_| {}).unwrap();
    let _watch = Watch::start(db.clone(), vec![home.clone()], Arc::new(Mutex::new(rules)), |_| {}).unwrap();
    let reader = Index::open(&db).unwrap();
    let at = |p: &str| format!("{home}/{p}");

    touch(Path::new(&at("Documents/watched.txt")));
    eventually("a created file", || reader.get(&at("Documents/watched.txt")).unwrap().is_some());
    fs::rename(at("Documents/watched.txt"), at("Documents/renamed.txt")).unwrap();
    eventually("a renamed file", || {
        reader.get(&at("Documents/watched.txt")).unwrap().is_none() && reader.get(&at("Documents/renamed.txt")).unwrap().is_some()
    });
    fs::remove_file(at("Documents/renamed.txt")).unwrap();
    eventually("a deleted file", || reader.get(&at("Documents/renamed.txt")).unwrap().is_none());
}

/// Several folders scanned at once, the watcher's writes, settings changes
/// and the worker reading inside files, each on its own connection: SQLite
/// has one writer at a time, and none of them may fail with "database is
/// locked".
#[test]
fn concurrent_writers_never_see_a_locked_database() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap().to_str().unwrap().to_string();
    let roots: Vec<String> = (0..4).map(|r| format!("{base}/root{r}")).collect();
    for root in &roots {
        for d in 0..40 {
            for f in 0..60 {
                touch(Path::new(&format!("{root}/d{d}/file{f}.txt")));
            }
        }
    }
    let db = dir.path().join("sifs.db");
    let registry = Arc::new(crate::extract::Registry::platform());
    crate::extract::store::register_kinds(&mut Index::open(&db).unwrap(), &registry).unwrap();
    let options = crate::extract::WorkerOptions { idle_poll: Duration::from_millis(20), ..Default::default() };
    let worker = crate::extract::Worker::start(db.clone(), registry, options).unwrap();
    let rules = Arc::new(Rules::new(&Rules::default_patterns(), &base).unwrap());
    let mut threads = Vec::new();
    // Every folder at once, each scanned three times over; the app never
    // scans one folder twice at the same time.
    for root in roots.clone() {
        let (db, rules) = (db.clone(), rules.clone());
        threads.push(std::thread::spawn(move || {
            let mut index = Index::open(&db).unwrap();
            for round in 0..3 {
                scan(&mut index, &root, &rules, |_| {}).map_err(|e| format!("scan {root} round {round}: {e:#}"))?;
            }
            Ok(())
        }));
    }
    {
        let (db, roots) = (db.clone(), roots.clone());
        threads.push(std::thread::spawn(move || {
            let mut index = Index::open(&db).unwrap();
            for i in 0..200 {
                let root = &roots[i % roots.len()];
                index.add_folder(root, i as i64).map_err(|e| format!("add_folder: {e:#}"))?;
                index.set_ignore_patterns(&Rules::default_patterns()).map_err(|e| format!("set_ignore: {e:#}"))?;
                let path = format!("{root}/d0/file0.txt");
                crate::index::crawl::refresh(&mut index, Path::new(&path), &roots, &Rules::new(&Rules::default_patterns(), "/").unwrap())
                    .map_err(|e| format!("refresh: {e:#}"))?;
            }
            Ok(())
        }));
    }
    let failures: Vec<String> = threads.into_iter().filter_map(|t| t.join().unwrap().err()).collect();
    assert!(failures.is_empty(), "{failures:#?}");
    let index = Index::open(&db).unwrap();
    assert_eq!(index.count().unwrap(), 4 * (1 + 40 + 40 * 60));
    worker.wake();
    eventually("every file read", || crate::extract::store::progress(&index).unwrap().waiting == 0);
    let stats = worker.stop();
    assert!(stats.errors.is_empty(), "{:#?}", stats.errors);
    assert_eq!(crate::extract::store::progress(&index).unwrap().read, 4 * 40 * 60);
}
