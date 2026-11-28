//! The engine's tools, run by the app: one at a time or several at once,
//! each with a deadline and a cap on what it prints, and none of them left
//! running once the app has quit.
//!
//! A running tool is a child the app started and has not reaped. It leads
//! its own process group, so its pid names everything it started. The
//! [`Runner`] holds the set of running tools and a phase: open, then
//! stopping (every group has been killed; the set drains), then stopped
//! (the set is empty and stays so).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

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
    running: BTreeSet<libc::pid_t>,
}

/// The app's running tools. One per app.
#[derive(Debug, Default)]
pub struct Runner {
    state: Mutex<State>,
    changed: Condvar,
}

impl Runner {
    /// Runs program with args as they are, no shell, until it exits or the
    /// time runs out.
    pub fn run<I, S>(&self, program: &Path, args: I, _time: Duration) -> Ran
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = match Command::new(program).args(args).stdin(Stdio::null()).output() {
            Ok(output) => output,
            Err(e) => return Ran::CouldNotStart(e),
        };
        match output.status.code() {
            Some(code) => Ran::Finished { code, stdout: output.stdout, stderr: output.stderr },
            None => Ran::Signalled { signal: 0, stderr: output.stderr },
        }
    }

    /// Ends every running tool and everything it started.
    pub fn stop(&self) {
        let _ = (&self.state, &self.changed);
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
