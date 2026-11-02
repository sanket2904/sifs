//! Walks a folder into the index.

use super::db::{FileRecord, Index};
use super::ignore::Rules;
use anyhow::Result;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

const BATCH: usize = 2000;

/// What a scan did, for progress and measurement.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ScanStats {
    pub root: String,
    pub indexed: u64,
    pub removed: u64,
    pub skipped: u64,
    pub millis: u64,
}

/// The record of one path, or None for one that can't be read.
pub fn record_of(path: &Path, root: &str) -> Option<FileRecord> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let path_str = path.to_str()?.to_string();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or(&path_str).to_string();
    let is_dir = meta.is_dir();
    let ext = if is_dir {
        String::new()
    } else {
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default()
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(FileRecord {
        path: path_str,
        root: root.to_string(),
        name,
        ext,
        is_dir,
        size: if is_dir { 0 } else { meta.len() as i64 },
        modified,
        inode: meta.ino() as i64,
    })
}

/// Indexes everything under root that the rules allow, then removes rows of
/// root the walk no longer found. progress is called after every batch with
/// the number indexed so far.
pub fn scan(index: &mut Index, root: &str, rules: &Rules, mut progress: impl FnMut(u64)) -> Result<ScanStats> {
    let started = std::time::Instant::now();
    let seen = index.next_scan()?;
    let mut stats = ScanStats { root: root.to_string(), ..Default::default() };
    let mut batch = Vec::with_capacity(BATCH);
    let mut walker = walkdir::WalkDir::new(root).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.skipped += 1;
                continue;
            }
        };
        let Some(path) = entry.path().to_str() else {
            stats.skipped += 1;
            continue;
        };
        if rules.is_ignored(root, path) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if let Some(record) = record_of(entry.path(), root) {
            batch.push(record);
        } else {
            stats.skipped += 1;
        }
        if entry.file_type().is_dir() && rules.is_opaque(path) {
            walker.skip_current_dir();
        }
        if batch.len() >= BATCH {
            index.upsert(&batch, seen)?;
            stats.indexed += batch.len() as u64;
            batch.clear();
            progress(stats.indexed);
        }
    }
    index.upsert(&batch, seen)?;
    stats.indexed += batch.len() as u64;
    progress(stats.indexed);
    stats.removed = index.remove_unseen(root, seen)? as u64;
    stats.millis = started.elapsed().as_millis() as u64;
    Ok(stats)
}

/// Brings one changed path up to date: indexed as it now is, with a new or
/// moved-in folder walked, or removed with everything under it if it is
/// gone or now ignored.
pub fn refresh(index: &mut Index, path: &Path, roots: &[String], rules: &Rules) -> Result<()> {
    let Some(path_str) = path.to_str() else { return Ok(()) };
    let Some(root) = roots.iter().find(|r| path_str == r.as_str() || path_str.starts_with(&format!("{}/", r.trim_end_matches('/')))) else {
        return Ok(());
    };
    if rules.is_inside_opaque(path_str) {
        return Ok(());
    }
    if rules.is_ignored(root, path_str) || std::fs::symlink_metadata(path).is_err() {
        index.remove_tree(path_str)?;
        return Ok(());
    }
    let Some(record) = record_of(path, root) else {
        index.remove_tree(path_str)?;
        return Ok(());
    };
    let seen = 0; // refreshed rows are kept by the next full scan only if still found
    if record.is_dir && !rules.is_opaque(path_str) {
        let known = index.get(path_str)?.is_some();
        index.upsert(&[record], seen)?;
        if !known {
            // A folder created or moved in: walk what came with it.
            let mut batch = Vec::new();
            let mut walker = walkdir::WalkDir::new(path).min_depth(1).follow_links(false).into_iter();
            while let Some(Ok(entry)) = walker.next() {
                let Some(p) = entry.path().to_str() else { continue };
                if rules.is_ignored(root, p) {
                    if entry.file_type().is_dir() {
                        walker.skip_current_dir();
                    }
                    continue;
                }
                if let Some(r) = record_of(entry.path(), root) {
                    batch.push(r);
                }
                if entry.file_type().is_dir() && rules.is_opaque(p) {
                    walker.skip_current_dir();
                }
                if batch.len() >= BATCH {
                    index.upsert(&batch, seen)?;
                    batch.clear();
                }
            }
            index.upsert(&batch, seen)?;
        }
    } else {
        index.upsert(&[record], seen)?;
    }
    Ok(())
}
