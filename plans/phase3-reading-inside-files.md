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
| `contents_fts` | FTS5 over `contents` (external content, kept in step by triggers). The tokenizer is `unicode61 remove_diacritics 2`, so "resume" finds "résumé", with combining marks as word characters (`categories 'L* N* Co M*'`, so Hindi words stay whole) except the emoji variation selectors and the keycap (U+FE0E, U+FE0F, U+20E3), which are separators: "I❤️you" has the word "you", and "❤️" matches nothing. Keeping the text in `contents` lets the tokenizer change later with a rebuild and no re-reading. Migration 1 is unshipped, so the tokenizer changed in place: a development database built before the separators were added (before aff34bf) keeps the old tokenizer, still works, but lacks "you" in "I❤️you"; delete it. No migration is needed. |
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
- **Only files, folders and links are indexed.** FIFOs, sockets and devices are not, since opening one can block.

At startup, if the readers or their versions changed (a signature kept in `meta`), a reconcile pass queues the files whose extract is missing or out of date, in chunks of 50,000 rows. A failed or skipped file whose `modified`, `size` and reader version still match is **not** queued again.

### The worker

One thread, `sifs-extract`, at low priority (macOS QoS "utility"; `setpriority` on Linux):
- **Order:** priority, then most recently modified, from the database, so a restart resumes where it left off. Opening a file in SIFS moves it to the front.
- **Pace:** one or two reads at a time; readers marked exclusive (OCR later) run one at a time. It pauses while a scan runs, and wakes on watcher changes and finished scans.
- **Each read:**
  - The file is checked just before reading. If it's gone, the queue row is dropped. If its extract already matches, nothing is read.
  - A file over the size cap is skipped and never opened. Anything that is not a regular file (or a package folder, for a reader of packages) is never opened, and what was read from it is cleared.
  - The read runs on its own thread, with a time limit. Past the limit it's recorded failed ("took longer than 10 s"), and the thread is abandoned.
  - The file is checked again after the read. If it changed meanwhile, the result is thrown away and the file stays queued.
