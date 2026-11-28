use super::*;
use crate::data::Build;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Instant;

/// How long an ended process may take to leave the process table.
const GONE: Duration = Duration::from_secs(10);
/// Every stand-in that waits gives up after 30 seconds on its own.
const LIVES: &str = "i=0; while [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done";
/// A stand-in's first line: it leaves a mark that it ran.
const MARK: &str = ": > \"${0%/*}/ran\"";

/// A tools folder whose reader is a stand-in, and a data folder whose path
/// has a space in it, as the real one has, holding a store file.
struct Bench {
    dir: tempfile::TempDir,
    tools: Result<Tools, NotFound>,
    data: DataDir,
    runner: Runner,
}

impl Bench {
    /// show is the stand-in reader's script; it finds its folder as ${0%/*}.
    fn new(show: &str) -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("tools");
        fs::create_dir(&folder).unwrap();
        for piece in Piece::ALL {
            let path = folder.join(piece.name());
            let body = if piece == Piece::Show { show } else { "exit 0" };
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let data = data_at(&dir.path().join("Application Support").join("sifs data"));
        fs::create_dir_all(data.path()).unwrap();
        fs::write(data.path().join(STORE_FILE), b"").unwrap();
        Bench { tools: Tools::find(&folder), data, runner: Runner::default(), dir }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join("tools").join(name)
    }

    fn ask(&self, question: &Question) -> Result<Answer, NoAnswer> {
        ask(&self.runner, &self.tools, &self.data, question)
    }

    fn ran(&self) -> bool {
        self.file("ran").exists()
    }
}

fn data_at(path: &Path) -> DataDir {
    crate::data::data_dir(Build::Release, Path::new("/unused"), Some(path.as_os_str())).unwrap()
}

fn questions() -> [Question; 3] {
    [Question::Kinds, Question::Find("Commerzbank".into()), Question::Page("thing-1".into())]
}

fn could_not_answer(said: &str) -> Result<Answer, NoAnswer> {
    Err(NoAnswer::CouldNotAnswer { said: said.into() })
}

fn said(got: Result<Answer, NoAnswer>) -> String {
    match got {
        Err(NoAnswer::CouldNotAnswer { said }) => said,
        other => panic!("not could_not_answer: {other:?}"),
    }
}

fn pid_in(path: &Path) -> Option<libc::pid_t> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Whether the system has no live process with this pid (a zombie is dead).
fn gone(pid: libc::pid_t) -> bool {
    let out = Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    stat.trim().is_empty() || stat.trim().starts_with('Z')
}

fn within(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Every file in a folder and its bytes, folders inside included.
fn listing(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut all = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return all;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            all.insert(path.clone(), None);
            all.extend(listing(&path));
        } else {
            all.insert(path.clone(), Some(fs::read(&path).unwrap()));
        }
    }
    all
}

// --- C2: every reason, and nothing left behind ---

#[test]
fn each_reason_reaches_the_screen_as_a_tag() {
    let why = NotFound::NoFolder(PathBuf::from("/tmp/sifs-tools"));
    let cases = [
        (NoAnswer::NoTools { why: why.clone() }, serde_json::json!({"reason": "no_tools", "why": why.to_string()})),
        (NoAnswer::NothingReadYet, serde_json::json!({"reason": "nothing_read_yet"})),
        (NoAnswer::TookTooLong, serde_json::json!({"reason": "took_too_long"})),
        (NoAnswer::CouldNotAnswer { said: "sifs-show: x".into() }, serde_json::json!({"reason": "could_not_answer", "said": "sifs-show: x"})),
        (NoAnswer::Quitting, serde_json::json!({"reason": "quitting"})),
    ];
    for (reason, json) in cases {
        assert_eq!(serde_json::to_value(&reason).unwrap(), json, "{reason:?}");
    }
    assert_eq!(serde_json::to_value(Unopened::NotFound).unwrap(), serde_json::json!({"reason": "not_found"}));
    assert_eq!(
        serde_json::to_value(Unopened::CouldNotOpen { said: "x".into() }).unwrap(),
        serde_json::json!({"reason": "could_not_open", "said": "x"})
    );
}

#[test]
fn no_tools_passes_the_reason_through_whole_and_runs_nothing() {
    let mut bench = Bench::new(&format!("{MARK}\necho '{{}}'"));
    let reasons = [
        NotFound::NoFolder(bench.dir.path().join("absent")),
        NotFound::NotAbsolute("tools".into()),
        NotFound::Pieces { folder: bench.file(""), faults: vec![(Piece::Show, crate::tools::Fault::NotRunnable)] },
    ];
    for why in reasons {
        bench.tools = Err(why.clone());
        for question in questions() {
            assert_eq!(bench.ask(&question), Err(NoAnswer::NoTools { why: why.clone() }), "{question:?}");
        }
    }
    assert!(!bench.ran(), "no reader ran");
}

#[test]
fn no_store_file_is_nothing_read_yet_and_the_reader_never_runs() {
    let bench = Bench::new(&format!("{MARK}\necho '{{}}'"));
    let data = bench.data.path();
    fs::remove_file(data.join(STORE_FILE)).unwrap();
    fs::write(data.join("sifs.db"), "the index").unwrap();
    fs::create_dir(data.join("things-work")).unwrap();
    fs::write(data.join("things-work").join("run.json"), "{}").unwrap();
    let before = listing(data);
    for question in questions() {
        assert_eq!(bench.ask(&question), Err(NoAnswer::NothingReadYet), "{question:?}");
    }
    assert!(!bench.ran(), "the reader was not run");
    assert_eq!(listing(data), before, "the data folder is as it was");

    let nowhere = bench.dir.path().join("no data folder");
    let absent = data_at(&nowhere);
    assert_eq!(ask(&bench.runner, &bench.tools, &absent, &Question::Kinds), Err(NoAnswer::NothingReadYet));
    assert!(!nowhere.exists() && !bench.ran(), "nothing made, nothing run");

    fs::write(data.join(STORE_FILE), b"").unwrap();
    assert!(bench.ask(&Question::Kinds).is_ok(), "with the store the same reader answers");
    assert!(bench.ran());
}

