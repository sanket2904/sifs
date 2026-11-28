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
        (
            NoAnswer::CouldNotAnswer { said: "sifs-show: x".into() },
            serde_json::json!({"reason": "could_not_answer", "said": "sifs-show: x"}),
        ),
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
    let bench =
        Bench::new(&format!("echo $$ > \"${{0%/*}}/pid\"\n/bin/sh -c 'echo $$ > \"$1\"; {LIVES}' child \"${{0%/*}}/child\" &\n{LIVES}"));
    let started = Instant::now();
    assert_eq!(bench.ask(&Question::Find("slow".into())), Err(NoAnswer::TookTooLong));
    let took = started.elapsed();
    assert!(took >= TIME && took < TIME + GONE, "after about {TIME:?}: {took:?}");
    let pids = [pid_in(&bench.file("pid")).expect("the reader ran"), pid_in(&bench.file("child")).expect("its child ran")];
    assert!(within(GONE, || pids.iter().all(|&pid| gone(pid))), "alive: {pids:?}");
}

#[test]
fn exit_2_gives_the_readers_last_line() {
    let bench =
        Bench::new("echo '{\"not\": \"an answer\"}'\necho 'sifs-show: first' >&2\necho 'sifs-show: no thing with id \"x\"' >&2\nexit 2");
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
    let bench = Bench::new("echo $$ > \"${0%/*}/pid\"\nprintf '\"'\nexec /usr/bin/yes e");
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

// --- C1: the three questions ---

/// A stand-in that records its arguments and prints the file answer.
fn printing() -> Bench {
    Bench::new(&format!("{MARK}\nprintf '%s\\0' \"$@\" > \"${{0%/*}}/args\"\ncat \"${{0%/*}}/answer\""))
}

fn recorded(bench: &Bench) -> Vec<Vec<u8>> {
    let mut args: Vec<Vec<u8>> = fs::read(bench.file("args")).unwrap().split(|&b| b == 0).map(<[u8]>::to_vec).collect();
    assert_eq!(args.pop(), Some(Vec::new()), "each argument ends in a NUL");
    args
}

fn nested(depth: usize) -> String {
    format!("{{\"a\":{}{}{}}}", "[{\"a\":".repeat(depth), "null", "}]".repeat(depth))
}

#[test]
fn the_answer_is_byte_for_byte_what_the_reader_printed() {
    let bench = printing();
    let big = format!("{{\"rows\":[{}\"end\"]}}\n", "\"0123456789abcdef0123456789abcdef\",".repeat(2 << 20 >> 5));
    assert!(big.len() > 2 << 20);
    let documents = [
        "{\"kinds\":[]}\n".to_string(),
        "{}".to_string(),
        big,
        "{\"t\":\"Relevé de compte – 取引明細 😀\",\"e\":\"\\u00e9 \\\"q\\\" \\\\ \\/ \\n\\t\\r\\b\\f \\ud83d\\ude00 \\u0000\"}\n"
            .to_string(),
        format!("{}\n", nested(10_000)),
        " \n\t{ \"b\" : 1.0 , \"a\":1E2,\"a\" :2, \"n\":123456789012345678901234567890, \"x\":-0.0e-0 }\r\n\n".to_string(),
        "[1, \"two\", null, true]\n".to_string(),
    ];
    for document in documents {
        fs::write(bench.file("answer"), &document).unwrap();
        for question in questions() {
            let answer = bench.ask(&question).unwrap_or_else(|e| panic!("{question:?}: {e:?}"));
            assert!(answer.as_str().as_bytes() == document.as_bytes(), "{question:?}: {:?}", &document[..60.min(document.len())]);
        }
    }
}

#[test]
fn output_that_is_not_one_document_is_never_passed_on() {
    let bench = printing();
    let cut = nested(50);
    let not_one: [&[u8]; 9] = [
        b"",
        b"\n",
        b"{}{}",
        b"{} {}\n",
        b"{\"a\":",
        &cut.as_bytes()[..cut.len() - 1],
        b"sifs-show: hello {}\n",
        b"{} trailing\n",
        b"{\"a\":\"\xff\xfe\"}\n",
    ];
    for output in not_one {
        fs::write(bench.file("answer"), output).unwrap();
        let got = bench.ask(&Question::Kinds);
        assert!(matches!(got, Err(NoAnswer::CouldNotAnswer { .. })), "{:?}: {got:?}", String::from_utf8_lossy(output));
    }
}

/// Words and ids as a person may type them, or a fact may give them.
const AWKWARD: [&str; 20] = [
    "two words",
    "it's \"quoted\"",
    "a\nnewline",
    "Relevé 取引明細 😀",
    "-n",
    "-",
    "--",
    "--reveal",
    "-reveal",
    "--reveal=true",
    "-reveal=1",
    "--kinds",
    "--page=x",
    "--find=x",
    "--data-dir=/",
    "",
    "*",
    "back\\slash",
    "=",
    "x --reveal",
];

#[test]
fn words_and_ids_reach_the_reader_whole_as_the_value_of_their_flag() {
    let bench = printing();
    fs::write(bench.file("answer"), "{}").unwrap();
    let made = bench.dir.path().join("made-by-a-shell");
    let shell = format!("$(touch {})", made.display());
    let mut words: Vec<&str> = AWKWARD.to_vec();
    words.push(&shell);
    let dir = [b"--data-dir=".as_slice(), bench.data.path().as_os_str().as_encoded_bytes()].concat();
    assert!(bench.data.path().to_str().unwrap().contains("Application Support/sifs data"));
    assert_eq!(bench.ask(&Question::Kinds).map(Answer::into_string), Ok("{}".into()));
    assert_eq!(recorded(&bench), [dir.clone(), b"--kinds".to_vec()]);
    for word in words {
        for (question, flag) in [(Question::Find(word.into()), "--find="), (Question::Page(word.into()), "--page=")] {
            assert!(bench.ask(&question).is_ok(), "{question:?}");
            assert_eq!(recorded(&bench), [dir.clone(), format!("{flag}{word}").into_bytes()], "{question:?}");
        }
    }
    assert!(!made.exists(), "no shell read the words");
}

/// Go's flag package reads `-name`, `--name`, `-name=value` and
/// `--name=value` alike; a value is the rest of its own argument.
fn reads_as_reveal(arg: &[u8]) -> bool {
    let name = arg.strip_prefix(b"--").or_else(|| arg.strip_prefix(b"-")).unwrap_or(b"");
    name == b"reveal" || name.starts_with(b"reveal=")
}

#[test]
fn no_argument_can_be_read_as_the_raw_values_switch() {
    let bench = printing();
    fs::write(bench.file("answer"), "{}").unwrap();
    assert!(reads_as_reveal(b"--reveal") && reads_as_reveal(b"-reveal=true") && !reads_as_reveal(b"--find=--reveal"));
    let mut asked = vec![Question::Kinds];
    for word in AWKWARD {
        asked.extend([Question::Find(word.into()), Question::Page(word.into())]);
    }
    for question in asked {
        assert!(bench.ask(&question).is_ok(), "{question:?}");
        let args = recorded(&bench);
        assert_eq!(args.len(), 2, "{question:?}");
        assert!(!args.iter().any(|arg| reads_as_reveal(arg)), "{question:?}: {args:?}");
    }
}

#[test]
fn words_too_long_or_with_a_nul_byte_are_no_answer_and_start_nothing() {
    let bench = printing();
    fs::write(bench.file("answer"), "{}").unwrap();
    for question in [
        Question::Find("x".repeat(2 << 20)),
        Question::Page("x".repeat(2 << 20)),
        Question::Find("a\0b".into()),
        Question::Page("a\0b".into()),
        Question::Find("\0".into()),
    ] {
        let said = said(bench.ask(&question));
        assert!(said.len() <= SAID_KEPT, "{} bytes", said.len());
        assert!(!bench.ran(), "nothing started");
    }
    assert!(bench.ask(&Question::Kinds).is_ok() && bench.ran(), "the same reader runs for a plain question");
}

// --- C4: the commands, read from the source ---

/// Non-test Rust in the crate: every file under src and build.rs, each up
/// to its first `#[cfg(test)]`, test files left out.
fn crate_source() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, all: &mut Vec<(PathBuf, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if path.is_dir() {
                walk(&path, all);
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                let text = fs::read_to_string(&path).unwrap();
                let text = text.split("#[cfg(test)]").next().unwrap().to_string();
                all.push((path, text));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = vec![(root.join("build.rs"), fs::read_to_string(root.join("build.rs")).unwrap())];
    walk(&root.join("src"), &mut all);
    all
}

#[test]
fn source_guard_the_four_commands_are_registered_and_take_no_path_or_switch() {
    let app = include_str!("app.rs");
    let start = app.find("tauri::generate_handler![").expect("one handler list");
    let handler = &app[start..start + app[start..].find(']').unwrap()];
    let registered: Vec<&str> = handler.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    for (command, signature) in [
        ("reader_kinds", "async fn reader_kinds(app: AppHandle) -> Result<Response, NoAnswer> {"),
        ("reader_find", "async fn reader_find(app: AppHandle, words: String) -> Result<Response, NoAnswer> {"),
        ("reader_page", "async fn reader_page(app: AppHandle, id: String) -> Result<Response, NoAnswer> {"),
        ("open_source", "async fn open_source(app: AppHandle, record: String) -> Result<(), Unopened> {"),
    ] {
        assert_eq!(registered.iter().filter(|&&w| w == command).count(), 1, "{command} is registered once");
        assert_eq!(app.matches(&format!("fn {command}(")).count(), 1, "{command} is defined once");
        assert_eq!(app.matches(&format!("#[tauri::command]\n{signature}")).count(), 1, "{command}'s arguments: {signature}");
    }
    assert_eq!(app.matches("reader::ask(").count(), 1, "the reader is asked in one place");
    assert_eq!(app.matches("reader::source_file(").count(), 1, "a source becomes a file in one place");
    assert_eq!(app.matches(".open_path(").count(), 1, "files are opened in one place");
    assert_eq!(app.matches("spawn_blocking(").count(), 4, "search_files, search_all, the reader and open_source run off the main thread");
}

#[test]
fn source_guard_no_door_to_raw_values() {
    let source = crate_source();
    assert!(source.iter().any(|(path, _)| path.ends_with("src/reader.rs")) && source.len() > 20, "the crate is read");
    for (path, text) in source {
        let text = text.to_lowercase().replace("reveal_path", "").replace("reveal_item_in_dir", "");
        assert!(!text.contains("reveal"), "{} names reveal", path.display());
    }
}

/// Every key in the reader's committed answers.
fn reader_keys() -> std::collections::BTreeSet<String> {
    fn walk(value: &serde_json::Value, keys: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                        keys.insert(key.clone());
                    }
                    walk(value, keys);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, keys)),
            _ => {}
        }
    }
    let mut keys = std::collections::BTreeSet::from(["query_ms".to_string()]);
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine/link/show/testdata");
    for entry in fs::read_dir(goldens).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            walk(&serde_json::from_slice(&fs::read(&path).unwrap()).unwrap(), &mut keys);
        }
    }
    keys
}

