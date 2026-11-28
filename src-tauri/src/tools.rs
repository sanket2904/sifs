//! The engine's tools: where the app finds them, and how it runs them, one
//! at a time or several at once, each with a deadline and a cap on what it
//! prints, and none of them left running once the app has quit.
//!
//! A running tool is a child the app started and has not reaped. It leads
//! its own process group, so its pid names everything it started. The
//! [`Runner`] holds the set of running tools and a phase: open, then
//! stopping (every group has been killed; the set drains), then stopped
//! (the set is empty and stays so).

use crate::data::DataDir;
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// A tool that prints more than this on its standard output is ended as
/// [`Ran::WroteTooMuch`]. The largest page in the committed goldens is
/// 6,237 bytes; this leaves room for a store over a thousand times larger.
pub const STDOUT_CAP: usize = 8 << 20;
/// How much of the end of a tool's standard error is kept.
pub const STDERR_KEPT: usize = 16 << 10;
/// How often a running tool is checked when it prints nothing.
const TICK: Duration = Duration::from_millis(10);
/// How long the rest of a group may take to die after SIGKILL.
const GROUP_GONE: Duration = Duration::from_secs(1);

/// Set to an absolute path, the app looks for its tools there instead.
pub const TOOLS_DIR_VAR: &str = "SIFS_TOOLS_DIR";
/// The script that builds the tools into a folder.
pub const BUILD_SCRIPT: &str = "sample/tools/build-tools.sh";

/// What a tools folder holds: three programs, and two data files the
/// extractor reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Extract,
    Load,
    Show,
    Banks,
    IbanRegistry,
}

impl Piece {
    pub const ALL: [Piece; 5] = [Piece::Extract, Piece::Load, Piece::Show, Piece::Banks, Piece::IbanRegistry];

    pub fn name(self) -> &'static str {
        match self {
            Piece::Extract => "m1a-extract",
            Piece::Load => "sifs-load",
            Piece::Show => "sifs-show",
            Piece::Banks => "banks.json",
            Piece::IbanRegistry => "iban-registry.json",
        }
    }

    fn is_program(self) -> bool {
        matches!(self, Piece::Extract | Piece::Load | Piece::Show)
    }
}

/// What is wrong with one piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Missing,
    /// A folder, or anything else that is not a plain file.
    NotAFile,
    /// A program with no permission to run.
    NotRunnable,
}

/// Why the app has no tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotFound {
    /// SIFS_TOOLS_DIR is empty or not an absolute path.
    NotAbsolute(OsString),
    NoFolder(PathBuf),
    /// Each piece that is not there as it must be, in [`Piece::ALL`] order.
    Pieces { folder: PathBuf, faults: Vec<(Piece, Fault)> },
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let build = |f: &mut fmt::Formatter<'_>, folder: &Path| {
            write!(f, "; build them with {BUILD_SCRIPT} {}", folder.display())
        };
        match self {
            NotFound::NotAbsolute(value) => write!(
                f,
                "{TOOLS_DIR_VAR} must be an absolute path, not {value:?}; SIFS does not fall back to its usual tools folder; build them with {BUILD_SCRIPT} <folder>"
            ),
            NotFound::NoFolder(folder) => {
                write!(f, "SIFS has no tools: there is no folder {}", folder.display())?;
                build(f, folder)
            }
            NotFound::Pieces { folder, faults } => {
                write!(f, "SIFS has no tools: in {}", folder.display())?;
                for (i, (piece, fault)) in faults.iter().enumerate() {
                    let what = match fault {
                        Fault::Missing => "is missing",
                        Fault::NotAFile => "is not a file",
                        Fault::NotRunnable => "cannot be run",
                    };
                    write!(f, "{} {} {what}", if i == 0 { "" } else { "," }, piece.name())?;
                }
                build(f, folder)
            }
        }
    }
}

impl std::error::Error for NotFound {}

/// A tools folder with every piece in it. Only [`Tools::find`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools(PathBuf);

impl Tools {
    /// The tools in folder: all five pieces are files, and the three
    /// programs have permission to run.
    pub fn find(folder: &Path) -> Result<Tools, NotFound> {
        if !folder.is_dir() {
            return Err(NotFound::NoFolder(folder.to_path_buf()));
        }
        let faults: Vec<_> = Piece::ALL
            .into_iter()
            .filter_map(|piece| fault(&folder.join(piece.name()), piece.is_program()).map(|fault| (piece, fault)))
            .collect();
        if faults.is_empty() {
            Ok(Tools(folder.to_path_buf()))
        } else {
            Err(NotFound::Pieces { folder: folder.to_path_buf(), faults })
        }
    }

