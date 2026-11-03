//! Reading inside files: readers that turn a file into text and facts, the
//! queue of files waiting to be read (kept in the index by triggers on
//! files), and the background worker that reads them.
//!
//! Each reader handles some extensions. The registry of readers is written
//! to the index at startup (extract_kinds), so the queue only holds files
//! some reader on this machine can read.

pub mod decode;
pub mod platform;
pub mod readers;
pub mod store;
pub mod worker;

#[cfg(test)]
mod tests;

pub use store::{Failure, Progress};
pub use worker::{RunStats, Waker, Worker, WorkerOptions};

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where a reader's files wait in the queue: documents first, then images,
/// then the rest (code, data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Document,
    Image,
    Other,
}

impl Class {
    pub fn priority(self) -> i64 {
        match self {
            Class::Document => 1,
            Class::Image => 2,
            Class::Other => 3,
        }
    }
}

pub const MB: u64 = 1024 * 1024;
/// Characters kept of one fact's value.
pub const MAX_FACT_CHARS: usize = 1000;

/// How much a reader may take on.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Larger files are skipped without being opened.
    pub max_bytes: u64,
    /// A read taking longer is recorded failed, and its thread abandoned.
    pub time: Duration,
    /// Text kept per file; the rest is cut, and a truncated fact recorded.
    pub max_chars: usize,
    /// At most one read of this reader at a time (OCR, later).
    pub exclusive: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_bytes: 4 * MB, time: Duration::from_secs(10), max_chars: 1_000_000, exclusive: false }
    }
}

/// Facts are (key, value) pairs; these are the keys, in one place.
pub mod fact {
    pub const TITLE: &str = "title";
    pub const AUTHOR: &str = "author";
    /// The text's encoding, when it was not UTF-8.
    pub const ENCODING: &str = "encoding";
    /// "1" when the text was cut at the reader's max_chars.
    pub const TRUNCATED: &str = "truncated";
    /// A table's column names, from its header row, joined by ", ".
    pub const COLUMNS: &str = "columns";
    /// The number of lines; for a table, its rows below the header.
    pub const LINES: &str = "lines";
}

pub trait Reader: Send + Sync + 'static {
    /// Stored with what it read, so a different reader reads it again.
    fn id(&self) -> &'static str;
    /// Raised when the reader reads better: its files are read again.
    fn version(&self) -> u32;
    /// Lowercase, as in files.ext; "" for files without an extension.
    fn exts(&self) -> &'static [&'static str];
    fn class(&self) -> Class {
        Class::Document
    }
    fn limits(&self) -> Limits {
        Limits::default()
    }
    /// Whether it reads packages, which the index holds as folders.
    fn packages(&self) -> bool {
        false
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError>;
}

#[derive(Debug)]
pub enum ReadError {
    /// Not text: recorded as skipped.
    Binary,
    /// Over max_bytes: recorded as skipped.
    TooLarge,
    /// Past the time limit: recorded as failed.
    Timeout,
    /// Recorded as skipped, with the reason.
    Unsupported(String),
    Io(std::io::Error),
    Other(String),
}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> ReadError {
        ReadError::Io(e)
    }
}

/// One page, sheet, slide or part of a file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub part: u32,
    pub label: String,
    pub text: String,
}

/// Whether a reader should keep going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    More,
    /// The character cap is reached: anything more is dropped.
    Full,
}

/// Where a reader puts what it reads. It keeps the character cap and the
/// time limit, so readers needn't.
pub struct Sink {
    parts: Vec<Part>,
    facts: Vec<(String, String)>,
    chars: usize,
    max_chars: usize,
    deadline: Instant,
    truncated: bool,
}

impl Sink {
    pub fn new(max_chars: usize, deadline: Instant) -> Sink {
        Sink { parts: Vec::new(), facts: Vec::new(), chars: 0, max_chars, deadline, truncated: false }
    }

    /// Starts a page, sheet, slide or part. Text before the first part goes
    /// in an unlabelled one.
    pub fn part(&mut self, label: impl Into<String>) {
        let part = self.parts.len() as u32;
        self.parts.push(Part { part, label: label.into(), text: String::new() });
    }

