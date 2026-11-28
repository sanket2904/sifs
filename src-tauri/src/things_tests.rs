use super::*;
use crate::data::{data_dir, Build};
use crate::index::db::Index;
use crate::tools::{Piece, Runner};
use rusqlite::params;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Barrier, Condvar, Mutex};
use std::thread;

/// How long a test waits for anything before it fails.
const BOUND: Duration = Duration::from_secs(60);
/// The time the made-up clock gives as now: 2026-11-27 12:00 UTC, the same
/// calendar day in every time zone from UTC-12 to UTC+11.
const WALL: i64 = 1_795_780_800;
const TODAY: &str = "2026-11-27";

/// The stand-in for both programs. It writes down each run (its arguments,
/// its pid, what the work folder held) and does what the files beside it
/// say: hold until released (giving up after 30 seconds), make the index
/// busy, print, say, exit with a code or end on a signal.
const STAND_IN: &str = r#"#!/bin/sh
me="$0"
name="${me##*/}"
dir="${me%/*}"
n=$(( $(cat "$me.count" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$me.count"
echo "$name" >> "$dir/log"
: > "$me.args.$n"
for a in "$@"; do printf '%s\n' "$a" >> "$me.args.$n"; done
[ -e "$dir/busy-in-$name" ] && : > "$dir/busy"
if [ -e "$me.hold" ]; then
  /bin/sh -c 'echo $$ > "$1.child"; i=0; while [ ! -e "$1.release" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done' sh "$me" &
  echo $$ > "$me.pid"
  wait
else
  echo $$ > "$me.pid"
fi
case "$name" in
m1a-extract)
  while [ $# -gt 0 ]; do case "$1" in -text) text="$2";; -out) out="$2";; esac; shift; done
  cp "$text/index.json" "$me.index.$n"
  ls -A "${out%/*}" > "$me.work.$n"
  if [ -e "$me.run" ]; then cp "$me.run" "$out"; else echo '{"documents":[]}' > "$out"; fi
  ;;
sifs-load)
  while [ $# -gt 0 ]; do case "$1" in --run) run="$2";; esac; shift; done
  ls -A "${run%/*}" > "$me.work.$n"
  if [ -e "$me.print" ]; then cat "$me.print"; else echo '{"documents":{"written":1},"turned_down":[]}'; fi
  ;;
esac
[ -e "$me.say" ] && cat "$me.say" >&2
[ -e "$me.signal" ] && kill -TERM $$
exit $(cat "$me.exit" 2>/dev/null || echo 0)
"#;

struct FakeClock {
    now: Mutex<Instant>,
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }

    fn wall(&self) -> i64 {
        WALL
    }
}

/// Every state the machine published, in order.
#[derive(Default)]
struct Seen {
    states: Mutex<Vec<State>>,
    more: Condvar,
}

impl Seen {
    fn push(&self, state: &State) {
        self.states.lock().unwrap().push(state.clone());
        self.more.notify_all();
    }

    fn all(&self) -> Vec<State> {
        self.states.lock().unwrap().clone()
    }

    /// Waits until the states seen so far satisfy done.
    fn wait_for(&self, what: &str, done: impl Fn(&[State]) -> bool) -> Vec<State> {
        let states = self.states.lock().unwrap();
        let (states, timeout) = self
            .more
            .wait_timeout_while(states, BOUND, |s| !done(s))
            .unwrap();
        assert!(!timeout.timed_out(), "{what}: seen {:?}", *states);
        states.clone()
    }
}

/// A data folder holding a made-up index, a tools folder of stand-ins, the
/// made-up clock, an index the test makes busy or idle, and the runner.
/// Both folders' paths have spaces, as the real data folder's has.
struct Scratch {
    dir: tempfile::TempDir,
    data: DataDir,
    tools: PathBuf,
    clock: Arc<FakeClock>,
    busy: Arc<AtomicBool>,
    /// Set, the index goes busy as soon as the export has written its text.
    busy_after_export: Arc<AtomicBool>,
    /// Set, the runner is stopped as soon as the export has written its text.
    stop_after_export: Arc<AtomicBool>,
    runner: Arc<Runner>,
    seen: Arc<Seen>,
    limits: Limits,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let data_path = dir.path().join("Application Support").join("sifs data");
        fs::create_dir_all(&data_path).unwrap();
        let data = data_dir(Build::Development, &data_path, Some(data_path.as_os_str())).unwrap();
        drop(Index::open(&data.index()).unwrap());
        // A reader takes text files, as the app registers its readers: a
        // changed file is queued again and keeps what was read from it.
        rusqlite::Connection::open(data.index())
            .unwrap()
            .execute("INSERT INTO extract_kinds (ext, reader, version, priority) VALUES ('txt', 'text', 1, 0)", [])
            .unwrap();
        let tools = dir.path().join("tool box");
        fs::create_dir_all(&tools).unwrap();
        for piece in [Piece::Extract, Piece::Load, Piece::Show] {
            write_program(&tools.join(piece.name()), STAND_IN);
        }
        fs::write(tools.join(Piece::Banks.name()), "{\"banks\":[]}").unwrap();
        fs::write(tools.join(Piece::IbanRegistry.name()), "{}").unwrap();
        Scratch {
            dir,
            data,
            tools,
            clock: Arc::new(FakeClock {
                now: Mutex::new(Instant::now()),
            }),
            busy: Arc::default(),
            busy_after_export: Arc::default(),
            stop_after_export: Arc::default(),
            runner: Arc::default(),
            seen: Arc::default(),
            limits: Limits {
                extract: BOUND,
                load: BOUND,
            },
        }
    }

    fn setup(&self) -> Setup {
        let runner = self.runner.clone();
        let (busy, after, stop) = (
            self.busy.clone(),
            self.busy_after_export.clone(),
            self.stop_after_export.clone(),
        );
        let (exported, busy_file, stopper) = (
            self.work().join("text").join("index.json"),
            self.tools.join("busy"),
            self.runner.clone(),
        );
        let seen = self.seen.clone();
        Setup {
            data: self.data.clone(),
            tools: Tools::find(&self.tools),
            run: Arc::new(move |program, args, time| runner.run(program, args, time)),
            idle: Arc::new(move || {
                if exported.exists() && after.swap(false, Ordering::SeqCst) {
                    busy.store(true, Ordering::SeqCst);
                }
                if exported.exists() && stop.swap(false, Ordering::SeqCst) {
                    stopper.stop();
                }
                !busy.load(Ordering::SeqCst) && !busy_file.exists()
            }),
            clock: self.clock.clone(),
            quiet: QUIET,
            limits: self.limits,
            on_change: Arc::new(move |state| seen.push(state)),
        }
    }

    /// A machine with no thread of its own: the test ticks it.
    fn machine(&self) -> Things {
        Things::new(self.setup())
    }

    fn work(&self) -> PathBuf {
        self.data.path().join(WORK_FOLDER)
    }

    fn fingerprint_file(&self) -> PathBuf {
        self.data.path().join(FINGERPRINT_FILE)
    }

    fn tool(&self, piece: Piece) -> PathBuf {
        self.tools.join(piece.name())
    }

    /// A file beside a stand-in that tells it what to do.
    fn tell(&self, piece: Piece, what: &str, content: &str) {
        fs::write(self.tools.join(format!("{}.{what}", piece.name())), content).unwrap();
    }

    fn untell(&self, piece: Piece, what: &str) {
        let _ = fs::remove_file(self.tools.join(format!("{}.{what}", piece.name())));
    }

    fn noted(&self, piece: Piece, what: &str) -> Option<String> {
        fs::read_to_string(self.tools.join(format!("{}.{what}", piece.name()))).ok()
    }

    /// How many times the stand-in ran.
    fn runs(&self, piece: Piece) -> usize {
        self.noted(piece, "count")
            .map_or(0, |n| n.trim().parse().unwrap())
    }

    /// Moves the made-up clock on and lets a machine look.
    fn pass(&self, things: &Things, time: Duration) {
        *self.clock.now.lock().unwrap() += time;
        things.tick();
    }

    fn idle(&self, idle: bool) {
        self.busy.store(!idle, Ordering::SeqCst);
        let _ = fs::remove_file(self.tools.join("busy"));
    }

    fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data.index()).unwrap()
    }

    /// Records files as read inside, as the worker would have written them.
    fn add(&self, paths: &[String]) {
        let mut conn = self.conn();
        let tx = conn.transaction().unwrap();
        for path in paths {
            let name = path.rsplit('/').next().unwrap();
            tx.execute(
                "INSERT INTO files (path, root, name, ext, is_dir, size, modified, inode) VALUES (?1, '/r', ?2, 'txt', 0, 1, 1700000000, 1)",
                params![path, name],
            )
            .unwrap();
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO extracts (file_id, reader, version, modified, size, status, millis, read_at) VALUES (?1, 'text', 1, 1700000000, 1, 'done', 0, 0)",
                params![id],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO contents (file_id, part, label, text) VALUES (?1, 0, '', ?2)",
                params![id, format!("text of {name}")],
            )
            .unwrap();
        }
        // And read them.
        tx.execute("DELETE FROM extract_queue", []).unwrap();
        tx.commit().unwrap();
    }

    fn add_one(&self, path: &str) {
        self.add(&[path.to_string()]);
    }

    /// A file that changed and was read again.
    fn touch(&self, path: &str, modified: i64) {
        let conn = self.conn();
        let changed = conn
            .execute(
                "UPDATE files SET modified = ?2 WHERE path = ?1",
                params![path, modified],
            )
            .unwrap();
        assert_eq!(changed, 1);
        assert_eq!(
            conn.execute("DELETE FROM extract_queue", []).unwrap(),
            1,
            "it was queued to be read again"
        );
        let listed = textset::listed(&conn).unwrap();
        assert!(
            listed
                .iter()
                .any(|f| f.path == path && f.modified == modified),
            "and is listed as it is now"
        );
    }

    fn remove(&self, path: &str) {
        assert_eq!(
            self.conn()
                .execute("DELETE FROM files WHERE path = ?1", params![path])
                .unwrap(),
            1
        );
    }

    fn remove_all(&self) {
        self.conn().execute("DELETE FROM files", []).unwrap();
    }

    /// The document ids the extractor's text set held on its nth run.
    fn exported(&self, n: usize) -> Vec<String> {
        let index: serde_json::Value =
            serde_json::from_str(&self.noted(Piece::Extract, &format!("index.{n}")).unwrap())
                .unwrap();
        index["documents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap().to_string())
            .collect()
    }

    /// A finished reading: the index changed, the quiet time passed.
    fn read_once(&self, things: &Things) -> State {
        things.changed();
        self.pass(things, QUIET);
        things.state()
    }
}

