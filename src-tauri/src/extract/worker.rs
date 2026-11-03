//! The background worker: takes files from the front of the queue, reads
//! each on its own thread with a time limit, and writes what it read in
//! small batches. It runs at low priority and pauses when told to (while a
//! scan runs, in the app).
//!
//! Each file is checked just before it is read (gone, already read as it
//! is, too large) and again just after (changed meanwhile: the result is
//! thrown away and the file stays queued). A read past its time limit is
//! recorded failed and its thread abandoned; a file that stopped SIFS twice
//! is recorded failed without being read again.

use super::store::{self, Extract, Finished, Job, Progress, Status};
use super::{describe_size, describe_time, platform, ReadError, Reader, Registry, Sink};
use crate::index::db::Index;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Why a file is recorded failed after two reads that never finished.
pub const STOPPED_TWICE: &str = "SIFS stopped twice while reading this file";
/// With this many abandoned reads still running, of any reader, no more
/// reads start until some finish.
const MAX_ABANDONED: usize = 4;
/// RunStats keeps at most this many errors (the latest) and lock holds (the
/// first), so a worker running for months doesn't grow without end.
const MAX_ERRORS: usize = 100;
const MAX_HOLDS: usize = 1_000_000;

#[derive(Clone)]
pub struct WorkerOptions {
    /// Reads at a time.
    pub concurrency: usize,
    /// Files claimed at a time.
    pub batch: usize,
    /// A batch is written early once its text passes this many bytes.
    pub batch_bytes: usize,
    /// How long it waits, idle or paused, before looking again.
    pub idle_poll: Duration,
    /// How long past a reader's time limit before its read is abandoned
    /// (the reader itself is meant to stop at the limit).
    pub grace: Duration,
    /// Whether to hold off for now (asked between batches).
    pub pause: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Called with the counts at most once a second while reading, and
    /// when it runs out of work.
    pub on_progress: Arc<dyn Fn(&Progress) + Send + Sync>,
}

impl Default for WorkerOptions {
    fn default() -> WorkerOptions {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        WorkerOptions {
            concurrency: (cores / 4).clamp(1, 2),
            batch: 16,
            batch_bytes: 2 * 1024 * 1024,
            idle_poll: Duration::from_secs(5),
            grace: Duration::from_secs(1),
            pause: Arc::new(|| false),
            on_progress: Arc::new(|_| {}),
        }
    }
}

/// What a run did, for tests and measurement.
#[derive(Debug, Clone, Default)]
pub struct RunStats {
    /// Files queued by the startup check against the readers.
    pub reconciled: usize,
    pub claimed: usize,
    /// Reads started.
    pub reads: usize,
    /// Reads abandoned past their time limit.
    pub abandoned: usize,
    /// How long each write held the writer lock (the first million).
    pub holds: Vec<Duration>,
    /// The latest hundred.
    pub errors: Vec<String>,
}

struct Shared {
    stop: AtomicBool,
    /// Files opened in SIFS since the worker last looked.
    opened: Mutex<Vec<String>>,
    woken: Mutex<bool>,
    wake: Condvar,
}

impl Shared {
    fn wait(&self, timeout: Duration) {
        let woken = self.woken.lock().unwrap_or_else(|p| p.into_inner());
        let (mut woken, _) = self.wake.wait_timeout_while(woken, timeout, |w| !*w).unwrap_or_else(|p| p.into_inner());
        *woken = false;
    }

    fn wake(&self) {
        *self.woken.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.wake.notify_all();
    }
}

/// Wakes a worker: there may be new work, or a pause may have ended.
#[derive(Clone)]
pub struct Waker(Arc<Shared>);

impl Waker {
    pub fn wake(&self) {
        self.0.wake();
    }
}

/// The running worker. Dropping it stops it.
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<RunStats>>,
}

