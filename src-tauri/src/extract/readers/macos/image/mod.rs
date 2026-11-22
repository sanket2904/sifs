//! Images and screenshots: the words in them, through Vision's text
//! recognition, and when, where and with what they were taken. Each image
//! is read by a helper process (helper.rs) that stays running between
//! images, because Vision loads its models in each new process, which
//! takes longer than most images (plan, step 4). SIFS kills the helper past
//! an image's processor time, memory or output cap, at the time limit, or
//! when its output ends early, and keeps the facts it had sent. Between
//! images a watcher thread ends it at its idle time, or should it exit or
//! use memory or processor time while it waits.

mod facts;
mod frame;
mod helper;

#[cfg(test)]
mod draw;
#[cfg(test)]
mod tests;

pub use helper::serve_if_asked;

use super::supervise::{Ended, Helper, Intake, Process, Taken, Watch, Watched, HELPER_SLACK, POLL};
use crate::extract::{describe_size, describe_time, fact, Class, Limits, ReadError, Reader, Sink, MB};
use frame::{Frame, Outcome, Request};
use helper::Exit;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub(crate) const DAMAGED: &str = "damaged, or not an image macOS can read";
pub(crate) const CRASHED: &str = "the image reader crashed";

/// What an image read may take on, on top of the worker's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Characters of text kept per image.
    pub chars: usize,
    /// Larger pictures, by their header, aren't decoded.
    pub pixels: u64,
    /// The helper's processor time for one image: past it, it is killed.
    pub cpu: Duration,
    /// The helper's physical footprint: past it, it is killed.
    pub memory: u64,
    /// Bytes read from the helper for one image: past it, it is killed.
    pub output: usize,
    /// The helper is ended after this long without an image.
    pub idle: Duration,
    /// A helper using more memory than this after an image, or while it
    /// waits for the next, is replaced.
    pub keep: u64,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            chars: 100_000,
            pixels: 200_000_000,
            cpu: Duration::from_secs(10),
            memory: 1024 * MB,
            output: MB as usize,
            idle: Duration::from_secs(30),
            keep: 512 * MB,
        }
    }
}

/// Images are opened up to this size.
pub const MAX_BYTES: u64 = 256 * MB;

/// The time limit, by the clock. The processor time cap stops an image
/// that never ends; this longer limit lets one finish on a busy Mac, where
/// images are read last.
pub const TIME: Duration = Duration::from_secs(60);

pub struct Image {
    pub caps: Caps,
    pub(crate) helper: Helper,
    running: Slot,
}

/// The helper, from its start until it is killed and reaped. Whoever holds
/// the lock watches it: a read while it serves an image, its watcher thread
/// while it waits for the next. Dropped with the reader, it ends the helper.
#[derive(Default)]
struct Slot(Arc<Mutex<Option<Running>>>);

impl Slot {
    fn lock(&self) -> MutexGuard<'_, Option<Running>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        *self.lock() = None;
    }
}

impl Default for Image {
    fn default() -> Image {
        #[cfg(test)]
        let args = tests::helper_args();
        #[cfg(not(test))]
        let args = Vec::new();
        Image { caps: Caps::default(), helper: Helper::this_program(args), running: Slot::default() }
    }
}

