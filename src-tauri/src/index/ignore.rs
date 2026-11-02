//! What the crawler and the watcher skip, and what they index without
//! looking inside.

use anyhow::Result;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// Folders skipped entirely, as glob patterns over the full path. `~` is the
/// home folder. The user can edit this list.
pub const DEFAULT_IGNORE: &[&str] = &[
    // Hidden files and folders: tool caches and settings (on one developer's
    // Mac, 4.4 million of 7.2 million entries in the home folder), the trash,
    // .git. Spotlight skips them too.
    "**/.*",
    "~/Library",
    "~/go/pkg",
    "**/node_modules",
    "**/bower_components",
    "**/__pycache__",
    "**/venv",
    "**/target/debug",
    "**/target/release",
    "**/DerivedData*",
    "**/Pods",
    // Xcode's own index, build products and package checkouts.
    "**/*.noindex",
    "**/SourcePackages",
];

/// Packages macOS shows as one item: indexed themselves, not their insides.
pub const OPAQUE: &[&str] = &[
    "**/*.app",
    "**/*.photoslibrary",
    "**/*.musiclibrary",
    "**/*.tvlibrary",
    "**/*.bundle",
    "**/*.framework",
    "**/*.xcodeproj",
    "**/*.xcworkspace",
    "**/*.pages",
    "**/*.numbers",
    "**/*.key",
    "**/*.rtfd",
];

pub struct Rules {
    ignored: GlobSet,
    opaque: GlobSet,
}

impl Rules {
    pub fn new(patterns: &[String], home: &str) -> Result<Rules> {
        Ok(Rules { ignored: build(patterns.iter().map(String::as_str), home)?, opaque: build(OPAQUE.iter().copied(), home)? })
    }

    pub fn default_patterns() -> Vec<String> {
        DEFAULT_IGNORE.iter().map(|p| p.to_string()).collect()
    }

    /// Whether path is skipped, itself or because a folder above it is,
    /// counting only the folders below root: a folder the user chose is
    /// indexed even if its own path passes through a hidden or ignored one.
    pub fn is_ignored(&self, root: &str, path: &str) -> bool {
        let root = std::path::Path::new(root);
        let mut current = std::path::Path::new(path);
        while current != root && current.starts_with(root) {
            if self.ignored.is_match(current) {
                return true;
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => return false,
            }
        }
        false
    }

    /// Whether path is a package indexed without its insides.
    pub fn is_opaque(&self, path: &str) -> bool {
        self.opaque.is_match(path)
    }

    /// Whether path lies inside a package.
    pub fn is_inside_opaque(&self, path: &str) -> bool {
        let mut current = std::path::Path::new(path).parent();
        while let Some(p) = current {
            if self.opaque.is_match(p) {
                return true;
            }
            current = p.parent();
        }
        false
    }
}

fn build<'a>(patterns: impl Iterator<Item = &'a str>, home: &str) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let expanded = match pattern.strip_prefix("~") {
            Some(rest) => format!("{}{}", home.trim_end_matches('/'), rest),
            None => pattern.to_string(),
        };
        // `*` stays within one path component, so "**/.*" means a hidden
        // component, not any path with "/." somewhere in it.
        builder.add(GlobBuilder::new(&expanded).literal_separator(true).build()?);
    }
    Ok(builder.build()?)
}
