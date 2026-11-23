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

Write the text an index has read as a text set, the input `sample/FORMAT.md` describes, so the extractor in `engine/` reads indexed text instead of the sample's own text set:

```bash
cd src-tauri && cargo run --release --no-default-features --bin sifs-bench -- --contents /tmp/try.db "$PWD/../sample/files"
cargo run --release --no-default-features --bin sifs-textset -- --db /tmp/try.db --out /tmp/try-text --reference-date 2026-11-22
```

- `--db` is the index. It is opened read-only and never written, migrated or locked for writing. SQLite may leave an empty `-wal` file and a `-shm` file beside an index that had none. When you try it out, point it at a copy or at an index made for the purpose, as above, not at the app's own.
- `--out` is the text set's folder. It must be absent or empty. Nothing in it is replaced.
- `--reference-date` is written to `index.json`. The output holds no clock, so the same index gives the same bytes.

Every file whose read ended `done` or `empty` is one document, in path order. Its id is the file name in `[a-z0-9-]` and a hash of the file's path, so a file keeps its id across rescans and rebuilt indexes. `created` is the file's time as the index holds it, `format` is its extension, and `lang` is empty. `sha256` is `null`, because the index holds no hash and the tool reads nothing but the index. The stored parts become pages in this way:

- A PDF's parts are its pages with text, labelled `page N`. Each is a page with `scanned: false` and exactly the stored text. Every other page up to the PDF's page count had no text layer, and is `scanned: true` with empty text, because SIFS does not read scanned PDF pages yet. When the read was cut short, the pages after the last stored page are left out.
- An image is one page with `scanned: true` and what Apple Vision read from it, or empty text.
- Every other reader's parts are not pages: each part (a text file's text, a sheet, a slide, slide notes, a Word section) is one page with `scanned: false`.

A file with no page (an empty text file, say) is left out, as are files that failed or were skipped.

## CI

`.github/workflows/ci.yml` runs on every pull request and every push to main, on GitHub's hosted macOS runners (`macos-26`, Apple silicon). The `ci` job passes only when every other job succeeded:

- **Rust tests:** `cargo test --no-default-features --lib`. The job fails if fewer tests pass than `TEST_FLOOR`. The summary shows the counts.
- **Clippy:** `cargo clippy --all-targets -- -D warnings`, once with default features and once with `--no-default-features`.
- **Svelte check:** `npm run check` may report at most `SVELTE_ERROR_CEILING` errors.
- **Sample tools:** the four `node sample/tools/...` commands below.

To run the same gates by hand:

```bash
cd src-tauri
cargo test --no-default-features --lib
mkdir -p ../build && cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cd ..
npm ci && npm run check
node --test sample/tools/score.test.mjs
node sample/tools/score-mutants.mjs
node sample/tools/text.mjs --check
node sample/tools/check.mjs
```

Tests a hosted runner cannot run, for a reason of the runner and not the code, are listed by name in `RUNNER_LIMITED` in the workflow, each with its reason and issue. They run in a separate step that may fail, and its result is shown in the job summary. The list is empty today.

## Layout

```text
src/routes/+page.svelte   search box, results, folder settings
src-tauri/src/app.rs      Tauri commands, background scans, watcher, tray
src-tauri/src/index/      the index: db (SQLite), migrations, crawl, watch, search, ignore rules, tests
src-tauri/src/extract/    reading inside files: readers, decoding, the queue, the background worker, tests
src-tauri/tests/fixtures/ sample files for the tests
```
