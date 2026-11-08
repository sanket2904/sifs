//! PDFs, page by page, through PDFKit. Each PDF is read in a helper process
//! (helper.rs), because one PDFKit call can't be interrupted and may run
//! for minutes or take every byte of memory (plan, step 2b). SIFS kills the
//! helper at the time limit, past its processor time, memory or output cap,
//! and keeps the pages it had sent.

mod frame;
mod helper;

#[cfg(test)]
mod sample;
#[cfg(test)]
mod tests;

pub use helper::serve_if_asked;

use crate::extract::readers::office::PASSWORD;
use crate::extract::{describe_size, describe_time, fact, Limits, ReadError, Reader, Sink, MB};
use frame::{Decoder, Frame};
use std::ffi::OsString;
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) const NOT_A_PDF: &str = "not a PDF";
pub(crate) const DAMAGED: &str = "damaged, or not a PDF PDFKit can read";
pub(crate) const CRASHED: &str = "PDFKit crashed reading it";

/// What a PDF read may take on, on top of the worker's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Pages read, in order; the rest are left out.
    pub pages: usize,
    /// The helper's physical footprint: past it, it is killed.
    pub memory: u64,
    /// The helper's processor time: past it, it is killed. A page that never
    /// ends uses it all; a helper starved by a busy Mac uses little, and has
    /// the reader's longer time limit.
    pub cpu: Duration,
    /// Bytes read from the helper: past it, it is killed.
    pub output: usize,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps { pages: 10_000, memory: 1024 * MB, cpu: Duration::from_secs(10), output: 8 * MB as usize }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Helper {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

impl Default for Helper {
    fn default() -> Helper {
        // The test binary would run every test again, not the helper,
        // without the arguments that pick the helper's test.
        #[cfg(test)]
        let args = tests::helper_args();
        #[cfg(not(test))]
        let args = Vec::new();
        Helper { program: std::env::current_exe().unwrap_or_default(), args, env: Vec::new() }
    }
}

/// PDFs are opened up to this size. PDFKit maps the file rather than
/// reading it, and the helper's caps bound the rest; the owner's largest
/// PDF is a 207 MB scan.
pub const MAX_BYTES: u64 = 512 * MB;

/// The time limit, by the clock. The processor time cap (10 s) stops a page
/// that never ends; this longer limit lets a helper finish on a busy Mac,
/// where it runs last (a file taking 0.2 s alone took over 10 s there).
pub const TIME: Duration = Duration::from_secs(60);

const POLL: Duration = Duration::from_millis(20);

/// The helper ends itself this long after SIFS's deadline, should SIFS not
/// have killed it.
pub(crate) const HELPER_SLACK: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Pdf {
    pub caps: Caps,
    pub(crate) helper: Helper,
}

impl Reader for Pdf {
    fn id(&self) -> &'static str {
        "pdf"
    }

    fn version(&self) -> u32 {
        1
    }

    fn exts(&self) -> &'static [&'static str] {
        &["pdf"]
    }

    fn limits(&self) -> Limits {
        Limits { max_bytes: MAX_BYTES, time: TIME, ..Limits::default() }
    }

    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        match header(path)? {
            Header::Empty => return Ok(()),
            Header::NotPdf => return Err(ReadError::Unsupported(NOT_A_PDF.into())),
            Header::Pdf => {}
        }
        let mut run = Run::start(&self.helper, path, self.caps, self.limits().max_chars, sink.deadline())?;
        let stop = run.supervise(sink, self.caps);
        let timed_out = stop == Stop::Time;
        let why = match stop {
            Stop::End => return Ok(()),
            Stop::Locked => return Err(ReadError::Unsupported(PASSWORD.into())),
            Stop::Unopenable => return Err(ReadError::Other(DAMAGED.into())),
            Stop::Time => format!("took longer than {}", describe_time(self.limits().time)),
            Stop::Memory => format!("used more than {} of memory", describe_size(self.caps.memory)),
            Stop::Cpu => format!("used more than {} of processor time", describe_time(self.caps.cpu)),
            Stop::Output => format!("sent more than {}", describe_size(self.caps.output as u64)),
            Stop::Crash(why) => why,
        };
        match run.last_page {
            Some(page) => {
                sink.fact(fact::STOPPED, format!("{why}, after page {}", page as u64 + 1));
                sink.cut();
                Ok(())
            }
            None if timed_out => Err(ReadError::Timeout),
            None => Err(ReadError::Other(why)),
        }
    }
}

enum Header {
    Empty,
    NotPdf,
    Pdf,
}