#[test]
fn the_store_file_is_named_as_the_loader_names_it() {
    let go = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../engine/link/load/load.go")).unwrap();
    let constant = regex::Regex::new(r#"(?m)^const StoreFile = "([^"]*)"$"#).unwrap();
    let names: Vec<&str> = constant.captures_iter(&go).map(|c| c.get(1).unwrap().as_str()).collect();
    assert_eq!(names, [STORE_FILE]);
}

#[test]
fn a_reader_that_never_answers_is_ended_with_its_child_after_the_deadline() {
    let bench = Bench::new(&format!(
        "echo $$ > \"${{0%/*}}/pid\"\n/bin/sh -c 'echo $$ > \"$1\"; {LIVES}' child \"${{0%/*}}/child\" &\n{LIVES}"
    ));
    let started = Instant::now();
    assert_eq!(bench.ask(&Question::Find("slow".into())), Err(NoAnswer::TookTooLong));
    let took = started.elapsed();
    assert!(took >= TIME && took < TIME + GONE, "after about {TIME:?}: {took:?}");
    let pids = [pid_in(&bench.file("pid")).expect("the reader ran"), pid_in(&bench.file("child")).expect("its child ran")];
    assert!(within(GONE, || pids.iter().all(|&pid| gone(pid))), "alive: {pids:?}");
}

#[test]
fn exit_2_gives_the_readers_last_line() {
    let bench = Bench::new(
        "echo '{\"not\": \"an answer\"}'\necho 'sifs-show: first' >&2\necho 'sifs-show: no thing with id \"x\"' >&2\nexit 2",
    );
    assert_eq!(bench.ask(&Question::Page("x".into())), could_not_answer("sifs-show: no thing with id \"x\""));
}

#[test]
fn a_megabyte_of_standard_error_does_not_reach_the_screen() {
    let one_line = Bench::new("head -c 1048576 /dev/zero | tr '\\0' e >&2\nexit 2");
    let line = said(one_line.ask(&Question::Kinds));
    assert!(line.len() <= SAID_KEPT, "{} bytes", line.len());
    assert!(line.starts_with("eeee"), "{line}");
    let many = Bench::new("yes eeeeeeeeeeeeeee | head -c 1048576 >&2\necho >&2\necho 'sifs-show: the end' >&2\necho >&2\nexit 2");
    assert_eq!(many.ask(&Question::Kinds), could_not_answer("sifs-show: the end"));
}

#[test]
fn exit_0_with_nothing_printed_is_no_answer() {
    let bench = Bench::new("exit 0");
    let said = said(bench.ask(&Question::Kinds));
    assert!(!said.is_empty());
}

#[test]
fn exit_1_is_no_answer_even_with_a_document_printed() {
    let bench = Bench::new("echo '{}'\necho 'it broke' >&2\nexit 1");
    assert_eq!(bench.ask(&Question::Kinds), could_not_answer("it broke"));
    let quiet = Bench::new("echo '{}'\nexit 1");
    assert!(!said(quiet.ask(&Question::Kinds)).is_empty());
}

#[test]
fn a_reader_killed_by_a_signal_is_no_answer() {
    let bench = Bench::new("echo '{}'\necho dying >&2\nkill -TERM $$");
    assert!(!said(bench.ask(&Question::Kinds)).is_empty());
}

#[test]
fn too_much_output_is_no_answer() {
    let bench = Bench::new(&format!("echo $$ > \"${{0%/*}}/pid\"\nprintf '\"'\nexec /usr/bin/yes e"));
    assert!(!said(bench.ask(&Question::Kinds)).is_empty());
    let pid = pid_in(&bench.file("pid")).unwrap();
    assert!(within(GONE, || gone(pid)), "the reader is ended");
}

#[test]
fn a_stopped_runner_is_quitting_and_runs_nothing() {
    let bench = Bench::new(&format!("{MARK}\necho '{{}}'"));
    bench.runner.stop();
    for question in questions() {
        assert_eq!(bench.ask(&question), Err(NoAnswer::Quitting), "{question:?}");
    }
    assert!(!bench.ran());
}

#[test]
fn twenty_questions_at_once_get_twenty_answers_each_its_own() {
    let bench = Bench::new("echo $$ > \"${0%/*}/pids/$$\"\nsleep 0.2\nprintf '{\"asked\":\"%s\"}\\n' \"$2\"");
    fs::create_dir(bench.file("pids")).unwrap();
    let bench = &bench;
    let answers: Vec<_> = thread::scope(|s| {
        let asked: Vec<_> = (0..20).map(|i| s.spawn(move || bench.ask(&Question::Find(format!("q-{i}"))))).collect();
        asked.into_iter().map(|t| t.join().unwrap()).collect()
    });
    for (i, answer) in answers.into_iter().enumerate() {
        assert_eq!(answer.map(Answer::into_string), Ok(format!("{{\"asked\":\"--find=q-{i}\"}}\n")), "question {i}");
    }
    let pids: Vec<_> = fs::read_dir(bench.file("pids")).unwrap().map(|e| pid_in(&e.unwrap().path()).unwrap()).collect();
    assert_eq!(pids.len(), 20);
    assert!(within(GONE, || pids.iter().all(|&pid| gone(pid))), "every reader is gone");
}
