//! What the readers that run in a helper process share: starting SIFS's own
//! program as the helper, the guard that kills and reaps it, the frames it
//! sends back, and the loop that watches it. Each reader keeps its own
//! frames, the order it allows them in, its caps, and what a stop means.

use crate::extract::ReadError;
use std::ffi::OsString;
use std::io::{ErrorKind, Read};
use std::ops::{Deref, DerefMut};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How often a helper's processor time and memory are checked.
pub(super) const POLL: Duration = Duration::from_millis(20);

/// A helper ends itself this long after SIFS's deadline, should SIFS not
/// have killed it.
pub(crate) const HELPER_SLACK: Duration = Duration::from_secs(1);

/// The program to start as a helper.
#[derive(Debug, Clone)]
pub(crate) struct Helper {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

impl Helper {
    /// The program SIFS is running. The test binary needs args that pick the
    /// helper's test, or it would run every test again.
    pub(super) fn this_program(args: Vec<OsString>) -> Helper {
        Helper { program: std::env::current_exe().unwrap_or_default(), args, env: Vec::new() }
    }
}

/// A running helper. Made before any setup that can fail, and killed and
/// reaped when dropped, however a read ends.
pub(super) struct Process(Child);

impl Deref for Process {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// How a helper's output ended before its last frame.
#[derive(Debug)]
pub(super) enum Ended {
    /// It was still running, and was killed.
    Killed,
    /// It ended on a signal of its own.
    Crashed,
    Exited(ExitStatus),
    Lost(std::io::Error),
}

/// Why watching a helper stopped.
#[derive(Debug)]
pub(super) enum Watched<S> {
    /// The reader's frame handler stopped it.
    Taken(S),
    Time,
    Cpu,
    Memory,
    Ended(Ended),
    Unreadable(std::io::Error),
}

/// What a helper may take on while it is watched.
pub(super) struct Watch {
    pub deadline: Instant,
    pub cpu: Duration,
    pub memory: u64,
    /// The processor time it had used before: a helper that lives on is
    /// counted from the start of each request.
    pub cpu_before: Duration,
}

impl Process {
    /// Starts the helper with its stdin and stdout piped; setup adds what the
    /// reader passes it. name is the reader as the user reads it ("the PDF
    /// reader").
    pub(super) fn start(helper: &Helper, name: &str, setup: impl FnOnce(&mut Command)) -> Result<Process, ReadError> {
        let mut command = Command::new(&helper.program);
        command.args(&helper.args).envs(helper.env.iter().map(|(k, v)| (k, v)));
        setup(&mut command);
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ReadError::Other(format!("couldn't start {name}: {e}")))?;
        let process = Process(child);
        set_nonblocking(process.0.stdout.as_ref().unwrap())?;
        Ok(process)
    }

    /// Its memory and processor time so far.
    pub(super) fn usage(&self) -> Usage {
        usage(self.0.id())
    }

    /// Hands what the helper sends to take until take says stop, checking
    /// the clock, processor time and memory on every turn, whatever the pipe
    /// says. Its output ending first kills and reaps it at once.
    pub(super) fn watch<S>(&mut self, watch: &Watch, take: impl FnMut(&[u8]) -> Option<S>) -> Watched<S> {
        let mut stdout = self.0.stdout.take().unwrap();
        let watched = self.watch_output(&mut stdout, watch, take);
        self.0.stdout = Some(stdout);
        watched
    }

    fn watch_output<S>(
        &mut self,
        stdout: &mut ChildStdout,
        watch: &Watch,
        mut take: impl FnMut(&[u8]) -> Option<S>,
    ) -> Watched<S> {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if Instant::now() >= watch.deadline {
                return Watched::Time;
            }
            let usage = self.usage();
            if usage.footprint > watch.memory {
                return Watched::Memory;
            }
            if usage.cpu.saturating_sub(watch.cpu_before) > watch.cpu {
                return Watched::Cpu;
            }
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => return Watched::Ended(self.ended_early()),
                    Ok(n) => {
                        if let Some(stop) = take(&buf[..n]) {
                            return Watched::Taken(stop);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Watched::Unreadable(e),
                }
            }
            let left = watch.deadline.saturating_duration_since(Instant::now()).min(POLL);
            let mut fd = libc::pollfd { fd: stdout.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut fd, 1, left.as_millis() as libc::c_int) };
        }
    }

    /// The output ended before the helper's last frame: killed and reaped
    /// at once, so it can't go on unwatched. Its stdin stays open until it
    /// is reaped (wait would close it first): a closed stdin can end the
    /// helper on its own before the kill lands, hiding that it was killed.
    pub(super) fn ended_early(&mut self) -> Ended {
        use std::os::unix::process::ExitStatusExt;
        let _stdin = self.0.stdin.take();
        let _ = self.0.kill();
        match self.0.wait() {
            Ok(status) if status.signal() == Some(libc::SIGKILL) => Ended::Killed,
            Ok(status) if status.signal().is_some() => Ended::Crashed,
            Ok(status) => Ended::Exited(status),
            Err(e) => Ended::Lost(e),
        }
    }
}