#[test]
fn source_guard_the_reader_module_names_nothing_in_the_readers_answers() {
    let keys = reader_keys();
    assert!(keys.contains("sources") && keys.contains("groups") && keys.contains("things"), "{keys:?}");
    let source = include_str!("reader.rs").split("#[cfg(test)]").next().unwrap();
    assert!(!source.contains("Deserialize"), "nothing is read into a type");
    let types = regex::Regex::new(r"\b(?:struct|enum)\s+(\w+)").unwrap();
    let fields = regex::Regex::new(r"(?m)^\s*(?:pub(?:\(crate\))?\s+)?([a-z_][a-z0-9_]*):\s").unwrap();
    let snake = |name: &str| {
        let mut out = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_uppercase() && i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        }
        out
    };
    let names: Vec<String> =
        types.captures_iter(source).map(|c| snake(&c[1])).chain(fields.captures_iter(source).map(|c| c[1].to_string())).collect();
    assert!(names.contains(&"no_answer".to_string()) && names.contains(&"said".to_string()), "{names:?}");
    for name in &names {
        assert!(!keys.contains(name), "reader.rs names {name}, a key of the reader's answers");
    }
    for key in &keys {
        assert!(!source.contains(&format!("\"{key}\"")), "reader.rs names the key {key:?}");
    }
}