fn write_program(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn paths(from: usize, n: usize) -> Vec<String> {
    (from..from + n)
        .map(|i| format!("/r/folder one/file {i:05}.txt"))
        .collect()
}

fn is_read(state: &State) -> bool {
    matches!(state, State::Read { .. })
}

/// Ticks a machine on another thread; the receiver hears when it is done.
fn tick_aside(things: &Things) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    let things = things.clone();
    thread::spawn(move || {
        things.tick();
        let _ = tx.send(());
    });
    rx
}

/// Waits for a stand-in's run to be under way, holding: its pid and its
/// child's.
fn holding(scratch: &Scratch, piece: Piece) -> (libc::pid_t, libc::pid_t) {
    let pid = |what: &str| {
        scratch
            .noted(piece, what)
            .and_then(|p| p.trim().parse().ok())
    };
    let until = Instant::now() + BOUND;
    loop {
        if let (Some(me), Some(child)) = (pid("pid"), pid("child")) {
            return (me, child);
        }
        assert!(Instant::now() < until, "{} started and held", piece.name());
        thread::yield_now();
    }
}

fn release(scratch: &Scratch, piece: Piece) {
    scratch.untell(piece, "hold");
    scratch.tell(piece, "release", "");
}

// --- R2: a half-read index never empties the store ---

