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
| PDF | PDFKit (macOS), page by page, in a helper process (step 2b) | no text layer → OCR the page images (below) |
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
- **Queue order:** first the files the user opened or changed recently, then documents (PDF, Office, text), then the rest, then images (OCR, the slow step; step 4 measured why). The order is kept in the database, so a restart picks up where it left off.
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
- **A file's `modified` or `size` changes:** it's queued again at priority 0. The old text stays in the index until the new read replaces it, but from step 3 search leaves it out (its extract no longer matches the file). An unchanged rescan queues nothing (a test pins this, since the upsert rewrites every column).
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
- **The zip's list of parts:** opening a zip reads its central directory into memory, one entry for every 46 bytes of it. At most **4 MB** may be read while opening (a reader around the file counts), which bounds that memory whatever the zip claims; past it the file is recorded failed ("damaged, or its list of parts is larger than 4 MB"). A zip whose end records declare more than **10,000 parts** is skipped before the zip crate reads it (it reserves room for every declared entry first), and so is one that turns out to hold more. Every end record the zip crate could use is looked at (the last 22 + 4 + 65,536 bytes, and the 20 before them for a locator), with every zip64 record from where its locator points up to the locator, since the zip crate moves past one it rejects (at most 4 MB per locator and 8 MB in all; past that, or with no zip64 record there, the file is recorded as damaged).
- **Zip bombs:** every part is unpacked through a counter, never trusting the sizes the zip declares. A part that has unpacked more than **1 MB** at more than **100 times** its compressed size is a bomb: the file is skipped ("looks like a zip bomb"), and nothing is kept. XML compresses 10 to 20 times; a sheet of repeated empty cells may reach 50. Over all parts, at most **128 MB** is unpacked; past that, reading stops and what was read is kept, with `truncated`, as at the character cap.
- **XML:** `quick-xml` does not read DTDs: a `<!DOCTYPE>` is passed over, and an entity it declares is never expanded. Only XML's five named entities and character references are decoded; any other `&name;` is dropped. So "billion laughs" is one short, unexpanded reference (a test pins it). `quick-xml` 0.42 is past both of 2026's advisories (quadratic duplicate-attribute checks and namespace growth, fixed in 0.41); the namespace reader isn't used, names are matched by their local part.
- **Password-protected files** are not zips: Office wraps them in an OLE compound file holding `EncryptionInfo` and `EncryptedPackage`. A file starting with the compound-file signature has its directory read (bounded: at most 1,000 directory sectors, following the FAT, so a loop ends). With `EncryptedPackage` it is skipped as "password-protected"; without, as "an older Word, Excel or PowerPoint format" (a `.doc` renamed). A zip whose parts use zip encryption is skipped as "password-protected" too. Skipped files are not retried until they change, so there is no error loop.
- **Damaged zips** (truncated, bad entries, a compression method other than stored or deflate): recorded failed ("damaged or incomplete: …"), not retried until the file changes. A file that is neither a zip nor a compound file, or a zip without the main part (a `.zip` renamed `.docx`), is skipped ("not a Word document", "not an Excel workbook", "not a PowerPoint presentation").
- **Very large sheets:** at most **1,000,000 cells** per workbook are looked at, empty ones included, matching the million-character cap (a kept cell costs at least two characters, so the character cap usually comes first). Shared strings are kept up to **4 million characters** (counted as characters, however a string's runs split it) and **1,000,000 strings** in all; past the characters, later strings read as empty, and past the count the rest of the list isn't read; either way `truncated` is recorded. A cell's value is kept up to Excel's own limit, 32,767 characters; the rest is dropped and `truncated` recorded. Cell styles and custom number formats are kept up to **65,536** each (Excel's own limits are 64,000 and about 250); each format is classified as date, time or number once, as it is read. Shared strings are stored trimmed, as a cell shows them (a blank one is empty, so a cell naming it costs nothing). Each cell is written as it ends and a shared string is borrowed, not copied, so a row of many cells naming one long string holds it once (the review's 650 KB file had asked for 571 MB). Reading stops at whichever cap comes first, and keeps what was read.
- **Many sheets or slides:** at most **10,000** are read, in order; past that the rest are left out and `truncated` is recorded (the `slides` fact still counts them all). Their relationships are looked up in a map, not one by one: the review's 60,000 slides against 60,000 relationships had taken 43 s.
- **Time:** the worker's limit (10 s) is used as it is. A reader past its time stops itself, rather than being abandoned: the XML loop asks the sink every 256 events (a clock read costs tens of nanoseconds), counted over every part of the file, and each sheet, slide and Word part asks before it is read (a part with no text never asks otherwise). Every compressed byte the zip crate reads asks too, so a part of empty deflate blocks, which unpacks to nothing that any cap counts, still stops at the deadline; a read refused for its time is a timeout, not damage.
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

### Reviews (2026-11-04)

Step 2 had seven review rounds. Each finding was fixed with a test that fails without its fix (60 fixes, each checked by undoing it). Rounds 1 to 6 found 2, 4, 5, 3, 1 and 1 blocking problems; round 7 said RIGHT, with two minor findings, both fixed.
- **Round 1:** an Excel row naming one long shared string held a copy per cell (571 MB from a 650 KB file); slide and sheet lists were matched to relationships one by one (43 s).
- **Round 2:** blank pages, a 20 MB relationship path and an 80 MB text node each held 290 to 440 MB; one long number format named by many styles took 24 s.
- **Round 3:** deep nesting, repeated properties and nested tables (the round's report was lost to a dropped connection; its tests survived).
- **Round 4:** repeated attribute names (quadratic in quick-xml's check), relationship paths joined to long folders, long element names.
- **Round 5:** a blank shared string re-trimmed per cell between time checks; a zip64 entry count reserved before reading.
- **Round 6:** empty deflate blocks, which no cap counted, decoded inside one read.
- **Round 7:** two zip64 edge cases of the entry-count check; a test of the XML loop's own time check.

The reviewers' tests are kept as `office/review.rs` to `review7.rs`. The reports were in `handover/`, removed when the step merged: `git show <the commit before "Remove the handover folder">:handover/step2-review-r7/REPORT.txt` brings one back. Rounds 1 to 3 were reviewed by gpt-6-astra, rounds 4 to 6 by claude-opus-5-5 (astra was stopped by its provider's content filter), round 7 by gpt-6.1-sol.

**Checked on a Mac (2026-11-04):** the three readers over the 706 Word, Excel and PowerPoint files on the owner's Mac: no crash and no timeout; 669 read (at most 131 ms each, median under 1 ms), 2 skipped as password-protected. The other 35 were iCloud placeholders: opening one asks macOS to download it, and the read blocked about 4.6 s before failing. Step 1 already skips placeholders before opening them (`platform::may_read`, the `SF_DATALESS` flag): checked against those files, it skips exactly the 35, agreeing with macOS's own flag on all 706.

### Left for a Mac session

PDF through PDFKit, page by page (labels `page 1`…, from the PDF's page index), and its tests; real Office sample files saved by Word, Excel, PowerPoint, Pages and LibreOffice, to check the page marks and the facts against what the apps write.

## Step 2b in detail: PDF through PDFKit

Step 2's second half. One reader, `pdf`, macOS only, registered in `readers/macos`. PDFKit reads each page's text; each PDF is read in a **helper process** (SIFS's own program, started again in a reader mode), which SIFS kills when it uses too much processor time or memory, or runs past the time limit. Linux has no PDF reader yet, and still builds.

### In one process, or in a helper: what the measurements decided

PDFKit runs in the process that calls it, and one call (`PDFPage.string`, a page's text) can't be interrupted. Before choosing, both shapes were measured (the prototype and its scripts are outside the repo, in the orchestration notes for this step: `phase4/p3-pdf/`). Each generated file below was read by PDFKit with no page or time cap, one file per process, killed at 120 s.

| Generated file | What PDFKit did |
|---|---|
| a page whose content stream inflates to 1 GB of `q Q` (1 MB file) | `string` on that page took **37 s** |
| a page of 2 million text operators (5 MB) | **48 s** for that page |
| a page of 400 MB of text operators, compressed (1.3 MB) | **20 s** for that page |
| form XObjects ten deep, each drawing the next ten times (3 KB) | never returned: **30 GB** of memory (14 GB resident) when killed at 120 s |
| 10,000 pages; 100,000 pages (16 MB) | 5.3 s; not finished at 120 s |
| a self-drawing XObject; a 12,000 × 12,000 image page | 25 ms; 170 ms, no text |
| damaged: cut in half, no xref, 40 flipped bytes, random bytes after `%PDF`, zero bytes, a page tree that contains itself, arrays nested 500,000 deep, `/Count 2000000000` | PDFKit refuses to open each in under 70 ms; no crash, no hang |
| a user password; an owner password only (copying not allowed) | opens locked in 75 ms; opens and reads normally |

So in one process, one page can hold the reader for tens of seconds or take the Mac's memory, and the worker can only abandon its thread (step 1's risk), which keeps running. **macOS doesn't enforce memory limits on a process:** a child with `RLIMIT_AS` and `RLIMIT_DATA` at 512 MB (setrlimit refused both) allocated and touched 4 GB. A limit has to be enforced by watching: the XObject file, read by a helper that SIFS polled and killed at a 1 GB footprint, was killed 2.8 s in, at 1,036 MB.

**The owner's PDFs.** Spotlight lists 2,698 under `~`; 75 are iCloud placeholders (not opened), and 2,623 were read, one helper process per file, a 10 s limit, no page cap, a 1 GB memory kill:

| | Count |
|---|---|
| text found | 1,718 (7 reached the million-character cap) |
| no text in any page (scans) | 896 |
| some pages without text | 67 files, 307 pages |
| password needed | 3 |
| PDFKit wouldn't open | 6 (4 empty files, 2 cut short at 512 KB) |
| opened, owner password only | 25 (read normally) |
| stopped by time or memory, or crashed | 0 |

Per file, in a helper: median **34 ms**, p95 110 ms, p99 422 ms, max 1.44 s. The helper's peak resident memory: median 19 MB, p99 86 MB, max 276 MB (a 588-page file at the character cap). The most pages was 973; the median page took 2.75 ms (files of more than 20 pages). The largest file (207 MB) is a scan with no text.

The same files in one long-lived process: median 5.2 ms per file, 65 s in all against the helper's 138 s; the process peaked at 539 MB resident and still held 153 MB after the last file. The difference is PDFKit starting in each new process: opening a document took a median of 21 ms in a fresh helper and 0.7 ms in a warm process (the spawn itself, about 6 ms). Over 400 of the files, alternating the two shapes five times on a loaded machine (a load average of 15 to 36 on 18 cores, from other builds), one process took a median of 13.4 s (8.8 to 22.4) and a helper per file 27.7 s (17.7 to 41.3): about twice as long in every pair (1.8 to 2.7 times).

**Decision: a helper process per PDF.** It bounds time and memory whatever the file is, a crash in PDFKit costs one file instead of SIFS, and PDFKit's caches leave with the helper. It costs about 30 ms per file: about 75 s of background time over the owner's PDFs, read once. This departs from decision 8 of step 1 ("Mac framework readers run in-process first; a helper process if crashes show up"): the measurements above show hangs and unbounded memory without any crash. A helper that serves many files in turn would save most of the 30 ms but needs a request loop and a rule for when to replace it; left for when step 6's measurements ask for it.

**Processor time, not the clock, stops a page that never ends.** The helper runs at "utility" priority, so a busy Mac runs it last. The first full run of the reader over the owner's PDFs, with the worker's 10 s limit by the clock, was made while other programs kept 18 cores at a load average of 100 to 320: 4 files that take 0.15 to 1.1 s alone were stopped at 10 s, and would not be read again until they changed. A page that never ends uses the processor the whole time; a starved helper hardly uses it. So the helper is stopped at **10 s of processor time**, and the clock limit for PDFs is **60 s**. The second run, at a load average of 300 to 780, stopped no file; the slowest took 18.5 s by the clock.

### The helper

- **Starting it:** the reader runs the program SIFS is running (`std::env::current_exe`) with `SIFS_PDF_HELPER` set. The program's `main` (the app, and `sifs-bench`) first calls `extract::serve_helper_if_asked()`, which reads the one PDF and exits, never starting the app. The PDF's path and its limits are in the environment. The test binary is its own helper, through an ignored test that calls the same function.
- **Talking to it:** the helper writes a marker, then frames on stdout: the page count, the facts, each page as it finishes (its text with its index, or its index alone when it has no text), then an end frame (or "password needed", "can't open"). Anything before the marker is ignored (the test binary prints a line first). A frame cut short is dropped.
- **Trusting it no further than the protocol:** SIFS takes one page count before any page, then page indexes strictly increasing, below the count and the page cap. Anything else stops the read as output that makes no sense, keeping the pages before it, so no text is labelled with a page that can't exist or comes out of order. Every whole frame that has arrived is taken before a broken frame or the output cap stops the read, however the pipe splits the bytes. SIFS counts the pages without text itself and records `textless_pages` once, when the read ends however it ends.
- **Output that ends early:** if stdout closes before the end frame, SIFS kills the helper at once and reaps it, so it can't go on using the processor or memory unwatched.
- **SIFS going away:** the helper's stdin is a pipe from SIFS that is never written. When it closes (SIFS quit or crashed), the helper exits. It also exits a second after SIFS's deadline, if SIFS has not killed it.
- **Priority:** the helper runs at macOS's "utility" quality of service, like the worker. The worker runs at most two reads at a time, so at most two helpers. The tests' helper keeps the test's priority: at "utility", with every core busy, a three-page read took 13 s alone and over 30 s in the full suite.
- **Always reaped:** however a read ends, the helper is killed if still running and waited for, so none is left behind.

### Limits

| Limit | Value | Enforced by | Past it |
|---|---|---|---|
| file size | 512 MB (owner's largest: 207 MB) | the worker | skipped, never opened |
| processor time | 10 s, polled every 20 ms | SIFS kills the helper | the pages read are kept, with `truncated`; with none, failed ("used more than 10 s of processor time") |
| time | 60 s by the clock, the worker's limit for this reader | SIFS kills the helper at the deadline | the same; with no pages, failed ("took longer than 60 s") |
| memory | 1 GB physical footprint, polled every 20 ms | SIFS kills the helper | the same, "used more than 1024 MB of memory" |
| output | 8 MB read from the helper | SIFS stops reading and kills it | the same, "sent more than 8 MB" |
| a crash | | the helper ends on a signal | the same, "PDFKit crashed reading it"; output that makes no sense is treated the same way |
| output ends early | | the helper closes stdout before its end frame | SIFS kills and reaps it at once; the same, "the PDF reader closed its output early" if it was still running, or how it ended |
| pages | 10,000, in order | the helper | the rest left out, `truncated` |
| characters | 1,000,000 | the helper cuts each page to what is left; the sink keeps the cap too | `truncated`, unless the text ends exactly at the cap with no page left |

Keeping the pages read before a stop departs from the other readers, which record a timeout as failed with nothing kept: a 900-page file whose page 600 stalls is still found by its first 599 pages. Pages without text count as read: a scan whose page 3 stalls is recorded `empty` with `textless_pages` 2, so OCR still finds it. Why it stopped is kept as a `stopped` fact. Such a file is not read again until it changes.

A file that doesn't start with `%PDF-` in its first 1,024 bytes (where PDFKit looks) is skipped as "not a PDF" without starting a helper; an empty file is recorded empty. A PDF that PDFKit can't open is failed ("damaged, or not a PDF PDFKit can read").

### Labels

One part per page with text, labelled `page 1`, `page 2`, … from the page's index in the document (the first page is `page 1`), not the labels a PDF may give its pages (`iv`, `A-3`): the index is what a viewer's page number shows by default and goes to. Pages without text cost nothing (the sink drops empty parts), so a part's number is not its page; its label is.

### Encrypted and image-only PDFs

- **A password needed to open** (a user password): skipped as "password-protected", as for Office. Not retried until the file changes.
- **An owner password only** (opens freely, with copying or printing restricted): read. The text is the owner's, on the owner's Mac, and Preview shows it.
- **No text in any page** (a scan): recorded `empty`, with nothing in `contents`, and the facts `pages` and `textless_pages` (equal). **Pages without text** in a file that has some: the file is read and `textless_pages` counts them. Step 4's OCR finds its work with `facts` where `key = 'textless_pages'`; it will raise the reader's version or add its own, which re-reads them. **No migration is needed:** `extracts` already has `empty`, and `facts` holds any key. SIFS PR #5's migration 2 is not touched.

### Facts

`pages` (the document's page count), `title` and `author` (the document's attributes, when set), `textless_pages` (when any), `stopped` (when the helper was stopped: why, and at which page).

### Dependencies

macOS only, under `[target.'cfg(target_os = "macos")'.dependencies]`, all pinned, default features off:
- `objc2-pdf-kit` **0.3.2** (new), features `PDFDocument` and `PDFPage`.
- `objc2-foundation` **0.3.2** and `objc2` **0.6.5**: already in the tree through Tauri; named directly for `NSString`, `NSURL` and `NSDictionary`.
- `libc` (already a dependency) for `poll`, `proc_pid_rusage` (the helper's footprint and processor time) and the helper's priority. `mach_timebase_info` is declared in the reader (libc's binding is deprecated): the processor times are in Mach ticks, which are not nanoseconds on Apple silicon.

### Tests

The PDFs are written byte by byte in the tests (a small PDF writer in the test module; Unicode text through a `ToUnicode` map, which PDFKit reads without the font), and the encrypted ones by PDFKit itself, saving a generated PDF with a password. No binary fixtures are committed. Each limit's test is checked to fail with the limit removed (the mutation scripts are in the orchestration notes).
- **Reading:** three pages labelled `page 1` to `page 3` in order; a blank page in the middle (no part, the third page still `page 3`); Hindi, Japanese, "résumé" and emoji found by `match_contents` with the page; the facts.
- **Encrypted:** a user password is skipped "password-protected"; an owner password only is read.
- **No text:** an image-only PDF is `empty` with `textless_pages` equal to `pages`; a mixed one keeps its text pages and counts the other.
- **Not a PDF, damaged:** no `%PDF-` (skipped, no helper started); an empty file (empty); a truncated file (failed).
- **Limits:** a page that never ends (form XObjects ten deep, ending in nothing) is killed at a lowered processor time cap, and within 0.75 s of a short clock deadline; with text pages before it, those are kept with `truncated` and `stopped`; the same forms ending in text are killed at a lowered memory cap; the clock limit is a minute; the page cap (lowered, `truncated`); the helper's character cap (its frames, read in the test process); the output cap (a test helper that floods); a test helper that crashes after a page, at once, or sends nonsense; a test helper that closes stdout and keeps running is killed and reaped at once, its page kept; a missing helper program; the helper ends when its stdin closes and a second after its deadline; no helper is left running or unreaped, including when making its stdout nonblocking fails (an interposed `fcntl`; an interrupted one is retried).
- **The protocol:** a page past the count or the page cap, out of order, twice, or before the count, and a second count, stop the read with the pages before kept; `textless_pages` is counted once and kept when the read stops; the scan before a stalled page stays marked for OCR. Each case is fed whole, a byte at a time and in sevens, with the same result.
- **The decoder:** what comes before the marker is passed over, frames arriving a byte at a time are put back together, an unknown frame ends the output after the frames before it, and the frames within the output cap are taken before it stops the read.
- **Through the worker:** a PDF is read by `run_until_empty` and found by "resume", a Hindi word and a Japanese word on its second page, labelled `page 2`; a scan is `empty` with `textless_pages`; a file that isn't a PDF is skipped; a 5 MB PDF is read and one over 512 MB is skipped; the platform registry reads `pdf` on macOS.

### Measured on the owner's Mac (2026-11-07)

The reader as built, over the 2,698 PDFs Spotlight lists under `~`, one after another, each with its limits (a scratch program calling `Pdf::read` as the worker does; the same program serves as the helper):

| | Files |
|---|---|
| read, text found | 1,718 (7 cut at the million-character cap) |
| empty (no text in any page, or an empty file) | 900 (896 scans, 4 empty files) |
| password needed (skipped) | 3 |
| damaged (failed) | 2 |
| iCloud placeholders (skipped, not opened) | 75 |
| stopped by a limit, or crashed | 0 |

27,526 pages; 43.8 million characters kept; 969 files have pages without text, marked for OCR. At a load average of 300 to 780 from other programs: 601 s in all, median 56 ms per file, p95 810 ms, max 18.5 s; the helpers used 108 s of processor time in all. SIFS's side peaked at 14 MB resident; the largest helper at 275 MB. Starting the helper from the app's own binary took a median of 50 ms against 47 ms from the small scratch program (15 runs each, alternating), so the app's size adds little to each start.

## Step 3 in detail: search over contents

**Goal:** one search box finds files by their names and by the words inside them. Each content result shows where its words were found (`page 3`, a sheet's name, `slide 2 notes`) and a short snippet with the words marked. Typing stays instant.

### How a query runs

Two commands:
- `search_files` (as now): names only, on every keystroke. It is shown at once.
- `search_all`, new: names and contents, ranked together. The window asks for it about 120 ms after typing stops and replaces the list when it comes. A response to an older query is dropped.

`search_all` runs off the main thread (`spawn_blocking`), on its own read connection. It has **400 ms**: past that the content query is stopped (SQLite's interrupt) and the name results come alone, or the snippets not yet made are left empty; either way the results are marked "contents took too long".

### The content query

- Each word is matched as a token through FTS5: `unicode61 remove_diacritics 2`, so "resume" finds "résumé" and Hindi words stay whole.
- The query is split into words as the index splits them (`index/query.rs`), and each is quoted, so the query language (`AND`, `NEAR(`, `*`, `"`, `:`) is never interpreted.
- The **last word matches as a prefix** while typing ("invo" finds "invoice"), but only when it has at least 3 characters, so one or two letters don't expand to most of the index.
- Every word must appear in the same part (page, sheet or slide).
- Per file, its best part counts (FTS5's bm25 rank), with its label and snippet. The first **10,000 matching parts** are looked at; past that the results are marked incomplete.

### Ranking (owner's decision 5)

A name match ranks above a content match only when kind and place are equal. Added-up scores can't give that both ways: an exact name match scores 25 to 100 by quality, while kind priors differ by up to 90 (documents +40, code −30, build products −50). So results are ordered in turn by:
1. **kind and place:** the kind prior plus the place prior (SDKs, dependencies and build folders −40);
2. **then, with those equal:** name matches, then content matches, then typo-tolerant name matches (a guess ranks below words that are really there);
3. **then score:** for names, match quality, recency and shorter names and paths, as now; for contents, the bm25 position among the content results (up to 10) and recency.

So a document whose text matches ranks above a source file whose name matches, and a document whose name matches ranks above one whose text does. A file matching by both name and contents is shown once, as a name match (score plus 5), with its snippet; a typo-tolerant name match whose contents match ranks as a content match.

The owner keeps decision 5 as it is for now (2026-11-04) and will revisit it after using it for a few days: the window check showed "screenshot" ranking 43 notes that mention the word above most screenshot images, and one option is to let a name that starts with the query beat the kind difference.

### Snippets

Made in Rust from the best part's text, for the shown results only: at most 200 characters, from the first whole word up to 40 characters before the first query word, every query word marked, and "…" where text is left out. Words are found and folded as the index finds and folds them (`index/query.rs`), so a hit is a position in the text, never a mark inside it, and the file's own characters can't pose as one. They become segments, `[{text, hit}]`, so the window never puts file text into HTML.

### The window

- A content result shows its file as now, then a second line: the label, then the snippet with the matched words in bold.
- The footer says how many were found, and whether contents were searched in full ("contents took too long", or "still reading N files" while the worker has a queue).

### Tests (Linux, `cargo test --no-default-features --lib`)

- **Ranking:**
  - a document's contents rank above a code file's name;
  - a document's name ranks above a document's contents;
  - a file matching both is shown once, first, with its snippet;
  - recency breaks ties.
- **Query safety:** queries with `"`, `*`, `AND`, `NEAR(`, `:`, `-` and parentheses give results or nothing, never an error. Every word must be in one part.
- **Prefix:** the last word at 3 or more characters matches a prefix; at 1 or 2 characters it matches only whole words.
- **Text:** "resume" finds "résumé" (composed and decomposed), and Hindi and Japanese samples are found by their words.
- **Snippets and labels:** the right label (page, sheet, slide, notes), and segments that rebuild the snippet with the hits marked.
- **Lifecycle:** a removed folder's files are never returned (`NOT_REMOVING`), and a file changed since it was read is found by its new words only.
- **Time:** a query stopped by the time limit returns names only, marked. Tested with a limit of 0 ms.
- **The command:** `search_all` returns the same as the library call.

The window has no tests; it is checked with `npm run check` and by hand on the Mac.

### Measuring

`sifs-bench --contents` gains query timings: common, rare and prefix words; the 95th percentile and the slowest; on the Linux fixtures here, and on the real home folder in step 6.

### Step 3 review, round 1 (2026-11-04): the redesign

Three independent checks of PR #5 at 6e11b57 (gpt-6.1-sol and gpt-6-astra code reviews, an Opus verifier driving the page in headless Chromium over a copy of the real index) found 14 distinct blocking problems, each shown by a test. Their reports: `devos-work/astra/s3sol/REPORT.txt`, `s3astra/REPORT.txt`, `s3ui/REPORT.txt`. They share three causes, so the content search changes shape rather than gaining patches:

1. **Candidates were capped before ranking.** Names were cut to the 60 best by name score, contents to 2,000 parts and then 200 files by bm25, and only then ranked by kind and place. So the right winner could be gone before the ranking ran (a document named or containing the word lost to 60 code files; one 2,000-page file hid every other match).
2. **SQLite's `snippet()` does unbounded work in one step.** On a part repeating the query word, it took 2.6 s to over 5 s, inside one VM operation the progress handler can't interrupt. Its markers (`\u{2}`, `\u{3}`) can also be in the file's own text, and its 12 tokens don't bound bytes (a 1 MB snippet).
3. **The query was split on spaces, not as the index splits words,** so punctuation changed what matched (`rent,invoice`, `alpha-beta` as a phrase, `in,` as a prefix), a NUL ended the FTS string, and 1,100 words overflowed SQLite's expression depth.

**The new shape.** The data, in order:
- **`Query`** (`index/query.rs`), parsed once at the boundary for both searches: control characters count as spaces; words split by the same rule as the `contents_fts` tokenizer (letters, digits, marks and private-use characters are word characters, everything else separates); folded as the tokenizer folds them (lowercase, then SQLite's own diacritic table); duplicates dropped; at most 16 distinct words, the rest ignored and reported; a word over 64 characters is cut there and matches by its start. An empty query returns nothing. The last word is a prefix when it has at least 3 characters. Name search keeps its substring terms but is bounded the same way.
- **Candidate files, without snippets.** Contents: one row per file, its best part by bm25 (grouped per file in SQL), from contents whose extract still matches the file (`extracts.modified = files.modified AND extracts.size = files.size`, so an edited file's old words are never found), not removing. Names: every name candidate (the name search's own bound, 5,000), not its top 60. The work is bounded by the deadline and a cap on matching parts scanned (the first 10,000, in rowid order: the order files were read, recent ones and documents first); hitting either marks the results incomplete, each in its own words (see the cap below).
- **Ranking over all candidates,** then the limit: kind and place priors first; then name matches, content matches, typo-tolerant names; a typo-tolerant name that also matches by contents ranks as a content match.
- **Snippets for the shown results only** (at most the limit), built in Rust from the best part's text: the first window around a query word, at most 200 characters, with hits as character ranges (`Segment{text, hit}`), never markers in the text. The scan checks the deadline as it goes.

**The window:** typing and Escape cancel both pending searches at once (the generation advances on input, both timers are cleared, error paths check the generation too); a finished read of new files refreshes the current search (coalesced, at most every 2 s); the selection follows the file only after the user moved it, else stays on the first row; the selection is kept across a refresh; separators keep their spaces.

**Not in this PR** (already on main, tracked as issues): light appearance rows unreadable (`app.html` sets a black body background); the search box loses focus at startup (SvelteKit's focus reset).

**As built.** Where it differs from the design above, and why:
- **Control characters count as spaces**, not dropped: a pasted tab must separate two words, not join them.
- **A word over 64 characters is cut and matched by its start**, not ignored: a pasted long file name must still find the file. Words past 16 are ignored, and the window says so ("too many words, some not searched").
- **Folding uses SQLite's own diacritic table** (`fts5_remove_diacritic`, copied from the bundled 3.46) with Rust's lowercase, so a word is marked in a snippet exactly when the index matched it. A test tokenizes a sample of scripts both ways and compares.
- **Migration 2 adds `contents_owner ON contents(id, file_id)`.** Learning a matching part's file from its own row reads that row, and a part's row holds its text, so most rows fill a page. On the index copy, 10,000 matching parts of "data" read 8,537 pages and took 0.5 to 6.1 s that way; through the index, 515 pages and 17 ms. The migration took 5.6 s once on the copy (173,113 parts); the owner's index has no contents yet. A test bounds the pages one search reads.
- **The 790 ms overshoot** in the window check came from work SQLite does inside one step, where the progress handler can't stop it: `snippet()` over whole parts, and reading text-filled rows from disk. Neither happens now before the limit: the content query reads no text, and snippets are made only for the shown results, checking the time as they go. The 400 ms covers the whole call. What remains unsplittable is one FTS5 prefix expansion (`com*`: 40 ms) and reading one shown part's text (at most 4 MB).
- **A cap hit is reported apart from the time limit** (round 2): `contents_capped` shows "very common words: only some files' contents searched" and only the time limit shows "contents took too long", because a cap hit is fast (about 70 ms for "the" or "rev") and its parts are the first 10,000 in storage order rather than the best, since ranking every match is the work the cap bounds.
- **The window's selection rule** pins the opposite of two of the reviewers' probes (sol's `transition`, astra's window `selection`): they expect an untouched selection to follow its file. They fail by design. Copies where the user moves the selection first pass.
- **Not done:** astra's suggestion that obsolete `search_all` calls stop queueing behind the content connection. Each call is now bounded by the limit and measured at 105 ms at most on the copy, and the window asks only after typing pauses for 180 ms. Revisit if the Mac shows a queue.

**Tests.** `extract/find_tests.rs` holds the reviewers' Rust tests, adapted to `search_all` (their helpers now write the extract row the worker writes), and new ones for the cap, the snippet window, the deadline inside a snippet, the fold and the pages read: 27 tests. Each fix was undone in a copy and its test failed (16 Rust mutations, all caught); the window's fixes were undone the same way against the reviewers' probes and `drive.mjs` (9 mutations, all caught). Script and log: `devos-work/astra/s3fix1/mutations.py`, `mutations.log`.

**Measured** (release build, the index copy from the window check: 385,983 files, 173,113 parts; `search_all` with limit 60 and the 400 ms limit; median and slowest of 5 runs, warm; the machine was loaded, with a VM and a disk scan running):

| Query | Before | After |
|---|---|---|
| revenue | 15.8 / 16.8 ms | 8.2 / 9.0 ms |
| quarterly | 28.5 / 29.7 ms | 8.8 / 9.1 ms |
| screenshot | 24.2 / 24.5 ms | 6.4 / 6.7 ms |
| resume | 12.7 / 13.0 ms | 16.2 / 16.3 ms |
| gross profit | 20.3 / 20.8 ms | 8.0 / 8.7 ms |
| rev | 46.4 / 47.0 ms | 33.8 / 34.1 ms, cap |
| "gross profit" OR revenue* | 26.5 / 27.1 ms | 8.5 / 8.8 ms |
| NEAR(a b) -x AND | 56.4 / 57.3 ms | 31.8 / 31.8 ms |
| привет | 4.9 / 5.1 ms | 2.6 / 3.7 ms |
| zzqxjvwk | 0.9 / 1.0 ms | 0.8 / 0.9 ms |
| customer | 328.1 / 330.4 ms | 14.9 / 15.1 ms |
| 日本 | 37.8 / 39.8 ms | 37.4 / 40.3 ms |
| शर्मा | 2.4 / 2.5 ms | 0.4 / 0.5 ms |
| the | 472.3 / 474.7 ms, timed out | 72.3 / 73.4 ms, cap |
| and | 470.2 / 470.7 ms, timed out | 37.8 / 38.5 ms, cap |
| data | 382.7 / 383.5 ms | 38.5 / 40.7 ms, cap |
| com | 554.4 / 3,825.8 ms, timed out | 102.4 / 103.3 ms, cap |
| inv | 139.5 / 140.5 ms | 31.8 / 32.8 ms, cap |
| invoice | 57.9 / 60.0 ms | 8.9 / 9.3 ms |

The slowest run of any query, first runs included: 3,826 ms before ("com"), 104.5 ms after ("com"). One part repeating "needle " 30,000 times: 2.70 to 2.78 s before, 0.4 to 0.7 ms after; "tapir " 100,000 times: 29.2 to 31.6 s before, 0.7 to 0.9 ms after. One part of "needle" and 999,980 emoji: a 3,999,932-byte snippet before, 782 bytes after. 200 parts of a million characters, limit 1: before, stopped at 400 ms with no result while peak memory rose from 32 to 136 MB; after, 2.6 ms, one result, peak memory 32.2 to 33.5 MB; with limit 60, 51.5 ms and 23,730 bytes of snippets in all.

### Step 3 review, round 2 (owner's bar: real bugs)

Three checks of PR #5 at 5054213: gpt-6.1-sol and gpt-6-astra code reviews, and a run of the real window through the verify skill. Their reports: `devos-work/reviews/s3r2sol/REPORT.txt`, `s3r2astra/REPORT.txt`, `s3r2ui/REPORT.txt`. The owner's decision for this round (2026-11-08): fix every problem a real user could hit, plus cheap bounds and the small window items. Stress cases nobody produces by hand are recorded below as known limits, with their numbers, and measured again on the real home folder in step 6. Each fix gets a test that fails without it, checked by undoing the fix.

1. **One snapshot per search.** A worker commit between the candidate query and the snippet read could attach another file's label and text to a result (part ids are reused), and a part deleted in between was reported as "contents took too long". Now `search_all` reads names, contents and snippets inside one read transaction, so every read sees the same index. A candidate keeps its file id with its part id, and the label and text are read together, by both ids. A missing part gives no snippet and does not count as a timeout. Only the time limit sets `contents_timed_out`.
2. **Edits within the same second.** File times were stored in whole seconds, so an edit that kept the size and the second stayed searchable by its old words, permanently. Migration 3 adds `modified_ns` (the nanoseconds within the second) to `files`, `extracts` and `extract_queue`. NULL means unknown: rows written before the migration. The queue's triggers queue a file again when its second or size changes, or when both nanosecond values are known and differ. Contents count as current when second and size are equal and the nanoseconds are equal where both are known. The worker reads a queued file again unless what it read has the same known nanoseconds. So the first scan after the upgrade fills in the nanoseconds without queuing every file again. The migration runs in one transaction with its version number, like migration 2, and a test interrupts it and opens the index again.
3. **SQLite's own tokenizer.** Query words were split and folded in Rust (a regex and a copy of SQLite's diacritic table), which disagrees with SQLite on newer emoji, Cherokee, Greek symbol variants and about 1,900 BMP characters. So a file could not be found by its own text, or was found without the word marked. Now query words and snippet word positions come from the `contents_fts` tokenizer itself, created through FTS5's C API (`xFindTokenizer("unicode61")`) with the same arguments as the table. One constant holds the arguments, and a test checks it against the stored table. Each word's own text goes into the FTS query, quoted, so FTS folds it as it folded the file. The regex and the diacritic table are deleted. A test tokenizes every BMP character both ways (through `fts5vocab` and through the new path) and compares.
4. **Bounded input.** A pasted 30 MB query took about 1 s and 350 MB to parse, outside the time limit. Now `Query::parse` reads at most 4 KiB of what was typed, cut at a character boundary. The window sends at most 4,097 characters, so the message to the backend is bounded and the backend still knows that the text was cut. Words past 16 set a flag; they are not kept to be counted. `Results.words_left_out` replaces `ignored_words`, and the window shows "too many words, some not searched" for a cut query too.
5. **Candidates in ranking order.** The 5,000 name candidates were the first 5,000 stored, and the typo-tolerant pass ran only when fewer than the display limit matched, so a document named for the query could lose to 5,000 code files, and the first result changed between limit 60 and 61. Now the name rows stream into a bounded selection that keeps the 5,000 best by the order `search_all` ranks in (kind and place, then exact before close, then score). The typo-tolerant pass runs unless more than 5,000 names match exactly, whatever the limit, and its rows are selected the same way. More than 5,000 matching names sets `names_capped`, shown as "many names match: the best 5,000 compared". The names-only command selects by its own order (score). The 10,000-part cap for very common words stays as it is, disclosed: taking parts in ranking order means looking up the file of every matching part, which is the work the cap bounds.
6. **The whole call within 400 ms.** The clock started after the command waited for the contents connection, and the name queries ran before the time limit was installed. Now the clock starts when the command is called. One stop condition (the deadline, or a newer `search_all` call) stops every SQL statement of the call, names included, and the snippet scans. A newer call advances a counter, so an older call that is still running or waiting stops at its next check. Names cut short set `names_timed_out` ("names took too long"). `search_files` runs under the same limit.
7. **Small items.** A snippet is at most 200 characters including its ellipses. The footer says "still reading N files" while the worker has a queue. A successful search clears the previous search's error, not other errors. Leaving the page clears all three timers and drops replies still to come. While the contents answer is pending, an empty name list shows nothing instead of "Nothing found" (it flashed for about 120 ms before content results arrived).

**Known limits, left as they are** (measured by the reviewers on a loaded machine; revisit in step 6 on the real home folder):
- **A million long names matching many short terms.** 1,000,000 files with 234-byte names and a query of 16 two-letter terms took 0.98 to 1.58 s (median 1.02 s) before this round, because a term under 3 characters has no trigram index and is checked against every name. Item 6 stops it at the time limit and says so, so the window never hangs, but the names after the cut are not compared. Nobody has a million such names; the index copy has 385,983 files in all.
- **Memory for a prefix over millions of matching parts.** For the prefix query `common*` with limit 1, SQLite's extra heap peaked at 0.65 MB with 10,000 matching parts, 1.55 MB with 100,000 and 174 MB with 8,000,000. FTS5 reads the whole prefix's lists and ranks the phrase across them, whatever the outer LIMIT. Bounding it needs a prefix index on `contents_fts` (a rebuild of the index) or no bm25 ranking. 8 million matching parts is 800 files of 10,000 pages each, all containing the word; the index copy has 173,113 parts in all.

**As built.** Where it differs from the plan above, and why:
- **A missing part can't happen inside one snapshot.** With the read transaction, a part that matched is still there when its snippet is read, so the reviewers' delete-during-search test now finds the snippet. Reading by part and file stays, since a part id is reused once deleted, and a missing part still counts as no timeout.
- **The typo-tolerant names run whenever 5,000 or fewer names match exactly.** Any fixed number below that is a cliff where one more exact code file hides a close-named document, which the owner's ranking puts first. Close names still rank below exact names of the same kind and place.
- **Names are selected without building every row.** A name whose kind alone can't beat the 5,000th kept is skipped before it is ranked, and `place_prior` checks its markers with one case-insensitive regex instead of lowercasing every path. Without these, "a" took 176 ms for names on the index copy.
- **`search_files` runs off the main thread too,** and a newer call stops an older one, as for `search_all`: a one-letter query now compares every name.
- **Not covered by a test:** that the clock starts before the wait for the contents connection is the order of two lines in `app.rs`, which needs Tauri to test. The test that a newer call stops an older one covers what bounds the wait.

**Tests.** `extract/find_review_tests.rs` holds the reviewers' round 2 tests, adapted, and one or more per fix; `index/tokenize_tests.rs` compares the tokenizer with `contents_fts` over every BMP character. Each fix was undone in place and its tests failed (17 Rust mutations, all caught); the window's fixes were undone in a copy of the page and the reviewers' Chromium probes, extended by four, failed (6 mutations, all caught). Scripts and logs: `devos-work/reviews/s3fix3/mutations.py`, `mutations2.out`, `ui/ui-mutations.py`, `ui/ui-mutations.log`.

**Measured** (release build, the index copy: 385,983 files, 173,113 parts; median and slowest of 5 warm runs of `search_all` with limit 60, and one run of the names-only search; 5054213 against this round's head, run alternately on a loaded machine):

| Query | Names only, before → after | search_all, before → after |
|---|---|---|
| revenue | 6.6 → 9.1 ms | 8.8 / 10.3 → 11.6 / 12.2 ms |
| resume | 5.9 → 11.3 ms | 17.1 / 18.3 → 22.1 / 23.0 ms |
| notes | 2.3 → 27.2 ms | 12.7 / 13.0 → 36.1 / 37.7 ms |
| data | 5.3 → 25.9 ms | 40.4 / 40.6 → 59.9 / 61.9 ms, both caps |
| the | 2.0 → 2.1 ms | 76.3 / 79.0 → 75.9 / 77.1 ms, part cap |
| com | 6.2 → 10.6 ms | 103.6 / 110.2 → 115.0 / 127.0 ms, both caps |
| re | 6.9 → 73.7 ms | 30.1 / 30.7 → 94.9 / 102.6 ms, both caps |
| a | 3.2 → 146.6 ms | 23.3 / 24.8 → 116.9 / 119.0 ms, both caps |

The slowest run of any query: 110.2 ms before ("com"), 146.6 ms after (names only, "a"). The added time is the work the old code skipped: it compared the first 5,000 names stored and stopped, and for "a" 302,651 names match; reading those rows alone takes SQLite about 70 ms. One- and two-letter queries pay most. If that is felt in use, a name index for short terms is the fix, not a return to storage order. Full tables: `devos-work/reviews/s3fix3/timings-before-r4.tsv`, `timings-after-r4.tsv`.

### Step 3 review, round 3 (frozen scope)

Three checks of PR #5 at a4668b4: gpt-6.1-sol and gpt-6-astra code reviews, and a run of the real window through the frozen verify skill (`devos-work/reviews/s3r3sol/REPORT.txt`, `s3r3astra/REPORT.txt`, `s3r3ui/REPORT.txt`). This is the last round under the owner's round cap, so its scope is the six items below. Anything else found goes into an issue. First-letter speed (#13) and the watcher's missed writes (#14) are issues already. Each fix gets a test that fails without it, checked by undoing the fix.

1. **A stale files row hid read contents.** In the window check, 6 of about 150 .docx written in bursts kept the size and time of a partial write in `files`, while `extracts` had the full file. Search shows contents only when the two agree, so their words were never found and nothing said so. The worker already stats a file before and after reading it. Now, when it writes what it read, it checks the file once more under the writer lock. If the file is still as read, the `files` row takes that size and time in the same transaction, and the file leaves the queue. If the file changed since, nothing is written and the file stays queued. A file whose contents are current but whose `files` row differs is read again instead of dropped, so that write corrects the row too. The watcher missing the last write is #14 and is not fixed here.
2. **Legacy rows without nanoseconds (#15).** No released build has written contents (the owner's installed app is the phase 1 build), so the rule that an unknown nanosecond value matches the second goes. Migration 3 queues every file read before it for one more read. Until then its contents count as current only while the file's nanoseconds are unknown too, so the first scan after the upgrade hides them and an edit before or after that scan is never missed.
3. **The name shortlist dropped the combined winner.** A file whose name and contents both match could fall outside the 5,000 best names and come back as a contents-only match, below its own name tier and without the both-match bonus. Now every content match is checked against the query's name terms, the same substring test the name search uses, and a match keeps the name tier, its name score and the bonus. The other case, 10,000 exact code names hiding a close-named document, is in the names-only selection and goes to an issue.
4. **The tokenizer can't be re-entered.** `Tokenizer::words` took `&self`, so a callback could call it again on the same tokenizer, and SQLite reused the buffer holding the outer word. `words` now takes `&mut self`, and a compile-fail doc test shows that a nested call does not build. Two tokenizers nested keep their own words, which a runtime test checks.
5. **The query is cut on a character.** The window cut the query at 4,097 UTF-16 units, which can split an emoji's surrogate pair, and the IPC message then fails to decode. The cut now moves one unit on when it would split a pair, so the text sent is still more than 4,096 bytes when the query was longer and the backend still reports the cut. Only the first 4,098 units are copied.
6. **Flashes and the footer.** Typing a new query after an empty one showed "Nothing found" for the new text for 60 ms, before its search ran. Now any input marks the contents as pending until that query's combined search answers. An `index-changed` event searched names first, which replaced content-only rows with the names-only list for 150 to 250 ms. It now runs the combined search directly and keeps the current rows until it answers, as the refresh after files are read does. While contents are pending and no name matches, the footer says "searching inside files" instead of "0 results".

**As built.** Where it differs from the plan above, and why:
- **The queue's update trigger had to change for item 2.** The crawler and the watcher write `files` with an upsert, and SQLite runs an upsert's update with ABORT, which overrides a trigger's `OR REPLACE`. So updating a file still in the queue failed (`UNIQUE constraint failed: extract_queue.file_id`) and the upsert rolled back. With migration 3 queuing every file read before it, each edit to such a file before its re-read would have been lost. Migration 3 recreates `files_au_extract` with `INSERT … ON CONFLICT (file_id) DO UPDATE`, which is not overridden. This is a probable cause of the stale rows in item 1 too, so it is noted on #14, which stays open for the watcher itself.
- **The migration test stops a statement once.** `migration_3_stopped_at_any_point…` returned true from the progress handler from the stop point on, so it also stopped the rollback; with an `INSERT … SELECT` in the migration, that left the transaction open. It now stops exactly one step, and a stop after the commit counts as wholly migrated. Never half migrated is still what it checks, and opened again the file is queued once.
- **Round 2's upgrade test is replaced.** `after_the_upgrade_the_first_scan_queues_nothing…` asserted the rule item 2 removes. Its replacements are the reviewers' two upgrade tests, adapted.
- **The compile-fail doc test runs with `cargo test --doc`**, not in the `--lib` gate. Undoing the fix makes it compile, which fails it.
- **10,000 exact code names hiding a close-named document** is #17: that document matches by name only, so item 3 doesn't reach it.

**Tests.** `extract/find_round3_tests.rs` holds the reviewers' round 3 tests, adapted; `extract/tests.rs` holds item 1's four. Each fix was undone in place and its tests failed: 8 Rust mutations and 4 window mutations, all caught. The window's probes extend round 2's Chromium probes, and the query cut is also checked through Tauri's own IPC serializer and runtime, decoded by `serde_json`. Scripts and logs: `devos-work/reviews/s3fix4/RESULTS.md`, `mutations.py`, `mutations.log`, `ui/`.

**In the window** (ba125fd, the frozen verify skill; report `devos-work/reviews/s3fix4/window/REPORT.txt`, PASS+NOTES). 411 .docx written in five bursts, 140 of them partial writes, were all read and found: in the database every files row matched its contents row and the file on disk, and 140 searches for phrases only one file holds each found that file with its real size (9.2 to 9.3 KB, never 495 B). No files row stayed stale, so the window did not exercise the worker's correction; the tests above do. Typing after "Nothing found" never showed it for the new text (6 trials), a file written during a content-only search never blanked its rows (12 trials, 0 blank reads of about 1,000), the pending footer never said "0 results", and a PDF's word showed "page 2" with the word in bold.

### Step 3 review, round 4: two fixes

Round 4 (`devos-work/reviews/s3r4sol/REPORT.txt`, `s3r4astra/REPORT.txt`) found two blockers at 55bd820, and the owner approved exactly these two fixes. **Refreshes no longer starve.** Each `index-changed` event cancelled the refresh in flight, so with changes arriving faster than a combined search answers, the list kept old rows for as long as they kept coming. Now a change while a refresh runs only marks it for one more search. The answer in flight renders, then the rerun goes out. Typing, Escape and leaving the page still drop both, and the rows stay until an answer replaces them. **The both-match winner holds for any name.** Round 3 checked whether a content match's name matched with Rust's `to_lowercase().contains()`, which disagrees with the name search's trigram and LIKE folding (Greek capitals, ϐ for β). Now the name search's own statement decides it, limited to the content matches' ids. Measured on a synthetic 300,000-file index with 10,000 content ids in a debug build on a loaded machine, that statement took 15 to 39 ms, and 104 ms when every name matched. Tests: two Unicode shortlist cases next to the ASCII one in `extract/find_round3_tests.rs`, and sol's storm schedules plus three coalescing cases in the window probes. All of them fail with the fix undone. Three of round 4's window probes, two of sol's and one of astra's, settled refresh requests that a coalesced page never sends, so adapted versions replace them. Logs: `devos-work/reviews/s3fix5/`.

### Step 3 review, round 5: a lost reply no longer stops refreshes

Round 5 (`devos-work/reviews/s3r5sol/REPORT.txt`, `s3r5astra/REPORT.txt`) found one blocker at 26a893d. Round 4's coalescing waited for the running refresh's reply, so a reply that never reached the page, which sol showed Tauri's own runtime can lose when Enter opens a file during a refresh, left every later change waiting and the list on old rows until the user typed. **A refresh now holds the queue for at most 2 s**, five times `search_all`'s own 400 ms bound (`CONTENT_TIME`, its wait for the connection included). Past it, a change queued meanwhile starts a fresh search under a new generation, so the late reply, if it comes, is dropped; with none queued nothing is retried, and the next change searches at once. Ordinary refreshes still coalesce into one rerun. Tests: both reviewers' lost-reply probes, sol's storm schedules and four new window cases pass, and fail at 26a893d or with the bound removed; a 400 ms bound fails the case where honest 1.9 s replies must still coalesce. Logs: `devos-work/reviews/s3fix6/`.

### Step 3 review, round 7: the search loop

Rounds 4 to 6 patched the page's search and refresh code, and each review found another order of the same events. Every patch assumed the events came in an order the code expected and that a reply arrived. Round 6 (`devos-work/reviews/s3r6sol/REPORT.txt`, `s3r6astra/REPORT.txt`) showed the cost. With query A's rows on screen the user types B, an index change replaces B's names search, and the refresh's reply is lost. A's rows then stayed under B for good. A lost reply for the last change also left old rows for good, and a backend answering one search every 5 s piled up 37 outstanding searches in 120 s.

**One loop now owns searching** (`src/lib/search.js`). It holds WANTED, the query and a version that goes up on every change that could alter the answer (a new query, an index change, files read), and SHOWN, the rows and the rank they answer (version × 2, plus 1 for names with contents). After every event it starts what is due, and it rests once SHOWN answers WANTED. A reply changes the screen only if it is for the current query, for the newest version searched, and ranks above what is shown. The page only feeds it events and renders what it shows.

- **Another query's rows** stay at most 2,060 ms after the box changes (the 60 ms pause plus one 2 s search). Past that the list empties and the footer says "still searching". An error for the new query also takes them down.
- **A search with no reply after 2 s is given up.** A newer version is searched at once while fewer than 3 searches have no reply. The same version is searched again after 4 s, then 8, 16 and 30 s, counted from the last reply of any kind. A late reply still counts if it answers the newest version searched.
- **Bounds.** Against a backend that never answers, 5 searches start in the first minute and at most 2 a minute after, the same with 60 or 600 changes a minute. One reply every 5 s gives 12 a minute with at most 3 unanswered. Every reply taking 5 s gives 30 a minute, again at most 3 unanswered (`devos-work/reviews/s3fix7/p3-counts.log`).
- **Unchanged:** names after 60 ms, names with contents 120 ms after their answer, files read at most every 2 s, one more search for any number of changes during one, rows kept during a refresh, the flags, errors and elapsed time, the bounded query. Typing, Escape and leaving the page clear every timer and ignore every reply. Typing the same text once trimmed searches nothing. A finished scan or a removed folder now refreshes like an index change, keeping the rows, instead of a fresh names search.

The coordinator's read held except in one point. Accepting only replies for the current version would show nothing while changes keep coming, which the storm probes and round 4's coalescing forbid. The rule is therefore "the newest version searched".

Tests: `npm test`, 22 cases without a window, with a fake clock and backend. Undoing each piece that gives P1 to P4, one at a time (8 mutants, `devos-work/reviews/s3fix7/mutate.mjs`), fails its property's test. In the window, the s3fix6 suite passes 57 of 58, plus the Tauri callback probe, three storm schedules, sol's 13 round 6 extras and two round 7 cases. The one failure, "a lost reply with no change queued retries nothing", asserts the round 5 behavior that this round removes. At 7207a6b the extras fail 6 of 13 and the round 7 cases 2 of 2. The probes are unchanged except that the compiled page now resolves its imports from its own folder. Logs: `devos-work/reviews/s3fix7/`. No Rust changed.

## Step 4 in detail: images

Step 4 is split. **Part A** (this section) reads images and screenshots: the words in them, through Vision's text recognition (OCR), and when, where and with what they were taken, from their EXIF. **Part B** reads the scanned pages of PDFs (the `textless_pages` that step 2b records) and comes after the PDF reader is merged. Email, RTF, iWork previews and media tags stay in step 5.

One reader, `image`, macOS only, registered in `readers/macos`. Each image is read by a **helper process that stays running** between images: SIFS's own program, started again in a helper mode, which reads one image at a time on request. SIFS stops it when an image uses too much processor time or memory or runs past the time limit, and the helper exits by itself once no image has come for 30 s. Linux has no image reader and still builds.

### In one process, a helper per image, or one helper for many: what the measurements decided

A prototype (outside the repo, in the orchestration notes for this step: `phase4/p3-ocr/`) read generated images and a sample of the owner's in each shape. It used the same calls as the reader: ImageIO for the facts and a decoded copy at most 4,096 pixels on its longest side, then `VNRecognizeTextRequest` (accurate, language correction, automatic language detection). The Mac is an M5 Pro with 18 cores and macOS 26.4; other programs kept the load average at 8 to 25 during the runs.

| | One process | A process per image |
|---|---|---|
| 200 of the owner's images, three runs each, alternating | 13.1, 15.4, 15.0 s | 77.1, 60.2, 59.3 s |
| per image (one run) | median 36 ms, p95 342 ms | median 260 ms, p95 550 ms |

**A process per image is about four times slower.** Vision loads its recognition models in each new process (the first image in a process takes 200 to 280 ms, later ones a median of 30 ms), and most of the owner's images are small. Over his library that is 2 to 2.7 hours of background work in place of about half an hour. PDFs could afford a helper per file (30 ms each); images cannot.

**One process keeps Vision's memory in SIFS.** After reading images in the prototype's own process, its footprint stayed at 200 to 250 MB once idle (measured for 20 s after the last image), and peaked at 465 MB over 1,000 of the owner's images. SIFS would carry that from its first image until it quits.

**The clock cannot tell a starved read from a stuck one.** With every core busy (`yes` on each), at "utility" priority, a 5K screenshot took 12.9 s by the clock and 1.3 s of processor time; 200 of the owner's images took 215 s in place of 15 s. Only a separate process gives a count of the processor time one image used: Vision does its work on threads of its own.

**Hostile images, for ImageIO and Vision:** 43 generated files, each in its own process with no limit, timed and watched for memory.

| Generated file | What happened |
|---|---|
| PNG of 30,000 × 30,000 (0.9 MB file) | decoded to a 4,096 copy in 3.0 s; 162 MB |
| PNG of 60,000 × 60,000 in colour (10 MB file) | **26.5 s** to decode; 400 MB |
| PNG and JPEG declaring 60,000 × 60,000 and 2³¹ × 2³¹ with no data; 200 KB of random bytes named `png`, `jpg`, `heic`, `tiff`, `gif`, `webp`; an empty file | refused in under 15 ms |
| cut short (JPEG, HEIC, PNG); 20 bytes flipped in each of five JPEG, HEIC, PNG, GIF and TIFF files, five times | read or refused, no crash, no hang, at most 296 MB |
| TIFF of 20,000 × 20,000; a 500-page TIFF; a 200-frame GIF | 0.86 s; 0.12 s (first page); 0.14 s (first frame) |
| 4,096 and 8,192 pixel pages of small text | at most 0.6 s: Vision reads at a resolution of its own, so a larger picture costs it little more |

`VNRequest.cancel` stops a recognition already running within 3 to 32 ms, so Vision itself could be stopped in SIFS's process. ImageIO has no call that stops a decode: only the size checked before decoding (below) bounds it.

**Decision: one helper process, kept for many images.** It costs what one process costs per image, plus about 0.2 s to load Vision's models each time a helper starts. It keeps Vision's 200 to 470 MB out of SIFS and gives it back 30 s after the last image. Processor time and memory are measured for exactly one image, so a starved read is never mistaken for a stuck one, and a crash in one of ImageIO's 60 formats costs one image, not SIFS. This keeps step 2b's departure from decision 8 of step 1 (framework readers in-process first), for the measured reasons above, and does what step 2b left for later ("a helper that serves many files in turn").

### The owner's images

Spotlight lists 100,917 images under `~`. What SIFS would read, after its ignore rules (hidden folders, `~/Library`, `node_modules` and the rest; the Photos library is a package and is not read), and the extensions below: **25,397**, 8.9 GB. None is an iCloud placeholder. 23,743 are PNG, then 736 `jpg`, 388 `jpeg`, 167 GIF, 138 WebP, 86 HEIC, 86 TIFF, and 53 others. The median is 20 KB and 0.25 megapixels; the largest is 57 MB and 57.9 megapixels; 4,590 have a side of 128 pixels or less (icons).

**Facts (counts only):** 153 have the date they were taken, 11 a GPS position, 17 a camera model. His photos are in the Photos library, which SIFS doesn't open.

**OCR** (the prototype, one process, "utility" priority, 1,000 of them chosen at random): 756 have some text (median 23 characters, p95 1,384, most 4,663). Per image: median 31 ms, p95 243 ms, p99 404 ms, most 637 ms; processor time per image: median 23 ms, p99 1.2 s, most 2.8 s. All 1,000 took 62 s, so all 25,397 would take about **26 minutes** at that load. With every core busy, the same 200 images took 14 times as long: about **7.6 hours** for the library, at low priority, without slowing what is in front.

### Languages

Vision on macOS 26.4 recognizes 30 languages: English and the main European languages, Chinese, Cantonese, Japanese, Korean, Russian, Ukrainian, Thai, Vietnamese, Arabic, Turkish, Indonesian and Malay. The newer Swift-only requests (`RecognizeTextRequest`, `RecognizeDocumentsRequest`) list the same 30. **A language outside them gives no text** (no wrong text either): measured with Hindi, which is among the owner's files and which Vision does not read (nor any other Devanagari or Indic script). Such images are found by their facts and names only. **This is a known limit, by the owner's decision (2026-11-08):** OCR uses Apple's Vision only (`VNRecognizeTextRequest`, accurate), with no Tesseract or other installed engine; it is revisited only if step 6's measurement on the real home folder shows a real need. When Apple adds a language, its text appears once the reader's version is raised.

The request uses **automatic language detection** (revision 3) over every language Vision supports, rather than a fixed list (SIFS is used worldwide, so no language is preferred): with English alone, a line of Japanese next to English was dropped; with detection, both were found ("Résumé café naïve" and "東京駅 新幹線"). Recognition is **accurate** with **language correction** on: the fast level missed a shop sign in a photo and halved the time on screenshots only.

### The reader

| | |
|---|---|
| Extensions | `jpg`, `jpeg`, `jpe`, `png`, `gif`, `heic`, `heif`, `hif`, `heics`, `avif`, `webp`, `tiff`, `tif`, `bmp`, `dib`, `jp2`, `jxl`, `psd`, and camera raw: `dng`, `cr2`, `cr3`, `crw`, `nef`, `nrw`, `arw`, `srf`, `sr2`, `raf`, `rw2`, `orf`, `pef`, `srw`, `erf`, `dcr`, `iiq`, `3fr`, `fff`, `mos`, `mrw`, `rwl`. Not icons (`ico`, `icns`), textures (`dds`, `ktx`, `astc`, `pvr`), `exr`, `hdr`, DICOM or `svg`: they hold no photos or screenshots, and `svg`'s words are its XML (a text reader's job, later). |
| Class | images: **after everything else** in the queue (priority 4; documents are 1 and the rest 3). A file changed in the last week still comes first (priority 0), so a new screenshot is read soon. |
| Exclusive | yes: one image at a time, as the plan says for OCR. |
| Which image | the primary one (`CGImageSourceGetPrimaryImageIndex`): a GIF's first frame, a TIFF's first page, a HEIC's main picture. |

**Queue order, measured:** the other readers take about 1 ms a file; an image takes 30 ms to a few seconds. Images before code would hold the code files back by the whole OCR (26 minutes to hours); code before images holds images back by minutes. This follows the readers table ("queued behind everything else") and changes step 1's queue order, which had images before the rest; the test that pins the order changes with it.

### Where text and facts go

The text found is **one part per image**, unlabelled, in `contents` and so in `contents_fts`, as Vision orders the lines (top to bottom), one line each. No migration: `contents` and `facts` take it as they are.

**Facts are stored the same way everywhere:** ISO 8601 times with the camera's own offset, WGS 84 decimal degrees, pixels. Nothing is turned into a local format or a place name here. Naming a place from its coordinates (from a worldwide source) and showing dates and units in the user's locale are left for later phases.

**Facts come first:** the helper reads them from the image's properties (no decoding) and sends them before it decodes the picture. If recognition is then stopped, the image keeps its date and place.

| Key | Value | From |
|---|---|---|
| `taken` | When it was taken, ISO 8601 in the camera's local time: `2025-12-24T18:30:05`, or with the offset when the file has one: `2025-12-24T18:30:05-05:00` | EXIF `DateTimeOriginal` with `OffsetTimeOriginal`, else `DateTimeDigitized` with `OffsetTimeDigitized`. Not TIFF `DateTime` (when it was last edited). A date that isn't one (`0000:00:00 00:00:00`, month 13) gives no fact. |
| `camera_make`, `camera_model` | as written: `Apple`, `iPhone 15 Pro` | TIFF `Make`, `Model` |
| `latitude`, `longitude` | decimal degrees, WGS 84, north and east positive, six decimals (about 0.1 m): `-33.856800`, `151.215300` | GPS `Latitude` and `LatitudeRef`, `Longitude` and `LongitudeRef`. Outside ±90 and ±180, not a finite number, or exactly 0, 0 (what a camera without a fix writes): no fact. |
| `orientation` | EXIF orientation, `1` to `8` (`1` upright, `6` turned 90° clockwise to view) | `Orientation` |
| `width`, `height` | pixels, as stored (before orientation) | `PixelWidth`, `PixelHeight` |
| `stopped` | why the text wasn't read, or was read only in part (the same key as the PDF reader's) | the helper or SIFS |
| `truncated` | `1` when text was cut at the cap | the sink |

### Limits

| Limit | Value | Enforced by | Past it |
|---|---|---|---|
| file size | 256 MB (owner's largest: 57 MB) | the worker | skipped, never opened |
| pixels | 200 megapixels, from the header before decoding (owner's largest: 57.9; a 60,000² PNG took 26.5 s) | the helper | facts kept, no text, `stopped` ("larger than 200 megapixels") |
| small | a side under 8 pixels: no letter fits (Vision fails on a side of 2 or less) | the helper | facts kept, no text, nothing recorded as stopped |
| decoding | a copy at most 4,096 pixels on its longest side (ImageIO's thumbnail). A 5K screenshot gave the same lines at 4,096 and 8,192. | the helper | |
| processor time | 10 s per image, polled every 20 ms (owner's most: 2.8 s) | SIFS kills the helper | facts kept, `stopped` ("used more than 10 s of processor time"); with no facts yet, failed with that reason |
| time | 60 s by the clock, the worker's limit for this reader | SIFS kills the helper at the deadline | the same, "took longer than 60 s" |
| memory | 1 GB physical footprint, polled every 20 ms (largest seen: 465 MB) | SIFS kills the helper | the same, "used more than 1024 MB of memory" |
| a crash | | the helper ends on a signal, or its output ends early | the same, "the image reader crashed" |
| output | 1 MB per image from the helper | SIFS stops reading and kills it | the same, "sent more than 1 MB" |
| characters | 100,000 per image (owner's most: 4,663) | the helper cuts; the sink keeps the cap too | `truncated` |
| idle | 30 s without an image | SIFS kills it (the helper also exits by itself a second later) | the next image starts a new one |
| memory kept | above 512 MB after an image, or at any time between images | SIFS replaces the helper | |

A file ImageIO can't open, or one with no image in it, is failed ("damaged, or not an image macOS can read") and not retried until it changes. A recognition that Vision reports as failed keeps the facts, with `stopped` ("text recognition failed: …").

### The helper

- **Starting it:** the reader runs the program SIFS is running (`std::env::current_exe`) with `SIFS_IMAGE_HELPER` set. `main` (the app, and `sifs-bench`) first calls `extract::serve_helper_if_asked()`, which serves images and exits, never starting the app. In the tests, the test binary is the helper, through an ordinary test that serves when that variable is set and passes at once when it isn't (no test is ignored).
- **One shared helper:** the reader holds it behind a lock; the worker reads one image at a time. Its stdin carries requests (an id, the path, the time it has, the caps); its stdout carries frames back. Anything before a marker is passed over (the test harness prints a line first).
- **Frames for each image, in this order:** facts (any number, each key once, only known keys), then text (at most once), then an end with the outcome. Every frame names the request's id, random for each request (review round 3). Anything else (a frame for another id, text twice, a fact after the text, a frame too long to be one, an unknown tag) is a broken helper: it is killed, and the frames before are kept.
- **One supervisor for both readers.** The PDF reader's supervision (step 2b, with its review fixes) moves to `readers/macos/supervise.rs` and both readers use it: the program to start and its guard (made before any setup that can fail, so a helper is always killed and reaped), non-blocking output retried when interrupted, processor time and footprint from `proc_pid_rusage`, the marker and the frames (a tag, a length, a body; whole frames handed on before a bad one or the output cap), and the loop that checks the clock, processor time and memory on every turn whatever the pipe says, and kills the helper at once when its output ends early. What differs stays in each reader: its frames and the order it allows them in, its caps, and what a stop means. For images, processor time is counted from the start of each image, since the helper lives on. The PDF reader's behaviour and its tests don't change.
- **Image frames are checked against the protocol** before anything reaches the sink (ids, order, each kind's count), so a broken helper can't put text where it doesn't belong. **Facts are sent as they are read**, before the slow part, so a stop never loses them; `truncated` is set only when text was left out.
- **An image interrupted by the idle exit:** if the helper exits by itself (its idle code) before sending anything for the image, a new helper reads it, once.
- **SIFS going away:** the helper's stdin closes, and it exits. It also exits by itself a second after an image's deadline if SIFS hasn't killed it.
- **Priority:** "utility", as the worker. The tests' helper keeps the test's priority (step 2b found "utility" helpers too slow for the suite on a busy Mac).
- **Always reaped:** however a read ends, a helper that is to go is killed and waited for; dropping the reader does the same.

### Who watches the helper (review round 1)

Two reviews of PR #19 found three ways a helper escaped supervision: a request written to a helper that doesn't read its stdin blocked SIFS with no deadline; a helper that sent its end and then spun or allocated was not watched until the next image; and stopping the worker left the helper running while anything else still held the reader. All three came from one assumption: that supervision belongs to a request, starting once the request is written and ending at the end frame. The fix makes it belong to the helper's whole life.

**One owner, from start to reap.** The reader's slot (a lock around the running helper) owns the process. Whoever holds the lock watches it, and nothing touches the helper's pipes without the lock. Dropping it from the slot kills and reaps it.

| State | Who watches | What stops it |
|---|---|---|
| sending a request | the reading thread | stdin is non-blocking: the request is written in pieces as the pipe takes them, with the image's deadline, processor time and memory checked between pieces, as when reading. A helper that won't take its request is killed at the image's deadline. |
| receiving frames | the reading thread | as before: the deadline, processor time, memory, output cap, the protocol, and its output ending. |
| kept between images | a watcher thread, one per helper, which takes the lock every 20 ms | its idle time (30 s), enforced by SIFS; memory over the keep threshold (512 MB); processor time past the image cap (10 s) since its last image ended; and exiting by itself, which is reaped at once. Any of these drops it from the slot. The watcher ends when its helper leaves the slot. |
| shutting down | `Reader::stop`, which the worker calls once its reads in flight are done, and as its thread ends however it ends; and dropping the reader | the helper is killed and reaped then, whoever else holds the reader. The reader stays stopped: a later read is refused and starts nothing (review round 3). |

**The helper's own deadline** belongs to the thread that serves requests: it sets the deadline as it takes a request and clears it when the request ends. A separate watchdog thread waits for that deadline and exits the helper a second past it. Before, the thread reading stdin set it as each request arrived, so the end of one request could clear the next one's.

The helper still exits by itself when its stdin closes (SIFS gone), a second past an image's deadline, and after its idle time plus a second, should SIFS's watcher not have ended it first.

**Facts, from the same reviews.** A latitude's reference must be N or S and a longitude's E or W; any other reference gives no position. ImageIO itself reads an unknown reference (`X`) as N or E, so only a wrong-axis one (a latitude's `W`) reaches the reader to be refused; the raw value is in the image's XMP metadata (issue #21). A position whose `GPSMapDatum` names a datum other than WGS 84 gives no position (no conversion yet); one with no datum is taken as WGS 84. Text counts as received only once the sink takes it. The longest frame the helper may send allows a fact at its own cap (1,000 characters) whatever the text cap.

### Stopping for good, and replies sent early (review round 3)

**Stopped is a state, not an event.** Rounds 1 and 2 reaped the helper running when the worker stopped, but a read the worker had already handed to a thread could start a new one after: the worker's thread panicked with a read dispatched, the guard reaped the helper, and the read then started another that nothing would end. Now the reader's slot holds, under the lock `stop` takes, whether the reader was stopped, and the one place that starts a helper checks it there. A read after the stop is refused ("the image reader was stopped"); a read already holding the lock finishes first and `stop` reaps its helper after. There is no window between the check and the start.

**A reply can't be told from a pipe's boundaries.** Three rounds counted the helper's output at the point a pipe read ended, assuming the reader could tell from what it had read that the helper had written nothing more. A pipe doesn't say that: bytes the helper wrote after an image's end could sit unread and be taken as the next image's reply, and since ids were counted, a helper could aim a whole reply at the next id. Now:

- **Each request's id is random** (`arc4random_buf`), so nothing the helper wrote before a request can carry its id, whatever the pipe splits. The wire format is unchanged (the id was already in every frame), so the protocol stays version 1.
- **The end is the end.** An image's frames up to its end are its own. Anything with or after the end in the same read, within the output cap or past it, means the helper is broken: the image keeps what it read, and the helper reads no more images. It used to be a broken read (or the output cap), which made the image's result depend on where the pipe split the bytes.
- **Something that isn't for the image, before anything that is** (a frame with another id, or bytes that aren't a frame) is taken as what an earlier image left in the pipe: that helper goes, and a new one reads the image, once, as after the idle exit. After any frame of the image's own, it is a broken helper, as before.

So for every split of a helper's writes, nothing written before a request is accepted as its reply, unless it carries a 64-bit id drawn after it was written (one chance in 2^64), and the outcome for both images is the same. The tests split one such stream at every byte, frames and bytes that aren't one.

### Dependencies

macOS only, pinned, default features off, beside step 2b's: `objc2-image-io` **0.3.2** (`CGImageSource`, `CGImageProperties`), `objc2-vision` **0.3.2** (`VNRecognizeTextRequest`, `VNRequestHandler`, `VNObservation`, `VNRequest`), `objc2-core-graphics` **0.3.2** and `objc2-core-foundation` **0.3.2** (already in the tree through Tauri). The tests draw their images with Core Text and write them with `CGImageDestination`: `objc2-core-text` **0.3.2** as a macOS dev-dependency.

### Tests

Images are drawn in the tests (Core Text into a bitmap, written as PNG, JPEG, HEIC, TIFF and GIF with `CGImageDestination`, with EXIF and GPS properties where needed). No binary fixtures are committed. Each limit's and failure path's test is checked to fail with that limit or path removed (the mutation script and its results are in the orchestration notes, `phase4/p3-ocr/`).
- **Reading:** a screenshot's words are found by `match_contents`; a photo's sign; JPEG, HEIC, TIFF, a GIF's first frame; "résumé"; Japanese next to English; Arabic and Chinese next to English; an image in a script Vision doesn't read (Devanagari) gives no text and keeps its facts.
- **Facts:** `taken` with and without an offset, from `DateTimeDigitized` when there's no original, none from a bad date; latitude and longitude with south and west negative, none at 0, 0 or out of range; camera, orientation, size. The date and coordinate parsing are pure functions with tests of their own.
- **Limits:** a picture over the pixel cap keeps its facts and has `stopped`; a tiny one is read without OCR; a test helper that spins is killed at a lowered processor time cap, one that allocates at a lowered memory cap, one that sleeps at a short clock deadline, one that floods at the output cap; facts sent before each stop are kept; the character cap.
- **The helper's protocol:** a test helper that closes its output and then spins or allocates is killed at once; frames for another id, text twice, a fact after text, an unknown tag; whole frames before a bad frame or the cap are kept; the guard reaps a helper whose setup fails; a crash after the facts, and before them; the idle exit retried once and only once; the helper ends when its stdin closes and a second after its deadline; the same helper serves several images (its process id is unchanged) and is replaced after a stop; none is left running or unreaped.
- **Through the worker:** images are read by `run_until_empty` and found by their words; a damaged image is failed; one over 256 MB is skipped; images are claimed after code; the platform registry reads `png` and `heic` on macOS.

### Measuring

A scratch program (in the orchestration notes) calls the reader as the worker does over the owner's images, and prints counts and times only (no text, paths or places).

**Measured on the owner's Mac (2026-11-08),** the reader as built, over all 25,397 images, one after another, at a load average of 5 to 8 from other programs:

| | Images |
|---|---|
| text found | 18,831 (median 26 characters, p95 1,275, most 11,312; 4.0 million in all) |
| no text | 6,540 |
| damaged, or not an image macOS can read (failed) | 26 |
| stopped by a limit, or crashed | 0 |
| with the date taken / a GPS position / a camera model | 188 / 11 / 17 |

Per image: median **32 ms**, p95 **231 ms**, p99 462 ms, most 4.9 s; **28 minutes** for all of them. The helper's peak footprint was **688 MB**; SIFS's side (the scratch program) peaked at 6 MB. With every core busy (`yes` on each), 200 of them took 209 s in place of 13.5 s, a median of 756 ms and at most 5.3 s each, none stopped: about **7.4 hours** for the library at that load, at low priority.
