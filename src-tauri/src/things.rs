//! Keeps the store of things up to date by itself.

use crate::data::DataDir;
use crate::tools::{NotFound, Ran, Tools};
use serde::Serialize;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const QUIET: Duration = Duration::from_secs(10);
pub const EXTRACT_TIME: Duration = Duration::from_secs(10 * 60);
pub const LOAD_TIME: Duration = Duration::from_secs(15 * 60);
pub const WORK_FOLDER: &str = "things-work";
pub const FINGERPRINT_FILE: &str = "things-read.json";
const FINGERPRINT_FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Export,
    Extract,
    Load,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    NoTools {
        said: String,
    },
    Idle,
    Waiting {
        owed: bool,
    },
    Reading {
        step: Step,
        owed: bool,
    },
    Read {
        when: i64,
        documents: usize,
        turned_down: usize,
    },
    Failed {
        step: Step,
        said: String,
    },
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn wall(&self) -> i64;
}

pub type RunTool = Arc<dyn Fn(&Path, &[OsString], Duration) -> Ran + Send + Sync>;
pub type Idle = Arc<dyn Fn() -> bool + Send + Sync>;
pub type OnChange = Arc<dyn Fn(&State) + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub extract: Duration,
    pub load: Duration,
}

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

#[derive(Clone)]
pub struct Things;

impl Things {
    pub fn start(_setup: Setup) -> Things {
        todo!()
    }
    fn new(_setup: Setup) -> Things {
        todo!()
    }
    fn drive(&self) -> Option<JoinHandle<()>> {
        todo!()
    }
    fn tick(&self) {
        todo!()
    }
    fn wake(&self) {
        todo!()
    }
    pub fn state(&self) -> State {
        todo!()
    }
    pub fn changed(&self) {
        todo!()
    }
    pub fn read_again(&self) {
        todo!()
    }
    pub fn stop(&self) {
        todo!()
    }
}

pub fn nothing_waiting(_db: &Path) -> bool {
    todo!()
}

#[cfg(test)]
#[path = "things_tests.rs"]
mod tests;