#[test]
fn r2_no_reading_starts_while_the_index_is_busy() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.idle(false);
    things.changed();
    for _ in 0..50 {
        scratch.pass(&things, QUIET * 3);
    }
    assert_eq!(scratch.runs(Piece::Extract), 0, "nothing ran");
    assert!(!scratch.work().exists(), "no work folder");
    assert_eq!(things.state(), State::Waiting { owed: false });
    scratch.idle(true);
    scratch.pass(&things, QUIET);
    assert!(is_read(&things.state()), "{:?}", things.state());
    assert_eq!(scratch.runs(Piece::Load), 1, "once idle, one reading");
}

#[test]
fn r2_busy_during_the_export_drops_the_reading_and_owes_one() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.busy_after_export.store(true, Ordering::SeqCst);
    things.changed();
    scratch.pass(&things, QUIET);
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (0, 0),
        "neither tool ran"
    );
    assert_eq!(things.state(), State::Waiting { owed: true });
    assert!(!scratch.work().exists(), "the work folder is gone");
    assert!(
        !scratch.fingerprint_file().exists(),
        "nothing was recorded as read"
    );
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 0, "still busy: still nothing");
    scratch.idle(true);
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 1, "idle again: one reading");
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 1, "and only one");
    assert!(is_read(&things.state()), "{:?}", things.state());
}

#[test]
fn r2_busy_during_the_extract_drops_the_reading_before_the_load() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    // The stand-in extractor makes the index busy while it runs.
    fs::write(scratch.tools.join("busy-in-m1a-extract"), "").unwrap();
    things.changed();
    scratch.pass(&things, QUIET);
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (1, 0),
        "extracted, never loaded"
    );
    assert_eq!(things.state(), State::Waiting { owed: true });
    assert!(!scratch.work().exists());
    assert!(!scratch.fingerprint_file().exists());
    fs::remove_file(scratch.tools.join("busy-in-m1a-extract")).unwrap();
    scratch.idle(true);
    scratch.pass(&things, QUIET);
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (2, 1),
        "idle again: one reading"
    );
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 1);
}

#[test]
fn r2_a_change_noted_during_the_extract_drops_the_reading_even_if_idle_again() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.tell(Piece::Extract, "hold", "");
    things.changed();
    *scratch.clock.now.lock().unwrap() += QUIET;
    let ticked = tick_aside(&things);
    holding(&scratch, Piece::Extract);
    // A scan ran and ended, and its files were read, all during the extract.
    things.changed();
    release(&scratch, Piece::Extract);
    ticked.recv_timeout(BOUND).unwrap();
    assert_eq!(scratch.runs(Piece::Load), 0, "not loaded");
    assert_eq!(things.state(), State::Waiting { owed: true });
}

#[test]
fn r2_the_index_changed_during_the_extract_without_a_word_drops_the_reading() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.tell(Piece::Extract, "hold", "");
    things.changed();
    *scratch.clock.now.lock().unwrap() += QUIET;
    let ticked = tick_aside(&things);
    holding(&scratch, Piece::Extract);
    scratch.add(&paths(3, 1));
    release(&scratch, Piece::Extract);
    ticked.recv_timeout(BOUND).unwrap();
    assert_eq!(
        scratch.runs(Piece::Load),
        0,
        "what was exported is no longer the index: not loaded"
    );
    assert_eq!(things.state(), State::Waiting { owed: true });
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 1);
    assert_eq!(scratch.exported(2).len(), 4);
}

#[test]
fn r2_busy_after_the_load_began_lets_the_load_finish_and_owes_one() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.tell(Piece::Load, "hold", "");
    things.changed();
    *scratch.clock.now.lock().unwrap() += QUIET;
    let ticked = tick_aside(&things);
    holding(&scratch, Piece::Load);
    scratch.idle(false);
    scratch.add(&paths(3, 1));
    things.changed();
    release(&scratch, Piece::Load);
    ticked.recv_timeout(BOUND).unwrap();
    assert_eq!(scratch.runs(Piece::Load), 1, "the load finished");
    assert!(scratch.fingerprint_file().exists(), "and was recorded");
    assert!(
        scratch.seen.all().contains(&State::Reading {
            step: Step::Load,
            owed: true
        }),
        "{:?}",
        scratch.seen.all()
    );
    assert_eq!(things.state(), State::Waiting { owed: false });
    scratch.pass(&things, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 1, "busy: nothing more");
    scratch.idle(true);
    scratch.pass(&things, QUIET);
    assert_eq!(
        scratch.runs(Piece::Load),
        2,
        "idle: the file added during the load is read"
    );
    assert_eq!(scratch.exported(2).len(), 4);
}

#[test]
fn r2_a_first_scan_in_bursts_reads_once_after_the_last_burst() {
    let scratch = Scratch::new();
    let things = scratch.machine();
    for burst in 0..10 {
        scratch.idle(false);
        things.changed();
        scratch.add(&paths(burst * 1000, 1000));
        scratch.pass(&things, QUIET / 2);
        scratch.idle(true);
        // The reader of files ran out of work, and the next burst comes
        // before the quiet time is over.
        things.changed();
        scratch.pass(&things, QUIET - Duration::from_secs(1));
        assert_eq!(
            scratch.runs(Piece::Extract),
            0,
            "nothing read after burst {burst}"
        );
    }
    scratch.pass(&things, Duration::from_secs(1));
    assert_eq!(
        scratch.runs(Piece::Extract),
        1,
        "one reading after the last burst"
    );
    assert_eq!(scratch.exported(1).len(), 10_000);
    assert!(
        matches!(
            things.state(),
            State::Read {
                documents: 10_000,
                ..
            }
        ),
        "{:?}",
        things.state()
    );
}

