# Phase 3: reading inside files

Part of the SIFS plan (`ontology-framework/plans/sifs-intelligence-backend.md`, phase 3).

**Goal:** search by the words inside files, not only their names, across every kind of file. Everything is local; nothing leaves the Mac.

**Done when:** a word search finds files of every kind below by their contents, with the page or sheet it's on. Extraction is measured per 1,000 files (time, index size, memory).

## How it fits the index

The name index stays as it is. Three additions to the same SQLite file:

| Table | What it holds |
|---|---|
| `extracts` | One row per file read: `file_id`, `reader`, its `version`, the file's `modified` and `size` when read, `status` (done, empty, failed, skipped), the error, the time taken and the characters kept. A file is read again only if its `modified` or `size` changed, or the reader's version went up. |
| `contents` | One row per page, sheet, slide or email part: `file_id`, `part` (its number), `label` ("page 3", "Q2", "subject") and `text`. |
| `contents_fts` | FTS5 over `contents` (external content, kept in step by triggers). The tokenizer is `unicode61 remove_diacritics 2`, so "resume" finds "résumé". Keeping the text in `contents` lets the tokenizer change later with a rebuild and no re-reading. |
| `facts` | The small values read alongside the text, as (`file_id`, `key`, `value`): EXIF date, camera, GPS latitude and longitude, where the file was downloaded from, author, title, page count, duration, artist, and so on. Phase 4 hands these to the engine. |
| `extract_queue` | The files waiting to be read, with their priority, and how many reads were started and never finished (crash safety). |
| `extract_kinds` | Which extensions have a reader on this machine, with its version and priority. Rust rewrites it at startup, so Linux and the Mac can register different readers. |

## Readers, by kind

| Kind | How | Notes |
|---|---|---|
| Text, markdown, code, CSV, JSON, HTML | read directly; HTML tags stripped | capped at a few MB |
| PDF | PDFKit (macOS), page by page | no text layer → OCR the page images (below) |
| Word, Excel, PowerPoint (`docx`, `xlsx`, `pptx`) | they're zip files of XML: read the document, sheet and slide XML | Excel: one part per sheet, cells as rows |
| Pages, Numbers, Keynote | the preview image inside the bundle, through OCR | their own format is a private binary; the preview is good enough to start |
| RTF, old `doc` | AppKit's text import | |
| Email (`.eml`, `.emlx`) | a MIME parser: from, to, subject, date, body; attachment names | |
| Images, screenshots | **Vision** text recognition (OCR); EXIF date, camera, GPS | OCR is the slow step: queued behind everything else |
| Audio, video | tags and duration (AVFoundation) | transcripts are for later |
| Every file | where it was downloaded from (`com.apple.metadata:kMDItemWhereFroms`, a binary plist in an extended attribute) | already computed by macOS |

Calling PDFKit, Vision and AVFoundation from Rust uses the `objc2` framework crates.

## Doing the work without getting in the way

- **A background worker** goes through the queue at low priority (macOS "utility"), a few files at a time, one OCR at a time.
- **Queue order:** first the files the user opened or changed recently, then documents (PDF, Office, text), then images, then the rest. The order is kept in the database, so a restart picks up where it left off.
- **When a file changes,** the watcher marks its extract stale, and the worker reads it again.
- **Limits:** files over a size cap are skipped; a reader that takes too long or fails is recorded and not retried until the file changes. Settings show progress (read, waiting, failed).

## Search

- A query looks in names and contents. A name match still ranks higher. A content match ranks by FTS5's bm25, then the same kind, place and recency priors as now.
- Results show where the words were found: a short snippet and the page or sheet.
- **Typing stays instant:** names are searched first and shown at once; content results join a moment later.

## Steps

1. The `extracts`, `contents_fts` and `facts` tables, the queue and worker, and plain text and code, with tests.
2. PDF through PDFKit, and Office files through their XML.
3. Search over contents: snippets, page numbers, ranking with names, and UI.
4. Images: Vision OCR and EXIF; OCR for PDFs with no text.
5. Email, RTF, iWork previews, audio and video tags, download source.
6. Measure on the real home folder: time per 1,000 files by kind, index size, memory, query time.

Each step is a PR, with tests on a folder of sample files of every kind.

