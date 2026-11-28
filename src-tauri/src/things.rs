//! Keeps the store of things up to date by itself: when what the index has
//! read differs from what the store was last filled from, it runs the
//! engine's three steps once, and the person never has to ask.
//!
//! - One reading at a time, as a state machine: [`Phase`] is resting,
//!   waiting out the quiet time, or reading one [`Step`]; [`State`] is what
//!   the screen is shown. One lock guards it, and a request during a
//!   reading only marks that another is owed.
//! - When. Something changed (a scan, a removal, the watcher, the reader of
//!   files) or the person asked; then nothing changed for the quiet time;
//!   then the reader of files has run out of work and no scan or removal
//!   runs. Only then is the index looked at.
//! - Only when it differs. A fingerprint, never counts: a hash of the
//!   path and modified time of every file the export lists, in its order,
//!   and of the name, size and modified time of the five tool pieces. A
//!   changed file and a rebuilt tool both differ; an unchanged index does
//!   not, after a restart too. The fingerprint of the last reading that
//!   loaded is kept in one small file in the data folder, written whole
//!   (a temp file, then a rename) only after the load finished. A file
//!   that cannot be read, or is from another version, counts as never read.
//! - A half-read index never empties the store, because the loader removes
//!   every stored document the run lacks. A reading starts only on an idle
//!   index, and before the extract and again before the load it is dropped
//!   (nothing loaded, another owed) if the index is busy, if anything
//!   changed since it started, or if the index's fingerprint is no longer
//!   the one it exported. A failed export or extract never loads. Once the
//!   load has begun it finishes: it is from one snapshot.
//! - The steps run in a work folder in the data folder, made empty before
//!   a reading and removed after it whatever the outcome: the export of
//!   the index with today's date, the extractor (10 minutes at most), the
//!   loader (15 minutes at most). The loader's exit 0, or 1 when the store
//!   turned some records down, is read; anything else fails at that step
//!   with what the tool said.
//! - Nothing outlives the app. The tools run only through the app's one
//!   runner; once it is stopped the reading ends where it is, nothing more
//!   starts and the thread ends. The thread never keeps the app from
//!   exiting.
//! - The clock, the quiet time, whether the index is idle and the tools
//!   are given, so all of this is tested without the app.

use crate::data::DataDir;
use crate::extract::textset;
use crate::reader::SAID_KEPT;
use crate::tools::{NotFound, Piece, Ran, Tools};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long nothing must change before a reading.
pub const QUIET: Duration = Duration::from_secs(10);
pub const EXTRACT_TIME: Duration = Duration::from_secs(10 * 60);
pub const LOAD_TIME: Duration = Duration::from_secs(15 * 60);
/// In the data folder.
pub const WORK_FOLDER: &str = "things-work";
/// In the data folder: the fingerprint of the last reading that loaded.
pub const FINGERPRINT_FILE: &str = "things-read.json";
const FINGERPRINT_FORMAT: u32 = 1;
/// What a reading the app's quitting ended says.
const QUIT: &str = "SIFS quit during this step";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Export,
    Extract,
    Load,
}

/// What the screen is shown, tagged by `state`. `owed` is whether another
/// reading is to follow: one asked for, or one dropped because the index
/// changed under it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    /// Nothing ever runs: `said` is why the app has no tools.
    NoTools {
        said: String,
    },
    /// Nothing read since the app started, and nothing to read.
    Idle,
    /// Something changed or was asked; the quiet time is running, or the
    /// index is busy.
    Waiting {
        owed: bool,
    },
    Reading {
        step: Step,
        owed: bool,
    },
    /// The last reading loaded `documents` at `when` (seconds since 1970);
    /// the store turned down `turned_down` records.
    Read {
        when: i64,
        documents: usize,
        turned_down: usize,
    },
    /// The last reading failed at `step`, and why.
    Failed {
        step: Step,
        said: String,
    },
}

pub trait Clock: Send + Sync + 'static {
    /// For the quiet time.
    fn now(&self) -> Instant;
    /// Seconds since 1970: when a reading finished, and today's date.
    fn wall(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// Runs one tool through the app's runner.
pub type RunTool = Arc<dyn Fn(&Path, &[OsString], Duration) -> Ran + Send + Sync>;
/// Whether the reader of files has run out of work and no scan or removal
/// runs. Asked only on the machine's own thread.
pub type Idle = Arc<dyn Fn() -> bool + Send + Sync>;
/// Told each new state, in order. It must not call back into the machine.
pub type OnChange = Arc<dyn Fn(&State) + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub extract: Duration,
    pub load: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            extract: EXTRACT_TIME,
            load: LOAD_TIME,
        }
    }
}

