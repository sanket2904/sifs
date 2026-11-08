---
name: verify
description: Drive the real SIFS macOS window (Tauri 2 + Svelte 5 menu-bar file search) the way a user does, in an isolated test instance that runs in the background and never takes the owner's focus or touches the owner's SIFS. Use to prove any SIFS change on the real surface (search, folders, skipped patterns, reading inside files, a PR's head), or to reproduce a SIFS bug before fixing it.
---

# Verify SIFS in its real window

This skill starts an isolated SIFS from a fake home folder with seeded files, drives its real window through the macOS accessibility API, and keeps screenshots and read-backs as evidence. Everything goes through `sifs-verify` in this folder. Run it with no arguments for its usage.

```bash
V=.pi/skills/verify/sifs-verify
```

## Threat model

The guarantees below hold against a careless or confused agent, stale state left by earlier runs, pid reuse, and odd paths (symlinks, `..`, folders that already exist). They do not hold against a hostile local user who races the tool on purpose, for example by swapping a folder for a symlink between a check and the write that follows it. Where it can, the tool avoids reusing state rather than checking it: a build starts from a new source copy, a file is replaced rather than written in place, and a run is started once.

## Non-negotiables

The owner uses this Mac while you verify, and runs his own SIFS. These rules protect his work. Breaking one is a failed run, whatever the result.

1. **No synthesized mouse events and no coordinate clicks.** No CGEvent mouse posts, no "click at", no clicking menu bar items by position. They move the owner's cursor and click in his menu bar, where his own SIFS icon sits.
2. **Never activate the test app or bring it to the front.** No `activate`, no setting `frontmost`, no System Events keystrokes (they go to whichever app is in front).
3. **Everything goes through the accessibility API, addressed by the run.** AXPress on its own buttons, AXFocused on its own elements, key events posted to its pid with `CGEventPostToPid`, screenshots with `screencapture -l <window id>`. `sifs-ax` (compiled from `ax.swift`) does only these. It takes a run folder that the workspace manifest lists (below), not a pid, and refuses unless that run's pid is still the process the run started: the same start time, the run's copy of the binary, the run's fake HOME. It checks again before every action and event it sends.
4. Kill only processes this skill started. The app runs as the child of its run's supervisor (`sifs-ax supervise`), which reaps it only when asked to stop, so until then its pid can't pass to another process. `cleanup` asks the supervisor, and the supervisor signals its own child. Nothing else in this skill sends a signal. A run whose supervisor doesn't answer (within 3 s for status, 10 s for stop) gets no signal at all: cleanup reports it and keeps the run. Never kill by name.
5. Never address the owner's SIFS (`SIFS.app`, often built at `~/Desktop/code/sifs/src-tauri/target/release/bundle/macos/SIFS.app`). `sifs-ax` refuses it. `launch` and `doctor` list other SIFS processes and leave them alone.
6. Never press Open in the Add a folder… dialog. The dialog starts in the owner's real home whatever HOME says, so Open can add his real Documents to a test instance. One development run did exactly that. Three guards now stand behind this rule. While a dialog is open, or when `sifs-ax` can't rule one out (an accessibility read that fails or returns something unexpected), it refuses every input except Escape and the dialog's Cancel. A verification instance refuses to add any folder outside its fake home. `doctor` fails when any searched folder is outside the fake home.

## Prerequisites

- macOS with the Xcode command-line tools (`swiftc`, `clang`), Rust (the repo's `rust-toolchain.toml`), Node.js, `jq`, `/usr/bin/sqlite3`.
- The terminal running the agent needs Accessibility (System Settings > Privacy & Security > Accessibility) and Screen Recording. `sifs-ax` stops with a clear message without Accessibility. Without Screen Recording, screenshots come out blank or show only the wallpaper.
- The workspace (builds, runs, source copies, compiled helpers) is `/private/tmp/sifs-verify-<uid>`. `sifs-verify` creates it with mode 700 and a manifest, `.sifs-verify-manifest`, its one state file. The manifest lists by device and inode the workspace, each run, build and source folder `sifs-verify` creates in it, and each run's evidence folder and every file the run writes there. Before every read and write `sifs-verify` checks that the manifest is your regular file, not a symlink, and that its first line names the workspace. It refuses any path it uses that has a symlink on the way or leaves the workspace. Cleanup deletes only listed folders that are still the folder it created and still yours. Anything else in the workspace is left alone and named.
- `sifs-verify` never writes into a file that already exists. Every `>` creates a new file (`set -C`). A file that changes (the manifest, `current`, a transcript, a compiled helper) is written whole to a new file with a unique `mktemp` name beside it, which is then renamed over it, so a symlink or hard link at that path is replaced, not followed. A manifest change holds a lock on the workspace folder (`lockf`), so commands running at the same time don't lose each other's lines. `SIFS_VERIFY_DIR` overrides the workspace with an absolute path that doesn't exist yet or that `sifs-verify` created. Evidence lives in `<repo>/verify-evidence/<run>/` (gitignored). `SIFS_VERIFY_EVIDENCE` overrides it with an absolute path outside the workspace.
- Run names, build ids and shot labels are 1 to 64 letters, digits or `-`.

## Launch

Build once per commit, then launch:

```bash
$V build                                # this checkout; id is <sha7>, or <sha7>-dirty with local changes
$V build --ref origin/phase3-step3      # a branch or PR head, from a copy; your checkout is not touched
$V launch                               # newest build; or: $V launch --build d470ac5 --name pr5
```

`build` runs `npx tauri build --debug --no-bundle`, so the page is inside the binary and no Vite server runs. That avoids port 5173, which `npm run tauri dev` takes and which two dev instances would fight over. Do not use `tauri dev` for verification. A `--ref` build is a `git archive` copy in a new folder, `<workspace>/src/<sha7>-<random>`, that no earlier command used. It gets its own `node_modules` from `npm ci` and its own Cargo target: `CARGO_TARGET_DIR` is set to the copy's `src-tauri/target`, whatever the environment says. The copy is deleted when the build command exits; `cleanup --builds` deletes any copy a killed build left, and no build reuses one. Cargo, npm and Vite write where they like inside the tree they build, so nothing an earlier command left is built in. That costs a full build each time: 30 s for ce7eacf on this Mac (an 18-core M5 Pro) against 4 s with a warm target, each measured once. An existing build is reused only while it is still the folder the manifest lists and its binary still has the sha256 recorded with it; otherwise it is built again. `--src` builds in that folder (default: this checkout) with its own `node_modules` and target, as `npm run tauri build` would, and refuses a folder inside the workspace. Commits from before this skill lack the verification support (below), so `build` adds it from `verify-support.patch` and records `PATCHED=yes`. The patch applies to main up to ce7eacf (checked) and other commits whose `app.rs` has no verification hook. PR #9's drafts 4d8632c to 2663a77 carry an earlier hook without `verification.rs`; `build` refuses them with that reason. a9960fa and later carry the support. It refuses to build source that doesn't wire the support into `app.rs`.

`launch` does this, and fails loudly at the first step that doesn't happen:

1. Seeds `runs/<name>/home` with `seed-home.sh`: Documents (a .docx, an .xlsx with two sheets, .txt, .md, a folder, an unreadable file, a non-zip .docx, a binary .txt, a 1 MB .json), Desktop/sifs-demo-code (code, a `node_modules`, a hidden file, a `generated` folder), Downloads and Projects. Unique words for content search: `zanzibar`, `pelican`, `marmalade`.
2. Starts the run's supervisor, `sifs-ax supervise`, in its own session. It refuses unless the manifest lists the run, nothing started the run before (no `run.env`, `app.log` or `control.sock`), the binary is your plain file and the home is your plain folder with no symlink anywhere inside. The supervisor starts the binary as its child with `HOME` and `CFFIXED_USER_HOME` set to that folder. HOME moves SIFS's index (Tauri's app data dir) and the folders it offers. CFFIXED_USER_HOME moves WebKit's data and caches, which otherwise land in the real `~/Library/WebKit/sifs` and `~/Library/Caches/sifs`. Both were observed on this Mac.
3. Sets `SIFS_VERIFY_BACKGROUND=1`, read by `src-tauri/src/verification.rs`. The app doesn't activate at launch, stays out of the Dock, and opens its window behind other apps' windows. Accessibility and `screencapture -l` reach it there and on other Spaces. It also refuses to add a folder outside its HOME.
4. Records the process in `run.env`: its pid and its start time, as the supervisor reports them. The binary and HOME it must have follow from the run's folder. The supervisor answers on `runs/<name>/control.sock`. Once the run's path no longer names the folder and socket it started with (deleted, moved, or replaced by another folder), the supervisor stops the app and exits. It checks once a second while no client is connected and again after each client, and it drops a client that sends nothing for 2 s.
5. Loads `open-spy.dylib` (built from `open-spy.m`) into the app alone: the supervisor sets `DYLD_INSERT_LIBRARIES` for its child only. Opening a result (Enter) and showing it in Finder (⌘Enter) are recorded in `open-requests.log` in the evidence folder and report success without opening anything. Launch creates that log exclusively and passes the open file to the app through the supervisor, as descriptor 7 (`SIFS_VERIFY_OPEN_LOG_FD`). The spy writes only through that descriptor and never opens a path, so a file put at the log's path later is never written. Performing them would bring Finder or another app forward and could open the owner's apps.
6. Waits for the first-run screen and saves it (`01-first-run.png` and its read-back) before any input. Then, in the window, presses `+ ~/Documents` and `+ ~/Desktop`, the suggestions a new user sees. It waits until both show an item count and `Reading inside files:` shows `0 waiting`. It closes settings, then searches `quarterly-budget`. **Ready** means that search returned `quarterly-budget.xlsx` in the real window. Last, it runs `doctor`.

A launch takes about 10 seconds after the build (7 s measured on this Mac). The settled index holds 23 items. Reading inside files ends at 12 read, 1 couldn't be read (`locked-notes.txt`, permission denied) and 3 skipped (`broken-report.docx`: not a Word document; `firmware-dump.txt`: not text; `huge-export.json`: larger than 1 MB). These counts are from main at 6a45930.

## Doctor

```bash
$V doctor            # current run; or: $V doctor pr5
```

Read-only. Exits non-zero on any FAIL. It checks that:

- the pid is alive and is the process this run started (start time, binary, HOME);
- the run's supervisor answers and holds that pid as its child;
- the binary still matches the build (sha256), and it shows the commit, the dirty flag and whether the hook was added;
- the binary is not an installed SIFS.app;
- the index it has open is in the fake home, and nothing is open in the owner's data folder (`~/Library/Application Support/com.sanket2904.sifs`);
- it started with `SIFS_VERIFY_BACKGROUND=1`, the open spy loaded, and it doesn't have the keyboard;
- every folder in its index (read from its own sifs.db, read-only), resolved, is the fake home or inside it. The home is found among the folder's ancestors by device and inode, so every spelling of a folder (another case, `/System/Volumes/Data/...`) gets the same answer. The app's own guard checks the same way. An index it can't read fails the check;
- the window server knows its window.

It lists every other SIFS process as "not ours, never touched". Run doctor first whenever anything looks off.

## Drive

Drive commands act on the current run. Set `SIFS_VERIFY_RUN=<name>` to drive another. Each one, `opened` and `shot` included, appends the action and its output to `transcript.md` in the evidence folder. A command that sends input (press, key, type, focus, cancel-dialog) also appends the read-back of the screen it leaves.

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

Evidence stays in `<repo>/verify-evidence/<run>/`, or `$SIFS_VERIFY_EVIDENCE/<run>/`. It holds `run.env` (build, commit, pid and start time), `transcript.md`, the shots and `open-requests.log`. Each run gets a new evidence folder: `launch` refuses a name whose folder exists. The run lists that folder and every file it creates there, by device and inode, in the workspace manifest. Every later write checks that record without following symlinks: a folder or file that became a symlink, was replaced, or that the run didn't create is refused. A new file is created exclusively, so an existing one is never overwritten. The transcript grows by writing its whole new content to a new file renamed over the old one, and the record moves to the new file; a hard link to the old transcript keeps the old content. Cleanup never deletes evidence. It drops the run's evidence records, since no command writes evidence for a run that is gone.

## Cleanup

```bash
$V cleanup                    # the current run; or --run NAME, or --all
$V cleanup --all --builds     # also the builds and source copies
```

Cleanup asks the run's supervisor to stop the app and waits at most 10 s for its answer. The supervisor sends its child TERM, waits up to 5 seconds, sends KILL, and reaps it. Because the child stays unreaped until then, those signals can only reach the process the run started, even if it has exited or run another program. A run whose supervisor doesn't answer gets no signal: cleanup says so, and keeps the run folder while the run's process still runs. Then cleanup deletes the run folder (the fake home, the index and the copied binary) if the manifest lists it. It prints the evidence path and file count. `--builds` deletes only the build and source folders the manifest lists, and names anything else it found. `run.env` is parsed, never sourced, and a malformed one stops cleanup before it stops or deletes anything. Run cleanup after every failed attempt too. The supervisor runs in its own session, so a launch that the tool you run commands through kills on a timeout leaves the app running under its supervisor until cleanup. Give `launch` 3 minutes or run it in the background.

## Helpers

| File                   | What it is                                                                                                                                                                                                                                                                                                                                                        |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sifs-verify`          | The entry point: build, launch, doctor, drive, shot, cleanup.                                                                                                                                                                                                                                                                                                     |
| `ax.swift`             | Source of `sifs-ax`, compiled on first use to `<workspace>/bin/sifs-ax`. It reads and drives the window of one run. Direct use: `sifs-ax read <workspace>/runs/<name>`.                                                                                                                                                                                           |
| `seed-home.sh`         | `seed-home.sh <new absolute dir>` creates and writes the fake home. Office files are zipped from inline XML, so no fixture files are needed.                                                                                                                                                                                                                      |
| `open-spy.m`           | Source of the dylib that records open and reveal requests instead of performing them.                                                                                                                                                                                                                                                                             |
| `verify-support.patch` | `verification.rs` and its hooks in `app.rs`, for building commits from before them.                                                                                                                                                                                                                                                                               |
| `tests/run`            | Tests of every guard above. They start no app and send no input. Stand-in processes play test instances, each under a real supervisor, and a stand-in `npx` plays the build. A test build of `sifs-ax` reads fake accessibility trees and sends no events, so the real dialog detector runs against open, closed and failing trees. Run it after any change here. |

## Known limitations

- **A window closed to the menu bar can't be shown again.** The tray icon opens its menu only on a real mouse click (AXPress on the status item does nothing), and rule 1 forbids clicks. Nothing in this skill closes the window. If it is closed, run `cleanup` and `launch` again.
- **The Add a folder… dialog is only opened and cancelled.** It does open in the background, as a sheet in the test window's accessibility tree (`$V dialog-tree`, `$V cancel-dialog`). Picking a folder in it is not supported (rule 6). Adding folders is proven through the suggestion buttons. `sifs-ax` checks for a dialog when a command starts, so a Return sent in the instant a dialog is still opening would reach it. The folder guard in the app is what stops a scan then.
- **A test instance is not invisible.** SIFS_VERIFY_BACKGROUND stops activation and the Dock icon: across launches on this Mac the app in front never changed. Whether the window shows for a moment as it opens behind other windows was not observed. Its tray icon sits in the menu bar next to the owner's until cleanup. Tell the owner a test instance is running.
- **Nothing can be driven while the Mac's screen is locked.** Accessibility then lists no window for the test app, so `launch` stops with "shows no SIFS window to accessibility" even though `doctor` passes. Run `cleanup` and launch again once the screen is unlocked. Observed on this Mac with this branch and with the previous round's code alike.
- Dates in `meta` assume an English (US) system locale, as on this Mac.

## Feature map

`features/README.md` indexes one recipe per user-facing feature. Read it before driving, and use the matching file. A proof that drives one convenient entry point is incomplete when the map lists others. When the app changes, update the map in the same PR. `/maintain-verification-skill` audits it.
