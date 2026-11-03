# Phase 3: reading inside files

Part of the SIFS plan (`ontology-framework/plans/sifs-intelligence-backend.md`, phase 3).

**Goal:** search by the words inside files, not only their names, across every kind of file. Everything is local; nothing leaves the Mac.

**Done when:** a word search finds files of every kind below by their contents, with the page or sheet it's on. Extraction is measured per 1,000 files (time, index size, memory).

## How it fits the index

The name index stays as it is. Three additions to the same SQLite file:

| Table | What it holds |
|---|---|
| `extracts` | One row per file read: `file_id`, `extractor`, the file's `modified` and `size` when read, `status` (done, empty, failed, skipped), the error, and the time taken. A file is read again only if its `modified` or `size` changed, or the extractor's version went up. |
| `contents_fts` | FTS5 over the text, one row per page, sheet, slide or email part: `file_id`, `part` (page 3, sheet "Q2", the subject), `text`. The tokenizer is `unicode61 remove_diacritics 2`, so "resume" finds "résumé". |
| `facts` | The small values read alongside the text, as (`file_id`, `key`, `value`): EXIF date, camera, GPS latitude and longitude, where the file was downloaded from, author, title, page count, duration, artist, and so on. Phase 4 hands these to the engine. |

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
