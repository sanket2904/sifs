//! The screen's questions to the engine's reader, `sifs-show`: what kinds
//! of thing the store holds, what a word finds, the page of one thing, and
//! which file a fact came from.
//!
//! - A pipe, not a model. The answer is the one JSON document the reader
//!   printed, byte for byte. SIFS checks that it is one document and names
//!   nothing inside it, so a store of any schema is shown with no change
//!   here.
//! - No door to raw values. The reader masks private numbers unless it is
//!   told not to, and nothing here can tell it so: a question is a kind, a
//!   word or an id, and each goes to the reader as the value of its own flag.
//! - No answer has a reason, as a type: [`NoAnswer`], sent to the screen
//!   tagged by `reason`.
//! - Nothing has been read yet when the data folder has no store file. That
//!   is checked before the reader runs, so a question never makes a store.
//! - A fact's sources are document ids. [`source_file`] finds the file the
//!   index holds for one, the same way the text set gave it that id; the
//!   app opens only that file, never a path the screen sent.

use crate::data::DataDir;
use crate::extract::textset;
use crate::tools::{NotFound, Piece, Ran, Runner, Tools};
use serde::{Serialize, Serializer};
use std::ffi::OsString;
use std::time::Duration;

/// The store's file name in the data folder, as the loader names it.
pub const STORE_FILE: &str = "engine.db";
/// How long one question may take.
pub const TIME: Duration = Duration::from_secs(3);
/// The most of what the reader said the screen is given, in bytes.
pub const SAID_KEPT: usize = 1024;

/// What the screen asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Kinds,
    Find(String),
    Page(String),
}

/// One JSON document, exactly as the reader printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer(String);

impl Answer {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// Why a question has no answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum NoAnswer {
    NoTools {
        #[serde(serialize_with = "shown")]
        why: NotFound,
    },
    NothingReadYet,
    TookTooLong,
    /// The reader ran and gave no answer. `said` is the last line it wrote
    /// on standard error when it said why, else what went wrong, in SIFS's
    /// words; at most [`SAID_KEPT`] bytes.
    CouldNotAnswer {
        said: String,
    },
    Quitting,
}

fn shown<S: Serializer>(why: &NotFound, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(why)
}

/// Asks the reader in tools one question about the store in data, within
/// [`TIME`].
pub fn ask(runner: &Runner, tools: &Result<Tools, NotFound>, data: &DataDir, question: &Question) -> Result<Answer, NoAnswer> {
    let tools = tools.as_ref().map_err(|why| NoAnswer::NoTools { why: why.clone() })?;
    if let Ok(false) = data.path().join(STORE_FILE).try_exists() {
        return Err(NoAnswer::NothingReadYet);
    }
    let could_not = |said: String| NoAnswer::CouldNotAnswer { said: kept(said) };
    match runner.run(&tools.path(Piece::Show), arguments(data, question), TIME) {
        Ran::Finished { code: 0, stdout, .. } => {
            one_document(stdout).ok_or_else(|| could_not("the reader's answer is not one JSON document".into()))
        }
        Ran::Finished { code, stderr, .. } => {
            Err(could_not(last_line(&stderr).unwrap_or_else(|| format!("the reader stopped with exit code {code}"))))
        }
        Ran::CouldNotStart(e) => Err(could_not(format!("the reader could not be started: {e}"))),
        Ran::TimedOut => Err(NoAnswer::TookTooLong),
        Ran::WroteTooMuch => Err(could_not("the reader's answer is too large".into())),
        Ran::Stopped => Err(NoAnswer::Quitting),
        Ran::Signalled { signal, stderr } => {
            Err(could_not(last_line(&stderr).unwrap_or_else(|| format!("the reader ended on signal {signal}"))))
        }
    }
}

/// Each value is part of its flag's own argument, so no word or id can be
/// read as a flag.
fn arguments(data: &DataDir, question: &Question) -> [OsString; 2] {
    let mut dir = OsString::from("--data-dir=");
    dir.push(data.path());
    let asked = match question {
        Question::Kinds => "--kinds".to_string(),
        Question::Find(words) => format!("--find={words}"),
        Question::Page(id) => format!("--page={id}"),
    };
    [dir, asked.into()]
}

/// The output as it is, when it is exactly one JSON document, with
/// whitespace around it.
fn one_document(stdout: Vec<u8>) -> Option<Answer> {
    let text = String::from_utf8(stdout).ok()?;
    serde_json::from_str::<serde::de::IgnoredAny>(&text).ok()?;
    Some(Answer(text))
}

fn last_line(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    text.lines().map(str::trim_end).rfind(|line| !line.is_empty()).map(str::to_string)
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

/// Why a source's file is not opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unopened {
    /// The index holds no file read inside with this document id.
    NotFound,
    CouldNotOpen {
        said: String,
    },
}

/// The path of the file the index holds for a source's document id.
pub fn source_file(data: &DataDir, record: &str) -> Result<String, Unopened> {
    match textset::file_of(&data.index(), record) {
        Ok(Some(path)) => Ok(path),
        Ok(None) => Err(Unopened::NotFound),
        Err(e) => Err(Unopened::CouldNotOpen { said: kept(format!("{e:#}")) }),
    }
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