// --- C5: with the real tools ---

/// The tools SIFS_TOOLS_DIR names. Unset, the test says so by name and passes.
fn real_tools(test: &str) -> Option<Tools> {
    let Some(folder) = std::env::var_os(crate::tools::TOOLS_DIR_VAR) else {
        eprintln!("{test}: NOT RUN, {} is not set", crate::tools::TOOLS_DIR_VAR);
        return None;
    };
    Some(Tools::find(Path::new(&folder)).unwrap_or_else(|reason| panic!("{reason}")))
}

const SAMPLE_RUN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../sample/runs/go-m1a-on-text.json");

/// A data folder with a space in its path, the sample's run loaded into it.
fn loaded(tools: &Tools, dir: &Path) -> DataDir {
    let data = data_at(&dir.join("Application Support").join("sifs data"));
    let load = Command::new(tools.path(Piece::Load))
        .args([OsString::from("--run"), SAMPLE_RUN.into(), "--data-dir".into(), data.path().into()])
        .output()
        .unwrap();
    assert_eq!(load.status.code(), Some(0), "{}", String::from_utf8_lossy(&load.stderr));
    data
}

fn untimed(answer: &str) -> String {
    regex::Regex::new(r#""query_ms":\d+"#).unwrap().replace_all(answer, "\"query_ms\":0").into_owned()
}

/// The values the sample's documents print in full: the IBANs and the
/// passport number of sample/README.md, and the passport numbers of the
/// sample's labels.
fn raw_values() -> Vec<String> {
    let mut raw: Vec<String> =
        ["NL91ABNA0417164300", "NL91 ABNA 0417 1643 00", "DE89370400440532013000", "DE89 3704 0044 0532 0130 00", "L898902C3"]
            .map(String::from)
            .to_vec();
    let labels: serde_json::Value =
        serde_json::from_slice(&fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sample/labels.json")).unwrap()).unwrap();
    fn walk(value: &serde_json::Value, raw: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("kind").and_then(|k| k.as_str()) == Some("passport_number") {
                    raw.extend(map.get("value").and_then(|v| v.as_str()).map(String::from));
                }
                map.values().for_each(|v| walk(v, raw));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, raw)),
            _ => {}
        }
    }
    walk(&labels, &mut raw);
    raw.sort();
    raw.dedup();
    raw
}