/// What the machine is given.
pub struct Setup {
    pub data: DataDir,
    pub tools: Result<Tools, NotFound>,
    pub run: RunTool,
    pub idle: Idle,
    pub clock: Arc<dyn Clock>,
    pub quiet: Duration,
    pub limits: Limits,
    pub on_change: OnChange,
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Shows how the last reading ended.
    Rest,
    /// Looks at the index once `until` has passed. `forced` reads whatever
    /// the fingerprint says.
    Waiting { until: Instant, forced: bool },
    /// `next` is Some once another reading is owed, forced or not;
    /// `overtaken` once something changed since this one started.
    Reading {
        step: Step,
        next: Option<bool>,
        overtaken: bool,
    },
}

/// How a reading ended.
enum Ending {
    Read {
        documents: usize,
        turned_down: usize,
    },
    Failed {
        step: Step,
        said: String,
    },
    /// The index changed under it: nothing loaded.
    Dropped,
    Quit,
}

struct Machine {
    phase: Phase,
    /// Idle, NoTools, or how the last reading ended.
    last: State,
    stopping: bool,
    /// The state last published, and how many have been.
    shown: State,
    version: u64,
}

impl Machine {
    fn shows(&self) -> State {
        match self.phase {
            Phase::Rest => self.last.clone(),
            Phase::Waiting { forced, .. } => State::Waiting { owed: forced },
            Phase::Reading { step, next, .. } => State::Reading {
                step,
                owed: next.is_some(),
            },
        }
    }

    fn changed(&mut self, now: Instant, quiet: Duration) {
        match &mut self.phase {
            Phase::Rest => {
                self.phase = Phase::Waiting {
                    until: now + quiet,
                    forced: false,
                }
            }
            Phase::Waiting { until, .. } => *until = now + quiet,
            Phase::Reading {
                next, overtaken, ..
            } => {
                *overtaken = true;
                next.get_or_insert(false);
            }
        }
    }

    fn asked(&mut self, now: Instant) {
        match &mut self.phase {
            Phase::Rest | Phase::Waiting { .. } => {
                self.phase = Phase::Waiting {
                    until: now,
                    forced: true,
                }
            }
            Phase::Reading { next, .. } => *next = Some(true),
        }
    }

    fn ended(&mut self, ending: Ending, now: Instant, quiet: Duration, wall: i64) {
        let Phase::Reading {
            step,
            next,
            overtaken,
        } = self.phase
        else {
            return;
        };
        let dropped = matches!(ending, Ending::Dropped);
        let next = match ending {
            Ending::Read {
                documents,
                turned_down,
            } => {
                self.last = State::Read {
                    when: wall,
                    documents,
                    turned_down,
                };
                next
            }
            Ending::Failed { step, said } => {
                self.last = State::Failed { step, said };
                next
            }
            Ending::Dropped => Some(true),
            Ending::Quit => {
                self.last = State::Failed {
                    step,
                    said: QUIT.into(),
                };
                self.stopping = true;
                None
            }
        };
        let until = if overtaken || dropped {
            now + quiet
        } else {
            now
        };
        self.phase = next.map_or(Phase::Rest, |forced| Phase::Waiting { until, forced });
    }
}

struct Shared {
    data: DataDir,
    /// None: nothing ever runs.
    tools: Option<Tools>,
    run: RunTool,
    idle: Idle,
    clock: Arc<dyn Clock>,
    quiet: Duration,
    limits: Limits,
    on_change: OnChange,
    machine: Mutex<Machine>,
    woken: Condvar,
    /// The last version on_change was told, so it hears them in order
    /// while the machine's lock is free.
    told: Mutex<u64>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Machine> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The machine. Clones share it.
#[derive(Clone)]
pub struct Things(Arc<Shared>);

impl Things {
    /// The machine, and with tools its thread, which looks at the index
    /// once the quiet time has passed.
    pub fn start(setup: Setup) -> Things {
        let things = Things::new(setup);
        drop(things.drive());
        things
    }