/// Whether the file starts like a PDF: `%PDF-` in its first 1,024 bytes,
/// where PDFKit looks for it.
fn header(path: &Path) -> Result<Header, ReadError> {
    let mut head = Vec::with_capacity(1024);
    std::fs::File::open(path)?.take(1024).read_to_end(&mut head)?;
    Ok(if head.is_empty() {
        Header::Empty
    } else if head.windows(5).any(|w| w == b"%PDF-") {
        Header::Pdf
    } else {
        Header::NotPdf
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Stop {
    End,
    Locked,
    Unopenable,
    Time,
    Memory,
    Cpu,
    Output,
    Crash(String),
}

struct Run {
    child: Child,
    last_page: Option<u32>,
    deadline: Instant,
}

impl Run {
    fn start(helper: &Helper, path: &Path, caps: Caps, chars: usize, deadline: Instant) -> Result<Run, ReadError> {
        let millis = deadline.saturating_duration_since(Instant::now()) + HELPER_SLACK;
        let child = Command::new(&helper.program)
            .args(&helper.args)
            .envs(helper.env.iter().map(|(k, v)| (k, v)))
            .env(helper::PATH, path)
            .env(helper::PAGES, caps.pages.to_string())
            .env(helper::CHARS, chars.to_string())
            .env(helper::MILLIS, millis.as_millis().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ReadError::Other(format!("couldn't start the PDF reader: {e}")))?;
        set_nonblocking(child.stdout.as_ref().unwrap())?;
        Ok(Run { child, last_page: None, deadline })
    }

    fn supervise(&mut self, sink: &mut Sink, caps: Caps) -> Stop {
        let mut stdout = self.child.stdout.take().unwrap();
        let mut decoder = Decoder::default();
        let mut buf = vec![0u8; 64 * 1024];
        let mut received = 0usize;
        loop {
            if Instant::now() >= self.deadline {
                return Stop::Time;
            }
            let usage = usage(self.child.id());
            if usage.footprint > caps.memory {
                return Stop::Memory;
            }
            if usage.cpu > caps.cpu {
                return Stop::Cpu;
            }
            let mut closed = false;
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        received += n;
                        if received > caps.output {
                            return Stop::Output;
                        }
                        let frames = match decoder.feed(&buf[..n]) {
                            Ok(frames) => frames,
                            Err(why) => return Stop::Crash(format!("the PDF reader sent {why}")),
                        };
                        for frame in frames {
                            match self.take(frame, sink) {
                                Ok(None) => {}
                                Ok(Some(stop)) => return stop,
                                Err(ReadError::Timeout) => return Stop::Time,
                                Err(e) => return Stop::Crash(format!("{e:?}")),
                            }
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Stop::Crash(format!("reading from the PDF reader: {e}")),
                }
            }
            if closed {
                return self.why_it_ended();
            }
            let left = self.deadline.saturating_duration_since(Instant::now()).min(POLL);
            let mut fd = libc::pollfd { fd: stdout.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut fd, 1, left.as_millis() as libc::c_int) };
        }
    }

    fn take(&mut self, frame: Frame, sink: &mut Sink) -> Result<Option<Stop>, ReadError> {
        match frame {
            Frame::Pages(n) => sink.fact(fact::PAGES, n.to_string()),
            Frame::Fact(key, value) => match key.as_str() {
                fact::TITLE => sink.fact(fact::TITLE, value),
                fact::AUTHOR => sink.fact(fact::AUTHOR, value),
                fact::TEXTLESS_PAGES => sink.fact(fact::TEXTLESS_PAGES, value),
                _ => {}
            },
            Frame::Page(index, text) => {
                sink.part(format!("page {}", index as u64 + 1));
                sink.text(&text)?;
                self.last_page = Some(index);
            }
            Frame::Cut => {
                sink.cut();
            }
            Frame::Locked => return Ok(Some(Stop::Locked)),
            Frame::Unopenable => return Ok(Some(Stop::Unopenable)),
            Frame::End => return Ok(Some(Stop::End)),
        }
        Ok(None)
    }

    fn why_it_ended(&mut self) -> Stop {
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    use std::os::unix::process::ExitStatusExt;
                    return Stop::Crash(match status.signal() {
                        Some(_) => CRASHED.into(),
                        None => format!("the PDF reader stopped early ({status})"),
                    });
                }
                Ok(None) if Instant::now() >= self.deadline => return Stop::Time,
                Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => return Stop::Crash(format!("the PDF reader: {e}")),
            }
        }
    }

}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn set_nonblocking(fd: &impl AsRawFd) -> Result<(), ReadError> {
    let fd = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ReadError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[derive(Default)]
struct Usage {
    /// What Activity Monitor shows as the process's memory.
    footprint: u64,
    cpu: Duration,
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