#[test]
fn r2_an_index_emptied_while_idle_empties_the_store_and_not_while_busy() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 40));
    let things = scratch.machine();
    assert!(is_read(&scratch.read_once(&things)));
    assert_eq!(scratch.exported(1).len(), 40);
    // The person removed their folders. Not idle yet: nothing.
    scratch.remove_all();
    scratch.idle(false);
    scratch.read_once(&things);
    assert_eq!(scratch.runs(Piece::Load), 1, "busy: the store keeps its 40");
    // Idle: what the store holds follows the files, down to nothing.
    scratch.idle(true);
    let state = scratch.read_once(&things);
    assert_eq!(scratch.runs(Piece::Load), 2);
    assert_eq!(scratch.exported(2).len(), 0);
    assert!(
        matches!(state, State::Read { documents: 0, .. }),
        "{state:?}"
    );
}

// --- R3: one at a time, and only when something changed ---

#[test]
fn r3_fifty_requests_during_a_reading_owe_exactly_one_more() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    let driver = things.drive().expect("a thread");
    scratch.tell(Piece::Extract, "hold", "");
    things.read_again();
    holding(&scratch, Piece::Extract);
    let all = Arc::new(Barrier::new(50));
    let askers: Vec<_> = (0..50)
        .map(|_| {
            let (things, all) = (things.clone(), all.clone());
            thread::spawn(move || {
                all.wait();
                things.read_again();
            })
        })
        .collect();
    for asker in askers {
        asker.join().unwrap();
    }
    assert_eq!(scratch.runs(Piece::Extract), 1, "one reading runs");
    assert_eq!(
        things.state(),
        State::Reading {
            step: Step::Extract,
            owed: true
        }
    );
    release(&scratch, Piece::Extract);
    let states = scratch.seen.wait_for("a reading, then one more", |s| {
        s.iter().filter(|s| is_read(s)).count() == 1
    });
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (2, 2),
        "{states:?}"
    );
    assert_eq!(
        states
            .iter()
            .filter(|s| matches!(
                s,
                State::Reading {
                    step: Step::Export,
                    ..
                }
            ))
            .count(),
        2,
        "{states:?}"
    );
    things.stop();
    join(driver);
    assert_eq!(scratch.runs(Piece::Load), 2, "and no more");
}

#[test]
fn r3_an_unchanged_index_and_tools_read_nothing_also_after_a_restart() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    assert!(is_read(&scratch.read_once(&things)));
    let read = things.state();
    scratch.read_once(&things);
    scratch.read_once(&things);
    assert_eq!(
        scratch.runs(Piece::Load),
        1,
        "nothing changed: nothing read"
    );
    assert_eq!(things.state(), read);
    let restarted = scratch.machine();
    scratch.pass(&restarted, QUIET);
    scratch.read_once(&restarted);
    assert_eq!(scratch.runs(Piece::Load), 1, "nor after a restart");
    assert_eq!(restarted.state(), State::Idle);
}

#[test]
fn r3_each_change_to_the_index_or_the_tools_reads_once() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.read_once(&things);
    let mut loads = 1;
    let first = paths(0, 1).remove(0);
    type Change<'a> = (&'a str, Box<dyn Fn() + 'a>);
    let changes: [Change; 6] = [
        (
            "one file's modified time",
            Box::new(|| scratch.touch(&first, 1_700_000_001)),
        ),
        ("one file added", Box::new(|| scratch.add(&paths(3, 1)))),
        (
            "one file removed",
            Box::new(|| scratch.remove(&paths(1, 1)[0])),
        ),
        (
            "the bank directory replaced",
            Box::new(|| fs::write(scratch.tool(Piece::Banks), "{\"banks\":[1]}").unwrap()),
        ),
        // Same size: the modified time tells it apart.
        (
            "the registry rewritten",
            Box::new(|| {
                let path = scratch.tool(Piece::IbanRegistry);
                fs::write(&path, "{}").unwrap();
                let later =
                    fs::metadata(&path).unwrap().modified().unwrap() + Duration::from_secs(5);
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(later)
                    .unwrap();
            }),
        ),
        (
            "the loader rebuilt",
            Box::new(|| {
                write_program(
                    &scratch.tool(Piece::Load),
                    &format!("{STAND_IN}\n# rebuilt\n"),
                )
            }),
        ),
    ];
    for (change, make) in changes {
        make();
        scratch.read_once(&things);
        loads += 1;
        assert_eq!(scratch.runs(Piece::Load), loads, "{change}: one reading");
        scratch.read_once(&things);
        assert_eq!(scratch.runs(Piece::Load), loads, "{change}: and no second");
    }
}

#[test]
fn r3_the_fingerprint_is_written_only_after_the_load_finished() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.read_once(&things);
    let recorded = fs::read(scratch.fingerprint_file()).expect("recorded after the first load");
    scratch.add(&paths(3, 1));
    scratch.tell(Piece::Load, "hold", "");
    things.changed();
    *scratch.clock.now.lock().unwrap() += QUIET;
    let ticked = tick_aside(&things);
    let (load, _) = holding(&scratch, Piece::Load);
    assert_eq!(
        fs::read(scratch.fingerprint_file()).unwrap(),
        recorded,
        "unchanged while the load runs"
    );
    // SAFETY: kill has no memory effects; the pid is the stand-in this test started.
    assert_eq!(unsafe { libc::kill(load, libc::SIGKILL) }, 0);
    ticked.recv_timeout(BOUND).unwrap();
    assert!(
        matches!(
            things.state(),
            State::Failed {
                step: Step::Load,
                ..
            }
        ),
        "{:?}",
        things.state()
    );
    assert_eq!(
        fs::read(scratch.fingerprint_file()).unwrap(),
        recorded,
        "unchanged after the killed load"
    );
    scratch.untell(Piece::Load, "hold");
    let restarted = scratch.machine();
    scratch.pass(&restarted, QUIET);
    assert_eq!(scratch.runs(Piece::Load), 3, "the next start reads again");
    assert!(is_read(&restarted.state()));
    assert_ne!(fs::read(scratch.fingerprint_file()).unwrap(), recorded);
}

