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
        tx.commit().unwrap();
    }

    fn add_one(&self, path: &str) {
        self.add(&[path.to_string()]);
    }

    fn touch(&self, path: &str, modified: i64) {
        assert_eq!(
            self.conn()
                .execute(
                    "UPDATE files SET modified = ?2 WHERE path = ?1",
                    params![path, modified]
                )
                .unwrap(),
            1
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
    let changes: [(&str, Box<dyn Fn()>); 6] = [
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