impl Worker {
    /// Starts the worker on the index at db_path. Its first job is checking
    /// the files already indexed against the readers, if they changed.
    pub fn start(db_path: PathBuf, registry: Arc<Registry>, options: WorkerOptions) -> Result<Worker> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            opened: Mutex::new(Vec::new()),
            woken: Mutex::new(false),
            wake: Condvar::new(),
        });
        let inner = shared.clone();
        let thread = std::thread::Builder::new().name("sifs-extract".into()).spawn(move || {
            platform::lower_thread_priority();
            let mut index = match Index::open(&db_path) {
                Ok(index) => index,
                Err(e) => {
                    log::error!("the reader of files cannot open the index: {e:#}");
                    return RunStats { errors: vec![format!("{e:#}")], ..RunStats::default() };
                }
            };
            run(&mut index, &registry, &options, &inner)
        })?;
        Ok(Worker { shared, thread: Some(thread) })
    }

    pub fn wake(&self) {
        self.shared.wake();
    }

    /// A file opened in SIFS: if it is waiting to be read, it is read next.
    /// The worker moves it, on its own connection, so the caller (the
    /// window) never waits for the index.
    pub fn opened(&self, path: &str) {
        self.shared.opened.lock().unwrap_or_else(|p| p.into_inner()).push(path.to_string());
        self.shared.wake();
    }

    pub fn waker(&self) -> Waker {
        Waker(self.shared.clone())
    }

    /// Stops after the reads in flight finish and are written; files claimed
    /// and not yet read go back to the queue as they were.
    pub fn stop(mut self) -> RunStats {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> RunStats {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake();
        self.thread.take().map(|t| t.join().unwrap_or_default()).unwrap_or_default()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Startup work before reading: files skipped as not downloaded are looked
/// at again, and if the readers changed, every file is checked against them.
fn prepare(index: &mut Index, registry: &Registry) -> Result<usize> {
    store::requeue_not_downloaded(index)?;
    if store::needs_reconcile(index, registry)? {
        let queued = store::reconcile(index, registry, now())?;
        log::info!("{queued} files queued to be read");
        return Ok(queued);
    }
    Ok(0)
}

fn run(index: &mut Index, registry: &Registry, options: &WorkerOptions, shared: &Shared) -> RunStats {
    let stopped = || shared.stop.load(Ordering::SeqCst);
    let mut runner = Runner::new(registry, options);
    let mut prepared = false;
    let mut last_progress: Option<Instant> = None;
    let mut unreported = true;
    while !stopped() {
        let opened = std::mem::take(&mut *shared.opened.lock().unwrap_or_else(|p| p.into_inner()));
        for path in opened {
            if let Err(e) = store::bump_opened(index, &path) {
                runner.error(e);
            }
        }
        if (options.pause)() {
            shared.wait(options.idle_poll);
            continue;
        }
        if !prepared {
            match prepare(index, registry) {
                Ok(queued) => {
                    runner.stats.reconciled += queued;
                    prepared = true;
                }
                Err(e) => {
                    runner.error(e);
                    shared.wait(options.idle_poll);
                    continue;
                }
            }
        }
        match runner.step(index, &stopped) {
            Ok(0) => {
                if unreported {
                    runner.report(index);
                    unreported = false;
                }
                shared.wait(options.idle_poll);
            }
            Ok(_) => {
                unreported = true;
                if last_progress.map_or(true, |t| t.elapsed() >= Duration::from_secs(1)) {
                    runner.report(index);
                    last_progress = Some(Instant::now());
                    unreported = false;
                }
            }
            Err(e) => {
                runner.error(e);
                shared.wait(options.idle_poll);
            }
        }
    }
    runner.stats
}

/// Reads until the queue is empty, on the calling thread: for tests and
/// measurement. It doesn't pause.
pub fn run_until_empty(index: &mut Index, registry: &Registry, options: &WorkerOptions) -> Result<RunStats> {
    let mut runner = Runner::new(registry, options);
    runner.stats.reconciled = prepare(index, registry)?;
    while runner.step(index, &|| false)? > 0 {}
    Ok(runner.stats)
}

/// A read's thread: running, finished (its result is on its way), or
/// abandoned by the worker.
const RUNNING: u8 = 0;
const FINISHED: u8 = 1;
const ABANDONED: u8 = 2;

/// The file as it was just before it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stat {
    modified: i64,
    size: i64,
    /// To the nanosecond, to tell a change within the same second.
    exact: Option<SystemTime>,
}

impl Stat {
    fn of(meta: &std::fs::Metadata) -> Stat {
        let exact = meta.modified().ok();
        Stat {
            modified: exact.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
            size: if meta.is_dir() { 0 } else { meta.len() as i64 },
            exact,
        }
    }
}

struct Pending {
    job: Job,
    reader: Arc<dyn Reader>,
    before: Stat,
}

struct Flight {
    job: Job,
    reader: Arc<dyn Reader>,
    before: Stat,
    deadline: Instant,
    state: Arc<AtomicU8>,
}

struct Outcome {
    result: Result<(), ReadError>,
    sink: Sink,
    millis: i64,
}

struct Runner<'a> {
    registry: &'a Registry,
    options: &'a WorkerOptions,
    tx: mpsc::Sender<(u64, Outcome)>,
    rx: mpsc::Receiver<(u64, Outcome)>,
    next_token: u64,
    /// Abandoned reads still running, of every reader.
    abandoned: Arc<AtomicUsize>,
    stats: RunStats,
}

