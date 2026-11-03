# Handover (2026-11-03): SIFS

The owner is continuing locally. Read this first. Delete `handover/` in the last commit before this branch merges.

## Where things are

- **main** (5841c20): phase 3 step 1 is merged as PR #3. It covers:
  - reading inside text, code, plain, JSON and HTML files;
  - the full-text index;
  - the background worker;
  - the folder lifecycle (`index/folders.rs`). Folders never nest; removal is batched.

  There are 108 lib tests. Step 1 had four review rounds, whose reports are `handover/step1-review-r2..r4.txt` (round 1's is in round 2's).
- **This branch, `claude/upbeat-clarke-2y401g`:** phase 3 step 2, the Office half. Word, Excel and PowerPoint are read through their XML in `src-tauri/src/extract/readers/office/`. The design is "Step 2 in detail" in `plans/phase3-reading-inside-files.md`.
  - 9923e8b..56005b3: the readers, with 132 lib tests passing.
    - 931d868 widens a flaky backoff test. It was mutation-checked: with stop ignored during the backoff, the test fails with 1.7 s.
    - 56005b3 sets `rust-version` to 1.90.
  - **The top commit is work in progress.** The builder was stopped halfway through fixing the step 2 review. The commit compiles and the 132 tests pass, but the fixes are incomplete and have no new tests yet.
- **There's no PR for step 2 yet.**

## Next: finish the step 2 review fixes

The review is in `handover/step2-review/REPORT.txt`. Its tests are in `handover/step2-review/review.rs.txt`; to use them, copy the file to `src-tauri/src/extract/readers/office/review.rs` and add `#[cfg(test)] mod review;` in `office/mod.rs`. The verdict is NOT RIGHT.

1. **BLOCKING: Excel row memory.** One big shared string referenced by many cells of a row is cloned into `self.row` for each cell, so a 650 KB file uses up memory and the app aborts; `catch_unwind` can't catch running out of memory. The fixes:
   - write each cell to the sink as it closes, or flush once the row passes the sink's remaining room or `LONG_LINE`;
   - borrow shared strings instead of cloning them;
   - cap the number of `<si>` entries.
2. **BLOCKING: time.** The slide and sheet lists are matched to relationships by a linear search with no time check: 60k × 60k is 43 s against the 10 s limit, and four such files stall the worker. The fixes:
   - a HashMap from relationship id to target;
   - `sink.check()?` for each slide and sheet, or an event counter that spans parts;
   - cap slides and sheets at about 10k, and note it when the cap is hit.

   The many-entries-to-one-empty-sheet case must be covered too.
3. **Minor:**
   - Word table cells are held whole, then split and re-joined, using about 4× the cap in memory.
   - Page breaks inside a table are dropped. Keep a pending break and apply it at the row's end.
4. **Minor: tests that fail without their rule.** Each of these needs one:
   - the cfb `MAX_DIRECTORY_SECTORS` bound with a self-referencing FAT chain (the most important);
   - `LONG_LINE`;
   - moveFrom;
   - PowerPoint Fallback;
   - inline-string rPh;
   - the cell value cap;
   - the Props fact cap;
   - TargetMode External.
5. **Minor: text and display.**
   - End the line before `w:txbxContent`.
   - Map `w:noBreakHyphen` to "-", drop `w:softHyphen`, and map `w:sym` where it's a plain Unicode character.
   - Count every typed `w:br type="page"`; drop only a rendered break that directly follows another break.
   - Show Excel serial 0 in a date format as the plain number.
6. **Hidden sheets (decided):** skip `state="veryHidden"` sheets, and label `state="hidden"` sheets "Name (hidden)". Note this in the plan.

Each fix needs a test that fails without it. Run `cd src-tauri && cargo test --no-default-features --lib` three times, plus `cargo clippy --no-default-features --lib --tests` and `cargo clippy`. Then run a short re-review on the fixes, open the PR, and merge it.

## After that

- **Phase 3:**
  - Step 3: search over contents (snippets, page or sheet labels, ranking with names, UI) can be done on Linux.
  - Steps 4–5 and PDF are for a Mac session: PDFKit, Vision OCR, EXIF, RTF and iWork.
  - Step 6 measures on the real home folder.
- **Phase 4 (SIFS side):**
  - S1: migration 2 (uid columns, outbox, triggers, hashing).
  - S2: the engined client.
  - S3: supervisor and packaging.
  - S4: the feeder.

  The plan is in the engine repo, `plans/phase4-engined.md`, on branch `claude/upbeat-clarke-2y401g` there.
- **Mac-only checks:**
  - skipping iCloud placeholders, which must happen before step 1 ships on the Mac;
  - utility QoS;
  - case-insensitive paths;
  - real Office samples;
  - the app end to end.
- **The frontend:** `npm run check` has 24 errors in `src/routes/+page.svelte`, the same as on main.