    fn new(setup: Setup) -> Things {
        let until = setup.clock.now() + setup.quiet;
        let (tools, last, phase) = match setup.tools {
            Ok(tools) => (
                Some(tools),
                State::Idle,
                Phase::Waiting {
                    until,
                    forced: false,
                },
            ),
            Err(why) => (
                None,
                State::NoTools {
                    said: why.to_string(),
                },
                Phase::Rest,
            ),
        };
        let mut machine = Machine {
            phase,
            last,
            stopping: false,
            shown: State::Idle,
            version: 0,
        };
        machine.shown = machine.shows();
        Things(Arc::new(Shared {
            data: setup.data,
            tools,
            run: setup.run,
            idle: setup.idle,
            clock: setup.clock,
            quiet: setup.quiet,
            limits: setup.limits,
            on_change: setup.on_change,
            machine: Mutex::new(machine),
            woken: Condvar::new(),
            told: Mutex::new(0),
        }))
    }

    /// Starts the machine's thread, unless it has no tools.
    fn drive(&self) -> Option<JoinHandle<()>> {
        self.0.tools.as_ref()?;
        let things = self.clone();
        std::thread::Builder::new()
            .name("sifs-things".into())
            .spawn(move || things.run())
            .map_err(|e| log::error!("the store of things cannot be kept up to date: {e}"))
            .ok()
    }

    fn run(&self) {
        loop {
            let mut machine = self.0.lock();
            loop {
                if machine.stopping {
                    return;
                }
                let now = self.0.clock.now();
                machine = match machine.phase {
                    Phase::Waiting { until, .. } if until <= now => break,
                    Phase::Waiting { until, .. } => {
                        self.0
                            .woken
                            .wait_timeout(machine, until - now)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0
                    }
                    Phase::Rest | Phase::Reading { .. } => self
                        .0
                        .woken
                        .wait(machine)
                        .unwrap_or_else(PoisonError::into_inner),
                };
            }
            drop(machine);
            self.tick();
        }
    }

    /// Changes the machine under its lock, then tells on_change if what it
    /// shows changed, and wakes its thread.
    fn update<R>(&self, change: impl FnOnce(&mut Machine) -> R) -> R {
        let (result, told) = {
            let mut machine = self.0.lock();
            let result = change(&mut machine);
            let shown = machine.shows();
            let told = (shown != machine.shown).then(|| {
                machine.shown = shown.clone();
                machine.version += 1;
                (machine.version, shown)
            });
            (result, told)
        };
        self.0.woken.notify_all();
        if let Some((version, shown)) = told {
            let mut last = self.0.told.lock().unwrap_or_else(PoisonError::into_inner);
            if version > *last {
                *last = version;
                (self.0.on_change)(&shown);
            }
        }
        result
    }

    /// Once the quiet time has passed: looks at the index, and reads it if
    /// it is idle and differs or a reading was asked for.
    fn tick(&self) {
        let shared = &self.0;
        let Some(tools) = &shared.tools else {
            return;
        };
        let due = {
            let machine = shared.lock();
            match machine.phase {
                Phase::Waiting { until, .. }
                    if !machine.stopping && until <= shared.clock.now() =>
                {
                    until
                }
                _ => return,
            }
        };
        if !(shared.idle)() {
            let later = shared.clock.now() + shared.quiet;
            self.update(|machine| {
                if let Phase::Waiting { until, .. } = &mut machine.phase {
                    if *until == due {
                        *until = later;
                    }
                }
            });
            return;
        }
        let looked = fingerprint(&shared.data.index(), tools);
        let recorded = recorded(&shared.data.path().join(FINGERPRINT_FILE));
        let start = self.update(|machine| {
            let Phase::Waiting { until, forced } = machine.phase else {
                return None;
            };
            if until != due || machine.stopping {
                return None;
            }
            match looked {
                Err(e) => {
                    machine.last = State::Failed {
                        step: Step::Export,
                        said: kept(format!("{e:#}")),
                    };
                    machine.phase = Phase::Rest;
                    None
                }
                Ok(looked) if !forced && recorded == Some(looked) => {
                    machine.phase = Phase::Rest;
                    None
                }
                Ok(looked) => {
                    machine.phase = Phase::Reading {
                        step: Step::Export,
                        next: None,
                        overtaken: false,
                    };
                    Some(looked)
                }
            }
        });
        let Some(exporting) = start else {
            return;
        };
        let ending = self.read(tools, exporting);
        let (now, wall) = (shared.clock.now(), shared.clock.wall());
        self.update(|machine| machine.ended(ending, now, shared.quiet, wall));
    }

