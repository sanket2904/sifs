//! PDFs, page by page, through PDFKit. Each PDF is read in a helper process
//! (helper.rs), because one PDFKit call can't be interrupted and may run
//! for minutes or take every byte of memory (plan, step 2b). SIFS kills the
//! helper at the time limit, past its processor time, memory or output cap,
//! or when its output ends early, and keeps the pages it had sent.

mod frame;
mod helper;

#[cfg(test)]
mod sample;
#[cfg(test)]
mod tests;

pub use helper::serve_if_asked;

use super::supervise::{Ended, Helper, Intake, Process, Taken, Watch, Watched, HELPER_SLACK};
use crate::extract::readers::office::PASSWORD;
use crate::extract::{describe_size, describe_time, fact, Limits, ReadError, Reader, Sink, MB};
use frame::{Decoder, Frame};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

pub(crate) const NOT_A_PDF: &str = "not a PDF";
pub(crate) const DAMAGED: &str = "damaged, or not a PDF PDFKit can read";
pub(crate) const CRASHED: &str = "PDFKit crashed reading it";
pub(crate) const CLOSED: &str = "the PDF reader closed its output early";

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

/// PDFs are opened up to this size. PDFKit maps the file rather than
/// reading it, and the helper's caps bound the rest; the owner's largest
/// PDF is a 207 MB scan.
pub const MAX_BYTES: u64 = 512 * MB;

/// The time limit, by the clock. The processor time cap (10 s) stops a page
/// that never ends; this longer limit lets a helper finish on a busy Mac,
/// where it runs last (a file taking 0.2 s alone took over 10 s there).
pub const TIME: Duration = Duration::from_secs(60);

pub struct Pdf {
    pub caps: Caps,
    pub(crate) helper: Helper,
}

impl Default for Pdf {
    fn default() -> Pdf {
        #[cfg(test)]
        let args = tests::helper_args();
        #[cfg(not(test))]
        let args = Vec::new();
        Pdf { caps: Caps::default(), helper: Helper::this_program(args) }
    }
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
        match run.received.order.last_page {
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
    child: Process,
    deadline: Instant,
    received: Received,
}

impl Run {
    fn start(helper: &Helper, path: &Path, caps: Caps, chars: usize, deadline: Instant) -> Result<Run, ReadError> {
        let millis = deadline.saturating_duration_since(Instant::now()) + HELPER_SLACK;
        let child = Process::start(helper, "the PDF reader", |command| {
            command
                .env(helper::PATH, path)
                .env(helper::PAGES, caps.pages.to_string())
                .env(helper::CHARS, chars.to_string())
                .env(helper::MILLIS, millis.as_millis().to_string());
        })?;
        Ok(Run { child, deadline, received: Received::default() })
    }

    fn supervise(&mut self, sink: &mut Sink, caps: Caps) -> Stop {
        let stop = self.until_stopped(sink, caps);
        self.received.finish(sink);
        stop
    }

    fn until_stopped(&mut self, sink: &mut Sink, caps: Caps) -> Stop {
        let watch = Watch { deadline: self.deadline, cpu: caps.cpu, memory: caps.memory, cpu_before: Duration::ZERO };
        let received = &mut self.received;
        match self.child.watch(&watch, |bytes| received.take(bytes, sink, caps)) {
            Watched::Taken(stop) => stop,
            Watched::Time => Stop::Time,
            Watched::Cpu => Stop::Cpu,
            Watched::Memory => Stop::Memory,
            Watched::Ended(Ended::Killed) => Stop::Crash(CLOSED.into()),
            Watched::Ended(Ended::Crashed) => Stop::Crash(CRASHED.into()),
            Watched::Ended(Ended::Exited(status)) => Stop::Crash(format!("the PDF reader stopped early ({status})")),
            Watched::Ended(Ended::Lost(e)) => Stop::Crash(format!("the PDF reader: {e}")),
            Watched::Unreadable(e) => Stop::Crash(format!("reading from the PDF reader: {e}")),
        }
    }
}

/// What the helper has sent, taken frame by frame in the order the
/// protocol allows: a page count, then pages in increasing order below it
/// and the page cap, then the end.
struct Received {
    intake: Intake<Frame>,
    order: Order,
}

impl Default for Received {
    fn default() -> Received {
        Received { intake: Intake::new(Decoder::default()), order: Order::default() }
    }
}

impl Received {
    /// Whole frames within the output cap are taken before the cap, or a
    /// frame that can't be decoded, stops the read.
    fn take(&mut self, bytes: &[u8], sink: &mut Sink, caps: Caps) -> Option<Stop> {
        let order = &mut self.order;
        Some(match self.intake.take(bytes, caps.output, |frame| order.frame(frame, sink, caps))? {
            Taken::Stop(stop) => stop,
            Taken::Bad(why) => Stop::Crash(format!("the PDF reader sent {why}")),
            Taken::Output => Stop::Output,
        })
    }

    fn finish(&self, sink: &mut Sink) {
        self.order.finish(sink);
    }
}

#[derive(Default)]
struct Order {
    pages: Option<u32>,
    last_page: Option<u32>,
    textless: u32,
}

impl Order {
    fn frame(&mut self, frame: Frame, sink: &mut Sink, caps: Caps) -> Option<Stop> {
        let Some(pages) = self.pages else {
            return match frame {
                Frame::Pages(n) => {
                    self.pages = Some(n);
                    sink.fact(fact::PAGES, n.to_string());
                    None
                }
                Frame::Locked => Some(Stop::Locked),
                Frame::Unopenable => Some(Stop::Unopenable),
                _ => Some(Stop::Crash("the PDF reader sent output before the page count".into())),
            };
        };
        match frame {
            Frame::Fact(key, value) => match key.as_str() {
                fact::TITLE => sink.fact(fact::TITLE, value),
                fact::AUTHOR => sink.fact(fact::AUTHOR, value),
                _ => {}
            },
            Frame::Page(index, text) => {
                if let Err(why) = self.next_page(index, pages, caps) {
                    return Some(why);
                }
                sink.part(format!("page {}", index as u64 + 1));
                match sink.text(&text) {
                    Ok(_) => {}
                    Err(ReadError::Timeout) => return Some(Stop::Time),
                    Err(e) => return Some(Stop::Crash(format!("{e:?}"))),
                }
            }
            Frame::Textless(index) => {
                if let Err(why) = self.next_page(index, pages, caps) {
                    return Some(why);
                }
                self.textless += 1;
            }
            Frame::Cut => {
                sink.cut();
            }
            Frame::End => return Some(Stop::End),
            Frame::Pages(_) | Frame::Locked | Frame::Unopenable => {
                return Some(Stop::Crash("the PDF reader sent frames out of order".into()))
            }
        }
        None
    }

    fn next_page(&mut self, index: u32, pages: u32, caps: Caps) -> Result<(), Stop> {
        if index >= pages || index as usize >= caps.pages {
            return Err(Stop::Crash("the PDF reader sent a page past the last".into()));
        }
        if self.last_page.is_some_and(|last| index <= last) {
            return Err(Stop::Crash("the PDF reader sent pages out of order".into()));
        }
        self.last_page = Some(index);
        Ok(())
    }

    /// The textless count goes in once, however the read ends: a fact
    /// can't be updated.
    fn finish(&self, sink: &mut Sink) {
        if self.textless > 0 {
            sink.fact(fact::TEXTLESS_PAGES, self.textless.to_string());
        }
    }
}
