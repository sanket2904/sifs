---
name: verify
description: Drive the real SIFS macOS window (Tauri 2 + Svelte 5 menu-bar file search) the way a user does, in an isolated test instance that runs in the background and never takes the owner's focus or touches the owner's SIFS. Use to prove any SIFS change on the real surface (search, folders, skipped patterns, reading inside files, a PR's head), or to reproduce a SIFS bug before fixing it.
---

# Verify SIFS in its real window

This skill starts an isolated SIFS from a fake home folder with seeded files, drives its real window through the macOS accessibility API, and keeps screenshots and read-backs as evidence. Everything goes through `sifs-verify` in this folder. Run it with no arguments for its usage.

```bash
V=.pi/skills/verify/sifs-verify
```

## Non-negotiables

The owner uses this Mac while you verify, and runs his own SIFS. These rules protect his work. Breaking one is a failed run, whatever the result.

1. **No synthesized mouse events and no coordinate clicks.** No CGEvent mouse posts, no "click at", no clicking menu bar items by position. They move the owner's cursor and click in his menu bar, where his own SIFS icon sits.
2. **Never activate the test app or bring it to the front.** No `activate`, no setting `frontmost`, no System Events keystrokes (they go to whichever app is in front).
3. **Everything goes through the accessibility API, addressed by the test instance's pid.** AXPress on its own buttons, AXFocused on its own elements, key events posted to its pid with `CGEventPostToPid`, screenshots with `screencapture -l <window id>`. `sifs-ax` (compiled from `ax.swift`) is built to do only these.
4. Kill only pids this skill started. `sifs-verify cleanup` checks that a pid still runs this run's binary with this run's HOME before it kills it. Never kill by name.
5. Never address the owner's SIFS (`SIFS.app`, often built at `~/Desktop/code/sifs/src-tauri/target/release/bundle/macos/SIFS.app`). `launch` and `doctor` list other SIFS processes and leave them alone.
6. Never press Open in the Add a folder… dialog. The dialog starts in the owner's real home whatever HOME says, so Open can add his real Documents to a test instance. One development run did exactly that. `doctor` now fails when any searched folder is outside the fake home.

## Prerequisites

- macOS with the Xcode command-line tools (`swiftc`, `clang`), Rust (the repo's `rust-toolchain.toml`), Node.js, `jq`, `/usr/bin/sqlite3`.
- The terminal running the agent needs Accessibility (System Settings > Privacy & Security > Accessibility) and Screen Recording. `sifs-ax` stops with a clear message without Accessibility. Without Screen Recording, screenshots come out blank or show only the wallpaper.
- Scratch lives in `/private/tmp/sifs-verify` (override with `SIFS_VERIFY_DIR`). Evidence lives in `<repo>/verify-evidence/<run>/` (gitignored, override with `SIFS_VERIFY_EVIDENCE`).

## Launch

Build once per commit, then launch:

```bash
$V build                                # this checkout; id is <sha7>, or <sha7>-dirty with local changes
$V build --ref origin/phase3-step3      # a branch or PR head, from a copy; your checkout is not touched
$V launch                               # newest build; or: $V launch --build d470ac5 --name pr5
```

`build` runs `npx tauri build --debug --no-bundle`, so the page is inside the binary and no Vite server runs. That avoids port 5173, which `npm run tauri dev` takes and which two dev instances would fight over. Do not use `tauri dev` for verification. A `--ref` build is a `git archive` copy under `$SIFS_VERIFY_DIR/src/<sha>`. Commits from before this skill lack the background hook (below), so `build` adds it from `verify-support.patch` and records `BUILD_PATCHED=yes`.

`launch` does this, and fails loudly at the first step that doesn't happen:

1. Seeds `runs/<name>/home` with `seed-home.sh`: Documents (a .docx, an .xlsx with two sheets, .txt, .md, a folder, an unreadable file, a non-zip .docx, a binary .txt, a 1 MB .json), Desktop/sifs-demo-code (code, a `node_modules`, a hidden file, a `generated` folder), Downloads and Projects. Unique words for content search: `zanzibar`, `pelican`, `marmalade`.
2. Starts the binary with `HOME` and `CFFIXED_USER_HOME` set to that folder. HOME moves SIFS's index (Tauri's app data dir) and the folders it offers. CFFIXED_USER_HOME moves WebKit's data and caches, which otherwise land in the real `~/Library/WebKit/sifs` and `~/Library/Caches/sifs`. Both were observed on this Mac.
3. Sets `SIFS_VERIFY_BACKGROUND=1`. `app.rs` reads it: the app doesn't activate at launch, stays out of the Dock, and opens its window behind other apps' windows. Accessibility and `screencapture -l` reach it there and on other Spaces.
4. Loads `open-spy.dylib` (built from `open-spy.m`) with `DYLD_INSERT_LIBRARIES`. Opening a result (Enter) and showing it in Finder (⌘Enter) are recorded in `open-requests.log` in the evidence folder and report success without opening anything. Performing them would bring Finder or another app forward and could open the owner's apps.
5. In the window, presses `+ ~/Documents` and `+ ~/Desktop`, the suggestions a new user sees. It waits until both show an item count and `Reading inside files:` shows `0 waiting`. It closes settings, then searches `quarterly-budget`. **Ready** means that search returned `quarterly-budget.xlsx` in the real window. Last, it runs `doctor`.

A launch takes under a minute after the build. The settled index holds 23 items. Reading inside files ends at 12 read, 1 couldn't be read (`locked-notes.txt`, permission denied) and 3 skipped (`broken-report.docx`: not a Word document; `firmware-dump.txt`: not text; `huge-export.json`: larger than 1 MB). These counts are from main at 6a45930.

## Doctor

```bash
$V doctor            # current run; or: $V doctor pr5
```

Read-only. Exits non-zero on any FAIL. It checks that:

- the pid is alive and runs this run's binary with this run's HOME;
- the binary still matches the build (sha256), and it shows the commit, the dirty flag and whether the hook was added;
- the binary is not an installed SIFS.app;
- the index it has open is in the fake home, and nothing is open in the owner's data folder (`~/Library/Application Support/com.sanket2904.sifs`);
- it started with `SIFS_VERIFY_BACKGROUND=1`, the open spy loaded, and it doesn't have the keyboard;
- every folder in its index (read from its own sifs.db, read-only) is inside the fake home;
- the window server knows its window.

It lists every other SIFS process as "not ours, never touched". Run doctor first whenever anything looks off.

## Drive

Drive commands act on the current run. Set `SIFS_VERIFY_RUN=<name>` to drive another. Each one appends the action and the resulting read-back to `transcript.md` in the evidence folder.

```bash
$V search "budget"            # focus the box, clear it with Delete, type, wait until results settle, print JSON
$V search "pelican" 1.5       # settle longer: PR #5's content pass lands about 120 ms after the names
$V read                       # what the window shows now
$V key down                   # also: up, return, escape, tab, delete; --cmd, --shift
$V key return                 # opens the selected row (captured; see $V opened)
$V key return --cmd           # show in Finder (captured)
$V opened                     # the open / reveal requests, one per line
$V press "⚙︎"                  # settings; buttons by visible title
$V press "+ ~/Downloads"
$V press Remove --after "~/Downloads"     # the Remove button on that folder's row
$V focus-textarea; $V key down --cmd; $V type $'\n**/generated'; $V press "Save and re-index"
$V wait '^~/Downloads [0-9]+ items' 30    # wait until a page line, a row or the footer matches
$V tree                       # raw accessibility tree, to find handles for new UI
```

What `read` and `search` print:

- `query` and `searchBoxFocused` come from the search box (placeholder `Search your files`).
- `rows` lists each result in order. A row has `index` (DOM id `hit-N`), `selected` (aria-selected), and `text`, the row exactly as shown. It also has `name`, `closeMatch`, `folder` (`~`-relative), `meta` (size · date) and `found` (PR #5's label · snippet). These fields are split from `text` by the page's fixed layout, because WebKit exposes a `role="option"` row as one text with no children. They are null when the split fails (a non-English date format, say). Assert on `text` when in doubt.
- `footer` is the footer line, for example `3 results in 2 ms · ↑↓ to move · Enter to open · ⌘Enter to show in Finder`.
- `lines` lists every line of the page in order: settings headings, folder rows (`~/Documents 13 items [Remove]`), skipped patterns, the counts line, the failures list, errors and `Nothing found for “…”.` Buttons appear in `[brackets]`.

## Evidence

```bash
$V shot budget-results        # NN-budget-results.png (the window only) and NN-budget-results.json (its read-back)
```

Proof standards:

- Drive the user's path: type in the box, press the page's buttons, use the keys. Never call Tauri commands, edit the index or use test-only endpoints.
- Capture the action and the resulting state. `search` and `read` log both. Take a `shot` at each state a reviewer should see.
- Check side effects next to the screen. Search after Save and re-index or Remove. Use `$V opened` for open and reveal. Use `doctor` for where the index lives.
- The open spy is the one stand-in. It replaces the hand-off to `/usr/bin/open` and NSWorkspace, which leave SIFS anyway. Everything up to that call is real. That `open` would then open the file is not verified.

Evidence stays in `<repo>/verify-evidence/<run>/`, or `$SIFS_VERIFY_EVIDENCE/<run>/`. It holds `run.env` (build, pid, paths), `transcript.md`, the shots and `open-requests.log`. Cleanup never deletes it.

## Cleanup

```bash
$V cleanup                    # the current run; or --run NAME, or --all
$V cleanup --all --builds     # also the builds and source copies
```

Cleanup stops the run's pid only after it confirms the binary and HOME, waits up to 5 seconds, then sends SIGKILL. Then it deletes the run folder: the fake home, the index and the copied binary. It prints the evidence path and file count. Run it after every failed attempt too, so nothing is left running. The tool you run commands through may kill a launch it times out, with its whole process group, so give `launch` 3 minutes or run it in the background.

## Helpers

| File | What it is |
| --- | --- |
| `sifs-verify` | The entry point: build, launch, doctor, drive, shot, cleanup. |
| `ax.swift` | Source of `sifs-ax`, compiled on first use to `$SIFS_VERIFY_DIR/bin/sifs-ax`. It reads the window and drives it by pid. `sifs-verify` passes the run's pid. Direct use: `sifs-ax read <pid>`. |
| `seed-home.sh` | `seed-home.sh <new dir>` writes the fake home. Office files are zipped from inline XML, so no fixture files are needed. |
| `open-spy.m` | Source of the dylib that records open and reveal requests instead of performing them. |
| `verify-support.patch` | The `SIFS_VERIFY_BACKGROUND` hook, for building commits from before it. |

## Known limitations

- **A window closed to the menu bar can't be shown again.** The tray icon opens its menu only on a real mouse click (AXPress on the status item does nothing), and rule 1 forbids clicks. Nothing in this skill closes the window. If it is closed, run `cleanup` and `launch` again.
- **The Add a folder… dialog is only opened and cancelled.** It does open in the background, as a sheet in the test window's accessibility tree (`$V dialog-tree`, `$V cancel-dialog`). Picking a folder in it is not supported (rule 6). Adding folders is proven through the suggestion buttons.
- **A test instance is not invisible.** SIFS_VERIFY_BACKGROUND stops activation and the Dock icon: across launches on this Mac the app in front never changed. Whether the window shows for a moment as it opens behind other windows was not observed. Its tray icon sits in the menu bar next to the owner's until cleanup. Tell the owner a test instance is running.
- Dates in `meta` assume an English (US) system locale, as on this Mac.

## Feature map

`features/README.md` indexes one recipe per user-facing feature. Read it before driving, and use the matching file. A proof that drives one convenient entry point is incomplete when the map lists others. When the app changes, update the map in the same PR. `/maintain-verification-skill` audits it.