    fn read(&self, tools: &Tools, exporting: Fingerprint) -> Ending {
        let work = self.0.data.path().join(WORK_FOLDER);
        let ending = match empty_folder(&work) {
            Ok(()) => self.steps(tools, &work, exporting),
            Err(e) => Ending::Failed {
                step: Step::Export,
                said: kept(format!("{e:#}")),
            },
        };
        if let Err(e) = fs::remove_dir_all(&work) {
            if e.kind() != ErrorKind::NotFound {
                log::warn!("removing {}: {e}", work.display());
            }
        }
        ending
    }

    fn steps(&self, tools: &Tools, work: &Path, exporting: Fingerprint) -> Ending {
        let shared = &self.0;
        let (text, run) = (work.join("text"), work.join("run.json"));
        let exported = match textset::export(
            &shared.data.index(),
            &text,
            &local_date(shared.clock.wall()),
        ) {
            Ok(exported) => exported,
            Err(e) => {
                return Ending::Failed {
                    step: Step::Export,
                    said: kept(format!("{e:#}")),
                }
            }
        };
        if let Some(ending) = self.next(Step::Extract, tools, exporting) {
            return ending;
        }
        let args = [
            "-text".into(),
            text.into(),
            "-directory".into(),
            tools.path(Piece::Banks).into(),
            "-iban-registry".into(),
            tools.path(Piece::IbanRegistry).into(),
            "-out".into(),
            run.clone().into(),
        ];
        match (shared.run)(&tools.path(Piece::Extract), &args, shared.limits.extract) {
            Ran::Finished { code: 0, .. } => {}
            Ran::Stopped => return Ending::Quit,
            other => {
                return Ending::Failed {
                    step: Step::Extract,
                    said: said(other),
                }
            }
        }
        if let Some(ending) = self.next(Step::Load, tools, exporting) {
            return ending;
        }
        let args = [
            "--run".into(),
            run.into(),
            "--data-dir".into(),
            shared.data.path().into(),
        ];
        match (shared.run)(&tools.path(Piece::Load), &args, shared.limits.load) {
            Ran::Finished {
                code: 0 | 1,
                stdout,
                ..
            } => match turned_down(&stdout) {
                Some(turned_down) => {
                    if let Err(e) = record(&shared.data, exporting) {
                        log::warn!("recording what the store was filled from: {e:#}");
                    }
                    Ending::Read {
                        documents: exported.documents.len(),
                        turned_down,
                    }
                }
                None => Ending::Failed {
                    step: Step::Load,
                    said: "the loader printed no line saying what it loaded".into(),
                },
            },
            Ran::Stopped => Ending::Quit,
            other => Ending::Failed {
                step: Step::Load,
                said: said(other),
            },
        }
    }

    /// Moves the reading on to step, or ends it: dropped when the index is
    /// busy, changed since the reading started, or no longer what was
    /// exported; quit once the app is quitting.
    fn next(&self, step: Step, tools: &Tools, exporting: Fingerprint) -> Option<Ending> {
        let shared = &self.0;
        let still =
            (shared.idle)() && fingerprint(&shared.data.index(), tools).ok() == Some(exporting);
        self.update(|machine| {
            if machine.stopping {
                return Some(Ending::Quit);
            }
            let Phase::Reading {
                step: at,
                overtaken,
                ..
            } = &mut machine.phase
            else {
                return Some(Ending::Quit);
            };
            if !still || *overtaken {
                return Some(Ending::Dropped);
            }
            *at = step;
            None
        })
    }

    pub fn state(&self) -> State {
        self.0.lock().shown.clone()
    }

    /// Something in the index may have changed. Never waits for a reading.
    pub fn changed(&self) {
        if self.0.tools.is_some() {
            let now = self.0.clock.now();
            self.update(|machine| machine.changed(now, self.0.quiet));
        }
    }

    /// A reading as soon as the index is idle, fingerprint or not; after
    /// the one running, if one is.
    pub fn read_again(&self) {
        if self.0.tools.is_some() {
            let now = self.0.clock.now();
            self.update(|machine| machine.asked(now));
        }
    }

