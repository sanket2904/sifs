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

#[derive(Default)]
struct Jobs {
    /// Folders being scanned, each with the flag that stops its scan.
    scanning: HashMap<String, Arc<AtomicBool>>,
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
            let jobs = self.jobs();
            if let Some(cancel) = jobs.scanning.get(path) {
                // Set under the lock its scan reads it under when it ends,
                // so the scan either sees it or has not ended yet.
                cancel.store(true, Ordering::SeqCst);
                return Ok(());
            }
        }
        self.start_removal(path.to_string());
        Ok(())
    }

    /// Scans one folder in the background, unless it is being scanned.
    pub fn scan(&self, root: String) {
        let cancel = {
            let mut jobs = self.jobs();
            if jobs.scanning.contains_key(&root) {
                return;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            jobs.scanning.insert(root.clone(), cancel.clone());
            cancel
        };
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
            let removed = {
                let mut jobs = this.jobs();
                jobs.scanning.remove(&root);
                cancel.load(Ordering::SeqCst)
            };
            match result {
                Ok(stats) => (inner.on_event)(Event::Scanned(stats)),
                Err(e) => (inner.on_event)(Event::ScanFailed { root: root.clone(), error: format!("{e:#}") }),
            }
            if removed {
                this.start_removal(root);
            }
        });
    }

    /// Removes a forgotten folder's files in the background, in small
    /// batches: with the text read from them, a large folder takes a while,
    /// and searches and other writes go on meanwhile.
    fn start_removal(&self, root: String) {
        {
            let mut jobs = self.jobs();
            if let Some(again) = jobs.removing.get_mut(&root) {
                *again = true;
                return;
            }
            jobs.removing.insert(root.clone(), false);
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let inner = &this.inner;
            let mut wait = inner.retry;
            loop {
                let result = Index::open(&inner.db_path).and_then(|mut index| {
                    // Added again since (once its mark was cleared): it stays.
                    if index.folders()?.contains(&root) {
                        return Ok(None);
                    }
                    index.remove_root_in_batches(&root, REMOVE_BATCH, |_| {}).map(Some)
                });
                match result {
                    Ok(removed) => {
                        wait = inner.retry;
                        if let Some(removed) = removed {
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