impl Reader for Image {
    fn id(&self) -> &'static str {
        "image"
    }

    fn version(&self) -> u32 {
        1
    }

    fn exts(&self) -> &'static [&'static str] {
        &[
            "jpg", "jpeg", "jpe", "png", "gif", "heic", "heif", "hif", "heics", "avif", "webp", "tiff", "tif", "bmp", "dib",
            "jp2", "jxl", "psd", "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "rw2", "orf", "pef",
            "srw", "erf", "dcr", "iiq", "3fr", "fff", "mos", "mrw", "rwl",
        ]
    }

    fn class(&self) -> Class {
        Class::Image
    }

    fn limits(&self) -> Limits {
        Limits { max_bytes: MAX_BYTES, time: TIME, max_chars: self.caps.chars, exclusive: true }
    }

    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let mut running = self.running.lock();
        let mut read = self.request(&mut running, path, sink)?;
        if read.stop == Stop::Idle {
            read = self.request(&mut running, path, sink)?;
        }
        let ended = matches!(read.stop, Stop::End(_));
        let why = match read.stop {
            Stop::End(Outcome::Read | Outcome::Small) => return Ok(()),
            Stop::End(Outcome::CantOpen) => return Err(ReadError::Other(DAMAGED.into())),
            Stop::End(Outcome::TooManyPixels) => format!("larger than {} megapixels", self.caps.pixels / 1_000_000),
            Stop::End(Outcome::RecognitionFailed(why)) => format!("text recognition failed: {why}"),
            Stop::Time => format!("took longer than {}", describe_time(self.limits().time)),
            Stop::Cpu => format!("used more than {} of processor time", describe_time(self.caps.cpu)),
            Stop::Memory => format!("used more than {} of memory", describe_size(self.caps.memory)),
            Stop::Output => format!("sent more than {}", describe_size(self.caps.output as u64)),
            Stop::Crashed | Stop::Idle => CRASHED.into(),
            Stop::Broken(why) => why,
        };
        if read.received || ended {
            sink.fact(fact::STOPPED, why);
            Ok(())
        } else {
            Err(ReadError::Other(why))
        }
    }

    /// Kills and reaps the helper, once any read using it ends. The next
    /// read starts another.
    fn stop(&self) {
        *self.running.lock() = None;
    }
}

impl Image {
    /// One request, on the helper there is or a new one. A helper that
    /// stopped, or must go after it, is gone after.
    fn request(&self, slot: &mut Option<Running>, path: &Path, sink: &mut Sink) -> Result<Read, ReadError> {
        if slot.as_mut().is_some_and(|running| running.must_go(self.caps)) {
            *slot = None;
        }
        let running = match slot {
            Some(running) => running,
            None => slot.insert(self.start()?),
        };
        let read = running.request(path, sink, self.caps);
        let ended = matches!(read.stop, Stop::End(_));
        if ended {
            running.wait_from_now();
        }
        if !ended || running.must_go(self.caps) {
            *slot = None;
        }
        Ok(read)
    }

    /// A new helper, and the thread that watches it between images.
    fn start(&self) -> Result<Running, ReadError> {
        let running = Running::start(&self.helper, self.caps)?;
        let (slot, started, caps) = (self.running.0.clone(), running.started, self.caps);
        std::thread::Builder::new()
            .name("sifs-image-helper".into())
            .spawn(move || watch_between_images(&slot, started, caps))
            .map_err(|e| ReadError::Other(format!("couldn't watch the image reader: {e}")))?;
        Ok(running)
    }
}

/// Watches the helper started as started while it waits between images
/// (a read holds the lock while it serves one), until it leaves the slot.
fn watch_between_images(slot: &Mutex<Option<Running>>, started: u64, caps: Caps) {
    loop {
        std::thread::sleep(POLL);
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        match slot.as_mut() {
            Some(running) if running.started == started => {
                if running.must_go(caps) {
                    *slot = None;
                    return;
                }
            }
            _ => return,
        }
    }
}

/// Why a read stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    End(Outcome),
    Cpu,
    Memory,
    Time,
    Output,
    /// The helper crashed, or its output ended early.
    Crashed,
    /// It broke the protocol, and how.
    Broken(String),
    /// It ended by itself, idle, before sending anything for the image.
    Idle,
}

struct Read {
    stop: Stop,
    /// Whether the image got any of its frames.
    received: bool,
}

/// A helper and its output.
struct Running {
    process: Process,
    intake: Intake<Frame>,
    last_id: u64,
    /// Which helper this is, among all the readers have started.
    started: u64,
    /// When it last finished an image, and its processor time then.
    waiting_since: Instant,
    cpu_then: Duration,
}

static STARTED: AtomicU64 = AtomicU64::new(0);

impl Running {
    fn start(helper: &Helper, caps: Caps) -> Result<Running, ReadError> {
        let process = Process::start(helper, "the image reader", |command| {
            command.env(helper::HELPER, (caps.idle + HELPER_SLACK).as_millis().to_string());
        })?;
        Ok(Running {
            process,
            intake: Intake::new(frame::decoder(caps.chars)),
            last_id: 0,
            started: STARTED.fetch_add(1, Ordering::Relaxed),
            waiting_since: Instant::now(),
            cpu_then: Duration::ZERO,
        })
    }