#[test]
fn real_reader_answers_as_it_does_run_directly_and_masks_every_private_number() {
    let Some(tools) = real_tools("real_reader_answers_as_it_does_run_directly_and_masks_every_private_number") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let data = loaded(&tools, dir.path());
    let (runner, found) = (Runner::default(), Ok(tools.clone()));
    let directly = |args: &[&str]| {
        let out = Command::new(tools.path(Piece::Show)).arg("--data-dir").arg(data.path()).args(args).output().unwrap();
        (out.status.code(), String::from_utf8(out.stdout).unwrap(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let ours = |question: &Question| ask(&runner, &found, &data, question);
    let mut answers = Vec::new();
    let mut same = |question: Question, args: &[&str]| -> serde_json::Value {
        let answer = ours(&question).unwrap_or_else(|e| panic!("{question:?}: {e:?}")).into_string();
        let (code, stdout, _) = directly(args);
        assert_eq!(code, Some(0), "{args:?}");
        assert_eq!(untimed(&answer), untimed(&stdout), "{question:?}");
        answers.push(answer.clone());
        serde_json::from_str(&answer).unwrap()
    };
    same(Question::Kinds, &["--kinds"]);
    let mut first = Vec::new();
    for words in ["Commerzbank", "NL91ABNA0417164300"] {
        let found = same(Question::Find(words.into()), &["--find", words]);
        let id = found["groups"][0]["things"][0]["id"].as_str().unwrap_or_else(|| panic!("{words} finds a thing")).to_string();
        same(Question::Page(id.clone()), &["--page", &id]);
        first.push(id);
    }
    for words in ["--reveal", "-reveal"] {
        same(Question::Find(words.into()), &["--find", words]);
    }
    let raw = raw_values();
    println!("searched every answer for: {raw:?}");
    let (_, revealed, _) = directly(&["--find", "NL91ABNA0417164300", "--reveal"]);
    assert!(revealed.contains("NL91ABNA0417164300"), "the search sees a raw value where the reader gives one");
    for answer in &answers {
        for value in &raw {
            assert!(!answer.contains(value.as_str()), "{value} is shown unmasked");
        }
    }

    let (code, _, stderr) = directly(&["--page", "no-such-thing"]);
    assert_eq!(code, Some(2));
    assert_eq!(ours(&Question::Page("no-such-thing".into())), Err(NoAnswer::CouldNotAnswer { said: stderr.trim_end().to_string() }));

    let empty = data_at(&dir.path().join("empty data"));
    fs::create_dir(empty.path()).unwrap();
    assert_eq!(ask(&runner, &found, &empty, &Question::Kinds), Err(NoAnswer::NothingReadYet));
    assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0, "no store made");

    for (name, question) in [
        ("kinds", Question::Kinds),
        ("find Commerzbank", Question::Find("Commerzbank".into())),
        ("find NL91ABNA0417164300", Question::Find("NL91ABNA0417164300".into())),
        ("find a word that is nowhere", Question::Find("Zanzibar".into())),
        ("page of the first thing Commerzbank finds", Question::Page(first[0].clone())),
    ] {
        let times: Vec<String> = (0..6)
            .map(|_| {
                let started = Instant::now();
                assert!(ours(&question).is_ok(), "{name}");
                format!("{:.0}", started.elapsed().as_secs_f64() * 1e3)
            })
            .collect();
        println!("{name}: cold {} ms, then {} ms", times[0], times[1..].join(", "));
    }
}

/// Reports what a question answers while a load writes the same store; it
/// passes whatever the answer.
#[test]
fn real_reader_during_a_load_report_only() {
    let Some(tools) = real_tools("real_reader_during_a_load_report_only") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let data = loaded(&tools, dir.path());
    let (runner, found) = (Runner::default(), Ok(tools.clone()));
    for attempt in 1..=3 {
        let mut load = Command::new(tools.path(Piece::Load))
            .args([OsString::from("--run"), SAMPLE_RUN.into(), "--data-dir".into(), data.path().into()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let got = ask(&runner, &found, &data, &Question::Kinds);
        let took = started.elapsed();
        let still_loading = load.try_wait().unwrap().is_none();
        let status = load.wait().unwrap();
        let what = match &got {
            Ok(answer) => format!("an answer of {} bytes", answer.as_str().len()),
            Err(reason) => serde_json::to_string(reason).unwrap(),
        };
        println!(
            "during a load, try {attempt}: {what} in {:.0} ms; load still running when it came: {still_loading}; load {status}",
            took.as_secs_f64() * 1e3
        );
    }
}