#[test]
fn r3_a_fingerprint_file_that_cannot_be_read_counts_as_never_read() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    scratch.read_once(&scratch.machine());
    let good = fs::read_to_string(scratch.fingerprint_file()).unwrap();
    let future = good.replacen(
        &format!("\"format\":{FINGERPRINT_FORMAT}"),
        "\"format\":99",
        1,
    );
    assert_ne!(future, good, "{good}");
    let mut loads = 1;
    for (what, content) in [
        ("empty", String::new()),
        ("garbage", "{\"format\":".into()),
        ("from a future version", future),
    ] {
        fs::write(scratch.fingerprint_file(), &content).unwrap();
        let things = scratch.machine();
        scratch.pass(&things, QUIET);
        loads += 1;
        assert_eq!(scratch.runs(Piece::Load), loads, "{what}: read again");
        assert!(is_read(&things.state()), "{what}: {:?}", things.state());
        assert_eq!(
            fs::read_to_string(scratch.fingerprint_file()).unwrap(),
            good,
            "{what}: rewritten"
        );
    }
}

#[test]
fn r3_read_again_reads_an_unchanged_index() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let things = scratch.machine();
    scratch.read_once(&things);
    things.read_again();
    things.tick();
    assert_eq!(
        scratch.runs(Piece::Load),
        2,
        "asked: read, fingerprint or not"
    );
    scratch.read_once(&things);
    assert_eq!(scratch.runs(Piece::Load), 2, "and once only");
}

/// Joins a thread within BOUND, or fails the test.
fn join(thread: JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || tx.send(thread.join().is_ok()));
    assert_eq!(rx.recv_timeout(BOUND), Ok(true), "the thread ended");
}

/// Each line of a stand-in's arguments, as it wrote them down.
fn lines(args: &[&dyn AsRef<Path>]) -> String {
    args.iter()
        .map(|a| format!("{}\n", a.as_ref().display()))
        .collect()
}

/// Whether the system has no live process with this pid (a zombie is dead).
fn gone(pid: libc::pid_t) -> bool {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    stat.trim().is_empty() || stat.trim().starts_with('Z')
}

/// Waits for every pid to be gone, or fails the test.
fn all_gone(pids: &[libc::pid_t]) {
    let until = Instant::now() + BOUND;
    while !pids.iter().all(|&pid| gone(pid)) {
        assert!(
            Instant::now() < until,
            "still alive: {:?}",
            pids.iter().filter(|&&p| !gone(p)).collect::<Vec<_>>()
        );
        thread::yield_now();
    }
}

/// Waits until no live process names the scratch folder on its command
/// line (every stand-in and its child do), or fails the test.
fn nothing_left(scratch: &Scratch) {
    let marker = scratch.dir.path().to_str().unwrap().to_string();
    let marked = || {
        let out = std::process::Command::new("/bin/ps")
            .args(["-axo", "stat=,command="])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.contains(&marker) && !line.trim_start().starts_with('Z'))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let until = Instant::now() + BOUND;
    while !marked().is_empty() {
        assert!(Instant::now() < until, "still alive: {:?}", marked());
        thread::yield_now();
    }
}

// --- R1: the steps ---

#[test]
fn r1_a_reading_runs_export_extract_and_load_in_order_with_their_arguments() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 2));
    scratch.add_one("/r/folder two/notes.txt");
    let things = scratch.machine();
    assert!(!scratch.work().exists(), "no work folder before");
    let state = scratch.read_once(&things);
    assert_eq!(
        state,
        State::Read {
            when: WALL,
            documents: 3,
            turned_down: 0
        }
    );
    assert_eq!(
        fs::read_to_string(scratch.tools.join("log")).unwrap(),
        "m1a-extract\nsifs-load\n",
        "extract, then load"
    );
    let (work, data) = (scratch.work(), scratch.data.path().to_path_buf());
    assert!(data.to_str().unwrap().contains(' ') && scratch.tools.to_str().unwrap().contains(' '));
    let (text, run) = (work.join("text"), work.join("run.json"));
    let (banks, registry) = (
        scratch.tool(Piece::Banks),
        scratch.tool(Piece::IbanRegistry),
    );
    assert_eq!(
        scratch.noted(Piece::Extract, "args.1").unwrap(),
        lines(&[
            &"-text",
            &text,
            &"-directory",
            &banks,
            &"-iban-registry",
            &registry,
            &"-out",
            &run
        ])
    );
    assert_eq!(
        scratch.noted(Piece::Load, "args.1").unwrap(),
        lines(&[&"--run", &run, &"--data-dir", &data])
    );
    assert_eq!(
        scratch.noted(Piece::Extract, "work.1").unwrap(),
        "text\n",
        "the extractor's work folder held the text set"
    );
    assert_eq!(
        scratch.noted(Piece::Load, "work.1").unwrap(),
        "run.json\ntext\n",
        "the loader's held the text set and the run"
    );
    let index: serde_json::Value =
        serde_json::from_str(&scratch.noted(Piece::Extract, "index.1").unwrap()).unwrap();
    assert_eq!(index["reference_date"], TODAY);
    assert_eq!(scratch.exported(1).len(), 3);
    assert!(!work.exists(), "the work folder is gone after");
    assert!(scratch.fingerprint_file().exists());
}

#[test]
fn r1_exit_1_of_the_load_is_read_with_the_turned_down_count() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    scratch.tell(
        Piece::Load,
        "print",
        "{\"documents\":{\"written\":1},\"turned_down\":[{\"record\":\"a\"},{\"record\":\"b\"}]}\n",
    );
    scratch.tell(Piece::Load, "exit", "1");
    let things = scratch.machine();
    assert_eq!(
        scratch.read_once(&things),
        State::Read {
            when: WALL,
            documents: 3,
            turned_down: 2
        }
    );
    assert!(
        scratch.fingerprint_file().exists(),
        "the others loaded: read"
    );
}