    pub fn path(&self, piece: Piece) -> PathBuf {
        self.0.join(piece.name())
    }
}

fn fault(path: &Path, program: bool) -> Option<Fault> {
    match path.metadata() {
        Err(_) => Some(Fault::Missing),
        Ok(meta) if !meta.is_file() => Some(Fault::NotAFile),
        Ok(meta) if program && meta.permissions().mode() & 0o111 == 0 => Some(Fault::NotRunnable),
        Ok(_) => None,
    }
}

/// The app's tools: in SIFS_TOOLS_DIR when it is set, which must be an
/// absolute path, else in `tools` in the data folder.
pub fn look(data: &DataDir, var: Option<&OsStr>) -> Result<Tools, NotFound> {
    let folder = match var {
        Some(value) if Path::new(value).is_absolute() => PathBuf::from(value),
        Some(value) => return Err(NotFound::NotAbsolute(value.to_owned())),
        None => data.path().join("tools"),
    };
    Tools::find(&folder)
}

/// What became of one run of a tool.
#[derive(Debug)]
pub enum Ran {
    /// It exited with this code. The caller decides what the code means.
    Finished { code: i32, stdout: Vec<u8>, stderr: Vec<u8> },
    CouldNotStart(std::io::Error),
    TimedOut,
    WroteTooMuch,
    /// The runner was stopped: before the tool started, which then never
    /// started, or while it ran.
    Stopped,
    /// It ended on a signal that was not SIFS's.
    Signalled { signal: i32, stderr: Vec<u8> },
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Open,
    Stopping,
    Stopped,
}

#[derive(Debug, Default)]
struct State {
    phase: Phase,
    /// The running tools by process group, which is each one's pid.
    running: BTreeSet<libc::pid_t>,
}

/// The app's running tools. One per app.
#[derive(Debug, Default)]
pub struct Runner {
    state: Mutex<State>,
    changed: Condvar,
}

impl Runner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs program with args as they are (no shell), standard input
    /// closed, in a process group of its own, until it exits, passes time,
    /// prints more than [`STDOUT_CAP`] or the runner is stopped. Whatever
    /// ends it, its whole group is killed and it is reaped before this
    /// returns.
    ///
    /// Known limit: a process the tool moves into a group of its own
    /// (setsid, setpgid, a shell's job control) is not in the tool's group
    /// and is not ended. The engine's tools start no processes. Such a
    /// process may hold the tool's output open; it is no longer read once
    /// the tool's group is gone, so the run still returns.
    pub fn run<I, S>(&self, program: &Path, args: I, time: Duration) -> Ran
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let deadline = Instant::now() + time;
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        // Started and put in the set under one lock: stop either sees the
        // tool in the set or has begun before it, and then it never starts.
        let mut tool = {
            let mut state = self.lock();
            if state.phase != Phase::Open {
                return Ran::Stopped;
            }
            match command.spawn() {
                Ok(child) => {
                    state.running.insert(group(&child));
                    Started { runner: self, child, ended: false }
                }
                Err(e) => return Ran::CouldNotStart(e),
            }
        };
        let mut output = Output {
            stdout: tool.child.stdout.take(),
            stderr: tool.child.stderr.take(),
            out: Vec::new(),
            err: Vec::new(),
        };
        let timed_out = output.watch(group(&tool.child), deadline);
        let (status, stopping) = tool.end();
        output.drain();
        if output.out.len() > STDOUT_CAP {
            return Ran::WroteTooMuch;
        }
        match status.code() {
            Some(code) => Ran::Finished { code, stdout: output.out, stderr: output.err },
            None if timed_out => Ran::TimedOut,
            None if stopping => Ran::Stopped,
            None => Ran::Signalled { signal: status.signal().unwrap_or_default(), stderr: output.err },
        }
    }

    /// Ends every running tool and everything it started, and returns once
    /// they are reaped. From its start on, no tool starts. It may be called
    /// any number of times, from any thread.
    pub fn stop(&self) {
        let mut state = self.lock();
        if state.phase == Phase::Open {
            state.phase = Phase::Stopping;
            for &group in &state.running {
                kill_group(group);
            }
        }
        while !state.running.is_empty() {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        state.phase = Phase::Stopped;
    }
}

/// A tool in the runner's set. However its run ends, even by a panic, it
/// leaves the set only once its group is killed and it is reaped.
struct Started<'r> {
    runner: &'r Runner,
    child: Child,
    ended: bool,
}