/// A frame: a tag byte, a little-endian u32 length and that many bytes.
pub(super) fn encode(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + body.len());
    out.push(tag);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Frames from a pipe, in whatever pieces it gives them. Anything before
/// the marker is passed over (the test binary, run as the helper, prints a
/// line first).
pub(super) struct Decoder<F> {
    marker: &'static [u8],
    decode: fn(u8, &[u8]) -> Result<F, String>,
    buf: Vec<u8>,
    /// Where the next frame starts in buf.
    at: usize,
    synced: bool,
}

impl<F> Decoder<F> {
    pub(super) fn new(marker: &'static [u8], decode: fn(u8, &[u8]) -> Result<F, String>) -> Decoder<F> {
        Decoder { marker, decode, buf: Vec::new(), at: 0, synced: marker.is_empty() }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.buf.drain(..self.at);
        self.at = 0;
        self.buf.extend_from_slice(bytes);
        if self.synced {
            return;
        }
        match self.buf.windows(self.marker.len()).position(|w| w == self.marker) {
            Some(at) => {
                self.at = at + self.marker.len();
                self.synced = true;
            }
            None => {
                let keep = self.buf.len().min(self.marker.len() - 1);
                self.buf.drain(..self.buf.len() - keep);
            }
        }
    }

    /// The next whole frame, if it has all come. A frame that can't be
    /// decoded is an error, after every frame before it.
    pub(super) fn next_frame(&mut self) -> Result<Option<F>, String> {
        let rest = &self.buf[self.at..];
        if !self.synced || rest.len() < 5 {
            return Ok(None);
        }
        let len = u32::from_le_bytes(rest[1..5].try_into().unwrap()) as usize;
        if rest.len() - 5 < len {
            return Ok(None);
        }
        let frame = (self.decode)(rest[0], &rest[5..5 + len])?;
        self.at += 5 + len;
        Ok(Some(frame))
    }
}

/// Why taking bytes stopped.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Taken<S> {
    /// The reader's frame handler stopped it.
    Stop(S),
    /// A frame that can't be decoded, and why.
    Bad(String),
    Output,
}

/// A helper's output: the decoder, and the bytes counted against the
/// output cap.
pub(super) struct Intake<F> {
    pub decoder: Decoder<F>,
    bytes: usize,
}

impl<F> Intake<F> {
    pub(super) fn new(decoder: Decoder<F>) -> Intake<F> {
        Intake { decoder, bytes: 0 }
    }

    /// Hands each whole frame within the output cap to handle, until it
    /// says stop; then a frame that can't be decoded, then the cap, stop
    /// it.
    pub(super) fn take<S>(&mut self, bytes: &[u8], cap: usize, mut handle: impl FnMut(F) -> Option<S>) -> Option<Taken<S>> {
        let room = cap - self.bytes;
        let allowed = &bytes[..bytes.len().min(room)];
        self.bytes += allowed.len();
        self.decoder.push(allowed);
        loop {
            match self.decoder.next_frame() {
                Ok(Some(frame)) => {
                    if let Some(stop) = handle(frame) {
                        return Some(Taken::Stop(stop));
                    }
                }
                Ok(None) => break,
                Err(why) => return Some(Taken::Bad(why)),
            }
        }
        (bytes.len() > room).then_some(Taken::Output)
    }
}

pub(super) fn set_nonblocking(fd: &impl AsRawFd) -> Result<(), ReadError> {
    let fd = fd.as_raw_fd();
    let flags = retrying(|| unsafe { libc::fcntl(fd, libc::F_GETFL) });
    if flags < 0 || retrying(|| unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) }) < 0 {
        return Err(ReadError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn retrying(mut call: impl FnMut() -> libc::c_int) -> libc::c_int {
    loop {
        let got = call();
        if got >= 0 || std::io::Error::last_os_error().kind() != ErrorKind::Interrupted {
            return got;
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Usage {
    /// What Activity Monitor shows as the process's memory.
    pub footprint: u64,
    pub cpu: Duration,
}

/// A process's memory and processor time so far; nothing once it is gone.
fn usage(pid: u32) -> Usage {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let got = unsafe {
        libc::proc_pid_rusage(pid as libc::c_int, libc::RUSAGE_INFO_V2, (&mut info as *mut libc::rusage_info_v2).cast())
    };
    if got != 0 {
        return Usage::default();
    }
    // The times are in Mach ticks, which are not nanoseconds on Apple silicon.
    let mut base = Timebase { numer: 0, denom: 0 };
    unsafe { mach_timebase_info(&mut base) };
    let ticks = (info.ri_user_time + info.ri_system_time) as u128;
    let nanos = ticks * base.numer as u128 / base.denom.max(1) as u128;
    Usage { footprint: info.ri_phys_footprint, cpu: Duration::from_nanos(nanos.min(u64::MAX as u128) as u64) }
}

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

extern "C" {
    // <mach/mach_time.h>. libc's binding is deprecated in favour of the
    // mach2 crate, which this one call doesn't justify.
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}