#[test]
fn r1_a_load_that_does_not_finish_fails_at_the_load_with_what_was_said() {
    for (case, said) in [
        (
            "exit 2",
            "exited with code 2: sifs-load: the store is locked",
        ),
        (
            "a signal",
            "ended on signal 15: sifs-load: the store is locked",
        ),
        (
            "no output line",
            "the loader printed no line saying what it loaded",
        ),
        (
            "a line with no count",
            "the loader printed no line saying what it loaded",
        ),
        ("too long", "took too long"),
    ] {
        let mut scratch = Scratch::new();
        scratch.add(&paths(0, 3));
        scratch.tell(Piece::Load, "say", "sifs-load: the store is locked\n");
        match case {
            "exit 2" => scratch.tell(Piece::Load, "exit", "2"),
            "a signal" => scratch.tell(Piece::Load, "signal", ""),
            "no output line" => scratch.tell(Piece::Load, "print", ""),
            "a line with no count" => scratch.tell(Piece::Load, "print", "{\"documents\":{}}\n"),
            _ => {
                scratch.limits.load = Duration::from_millis(300);
                scratch.tell(Piece::Load, "hold", "");
            }
        }
        let things = scratch.machine();
        let state = scratch.read_once(&things);
        assert_eq!(
            state,
            State::Failed {
                step: Step::Load,
                said: said.into()
            },
            "{case}"
        );
        assert!(!scratch.work().exists(), "{case}: the work folder is gone");
        assert!(
            !scratch.fingerprint_file().exists(),
            "{case}: nothing recorded as read"
        );
        if case == "too long" {
            // Ended at the limit, maybe before it wrote its pid down.
            nothing_left(&scratch);
        }
    }
}

#[test]
fn r1_a_failed_export_or_extract_never_starts_the_load() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    // A PDF whose part is not labelled as a page cannot be exported.
    let conn = scratch.conn();
    conn.execute(
        "UPDATE extracts SET reader = 'pdf' WHERE file_id = (SELECT MIN(id) FROM files)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE contents SET label = 'cover' WHERE file_id = (SELECT MIN(id) FROM files)",
        [],
    )
    .unwrap();
    let things = scratch.machine();
    let state = scratch.read_once(&things);
    assert!(
        matches!(&state, State::Failed { step: Step::Export, said } if said.contains("not \"page N\"")),
        "{state:?}"
    );
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (0, 0)
    );
    assert!(!scratch.work().exists());

    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    scratch.tell(Piece::Extract, "exit", "2");
    scratch.tell(
        Piece::Extract,
        "say",
        "m1a-extract: index.json: bad format\n",
    );
    let things = scratch.machine();
    let state = scratch.read_once(&things);
    assert_eq!(
        state,
        State::Failed {
            step: Step::Extract,
            said: "exited with code 2: m1a-extract: index.json: bad format".into()
        }
    );
    assert_eq!(
        (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
        (1, 0)
    );
    assert!(!scratch.work().exists());
    assert!(!scratch.fingerprint_file().exists());
}

// --- R4: quitting ---

#[test]
fn r4_quitting_during_each_step_ends_the_reading_and_its_thread() {
    for step in [Step::Export, Step::Extract, Step::Load] {
        let scratch = Scratch::new();
        scratch.add(&paths(0, 3));
        let things = scratch.machine();
        scratch.read_once(&things);
        let recorded = fs::read(scratch.fingerprint_file()).unwrap();
        scratch.add(&paths(3, 1));
        let driver = things.drive().expect("a thread");
        let holder = match step {
            Step::Export => None,
            Step::Extract => Some(Piece::Extract),
            Step::Load => Some(Piece::Load),
        };
        match holder {
            None => scratch.stop_after_export.store(true, Ordering::SeqCst),
            Some(piece) => scratch.tell(piece, "hold", ""),
        }
        things.read_again();
        let pids = holder.map(|piece| holding(&scratch, piece));
        if pids.is_some() {
            scratch.runner.stop();
        }
        join(driver);
        let state = things.state();
        assert!(matches!(state, State::Failed { .. }), "{step:?}: {state:?}");
        if let Some((tool, child)) = pids {
            all_gone(&[tool, child]);
        }
        nothing_left(&scratch);
        assert_eq!(
            fs::read(scratch.fingerprint_file()).unwrap(),
            recorded,
            "{step:?}: the fingerprint is unchanged"
        );
        assert!(!scratch.work().exists(), "{step:?}");
        let runs = (scratch.runs(Piece::Extract), scratch.runs(Piece::Load));
        things.read_again();
        scratch.pass(&things, QUIET);
        assert_eq!(
            (scratch.runs(Piece::Extract), scratch.runs(Piece::Load)),
            runs,
            "{step:?}: nothing more starts"
        );
        let expected = match step {
            Step::Export => (1, 1),
            Step::Extract => (2, 1),
            Step::Load => (2, 2),
        };
        assert_eq!(runs, expected, "{step:?}: no step after the one stopped");
    }
}

#[test]
fn r4_a_work_folder_left_by_a_killed_app_is_cleared_by_the_next_reading() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    fs::create_dir_all(scratch.work().join("text")).unwrap();
    fs::write(scratch.work().join("text").join("old-0000.json"), "{}").unwrap();
    fs::write(scratch.work().join("run.json"), "{}").unwrap();
    let things = scratch.machine();
    assert!(is_read(&scratch.read_once(&things)), "{:?}", things.state());
    assert_eq!(
        scratch.noted(Piece::Extract, "work.1").unwrap(),
        "text\n",
        "no old run beside the text set"
    );
    assert_eq!(scratch.exported(1).len(), 3);
    assert!(!scratch.work().exists());
}

// --- R5: what the app is given ---