    fn wait_from_now(&mut self) {
        self.waiting_since = Instant::now();
        self.cpu_then = self.process.usage().cpu;
    }

    /// Whether a helper waiting for an image must go: it has waited its
    /// idle time or ended by itself, or it holds more memory than a kept
    /// helper may, or it has used an image's processor time since its last.
    fn must_go(&mut self, caps: Caps) -> bool {
        if self.waiting_since.elapsed() >= caps.idle || !matches!(self.process.try_wait(), Ok(None)) {
            return true;
        }
        let usage = self.process.usage();
        usage.footprint > caps.keep || usage.cpu.saturating_sub(self.cpu_then) > caps.cpu
    }

    fn request(&mut self, path: &Path, sink: &mut Sink, caps: Caps) -> Read {
        self.last_id += 1;
        let request = Request {
            id: self.last_id,
            path: path.into(),
            deadline_ms: sink.deadline().saturating_duration_since(Instant::now()).as_millis() as u64,
            max_chars: caps.chars,
            max_pixels: caps.pixels,
        };
        let mut order = Order { id: request.id, step: Step::Facts { seen: 0 } };
        let watch = Watch { deadline: sink.deadline(), cpu: caps.cpu, memory: caps.memory, cpu_before: self.process.usage().cpu };
        self.intake.restart();
        let watched = match self.process.send(&watch, &request.encode()) {
            Ok(()) => {
                let intake = &mut self.intake;
                self.process.watch(&watch, |bytes| intake.take(bytes, caps.output, |frame| order.frame(frame, sink)))
            }
            Err(watched) => watched,
        };
        let stop = match watched {
            Watched::Taken(Taken::Stop(Stop::End(_))) if !self.intake.decoder.is_empty() => {
                Stop::Broken("the image reader sent a frame after the end".into())
            }
            Watched::Taken(Taken::Stop(stop)) => stop,
            Watched::Taken(Taken::Bad(why)) => Stop::Broken(format!("the image reader sent {why}")),
            Watched::Taken(Taken::Output) => Stop::Output,
            Watched::Time => Stop::Time,
            Watched::Cpu => Stop::Cpu,
            Watched::Memory => Stop::Memory,
            Watched::Ended(Ended::Exited(status)) if status.code() == Some(Exit::Idle as i32) && !order.received() => {
                Stop::Idle
            }
            Watched::Ended(_) | Watched::Unreadable(_) => Stop::Crashed,
        };
        Read { stop, received: order.received() }
    }
}

/// Where an image's frames are, in the order the protocol allows: facts,
/// each key once, then at most one text, then the end.
struct Order {
    id: u64,
    step: Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The keys of the facts so far, as bits.
    Facts { seen: u8 },
    Text,
}

impl Order {
    fn frame(&mut self, frame: Frame, sink: &mut Sink) -> Option<Stop> {
        let broken = |how: &str| Some(Stop::Broken(format!("the image reader sent {how}")));
        if frame.id() != self.id {
            return broken("a frame for another image");
        }
        match (self.step, frame) {
            (Step::Facts { seen }, Frame::Fact { key, value, .. }) if seen & key.bit() == 0 => {
                self.step = Step::Facts { seen: seen | key.bit() };
                sink.fact(key.name(), value);
                None
            }
            (Step::Facts { .. }, Frame::Fact { .. }) => broken("a fact twice"),
            (Step::Facts { .. }, Frame::Text { text, cut, .. }) => {
                if sink.text(&text).is_err() {
                    return Some(Stop::Time);
                }
                self.step = Step::Text;
                if cut {
                    sink.cut();
                }
                None
            }
            (Step::Text, Frame::Fact { .. }) => broken("a fact after the text"),
            (Step::Text, Frame::Text { .. }) => broken("the text twice"),
            (_, Frame::End { outcome, .. }) => Some(Stop::End(outcome)),
        }
    }

    fn received(&self) -> bool {
        self.step != Step::Facts { seen: 0 }
    }
}
