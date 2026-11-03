//! The folders the user chose, scanned and removed in the background. Kept
//! apart from the window so it can be tested; the app passes its events on.
//!
//! A folder removed while it is being scanned is removed once the scan
//! stops: the scan is told to stop before its next batch, so nothing it
//! writes is left behind. A removal that fails is tried again, waiting
//! longer each time, and any rows still left (from a race with the watcher,
//! or an older SIFS) are found at the next start.

use super::crawl::{self, ScanStats};
use super::db::Index;
use super::ignore::Rules;
use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The longest wait between tries of a removal that keeps failing.
const MAX_RETRY: Duration = Duration::from_secs(600);
/// Files removed per transaction.
const REMOVE_BATCH: usize = 100;

/// What happened, for the window.
#[derive(Debug, Clone)]
pub enum Event {
    Progress { root: String, indexed: u64 },
    Scanned(ScanStats),
    ScanFailed { root: String, error: String },
    Removed { root: String, removed: usize },
}

/// Scans and removals running. Cloning it gives another handle on the same.
#[derive(Clone)]
pub struct Folders {
    inner: Arc<Inner>,
}

struct Inner {
    db_path: PathBuf,
    home: String,
    /// The first wait after a removal fails; it doubles after each failure.
    retry: Duration,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
    jobs: Mutex<Jobs>,
}

/// A scan running.
struct Scan {
    /// Set when its folder is removed: the scan stops before its next batch.
    cancel: Arc<AtomicBool>,
    /// Asked for again while it ran (rules changed, or the folder was added
    /// again after a removal): another scan follows it.
    again: bool,
}

#[derive(Default)]
struct Jobs {
    /// Folders being scanned.
    scanning: HashMap<String, Scan>,
    /// Folders being removed, and whether to go over them once more when
    /// done (asked again meanwhile).
    removing: HashMap<String, bool>,
}

impl Folders {
    pub fn new(db_path: PathBuf, home: String, retry: Duration, on_event: impl Fn(Event) + Send + Sync + 'static) -> Folders {
        Folders {
            inner: Arc::new(Inner { db_path, home, retry, on_event: Box::new(on_event), jobs: Mutex::default() }),
        }
    }

    fn jobs(&self) -> MutexGuard<'_, Jobs> {
        self.inner.jobs.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The folders being scanned.
    pub fn scanning(&self) -> HashSet<String> {
        self.jobs().scanning.keys().cloned().collect()
    }

    pub fn is_scanning(&self) -> bool {
        !self.jobs().scanning.is_empty()
    }

    /// Nothing scanning or being removed.
    pub fn idle(&self) -> bool {
        let jobs = self.jobs();
        jobs.scanning.is_empty() && jobs.removing.is_empty()
    }

    /// At startup: catches up on what changed while SIFS was closed, and
    /// finishes removing folders, including rows no folder owns.
    pub fn resume(&self, index: &mut Index) -> Result<()> {
        let orphans = index.mark_orphan_roots()?;
        if orphans > 0 {
            log::info!("{orphans} folders no longer searched still had files indexed; removing them");
        }
        for folder in index.folders()? {
            self.scan(folder);
        }
        for root in index.removals()? {
            self.start_removal(root);
        }
        Ok(())
    }

    /// Adds a folder to search, if it is not part of one already and not
    /// still being removed. The caller watches it, then scans it.
    pub fn add(&self, index: &mut Index, path: &str, now: i64) -> Result<()> {
        let existing = index.folders()?;
        if existing.iter().any(|f| path == *f || path.starts_with(&format!("{}/", f.trim_end_matches('/')))) {
            bail!("{path} is already searched, as part of a folder you added");
        }
        if index.removals()?.iter().any(|r| r == path) {
            // A removal that stopped (an error, say) is started again.
            self.start_removal(path.to_string());
            bail!("{path} is still being removed; add it again in a moment");
        }
        index.add_folder(path, now)?;
        Ok(())
    }

    /// Stops searching a folder and removes its files in the background,
    /// once its scan, if one is running, has stopped.
    pub fn remove(&self, index: &mut Index, path: &str) -> Result<()> {
        index.forget_folder(path)?;
        {
            let mut jobs = self.jobs();
            if let Some(scan) = jobs.scanning.get_mut(path) {
                // Set under the lock its scan reads them under when it ends,
                // so the scan either sees them or has not ended yet. Removed
                // after being added again: no scan follows, a removal does.
                scan.cancel.store(true, Ordering::SeqCst);
                scan.again = false;
                return Ok(());
            }
        }
        self.start_removal(path.to_string());
        Ok(())
    }

