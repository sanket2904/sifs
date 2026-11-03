# SIFS

Search your files by name, content and meaning: a fast, private file search for macOS. Built with Rust, Tauri 2 and Svelte 5.

This is phase 1 of the plan in `ontology-framework/plans/sifs-intelligence-backend.md`, an instant, lasting file finder, and the first step of phase 3, reading inside files. Searching what it reads, the knowledge graph (cards for people, organisations, documents) and answers come next.

## What it does now

- **You choose the folders** to search, and can add or remove them at any time (⚙︎). Documents, Desktop and Downloads are offered as a start.
- **Skips what you don't want:** hidden folders, `~/Library`, `node_modules`, build folders. The list is editable. App bundles and other packages count as one item.
- **Keeps its index** in `~/Library/Application Support/com.sanket2904.sifs/sifs.db` (SQLite), so it doesn't re-scan at every launch. It catches up on changes made while it was closed.
- **Stays current** while running: creates, renames, moves and deletes are picked up through FSEvents.
- **Searches as you type:**
  - names, case-insensitive, any part of the name;
  - typo-tolerant;
  - ranked by how well the name matches and how recently the file changed.
- **Reads inside files** in the background, at low priority and never during a scan: text, Markdown, CSV, code, JSON, HTML and files without an extension (README, Makefile), in any common encoding. Files over 4 MB (1 MB for JSON and files without an extension) are skipped, and at most a million characters are kept per file. A file is read again only when it changes. Settings show how far it has got, and which files couldn't be read and why.
- **Keyboard:** ↑↓ to move, Enter to open, ⌘Enter to show in Finder. Closing the window keeps it in the menu bar.

Measured on one Mac's whole home folder: 1.9 million items indexed in 99 s, a 0.97 GB index, 27 MB peak memory while indexing, searches in 1 to 12 ms.

## Develop

Requirements: Rust (stable, Apple Silicon toolchain; `rust-toolchain.toml` pins it), Node.js, Xcode command-line tools.

```bash
npm install
npm run tauri dev          # run the app
npm run tauri build -- --bundles app   # build SIFS.app
cd src-tauri && cargo test --no-default-features --lib   # index and reading tests (no Tauri needed, so Linux too)
```

Measure the index on real folders without the app (read-only; writes only the index file given):

```bash
cd src-tauri && cargo run --release --no-default-features --bin sifs-bench -- /tmp/bench.db ~/Documents -- resume invoice
```

With `--contents` before the index file, it also reads inside the files and reports time per 1,000 files by reader, index growth by table, peak memory and how long writes held the index.

## Layout

```text
src/routes/+page.svelte   search box, results, folder settings
src-tauri/src/app.rs      Tauri commands, background scans, watcher, tray
src-tauri/src/index/      the index: db (SQLite), migrations, crawl, watch, search, ignore rules, tests
src-tauri/src/extract/    reading inside files: readers, decoding, the queue, the background worker, tests
src-tauri/tests/fixtures/ sample files for the tests
```