- **Writing:** results are written in batches of up to 16 files or 2 MB of text, in one transaction under the shared writer lock, so each hold is tens of milliseconds. Before writing, the file's path is checked again, since ids can be reused after a delete.
- **Crash safety:** a claim increments the queue row's `attempts` before reading. After a crash, files with unfinished attempts are read one at a time, each claim committed first. A file that stops SIFS twice is recorded failed ("SIFS stopped twice while reading this file"), so one bad file can't crash the app in a loop.
- **Errors:** an error after a claim puts the files back unread. Errors in a row are waited out longer each time: idle_poll (5 s), doubling up to 10 min, back to 5 s after a step that works. Wakes (the watcher's) don't cut the wait short; stopping does. When a write fails part way, the reads still running count as abandoned until they end.
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

**Encodings, for every reader:** a byte-order mark decides UTF-8 or UTF-16; UTF-16 without a mark is recognised from its zero bytes (on one side only, at least one code unit in 128, so Japanese with no spaces counts; Chinese and Korean without a mark have zeros on both sides and are still taken as binary); other zero bytes mean binary (skipped); valid UTF-8 is used as is; anything else is guessed (`chardetng`) and decoded (`encoding_rs`), and the encoding is kept as a fact. HTML's `<meta charset>` is honoured first.

**Limits:** skip files over 4 MB (JSON and files without an extension: 1 MB); keep at most 1 million characters per file (the rest is cut, and a `truncated` fact recorded); a 10-second limit per file.

**New dependencies:** `encoding_rs`, `chardetng`, `html-escape`, `libc`.

### In the app

- `setup` registers the readers and starts the worker before the watcher.
- Settings show "Reading inside files: 12,340 read · 2,104 waiting · 12 couldn't be read · 30 skipped", with a list of failures and their reasons.
- New command `content_failures`; new event `extract-progress`.
- **Removing a folder** (`index::folders`): searches leave it out at once, and its rows go in the background, 100 at a time. In each batch's transaction, a row that lies inside a folder (the same one added again, or one added around or inside it) is given to that folder; the rest are deleted. A scan of it still running is stopped before its next batch, and the removal starts once the scan has stopped. If the folder is added again before that scan ends, a fresh scan follows it instead (any scan asked for while one runs, after new ignore rules say, follows it). A removal that fails is tried again (5 s, doubling up to 10 min), and adding the folder while it is marked starts one.
- **Folders never nest.** Adding a folder inside one already searched is refused. Adding one around folders already searched takes them in: their folder rows go, their files (and what was read from them) become the new folder's, their scans stop (each is followed by a removal, which gives what it wrote late to the folder containing it), and they are no longer watched on their own. The window reloads the folder list after adding, so it shows only the new folder; removing a folder taken in does nothing, and it is not unwatched (on Linux that would drop the outer folder's watches there).
- **A row's root comes from `folders` when it is written** (one SQL helper, `folder_of`), never from a copy: the watcher's refresh writes each row under the folder containing it, and skips a path no folder contains; removals, take-ins and the startup tidy-up give rows the same way. Rows given to a folder are marked seen by the newest scan, so a scan of it already past their paths doesn't remove them as unseen.
- **Watching can fail** (inotify's limit): a folder that can't be watched is still scanned, and the window is told changes in it won't be noticed until a restart. At startup, a folder that can't be watched is logged and the others are watched.
- **At startup** folders inside others (allowed in phase 1) are taken in the same way, rows under a root that is not a folder but lies inside one are given to it, and every other root files are indexed under that is not a chosen folder is removed.

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
- **Removing a large folder now deletes its text too.** Done: folder removal runs in the background, in batches.
- **Deleted text leaves free pages,** and FTS5 segments fragment. Merge segments when the worker goes idle.
- **A stuck reader can't be killed in-process,** only abandoned (at most 4 at once, across every reader). A crash inside a Mac framework takes the app down; crash safety stops it repeating.
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

## Step 2 in detail: Word, Excel and PowerPoint

Step 2 is split. This half reads `docx`, `xlsx` and `pptx` through their XML, in pure Rust, built and tested on Linux like step 1. **PDF through PDFKit is macOS-only and is left for a Mac session** (below).

### The readers

Three readers, so a version bump to one re-reads only its files. All are documents (priority 1). Each takes the format's macro-enabled, template and show variants, which have the same insides.

| Reader | Extensions | What it reads, in this order |
|---|---|---|
| `docx` | docx, docm, dotx, dotm | the main document (found through `_rels/.rels`, so Strict Open XML works too), then headers and footers, footnotes, endnotes and comments (found through the document's relationships) |
| `xlsx` | xlsx, xlsm, xltx, xltm | `sharedStrings.xml`, `styles.xml` (which number formats are dates), then each sheet in the workbook's order (`workbook.xml` and its relationships), with cells as rows |
| `pptx` | pptx, pptm, potx, potm, ppsx, ppsm | each slide in the presentation's order (`presentation.xml`'s slide list and its relationships), each followed by its notes |

**Word.** Text is what is inside `w:t` (and `m:t` for equations); a paragraph ends a line, `w:tab` is a tab, `w:br` and `w:cr` a new line. Text a tracked change deleted (`w:del`) or moved away (`w:moveFrom`, found again at its `w:moveTo`) and field codes (`w:instrText`) are left out. In a table, a cell's paragraphs are joined by spaces, cells by tabs and rows by lines; each cell is written as it ends, so no row is held whole. A text box's text starts a line of its own ("Before", "BoxText", "After", not "BeforeBoxText"). `w:noBreakHyphen` is a hyphen, `w:softHyphen` is nothing, and `w:sym` is its character when that is plain Unicode (a symbol font's private-use characters are left out). Of an `mc:AlternateContent`, only the `mc:Choice` is read: the `mc:Fallback` repeats it (a text box's text would be found twice).

**Excel.** One part per sheet, labelled with the sheet's name. **Hidden sheets** (decided after the review) are read and labelled `Name (hidden)`; very hidden sheets (`state="veryHidden"`, reachable only from macros, sometimes holding lookup tables or credentials) are left out. A sheet listed twice is read once. Each row is a line, its non-empty cells joined by tabs (empty cells are not padded). Cell values:
- shared strings (`t="s"`), inline strings (`t="inlineStr"`, rich text runs joined, phonetic runs `rPh` left out) and formula strings (`t="str"`) as they are;
- booleans as TRUE and FALSE, errors as written (`#DIV/0!`);
- numbers as stored in the file (`1234.5`, not `1,234.50 €`): **a simplification**, since applying number formats is a project of its own and the digits are what a search needs;
- serial 0 in a date format is shown as `0` (Excel itself shows the impossible "1900-01-00");
- dates: a number whose cell style has a date format (built-in ids 14–22, 27–36, 45–47, 50–58, or a custom format with d, m, y, h or s outside quotes and brackets) is shown as `2026-11-03`, or `2026-11-03 14:30` when it has a time, or `14:30:00` when it is only a time; the 1904 date system is honoured.
Formulas are not indexed, only their values.

**PowerPoint.** One part per slide, `slide 3`, and one per slide's notes, `slide 3 notes`. Text is what is inside `a:t`; a paragraph ends a line, `a:br` is a new line; tables as in Word. Slide-number fields (`a:fld type="slidenum"`, on every notes page) are left out. Charts, SmartArt and comments are left for later.

### Where the text goes

Into `contents` as it is: one row per part, its `part` the order, its `label` where it is. **No migration 2 is needed:** step 1's `contents (file_id, part, label, text)` already keeps a location per row, and step 3 shows the label next to the snippet.

| Kind | Labels |
|---|---|
| Word | `page 1`, `page 2`, … where the file has page breaks; else the body is one unlabelled part. Then `headers and footers`, `footnotes`, `endnotes`, `comments`. |
| Excel | the sheet's name (`Q2`) |
| PowerPoint | `slide 3`, `slide 3 notes` |

Word pages are not in the file: they are where Word laid them out when it last saved, which it marks with `w:lastRenderedPageBreak`, plus page breaks typed by hand (`w:br w:type="page"`). Every typed break counts, so two in a row leave a blank page; a rendered break with no text since the last break is dropped (Word writes one after each typed break). Inside a table, a break at a row's start puts that row on the new page; breaks further into a row put the next row on the page after them. A page with no text costs nothing (the sink takes over a part that has no text yet), and a file keeps at most **25,000 parts** (10,000 slides with their notes fit); text past that is left out and `truncated` recorded. A file saved by something other than Word may have no marks (one unlabelled part) or stale ones (pages a little off). That is good enough for "found on page 3". `Sink` gains `relabel` (the body's first part becomes `page 1` once a break is seen) and `cut` (stop, recording `truncated`, for the readers' own caps).

### Facts

| Key | From |
|---|---|
| `title`, `author` | `docProps/core.xml`: `dc:title`, `dc:creator` |
| `created`, `modified` | `docProps/core.xml`: `dcterms:created`, `dcterms:modified`, as written (W3C dates, `2026-11-03T09:15:00Z`) |
| `pages` | `docProps/app.xml`'s `Pages` (Word's count when it last saved) |
| `sheets` | the sheet names in order, joined by ", " |
| `slides` | the number of slides in the presentation's slide list |