#[test]
fn r5_with_the_tools_missing_nothing_runs_and_no_work_folder_is_made() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 3));
    let missing = scratch.dir.path().join("no tools");
    let why = Tools::find(&missing).unwrap_err();
    let things = Things::new(Setup {
        tools: Err(why.clone()),
        ..scratch.setup()
    });
    assert!(things.drive().is_none(), "no thread");
    let state = State::NoTools {
        said: why.to_string(),
    };
    assert_eq!(things.state(), state);
    things.changed();
    things.read_again();
    scratch.pass(&things, QUIET * 2);
    assert_eq!(things.state(), state);
    assert!(!scratch.work().exists() && !scratch.fingerprint_file().exists());
    assert!(scratch.seen.all().is_empty(), "nothing to tell");
}

#[test]
fn r5_the_state_goes_to_the_screen_tagged() {
    let shown = |state: State| serde_json::to_value(state).unwrap();
    assert_eq!(
        shown(State::Reading {
            step: Step::Load,
            owed: true
        }),
        serde_json::json!({"state": "reading", "step": "load", "owed": true})
    );
    assert_eq!(
        shown(State::Read {
            when: WALL,
            documents: 3,
            turned_down: 1
        }),
        serde_json::json!({"state": "read", "when": WALL, "documents": 3, "turned_down": 1})
    );
    assert_eq!(
        shown(State::Failed {
            step: Step::Extract,
            said: "x".into()
        }),
        serde_json::json!({"state": "failed", "step": "extract", "said": "x"})
    );
    assert_eq!(
        shown(State::Waiting { owed: false }),
        serde_json::json!({"state": "waiting", "owed": false})
    );
    assert_eq!(
        shown(State::NoTools { said: "x".into() }),
        serde_json::json!({"state": "no_tools", "said": "x"})
    );
    assert_eq!(shown(State::Idle), serde_json::json!({"state": "idle"}));
}

#[test]
fn r5_nothing_waiting_only_when_the_queue_is_empty() {
    let scratch = Scratch::new();
    scratch.add(&paths(0, 1));
    assert!(nothing_waiting(&scratch.data.index()));
    scratch.conn().execute("INSERT INTO extract_queue (file_id, priority, modified, size, attempts) SELECT id, 0, 0, 0, 0 FROM files", []).unwrap();
    assert!(!nothing_waiting(&scratch.data.index()));
    assert!(
        !nothing_waiting(&scratch.dir.path().join("no index.db")),
        "no index: not idle"
    );
}

// A source guard: it reads app.rs as text. What the machine does is proven
// by the tests above; this catches the wiring coming undone.
#[test]
fn source_guard_the_app_feeds_the_machine_and_registers_its_two_commands() {
    let app = include_str!("app.rs");
    let start = app
        .find("tauri::generate_handler![")
        .expect("one handler list");
    let handler = &app[start..start + app[start..].find(']').unwrap()];
    let registered: Vec<&str> = handler
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    for (command, signature) in [
        (
            "things_state",
            "fn things_state(things: State<Things>) -> things::State {\n    things.state()\n}",
        ),
        (
            "read_again",
            "fn read_again(things: State<Things>) {\n    things.read_again();\n}",
        ),
    ] {
        assert_eq!(
            registered.iter().filter(|&&w| w == command).count(),
            1,
            "{command} is registered once"
        );
        assert_eq!(
            app.matches(&format!("#[tauri::command]\n{signature}"))
                .count(),
            1,
            "{command}: {signature}"
        );
    }
    assert_eq!(app.matches("Things::start(").count(), 1, "one machine");
    assert_eq!(
        app.matches("tools: app.state::<Result<Tools, NotFound>>().inner().clone(),")
            .count(),
        1,
        "given the tools looked for"
    );
    assert_eq!(
        app.matches("runner.state::<Runner>().run(program, args, time)")
            .count(),
        1,
        "the tools run through the app's runner"
    );
    assert_eq!(
        app.matches("idle: Arc::new(move || scans.idle() && things::nothing_waiting(&queue)),")
            .count(),
        1
    );
    assert_eq!(
        app.matches("let _ = told.emit(\"things-state\", state);")
            .count(),
        1,
        "each change goes to the screen"
    );
    assert_eq!(app.matches("app.manage(things.clone());").count(), 1);
    let at = |text: &str| {
        app.find(text)
            .unwrap_or_else(|| panic!("app.rs has {text}"))
    };
    assert!(
        at("Things::start(") < at("Worker::start("),
        "made before the reader of files reports"
    );
    let progress = &app[at("on_progress: Arc::new(")..at("..WorkerOptions::default()")];
    assert!(
        progress.contains("read.changed();"),
        "the reader of files feeds it: {progress}"
    );
    let watched = &app[at("Watch::start(")..at(".map_err(|e| log::warn!(\"watching for changes")];
    assert!(
        watched.contains("things.changed();"),
        "the watcher feeds it"
    );
    let events = &app[at("fn folder_event(")..at("pub fn run()")];
    assert!(events.ends_with("    if let Some(things) = app.try_state::<Things>() {\n        things.changed();\n    }\n}\n\n"), "every scan and removal event feeds it");
    let exit = &app[at("if let RunEvent::Exit = event {")..];
    assert!(exit.contains("app.state::<Runner>().stop();\n            if let Some(things) = app.try_state::<Things>() {\n                things.stop();"), "quitting stops it");
    assert_eq!(
        app.matches(".changed()").count(),
        3,
        "only notes: the callbacks never wait for a reading"
    );
}

// --- R6: with the real tools ---

/// The tools SIFS_TOOLS_DIR names. Unset, the test says so by name and
/// passes.
fn real_tools(test: &str) -> Option<Tools> {
    let Some(folder) = std::env::var_os(crate::tools::TOOLS_DIR_VAR) else {
        eprintln!(
            "{test}: NOT RUN, {} is not set",
            crate::tools::TOOLS_DIR_VAR
        );
        return None;
    };
    Some(Tools::find(Path::new(&folder)).unwrap_or_else(|reason| panic!("{reason}")))
}

