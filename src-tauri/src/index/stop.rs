//! When a search stops: at its deadline, or as soon as a newer search
//! starts (the window drops an older search's reply).

use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Until {
    deadline: Instant,
    /// The newest search's number, and this one's.
    newest: Option<(Arc<AtomicU64>, u64)>,
}

impl Until {
    pub fn after(time: Duration) -> Until {
        Until { deadline: Instant::now() + time, newest: None }
    }

    /// Also stops once a later call of this on the same counter has started.
    pub fn or_newer(mut self, counter: &Arc<AtomicU64>) -> Until {
        let mine = counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.newest = Some((counter.clone(), mine));
        self
    }

    pub fn passed(&self) -> bool {
        Instant::now() >= self.deadline
            || self.newest.as_ref().is_some_and(|(newest, mine)| newest.load(Ordering::SeqCst) != *mine)
    }
}

/// Stops the connection's statements once until has passed (SQLite asks
/// every 64 steps), until dropped.
pub(crate) struct Stop<'a>(&'a Connection);

impl<'a> Stop<'a> {
    pub(crate) fn at(conn: &'a Connection, until: &Until) -> Self {
        let until = until.clone();
        conn.progress_handler(64, Some(move || until.passed()));
        Stop(conn)
    }
}

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

/// Whether an error is a statement stopped by Stop.
pub(crate) fn stopped(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::OperationInterrupted)
}
