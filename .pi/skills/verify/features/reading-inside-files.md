# Reading inside files

In the background, after scans, SIFS reads the text inside documents (text, Markdown, CSV, code, JSON, HTML, Word, Excel, PowerPoint) into its index. Settings show a counts line, `Reading inside files: N read · N waiting · N couldn’t be read · N skipped`. When anything failed or was skipped, a `show` link lists each such file and why. On main this feeds nothing a user can search yet. Searching it lands with PR #5 ([Search inside files](./content-search.md)).

## Sub-features

- `ri-counts` shows the counts line, and it drains to `0 waiting`.
- `ri-failures` lists each file that couldn't be read or was skipped, with its reason, under `show`, and hides them with `hide`.

## How to get to it (user POV)

- ⚙︎, then the line under `N items indexed in total.`, then `show` or `hide` at its end.

## Driving it with sifs-verify

Preconditions:

- Baseline run. Launch already waited for `0 waiting`.

- **Counts.** `$V press "⚙︎"`, then `$V read`. `lines` holds `Reading inside files: 12 read · 0 waiting · 1 couldn’t be read · 3 skipped [show]`.
- **Failures panel.** `$V press show`, then `$V read`. `lines` holds, in this order:
  - `~/Documents/locked-notes.txt couldn’t be read: Permission denied (os error 13)`
  - `~/Documents/broken-report.docx skipped: not a Word document`
  - `~/Documents/firmware-dump.txt skipped: not text`
  - `~/Documents/huge-export.json skipped: larger than 1 MB`

  `$V shot reading-failures`.
- **Hide.** `$V press hide`. The four lines are gone, and the counts line ends in `[show]`.

## Gotchas

- The counts come from main at 6a45930. A change to the readers or to the seed changes them. Update this file and `seed-home.sh` together.
- `locked-notes.txt` is mode 000 in the seed. A run as root would read it, so the counts assume a normal user.
- The `show` button's title is the whole word `show` (later `hide`). `press` matches titles exactly.
- Reading pauses while any scan runs, so after adding a folder or saving patterns, wait for `0 waiting` again (`$V wait '0 waiting' 60`).
