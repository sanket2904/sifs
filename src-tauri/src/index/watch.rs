//! Keeps the index current: every path the file system reports changed is
//! brought up to date as it now is (see crawl::refresh). Rechecking the path
//! rather than trusting the event kind is what makes renames, moves and
//! FSEvents' coalesced events come out right.

use super::crawl;
use super::db::Index;
use super::ignore::Rules;
use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// A running watcher. Dropping it stops watching.
pub struct Watch {
    watcher: RecommendedWatcher,
    roots: Arc<Mutex<Vec<String>>>,
}

impl Watch {
    /// Starts watching roots, applying changes to the index at db_path.
    /// on_change is called after each batch of changes is applied.
    pub fn start(
        db_path: PathBuf,
        roots: Vec<String>,
        rules: Arc<Mutex<Rules>>,
        on_change: impl Fn(usize) + Send + 'static,
    ) -> Result<Watch> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx)?;
        for root in &roots {
            watcher.watch(Path::new(root), RecursiveMode::Recursive)?;
        }
        let roots = Arc::new(Mutex::new(roots));
        let shared = roots.clone();
        std::thread::Builder::new().name("sifs-watch".into()).spawn(move || {
            let mut index = match Index::open(&db_path) {
                Ok(index) => index,
                Err(e) => {
                    log::error!("watcher cannot open the index: {e:#}");
                    return;
                }
            };
            while let Ok(first) = rx.recv() {
                // Take everything already queued, so a burst is one batch.
                let mut paths: Vec<PathBuf> = Vec::new();
                for event in std::iter::once(first).chain(rx.try_iter()) {
                    match event {
                        Ok(event) => paths.extend(event.paths),
                        Err(e) => log::warn!("watch error: {e}"),
                    }
                }
                paths.sort();
                paths.dedup();
                let roots = shared.lock().unwrap().clone();
                let rules = rules.lock().unwrap();
                let mut applied = 0;
                for path in &paths {
                    match crawl::refresh(&mut index, path, &roots, &rules) {
                        Ok(()) => applied += 1,
                        Err(e) => log::warn!("refresh {}: {e:#}", path.display()),
                    }
                }
                drop(rules);
                on_change(applied);
            }
        })?;
        Ok(Watch { watcher, roots })
    }

    pub fn add_root(&mut self, root: &str) -> Result<()> {
        self.watcher.watch(Path::new(root), RecursiveMode::Recursive)?;
        self.roots.lock().unwrap().push(root.to_string());
        Ok(())
    }

    pub fn remove_root(&mut self, root: &str) -> Result<()> {
        self.roots.lock().unwrap().retain(|r| r != root);
        let _ = self.watcher.unwatch(Path::new(root));
        Ok(())
    }
}
