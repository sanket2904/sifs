# Folders

The user chooses which folders SIFS searches. Settings (⚙︎) list each folder with its item count, or `indexing…` while it scans, and a Remove button. Documents, Desktop and Downloads under the home folder are offered as one-click suggestions until added. A new install opens settings by itself because no folder is chosen yet.

## Sub-features

- `fo-first-run` opens settings on first launch, showing `No folders yet. Add the ones you want to search.`
- `fo-add-suggested` adds a suggested folder, scans it, and makes its files searchable.
- `fo-remove` removes a folder, and its files leave the results.
- `fo-dialog` opens the system folder picker from `Add a folder…`.

## How to get to it (user POV)

- Click ⚙︎ next to the search box to show or hide settings.
- Under `Folders SIFS searches`: `+ ~/Downloads` (and the other suggestions), `Add a folder…`, and each folder's `Remove`.

## Driving it with sifs-verify

Preconditions:

- Baseline run. This feature changes the folders, so run it after the read-only features or in its own run.
- `fo-first-run` is covered by `launch` itself: launch waits for `No folders yet` before it presses `+ ~/Documents` and `+ ~/Desktop`, and `transcript.md` holds the read-back.

- **Open settings.** `$V press "⚙︎"`, then `$V read`. `lines` holds `~/Desktop 10 items [Remove]` and `~/Documents 13 items [Remove] [Add a folder…] [+ ~/Downloads]`.
- **Add a suggestion.** `$V press "+ ~/Downloads"`, then `$V wait '^~/Downloads [0-9]+ items' 30`. It prints `~/Downloads 2 items [Remove] [Add a folder…]`, and the `+ ~/Downloads` button is gone. `$V shot folders-added`.
- **Its files are searchable.** `$V search invoice`. One row: `invoice-2041.txt`, folder `~/Downloads`.
- **Remove.** `$V press Remove --after "~/Downloads"`. `$V read` no longer lists `~/Downloads` with a count, and `[+ ~/Downloads]` is back.
- **Its files are gone.** `$V search invoice`. `rows` is empty, and `lines` holds `Nothing found for “invoice”.`. `$V shot folders-removed`.
- **Dialog opens.** `$V press "Add a folder…"`, then `$V dialog-tree` prints the sheet's tree (sidebar outline, buttons). Then `$V cancel-dialog`. `$V doctor` still passes `every folder it searches is inside the fake home`.

## Gotchas

- **Never press Open in the Add a folder… dialog.** It starts in the owner's real home, not the fake one. A development run pressed Open after Go to Folder (⌘⇧G) failed to take its path, and the test instance began indexing the owner's real Documents. Picking a folder in the dialog is not supported. `fo-dialog` proves only that the dialog opens.
- The `N items indexed in total.` line can stay stale after Remove: the window reloads settings as soon as it asks for the removal, which finishes in the background. One development run on main at 6a45930 kept the old total; the proof run showed the new one. Prove removal by searching, not by the total.
- Folder paths show as `~/…` only when the fake home's path has no symlink in it. `sifs-verify` uses `/private/tmp`, not `/tmp`. Under `/tmp` a folder shows as `/private~/Documents`, and the suggestion buttons don't hide after adding.
- Counts include the folders themselves: `~/Downloads 2 items` is the folder and `invoice-2041.txt`.