    /// Scans one folder in the background. If it is being scanned, another
    /// scan follows that one, so what was asked for now is seen (new rules,
    /// or the folder added again after a removal stopped that scan).
    pub fn scan(&self, root: String) {
        let cancel = {
            let mut jobs = self.jobs();
            if let Some(scan) = jobs.scanning.get_mut(&root) {
                scan.again = true;
                return;
            }
            new_scan(&mut jobs, &root)
        };
        self.spawn_scan(root, cancel);
    }

    fn spawn_scan(&self, root: String, cancel: Arc<AtomicBool>) {
        let this = self.clone();
        std::thread::spawn(move || {
            let inner = &this.inner;
            let result = (|| -> Result<ScanStats> {
                let mut index = Index::open(&inner.db_path)?;
                // Its own copy of the rules as saved, so a long scan holds
                // no lock the watcher needs.
                let patterns = index.ignore_patterns()?.unwrap_or_else(Rules::default_patterns);
                let rules = Rules::new(&patterns, &inner.home)?;
                crawl::scan_cancellable(&mut index, &root, &rules, &cancel, |indexed| {
                    (inner.on_event)(Event::Progress { root: root.clone(), indexed });
                })
            })();
            // What follows is registered under the same lock the scan is
            // taken off under, so the folder is never seen idle in between,
            // and a remove or scan asked for meanwhile finds it.
            let follow = {
                let mut jobs = this.jobs();
                let again = jobs.scanning.remove(&root).is_some_and(|s| s.again);
                if again {
                    // Its folder may have been removed and added again:
                    // this scan saw only part of it, so a fresh one runs.
                    Follow::Scan(new_scan(&mut jobs, &root))
                } else if cancel.load(Ordering::SeqCst) {
                    // What it wrote of a removed folder goes.
                    if new_removal(&mut jobs, &root) {
                        Follow::Remove
                    } else {
                        Follow::Nothing
                    }
                } else {
                    Follow::Nothing
                }
            };
            match result {
                Ok(stats) => (inner.on_event)(Event::Scanned(stats)),
                Err(e) => (inner.on_event)(Event::ScanFailed { root: root.clone(), error: format!("{e:#}") }),
            }
            match follow {
                Follow::Scan(cancel) => this.spawn_scan(root, cancel),
                Follow::Remove => this.spawn_removal(root),
                Follow::Nothing => {}
            }
        });
    }

    /// Removes a forgotten folder's files in the background, in small
    /// batches: with the text read from them, a large folder takes a while,
    /// and searches and other writes go on meanwhile.
    fn start_removal(&self, root: String) {
        if new_removal(&mut self.jobs(), &root) {
            self.spawn_removal(root);
        }
    }

    fn spawn_removal(&self, root: String) {
        let this = self.clone();
        std::thread::spawn(move || {
            let inner = &this.inner;
            let mut wait = inner.retry;
            loop {
                // A folder added again since (once its mark was cleared)
                // keeps its rows: the deletes check, in the same statement.
                let result = Index::open(&inner.db_path)
                    .and_then(|mut index| index.remove_root_in_batches(&root, REMOVE_BATCH, |_| {}));
                match result {
                    Ok(removed) => {
                        wait = inner.retry;
                        if removed > 0 {
                            log::info!("removed {root}: {removed} items");
                            (inner.on_event)(Event::Removed { root: root.clone(), removed });
                        }
                        let mut jobs = this.jobs();
                        match jobs.removing.get_mut(&root) {
                            Some(again) if *again => *again = false,
                            _ => {
                                jobs.removing.remove(&root);
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("removing {root}: {e:#}; trying again in {} s", wait.as_secs_f64());
                        std::thread::sleep(wait);
                        wait = (wait * 2).min(MAX_RETRY);
                    }
                }
            }
        });
    }
}

enum Follow {
    Scan(Arc<AtomicBool>),
    Remove,
    Nothing,
}

/// Registers a scan of root, returning its cancel flag.
fn new_scan(jobs: &mut Jobs, root: &str) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    jobs.scanning.insert(root.to_string(), Scan { cancel: cancel.clone(), again: false });
    cancel
}

/// Registers a removal of root: true if one must start, false if one is
/// running (it goes over the folder once more when done).
fn new_removal(jobs: &mut Jobs, root: &str) -> bool {
    if let Some(again) = jobs.removing.get_mut(root) {
        *again = true;
        return false;
    }
    jobs.removing.insert(root.to_string(), false);
    true
}