impl Started<'_> {
    /// Kills what is left of the tool's group, then reaps the tool and takes
    /// it out of the set under the lock, so stop never signals a group whose
    /// pid could be reused. Gives its status and whether a stop has begun.
    fn end(&mut self) -> (ExitStatus, bool) {
        self.ended = true;
        let group = group(&self.child);
        kill_group(group);
        // On macOS a group whose one member is its unreaped leader can no
        // longer be signalled, so this waits for every other member to die.
        let until = Instant::now() + GROUP_GONE;
        // SAFETY: kill with signal 0 only checks.
        while unsafe { libc::kill(-group, 0) } == 0 && Instant::now() < until {
            thread::sleep(Duration::from_millis(1));
        }
        let mut state = self.runner.lock();
        let status = self.child.wait().expect("a tool SIFS started and has not reaped can be reaped");
        state.running.remove(&group);
        let stopping = state.phase != Phase::Open;
        drop(state);
        self.runner.changed.notify_all();
        (status, stopping)
    }
}

impl Drop for Started<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.end();
        }
    }
}

fn group(child: &Child) -> libc::pid_t {
    child.id() as libc::pid_t
}

fn kill_group(group: libc::pid_t) {
    // SAFETY: kill has no memory effects; a negative pid names a group.
    unsafe { libc::kill(-group, libc::SIGKILL) };
}

/// Whether the tool has exited, leaving it unreaped.
fn exited(pid: libc::pid_t) -> bool {
    // SAFETY: waitid writes only into info.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    match unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) } {
        0 => info.si_pid != 0,
        _ => std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted,
    }
}

/// A tool's two outputs, read as it prints them.
struct Output {
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    out: Vec<u8>,
    err: Vec<u8>,
}

impl Output {
    /// Reads until the tool exits. Past the deadline, or once its standard
    /// output passes the cap, its group is killed. Gives whether the
    /// deadline ended it.
    fn watch(&mut self, group: libc::pid_t, deadline: Instant) -> bool {
        let mut killed = false;
        let mut timed_out = false;
        while !exited(group) {
            let left = deadline.saturating_duration_since(Instant::now());
            if !killed && left.is_zero() {
                kill_group(group);
                killed = true;
                timed_out = true;
            }
            self.read(if killed { TICK } else { TICK.min(left) });
            if !killed && self.out.len() > STDOUT_CAP {
                kill_group(group);
                killed = true;
            }
        }
        timed_out
    }

    /// Reads what is left once the tool's group is gone, without waiting on
    /// a process outside the group that still holds an output open.
    fn drain(&mut self) {
        let until = Instant::now() + TICK * 10;
        while self.read(Duration::ZERO) && Instant::now() < until {}
    }

    /// Waits up to wait for either output, and reads once from each that is
    /// ready. Gives whether there was anything to read.
    fn read(&mut self, wait: Duration) -> bool {
        let mut fds: Vec<libc::pollfd> = [self.stdout.as_ref().map(|s| s.as_raw_fd()), self.stderr.as_ref().map(|s| s.as_raw_fd())]
            .into_iter()
            .flatten()
            .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
            .collect();
        if fds.is_empty() {
            thread::sleep(wait);
            return false;
        }
        let ms = wait.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: fds is a live slice of pollfd of the given length.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) } <= 0 {
            return false;
        }
        let ready = |fd: Option<i32>| fd.is_some_and(|fd| fds.iter().any(|p| p.fd == fd && p.revents != 0));
        let mut buf = [0u8; 64 << 10];
        if ready(self.stdout.as_ref().map(|s| s.as_raw_fd())) {
            if let Some(n) = read_or_close(&mut self.stdout, &mut buf) {
                self.out.extend_from_slice(&buf[..n]);
                self.out.truncate(STDOUT_CAP + 1);
            }
        }
        if ready(self.stderr.as_ref().map(|s| s.as_raw_fd())) {
            if let Some(n) = read_or_close(&mut self.stderr, &mut buf) {
                self.err.extend_from_slice(&buf[..n]);
                let over = self.err.len().saturating_sub(STDERR_KEPT);
                self.err.drain(..over);
            }
        }
        true
    }
}

/// One read from a ready pipe: the bytes read, or None at its end, which
/// closes it.
fn read_or_close(pipe: &mut Option<impl Read>, buf: &mut [u8]) -> Option<usize> {
    match pipe.as_mut()?.read(buf) {
        Ok(0) => {
            *pipe = None;
            None
        }
        Ok(n) => Some(n),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Some(0),
        Err(_) => {
            *pipe = None;
            None
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