impl<'a> Runner<'a> {
    fn new(registry: &'a Registry, options: &'a WorkerOptions) -> Runner<'a> {
        let (tx, rx) = mpsc::channel();
        Runner { registry, options, tx, rx, next_token: 0, abandoned: Arc::default(), stats: RunStats::default() }
    }

    fn error(&mut self, e: anyhow::Error) {
        log::warn!("reading inside files: {e:#}");
        if self.stats.errors.len() >= MAX_ERRORS {
            self.stats.errors.remove(0);
        }
        self.stats.errors.push(format!("{e:#}"));
    }

    fn report(&mut self, index: &Index) {
        match store::progress(index) {
            Ok(progress) => (self.options.on_progress)(&progress),
            Err(e) => self.error(e),
        }
    }

    fn stuck(&self) -> bool {
        self.abandoned.load(Ordering::SeqCst) >= MAX_ABANDONED
    }

    /// Claims a batch, reads it and writes it. Returns how many files it
    /// claimed: 0 when the queue is empty, or while too many abandoned reads
    /// are still running. Once stopped returns true, it starts no more reads
    /// and puts back what it hadn't started. If anything fails, the files
    /// claimed and not yet written go back too: an error is not a crash.
    fn step(&mut self, index: &mut Index, stopped: &dyn Fn() -> bool) -> Result<usize> {
        if self.stuck() {
            return Ok(0);
        }
        let jobs = store::claim(index, self.options.batch)?;
        let claimed = jobs.len();
        self.stats.claimed += claimed;
        let mut written = HashSet::new();
        if let Err(e) = self.settle(index, jobs.clone(), stopped, &mut written) {
            let back: Vec<Finished> =
                jobs.into_iter().filter(|j| !written.contains(&j.file_id)).map(Finished::Unclaim).collect();
            if let Err(again) = store::write_results(index, &back, now()) {
                log::warn!("putting back claimed files: {again:#}");
            }
            return Err(e);
        }
        Ok(claimed)
    }

    /// Reads the claimed jobs and writes what came of them, noting each file
    /// written in written.
    fn settle(
        &mut self,
        index: &mut Index,
        jobs: Vec<Job>,
        stopped: &dyn Fn() -> bool,
        written: &mut HashSet<i64>,
    ) -> Result<()> {
        let mut results = Vec::new();
        let mut pending = VecDeque::new();
        for job in jobs {
            match self.check_before(index, job)? {
                Ok(finished) => results.push(finished),
                Err(read) => pending.push_back(read),
            }
        }
        let mut flights: HashMap<u64, Flight> = HashMap::new();
        loop {
            while flights.len() < self.options.concurrency.max(1) && !stopped() {
                let exclusive_busy = flights.values().any(|f| f.reader.limits().exclusive);
                let Some(at) = pending.iter().position(|p: &Pending| !(exclusive_busy && p.reader.limits().exclusive))
                else {
                    break;
                };
                let p = pending.remove(at).unwrap();
                if self.stuck() {
                    log::warn!("{MAX_ABANDONED} reads are stuck; files wait until one ends");
                    results.push(Finished::Unclaim(p.job));
                    continue;
                }
                match self.dispatch(p) {
                    Ok((token, flight)) => {
                        flights.insert(token, flight);
                    }
                    Err((job, e)) => {
                        self.error(e.into());
                        results.push(Finished::Unclaim(job));
                    }
                }
            }
            if flights.is_empty() {
                break;
            }
            let until = flights.values().map(|f| f.deadline + self.options.grace).min().unwrap();
            match self.rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok((token, outcome)) => {
                    // A late answer from an abandoned read has no flight.
                    if let Some(flight) = flights.remove(&token) {
                        results.push(check_after(flight, outcome));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    let late: Vec<u64> = flights
                        .iter()
                        .filter(|(_, f)| now >= f.deadline + self.options.grace)
                        .map(|(t, _)| *t)
                        .collect();
                    for token in late {
                        let flight = &flights[&token];
                        // If it finished just now, its result is on its way.
                        if flight.state.compare_exchange(RUNNING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
                        {
                            let flight = flights.remove(&token).unwrap();
                            self.abandoned.fetch_add(1, Ordering::SeqCst);
                            self.stats.abandoned += 1;
                            let limit = flight.reader.limits().time;
                            let error = format!("took longer than {}", describe_time(limit));
                            let millis = limit.as_millis() as i64;
                            results.push(recorded(
                                flight.job,
                                &*flight.reader,
                                flight.before,
                                Status::Failed,
                                Some(error),
                                millis,
                            ));
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => unreachable!("the runner holds a sender"),
            }
            if results.iter().map(text_bytes).sum::<usize>() >= self.options.batch_bytes {
                self.write(index, &mut results, written)?;
            }
        }
        results.extend(pending.into_iter().map(|p| Finished::Unclaim(p.job)));
        self.write(index, &mut results, written)
    }

    /// Settles a job without reading it, or returns it to be read.
    fn check_before(&self, index: &Index, job: Job) -> Result<std::result::Result<Finished, Pending>> {
        let Some(reader) = self.registry.get(&job.ext).cloned() else {
            return Ok(Ok(Finished::Drop(job)));
        };
        let meta = match std::fs::symlink_metadata(&job.path) {
            Ok(meta) => meta,
            Err(_) => return Ok(Ok(Finished::Drop(job))),
        };
        // Only a regular file (or a package folder, for a reader of
        // packages) is opened: a FIFO, socket or device would block in
        // open() or never end. A link's target is read where it is indexed,
        // if it is. A file replaced by any of these loses what was read
        // from it.
        let kind = meta.file_type();
        if !(kind.is_file() || (kind.is_dir() && reader.packages())) {
            return Ok(Ok(Finished::Clear(job)));
        }
        let stat = Stat::of(&meta);
        let read: Option<(String, u32, i64, i64)> = index
            .conn
            .query_row(
                "SELECT reader, version, modified, size FROM extracts WHERE file_id = ?1",
                params![job.file_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((id, version, modified, size)) = read {
            if id == reader.id() && version >= reader.version() && modified == stat.modified && size == stat.size {
                return Ok(Ok(Finished::Drop(job)));
            }
        }
        if job.attempts >= 3 {
            return Ok(Ok(recorded(job, &*reader, stat, Status::Failed, Some(STOPPED_TWICE.into()), 0)));
        }
        let limits = reader.limits();
        if stat.size as u64 > limits.max_bytes {
            let why = format!("larger than {}", describe_size(limits.max_bytes));
            return Ok(Ok(recorded(job, &*reader, stat, Status::Skipped, Some(why), 0)));
        }
        if let Err(why) = platform::may_read(&meta) {
            return Ok(Ok(recorded(job, &*reader, stat, Status::Skipped, Some(why.into()), 0)));
        }
        Ok(Err(Pending { job, reader, before: stat }))
    }

    fn dispatch(&mut self, p: Pending) -> std::result::Result<(u64, Flight), (Job, std::io::Error)> {
        let token = self.next_token;
        self.next_token += 1;
        let limits = p.reader.limits();
        let deadline = Instant::now() + limits.time;
        let state = Arc::new(AtomicU8::new(RUNNING));
        let abandoned = self.abandoned.clone();
        let (reader, path, tx, thread_state) =
            (p.reader.clone(), PathBuf::from(&p.job.path), self.tx.clone(), state.clone());
        let spawned = std::thread::Builder::new().name("sifs-read".into()).spawn(move || {
            platform::lower_thread_priority();
            let started = Instant::now();
            let mut sink = Sink::new(limits.max_chars, deadline);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(&path, &mut sink)))
                .unwrap_or_else(|panic| {
                    let why = panic
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned());
                    Err(ReadError::Other(format!("the reader crashed: {}", why.unwrap_or_default())))
                });
            let outcome = Outcome { result, sink, millis: started.elapsed().as_millis() as i64 };
            if thread_state.compare_exchange(RUNNING, FINISHED, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                abandoned.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            let _ = tx.send((token, outcome));
        });
        match spawned {
            Ok(_) => {
                self.stats.reads += 1;
                Ok((token, Flight { job: p.job, reader: p.reader, before: p.before, deadline, state }))
            }
            Err(e) => Err((p.job, e)),
        }
    }

    fn write(&mut self, index: &mut Index, results: &mut Vec<Finished>, written: &mut HashSet<i64>) -> Result<()> {
        if !results.is_empty() {
            let held = store::write_results(index, results, now())?;
            if self.stats.holds.len() < MAX_HOLDS {
                self.stats.holds.push(held);
            }
            written.extend(results.drain(..).map(|f| job_of(&f).file_id));
        }
        Ok(())
    }
}

fn job_of(f: &Finished) -> &Job {
    match f {
        Finished::Drop(job) | Finished::Unclaim(job) | Finished::Clear(job) | Finished::Read(job, _) => job,
    }
}

fn text_bytes(f: &Finished) -> usize {
    match f {
        Finished::Read(_, extract) => extract.text_bytes(),
        _ => 0,
    }
}

fn recorded(job: Job, reader: &dyn Reader, stat: Stat, status: Status, error: Option<String>, millis: i64) -> Finished {
    let extract = Extract {
        reader: reader.id(),
        version: reader.version(),
        modified: stat.modified,
        size: stat.size,
        status,
        error,
        millis,
        parts: Vec::new(),
        facts: Vec::new(),
    };
    Finished::Read(job, extract)
}

/// What a finished read comes to, if the file is still as it was.
fn check_after(flight: Flight, outcome: Outcome) -> Finished {
    let Flight { job, reader, before, .. } = flight;
    let after = match std::fs::symlink_metadata(&job.path) {
        Ok(meta) => Stat::of(&meta),
        Err(_) => return Finished::Drop(job),
    };
    if after != before {
        return Finished::Unclaim(job);
    }
    let limits = reader.limits();
    let (status, error) = match outcome.result {
        Ok(()) => (Status::Done, None),
        Err(ReadError::Binary) => (Status::Skipped, Some("not text".to_string())),
        Err(ReadError::TooLarge) => (Status::Skipped, Some(format!("larger than {}", describe_size(limits.max_bytes)))),
        Err(ReadError::Unsupported(why)) => (Status::Skipped, Some(why)),
        Err(ReadError::Timeout) => (Status::Failed, Some(format!("took longer than {}", describe_time(limits.time)))),
        Err(ReadError::Io(e)) => (Status::Failed, Some(e.to_string())),
        Err(ReadError::Other(why)) => (Status::Failed, Some(why)),
    };
    let mut finished = recorded(job, &*reader, before, status, error, outcome.millis);
    if status == Status::Done {
        if let Finished::Read(_, extract) = &mut finished {
            let (parts, facts) = outcome.sink.finish();
            if parts.is_empty() {
                extract.status = Status::Empty;
            }
            extract.parts = parts;
            extract.facts = facts;
        }
    }
    finished
}