    /// Adds text to the current part, cut on a character boundary at the cap.
    pub fn text(&mut self, s: &str) -> Result<Flow, ReadError> {
        self.check()?;
        if self.truncated {
            return Ok(Flow::Full);
        }
        if self.parts.is_empty() {
            self.part("");
        }
        let room = self.max_chars - self.chars;
        let (kept, flow) = match s.char_indices().nth(room) {
            Some((cut, _)) => {
                self.truncated = true;
                (&s[..cut], Flow::Full)
            }
            None => (s, Flow::More),
        };
        self.chars += if flow == Flow::Full { room } else { kept.chars().count() };
        self.parts.last_mut().unwrap().text.push_str(kept);
        Ok(flow)
    }

    /// Records a fact, its value cut at MAX_FACT_CHARS characters.
    pub fn fact(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        let value = value.trim();
        let value = match value.char_indices().nth(MAX_FACT_CHARS) {
            Some((cut, _)) => &value[..cut],
            None => value,
        };
        if !value.is_empty() {
            self.facts.push((key.to_string(), value.to_string()));
        }
    }

    /// Fails once the time limit has passed; long loops call it as they go.
    pub fn check(&self) -> Result<(), ReadError> {
        if Instant::now() > self.deadline {
            Err(ReadError::Timeout)
        } else {
            Ok(())
        }
    }

    pub fn chars(&self) -> usize {
        self.chars
    }

    /// The parts with text, and the facts.
    pub fn finish(mut self) -> (Vec<Part>, Vec<(String, String)>) {
        if self.truncated {
            self.facts.push((fact::TRUNCATED.to_string(), "1".to_string()));
        }
        let parts = self.parts.into_iter().filter(|p| !p.text.trim().is_empty()).collect();
        (parts, self.facts)
    }
}

/// The readers on this machine, by extension.
#[derive(Clone)]
pub struct Registry {
    by_ext: HashMap<&'static str, Arc<dyn Reader>>,
}

impl Registry {
    /// The readers this platform has.
    pub fn platform() -> Registry {
        #[allow(unused_mut)]
        let mut registry = Registry::with(readers::all());
        #[cfg(target_os = "macos")]
        readers::macos::register(&mut registry);
        registry
    }

    pub fn with(readers: Vec<Arc<dyn Reader>>) -> Registry {
        let mut registry = Registry { by_ext: HashMap::new() };
        for reader in readers {
            registry.register(reader);
        }
        registry
    }

    /// Adds a reader; it takes its extensions over from any earlier one.
    pub fn register(&mut self, reader: Arc<dyn Reader>) {
        for ext in reader.exts() {
            self.by_ext.insert(ext, reader.clone());
        }
    }

    pub fn get(&self, ext: &str) -> Option<&Arc<dyn Reader>> {
        self.by_ext.get(ext)
    }

    /// (ext, reader) for every extension, sorted.
    pub fn kinds(&self) -> Vec<(&'static str, &Arc<dyn Reader>)> {
        let mut kinds: Vec<_> = self.by_ext.iter().map(|(e, r)| (*e, r)).collect();
        kinds.sort_by_key(|(e, _)| *e);
        kinds
    }

    /// Changes whenever a reader, its version or its extensions change,
    /// which is when the files already indexed are checked against them.
    pub fn signature(&self) -> String {
        self.kinds()
            .iter()
            .map(|(ext, r)| format!("{ext}={}:{}:{}:{}", r.id(), r.version(), r.class().priority(), r.packages() as u8))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// "4 MB", "512 KB": for reasons shown to the user.
pub(crate) fn describe_size(bytes: u64) -> String {
    if bytes >= MB && bytes % MB == 0 {
        format!("{} MB", bytes / MB)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

/// "10 s", "200 ms".
pub(crate) fn describe_time(time: Duration) -> String {
    if time.as_millis() % 1000 == 0 {
        format!("{} s", time.as_secs())
    } else {
        format!("{} ms", time.as_millis())
    }
}