## Step 1 in detail: tables, queue, worker, plain-text readers

Everything in step 1 is pure Rust and is built and tested on Linux with `cargo test --no-default-features --lib` (the `app` feature needs Tauri's system libraries).

### Migrations

There's no migration system yet: `SCHEMA` (`index/db.rs`) is `CREATE … IF NOT EXISTS`, run at every open. Step 1 adds `index/migrate.rs`: a list of SQL migrations applied by `PRAGMA user_version`, each in one IMMEDIATE transaction with its version bump, run by `Index::open` under the writer lock. Today's schema is version 0, so phase 1 databases upgrade in place. Migration 1 creates the tables above and the triggers below.

### The queue fills itself

Triggers on `files` keep the queue right, so the crawler, watcher and search don't change:
- **A file is added:** it's queued if its extension has a reader. Priority 0 if it changed in the last week, else its reader's priority (documents 1, images 2, the rest 3).
- **A file's `modified` or `size` changes:** it's queued again at priority 0. The old text stays searchable until the new read replaces it. An unchanged rescan queues nothing (a test pins this, since the upsert rewrites every column).
- **A file is deleted** (including folder removal and ignore-pattern rescans): its contents, facts, extract and queue row go too.

At startup, if the readers or their versions changed (a signature kept in `meta`), a reconcile pass queues the files whose extract is missing or out of date, in chunks of 50,000 rows. A failed or skipped file whose `modified`, `size` and reader version still match is **not** queued again.

### The worker

One thread, `sifs-extract`, at low priority (macOS QoS "utility"; `setpriority` on Linux):
- **Order:** priority, then most recently modified, from the database, so a restart resumes where it left off. Opening a file in SIFS moves it to the front.
- **Pace:** one or two reads at a time; readers marked exclusive (OCR later) run one at a time. It pauses while a scan runs, and wakes on watcher changes and finished scans.
- **Each read:**
  - The file is checked just before reading. If it's gone, the queue row is dropped. If its extract already matches, nothing is read.
  - A file over the size cap is skipped and never opened.
  - The read runs on its own thread, with a time limit. Past the limit it's recorded failed ("took longer than 10 s"), and the thread is abandoned.
  - The file is checked again after the read. If it changed meanwhile, the result is thrown away and the file stays queued.
- **Writing:** results are written in batches of up to 16 files or 2 MB of text, in one transaction under the shared writer lock, so each hold is tens of milliseconds. Before writing, the file's path is checked again, since ids can be reused after a delete.
- **Crash safety:** a claim increments the queue row's `attempts` before reading. After a crash, files with unfinished attempts are read one at a time, each claim committed first. A file that stops SIFS twice is recorded failed ("SIFS stopped twice while reading this file"), so one bad file can't crash the app in a loop.
- **Progress:** counts by status plus the queue length, sent to the window at most once a second.

### Readers in step 1

A `Reader` trait (`id`, `version`, extensions, class, limits, `read(path, sink)`) and a registry by extension. Readers write pages and facts into a `Sink` that enforces the character cap and the time limit. Mac readers register later behind `cfg(target_os = "macos")`, with no change to the queue.

| Reader | Extensions | Notes |
|---|---|---|
| text | txt, text, log, md, markdown, rst, org, tex, csv, tsv | markdown title and author from front matter or the first heading; CSV/TSV column names as a fact |
| code | the code extensions from `search.rs` | lowest priority |
| plain | files without an extension (README, Makefile, LICENSE) | binary check, 1 MB cap |
| json | json, geojson, ipynb | keys and string values; raw text if it doesn't parse |
| html | html, htm, xhtml | tags, scripts, styles and comments stripped; entities decoded; title and author as facts |

**Encodings, for every reader:** a byte-order mark decides UTF-8 or UTF-16; UTF-16 without a mark is recognised from its zero bytes; other zero bytes mean binary (skipped); valid UTF-8 is used as is; anything else is guessed (`chardetng`) and decoded (`encoding_rs`), and the encoding is kept as a fact. HTML's `<meta charset>` is honoured first.

**Limits:** skip files over 4 MB (JSON and files without an extension: 1 MB); keep at most 1 million characters per file (the rest is cut, and a `truncated` fact recorded); a 10-second limit per file.

**New dependencies:** `encoding_rs`, `chardetng`, `html-escape`, `libc`.

### In the app

- `setup` registers the readers and starts the worker before the watcher.
- Settings show "Reading inside files: 12,340 read · 2,104 waiting · 12 couldn't be read · 30 skipped", with a list of failures and their reasons.
- New command `content_failures`; new event `extract-progress`.

### Tests

On a folder of sample files (`tests/fixtures/contents/`): text, markdown with "résumé", code, CSV, TSV, JSON (valid and broken), HTML with entities, Windows-1252 HTML, Latin-1, UTF-16 with and without a byte-order mark, Hindi, and a `.txt` that is really binary.
- **Readers:** every encoding case; HTML stripping and its facts; JSON and its fallback; CSV columns; markdown title; the character cap cuts on a character boundary; the time limit.
- **Migration:** a new database reaches version 1; a phase 1 database with files migrates, and its files are queued.
- **The queue:** an unchanged rescan queues nothing; deleting a file or folder removes its contents everywhere; order is recent, then documents, then code; opening a file moves it first.
- **Re-reading:** a second run reads nothing; a changed time or size re-reads exactly that file; a reader version bump re-reads that reader's files and no others.
- **Failures and caps:** a failing reader is recorded and not retried until the file changes; a file one byte over the cap is never opened; a slow reader is recorded failed and the worker carries on.
- **Restart:** stop the worker part way through 50 files and start a new one; all 50 are read exactly once. A file with one unfinished attempt is read alone; one with two is recorded failed unread.
- **The watcher:** overwrite a file and its new words are found, its old ones gone; delete it and its contents go.
- **Races:** a file deleted or changed during its read gets nothing written.
- **Locking:** the existing never-locked test, run with the worker going.
- **Search:** every sample is found by a word inside it, "resume" finds "résumé" composed and decomposed, and the Hindi sample by its own word.

### Measuring

`sifs-bench --contents` reads a folder after scanning it and reports: time per 1,000 files by reader, MB per second, the status mix, index growth per table (KB per file), peak memory, the longest and p95 hold of the writer lock, and how much the triggers slow the first scan. On Linux: `/usr/share/doc`, a `node_modules` tree, a Python library and a generated 10,000-file mix. Cold-cache numbers come from the Mac in step 6.

### Risks

- **The triggers slow the first scan.** If by more than about 10%, queue from Rust in the upsert batches instead.
- **Removing a large folder now deletes its text too.** Move folder removal to a background thread.
- **Deleted text leaves free pages,** and FTS5 segments fragment. Merge segments when the worker goes idle.
- **A stuck reader can't be killed in-process,** only abandoned (at most 4 at once). A crash inside a Mac framework takes the app down; crash safety stops it repeating.
- **Renames re-read the file** (a rename gets a new id). Cheap for text; carrying extracts over by inode comes before OCR.
- **The tokenizer doesn't split Chinese or Japanese** into words.
- **Extracted text is stored unencrypted** in `sifs.db`, protected by FileVault.

### Decisions (owner, 2026-11-03)

The owner took the recommended defaults, to be revisited once SIFS has run on real folders:
1. **Size:** skip files over 4 MB; keep at most 1 million characters per file.
2. **Files without an extension** (README, Makefile) are read, with the binary check and a 1 MB cap.
3. **Code files' contents are indexed,** at the lowest priority. A per-folder switch can come later.
4. **JSON** has a 1 MB cap.
5. **Ranking (step 3):** a name match ranks above a content match only when kind and place are equal.
6. **Reading pauses during scans** only, for now.
7. **Where a file was downloaded from** is read in a later pass, not during the crawl.
8. **Mac framework readers run in-process** first; a helper process if crashes show up.

Step 1 is built alongside the engine's PR #35, since it doesn't depend on it.

### Left for a Mac session

The step 1 tests on macOS and the utility priority; skipping iCloud placeholders (which must ship with step 1 on the Mac, or reading would download them); PDFKit; Vision OCR, EXIF and GPS; RTF and old Word through AppKit; iWork previews; AVFoundation tags; where-from; real Office sample files; and step 6's measurements.