    /// No reading starts from now on, and the thread ends. The app also
    /// stops its runner, which ends the step running.
    pub fn stop(&self) {
        self.update(|machine| machine.stopping = true);
    }
}

/// Whether the index's reader of files has nothing waiting.
pub fn nothing_waiting(db: &Path) -> bool {
    let waiting = || -> Result<i64> {
        let conn = textset::open_read_only(db)?;
        Ok(conn.query_row("SELECT COUNT(*) FROM extract_queue", [], |r| r.get(0))?)
    };
    matches!(waiting(), Ok(0))
}

fn empty_folder(folder: &Path) -> Result<()> {
    match fs::remove_dir_all(folder) {
        Err(e) if e.kind() != ErrorKind::NotFound => {
            Err(e).with_context(|| format!("clear {}", folder.display()))
        }
        _ => fs::create_dir_all(folder).with_context(|| format!("create {}", folder.display())),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint(u64);

/// The index's listing as the export reads it, in one snapshot, and the
/// tool pieces.
fn fingerprint(db: &Path, tools: &Tools) -> Result<Fingerprint> {
    let mut hash = Fnv::default();
    let conn = textset::open_read_only(db)?;
    let snapshot = conn.unchecked_transaction()?;
    for file in textset::listed(&snapshot)? {
        hash.field(file.path.as_bytes());
        hash.field(&file.modified.to_le_bytes());
    }
    drop(snapshot);
    for piece in Piece::ALL {
        let path = tools.path(piece);
        let meta = fs::metadata(&path).with_context(|| format!("look at {}", path.display()))?;
        let modified = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        hash.field(piece.name().as_bytes());
        hash.field(&meta.len().to_le_bytes());
        hash.field(&modified.to_le_bytes());
    }
    Ok(Fingerprint(hash.0))
}

/// FNV-1a, 64 bits, over length-prefixed fields: fixed by its definition,
/// so a fingerprint means the same after a Rust upgrade.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn field(&mut self, bytes: &[u8]) {
        for &b in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Recorded {
    format: u32,
    fingerprint: String,
}

/// The fingerprint in the file, or None when it cannot be read as one.
fn recorded(path: &Path) -> Option<Fingerprint> {
    let recorded: Recorded = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if recorded.format != FINGERPRINT_FORMAT {
        return None;
    }
    u64::from_str_radix(&recorded.fingerprint, 16)
        .ok()
        .map(Fingerprint)
}

fn record(data: &DataDir, fingerprint: Fingerprint) -> Result<()> {
    let path = data.path().join(FINGERPRINT_FILE);
    let temp = data.path().join(format!("{FINGERPRINT_FILE}.new"));
    let json = serde_json::to_string(&Recorded {
        format: FINGERPRINT_FORMAT,
        fingerprint: format!("{:016x}", fingerprint.0),
    })?;
    fs::write(&temp, json).with_context(|| format!("write {}", temp.display()))?;
    fs::rename(&temp, &path)
        .with_context(|| format!("rename {} to {}", temp.display(), path.display()))
}

/// How many records the store turned down, from the loader's line.
fn turned_down(stdout: &[u8]) -> Option<usize> {
    let line = std::str::from_utf8(stdout)
        .ok()?
        .lines()
        .find(|line| !line.trim().is_empty())?;
    let line: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(line.get("turned_down")?.as_array()?.len())
}

/// What a tool that failed said, in SIFS's words around its own.
fn said(ran: Ran) -> String {
    let with = |what: String, stderr: &[u8]| {
        let text = String::from_utf8_lossy(stderr);
        match text
            .lines()
            .map(str::trim_end)
            .rfind(|line| !line.is_empty())
        {
            Some(line) => format!("{what}: {line}"),
            None => what,
        }
    };
    kept(match ran {
        Ran::Finished { code, stderr, .. } => with(format!("exited with code {code}"), &stderr),
        Ran::Signalled { signal, stderr } => with(format!("ended on signal {signal}"), &stderr),
        Ran::CouldNotStart(e) => format!("could not be started: {e}"),
        Ran::TimedOut => "took too long".into(),
        Ran::WroteTooMuch => "printed too much".into(),
        Ran::Stopped => QUIT.into(),
    })
}

/// At most [`SAID_KEPT`] bytes, cut at a character.
fn kept(mut said: String) -> String {
    if said.len() > SAID_KEPT {
        let mut end = SAID_KEPT - '…'.len_utf8();
        while !said.is_char_boundary(end) {
            end -= 1;
        }
        said.truncate(end);
        said.push('…');
    }
    said
}

/// The person's date at seconds since 1970, `YYYY-MM-DD`.
fn local_date(seconds: i64) -> String {
    let time = seconds as libc::time_t;
    // SAFETY: tm is plain data; localtime_r writes only into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return textset::utc(seconds)[..10].to_string();
    }
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

#[cfg(test)]
#[path = "things_tests.rs"]
mod tests;