/// Indexes folder and reads inside its files, as the app does.
fn index_folder(db: &Path, folder: &Path, home: &Path) {
    use crate::extract::{store, worker::run_until_empty, Registry, WorkerOptions};
    use crate::index::{crawl::scan, ignore::Rules};
    let mut index = Index::open(db).unwrap();
    let registry = Registry::platform();
    store::register_kinds(&mut index, &registry).unwrap();
    let rules = Rules::new(&Rules::default_patterns(), home.to_str().unwrap()).unwrap();
    scan(&mut index, folder.to_str().unwrap(), &rules, |_| {}).unwrap();
    let stats = run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
}

fn folder_size(folder: &Path) -> u64 {
    walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_map(|e| e.ok()?.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[test]
fn r6_with_the_real_tools_the_store_follows_the_files() {
    let Some(tools) = real_tools("r6_with_the_real_tools_the_store_follows_the_files") else {
        return;
    };
    let scratch = Scratch::new();
    let files = scratch.dir.path().join("my files");
    fs::create_dir_all(&files).unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sample/files");
    for entry in fs::read_dir(&sample).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), files.join(entry.file_name())).unwrap();
    }
    let home = scratch.dir.path().join("home");
    let started = Instant::now();
    index_folder(&scratch.data.index(), &files, &home);
    eprintln!("r6: index built in {} ms", started.elapsed().as_millis());
    // Each loader line, and the largest the work folder grew.
    let lines: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let largest: Arc<Mutex<u64>> = Arc::default();
    let (runner, l, w, work) = (
        scratch.runner.clone(),
        lines.clone(),
        largest.clone(),
        scratch.work(),
    );
    let timed: Arc<Mutex<Vec<(State, Instant)>>> = Arc::default();
    let t = timed.clone();
    let things = Things::new(Setup {
        tools: Ok(tools.clone()),
        run: Arc::new(move |program, args, time| {
            if program.ends_with(Piece::Load.name()) {
                let mut largest = w.lock().unwrap();
                *largest = (*largest).max(folder_size(&work));
            }
            let ran = runner.run(program, args, time);
            if let Ran::Finished { stdout, .. } = &ran {
                if program.ends_with(Piece::Load.name()) {
                    l.lock()
                        .unwrap()
                        .push(serde_json::from_slice(stdout).unwrap());
                }
            }
            ran
        }),
        idle: Arc::new(|| true),
        on_change: Arc::new(move |state| t.lock().unwrap().push((state.clone(), Instant::now()))),
        ..scratch.setup()
    });
    let things_in_store = || {
        let answer = crate::reader::ask(
            &scratch.runner,
            &Ok(tools.clone()),
            &scratch.data,
            &crate::reader::Question::Kinds,
        )
        .unwrap();
        let kinds: serde_json::Value = serde_json::from_str(answer.as_str()).unwrap();
        kinds["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["count"].as_u64().unwrap())
            .sum::<u64>()
    };
    let step_times = |timed: &[(State, Instant)]| -> Vec<u128> {
        let at =
            |want: &dyn Fn(&State) -> bool| timed.iter().rev().find(|(s, _)| want(s)).unwrap().1;
        let export = at(&|s| {
            matches!(
                s,
                State::Reading {
                    step: Step::Export,
                    ..
                }
            )
        });
        let extract = at(&|s| {
            matches!(
                s,
                State::Reading {
                    step: Step::Extract,
                    ..
                }
            )
        });
        let load = at(&|s| {
            matches!(
                s,
                State::Reading {
                    step: Step::Load,
                    ..
                }
            )
        });
        let done = at(&|s| is_read(s));
        vec![
            (extract - export).as_millis(),
            (load - extract).as_millis(),
            (done - load).as_millis(),
        ]
    };

    let state = scratch.read_once(&things);
    let State::Read {
        documents,
        turned_down: 0,
        ..
    } = state
    else {
        panic!("{state:?}")
    };
    assert!(documents > 30, "{documents}");
    let total = things_in_store();
    assert!(total > documents as u64, "the store holds things: {total}");
    eprintln!(
        "r6: first reading of {documents} documents, export/extract/load ms {:?}, work folder at most {} bytes, index {} bytes",
        step_times(&timed.lock().unwrap()),
        largest.lock().unwrap(),
        fs::metadata(scratch.data.index()).unwrap().len()
    );

    // The extractor writes a document only for a file it found something in.
    let stored = lines.lock().unwrap()[0]["documents"]["written"]
        .as_u64()
        .unwrap();
    assert!(stored > 30 && stored <= documents as u64, "{stored}");

    scratch.read_once(&things);
    assert_eq!(
        lines.lock().unwrap().len(),
        1,
        "nothing changed: the loader did not run"
    );

    things.read_again();
    things.tick();
    let line = lines.lock().unwrap()[1].clone();
    assert_eq!(
        line["documents"],
        serde_json::json!({"written": 0, "unchanged": stored, "removed": 0}),
        "{line}"
    );
    eprintln!(
        "r6: read again, export/extract/load ms {:?}",
        step_times(&timed.lock().unwrap())
    );

    for name in [
        "kalpit-statement-a.pdf",
        "passport-rohan.pdf",
        "rent-receipt-october.pdf",
    ] {
        fs::remove_file(files.join(name)).unwrap();
    }
    index_folder(&scratch.data.index(), &files, &home);
    let state = scratch.read_once(&things);
    assert!(
        matches!(state, State::Read { documents: d, .. } if d == documents - 3),
        "{state:?}"
    );
    let line = lines.lock().unwrap()[2].clone();
    assert_eq!(
        line["documents"],
        serde_json::json!({"written": 0, "unchanged": stored - 3, "removed": 3}),
        "{line}"
    );
    assert!(
        things_in_store() < total,
        "what only those files gave is gone"
    );
    eprintln!(
        "r6: three files removed, export/extract/load ms {:?}",
        step_times(&timed.lock().unwrap())
    );
}