### Safety

Every Office file is an untrusted zip of untrusted XML.

- **Size:** files up to **100 MB** are opened (a deck's images and videos make it large, but only its XML is unpacked); larger ones are skipped. This departs from step 1's 4 MB, chosen for text read whole; the owner may want to revisit it.
- **The zip's list of parts:** opening a zip reads its central directory into memory, one entry for every 46 bytes of it. At most **4 MB** may be read while opening (a reader around the file counts), which bounds that memory whatever the zip claims; past it the file is recorded failed ("damaged, or its list of parts is larger than 4 MB"). A zip whose end record declares more than **10,000 parts** is skipped before the zip crate reads it (it reserves room for every declared entry first), and so is one that turns out to hold more.
- **Zip bombs:** every part is unpacked through a counter, never trusting the sizes the zip declares. A part that has unpacked more than **1 MB** at more than **100 times** its compressed size is a bomb: the file is skipped ("looks like a zip bomb"), and nothing is kept. XML compresses 10 to 20 times; a sheet of repeated empty cells may reach 50. Over all parts, at most **128 MB** is unpacked; past that, reading stops and what was read is kept, with `truncated`, as at the character cap.
- **XML:** `quick-xml` does not read DTDs: a `<!DOCTYPE>` is passed over, and an entity it declares is never expanded. Only XML's five named entities and character references are decoded; any other `&name;` is dropped. So "billion laughs" is one short, unexpanded reference (a test pins it). `quick-xml` 0.42 is past both of 2026's advisories (quadratic duplicate-attribute checks and namespace growth, fixed in 0.41); the namespace reader isn't used, names are matched by their local part.
- **Password-protected files** are not zips: Office wraps them in an OLE compound file holding `EncryptionInfo` and `EncryptedPackage`. A file starting with the compound-file signature has its directory read (bounded: at most 1,000 directory sectors, following the FAT, so a loop ends). With `EncryptedPackage` it is skipped as "password-protected"; without, as "an older Word, Excel or PowerPoint format" (a `.doc` renamed). A zip whose parts use zip encryption is skipped as "password-protected" too. Skipped files are not retried until they change, so there is no error loop.
- **Damaged zips** (truncated, bad entries, a compression method other than stored or deflate): recorded failed ("damaged or incomplete: …"), not retried until the file changes. A file that is neither a zip nor a compound file, or a zip without the main part (a `.zip` renamed `.docx`), is skipped ("not a Word document", "not an Excel workbook", "not a PowerPoint presentation").
- **Very large sheets:** at most **1,000,000 cells** per workbook are looked at, empty ones included, matching the million-character cap (a kept cell costs at least two characters, so the character cap usually comes first). Shared strings are kept up to **4 million characters** (counted as characters, however a string's runs split it) and **1,000,000 strings** in all; past the characters, later strings read as empty, and past the count the rest of the list isn't read; either way `truncated` is recorded. A cell's value is kept up to Excel's own limit, 32,767 characters; the rest is dropped and `truncated` recorded. Cell styles and custom number formats are kept up to **65,536** each (Excel's own limits are 64,000 and about 250); each format is classified as date, time or number once, as it is read. Shared strings are stored trimmed, as a cell shows them (a blank one is empty, so a cell naming it costs nothing). Each cell is written as it ends and a shared string is borrowed, not copied, so a row of many cells naming one long string holds it once (the review's 650 KB file had asked for 571 MB). Reading stops at whichever cap comes first, and keeps what was read.
- **Many sheets or slides:** at most **10,000** are read, in order; past that the rest are left out and `truncated` is recorded (the `slides` fact still counts them all). Their relationships are looked up in a map, not one by one: the review's 60,000 slides against 60,000 relationships had taken 43 s.
- **Time:** the worker's limit (10 s) is used as it is. A reader past its time stops itself, rather than being abandoned: the XML loop asks the sink every 256 events (a clock read costs tens of nanoseconds), counted over every part of the file, and each sheet, slide and Word part asks before it is read (a part with no text never asks otherwise).
- **Memory:** the XML reader holds each event whole (a text node, a tag with its attributes, a comment, a CDATA section), so one event may be at most **4 MB**: the bytes under the reader are handed out no faster than that allows, and past it reading stops and keeps what was read, with `truncated`, as at the unpacking cap. Real Office files never come near it (Excel's own limit on a cell is 32,767 characters). Elements may nest at most **10,000** deep, and an element's name may be at most **256** bytes (the XML reader keeps each open element's name; real files nest a few dozen deep, with names under 64 bytes), past which reading stops the same way. Attributes are read without quick-xml's check for repeated names, which scans every earlier name for each repeat inside one event. A byte-order mark doesn't count against the first event. An attribute value longer than 4 KB reads as absent; a .rels part's first **50,000** relationships are kept, and at most **4 MB** of their resolved targets (past either, `truncated`); a target that resolves to a path over 4 KB is left out. A paragraph, or a table cell (a nested table's text included), longer than 64 KB is written as it comes; a document property is held only up to 4,000 bytes (a fact keeps 1,000 characters), and only its first value is kept; a file keeps at most 1,000 facts. The second review's files (blank pages, a 20 MB relationship path, an 80 MB text node) had asked for 290 to 440 MB.
- **Paths inside the zip** are only looked up, never written: no extraction, so none of the zip crate's extraction advisories apply.

### Dependencies

- `zip` **9.3.0** (pinned), with default features off and only `deflate-flate2` (flate2's pure-Rust miniz_oxide, already in the tree through Tauri): no AES, bzip2, lzma, zstd or time crates. 9.0 fixes every RustSec advisory against the crate, the latest issued 2026-11-02; they all concern extraction, which SIFS never uses. It needs Rust 1.88; Tauri 2.12 already needs 1.90, so `rust-version = "1.77"` in Cargo.toml is already stale (left as it is).
- `quick-xml` **0.42.0** (pinned), default features (none).

### Tests

The `.docx`, `.xlsx` and `.pptx` samples are built in the tests with the `zip` crate from minimal XML, so no binary files are committed.
- **Word:** text in order across paragraphs, runs, tabs and breaks; a table as tab-separated rows; text boxes once; deleted text and field codes left out; pages from rendered and typed breaks, a doubled break counted once; headers, footers, footnotes and comments as their own parts; the facts.
- **Excel:** two sheets labelled with their names, in the workbook's order (not the files' names); shared and inline strings; rich text runs; booleans, errors, numbers as stored; dates and times through built-in and custom formats, and the 1904 system; formulas' values; the sheet names fact.
- **PowerPoint:** slides in the presentation's order, each followed by its notes; a table; the slide count.
- **Unicode:** Hindi, Japanese and emoji in each kind, found by search.
- **Safety:** a zip bomb (skipped, and its cap's test fails when the ratio check is removed); the total unpacked cap (truncated); too many parts; an oversized list of parts; a billion-laughs DOCTYPE; a password-protected file (a compound file built in the test) and a renamed old one; a truncated zip; a non-Office zip named `.docx`; a file over 100 MB never opened; the cell cap and the shared-strings cap; the time limit. Each cap's test is checked to fail with the cap removed.
- **Search:** `store::match_contents` finds a word from each kind, with its label (a page, a sheet's name, a slide, its notes).

### Left for a Mac session

PDF through PDFKit, page by page (labels `page 1`…, from the PDF's page index), and its tests; real Office sample files saved by Word, Excel, PowerPoint, Pages and LibreOffice, to check the page marks and the facts against what the apps write.
