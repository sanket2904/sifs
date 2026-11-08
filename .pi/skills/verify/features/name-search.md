# Name search

The user types in `Search your files` and, after a 60 ms pause, sees files and folders whose names match, case-insensitive, any part of the name, typo-tolerant, ranked by match quality and recency. Each row shows an icon, the name, its folder (`~`-relative), size and date. The footer reports the count and the time.

## Sub-features

- `ns-match` returns every seeded file whose name holds the query, and nothing else.
- `ns-close-match` returns names one typo away, marked `· close match`.
- `ns-none` shows `Nothing found for “…”.` for a query with no match.
- `ns-select` moves the highlight with ↓ and ↑, clamped at both ends.
- `ns-escape` clears the box and the rows.
- `ns-open` opens the selected row with Enter.
- `ns-reveal` shows the selected row in Finder with ⌘Enter.

## How to get to it (user POV)

- The search box at the top of the window has focus when the window opens. Type there.
- Keyboard: ↑ and ↓ move, Enter opens, ⌘Enter shows in Finder, Escape clears.
- Double-clicking a row opens it. Not driven: rule 1 forbids mouse events.

## Driving it with sifs-verify

Preconditions:

- Baseline run from the index, doctor all `ok`.

- **Match.** `$V search budget`. Rows, in order: `budget-draft-2025.txt` (`~/Documents`), `quarterly-budget.xlsx` (`~/Documents`), `team-budget.csv` (`~/Desktop/sifs-demo-code`). Row 0 is selected. The footer starts `3 results in`. `$V shot budget`.
- **Close match.** `$V search budgte`. The same three files, each with `closeMatch: true` and `· close match` in `text`.
- **None.** `$V search zzqxv`. `rows` is empty, and `lines` holds `Nothing found for “zzqxv”.`. The footer starts `0 results in`.
- **Select.** `$V search budget`, then `$V key down` twice and `$V read`. Exactly one row has `selected: true`: `team-budget.csv`. A third `$V key down` keeps it there. `$V key up` three times moves it to row 0. `$V shot budget-selected`.
- **Open.** With `team-budget.csv` selected, `$V key return`, then `$V opened`. The last line is `open<TAB>--<TAB><run home>/Desktop/sifs-demo-code/team-budget.csv`. The window doesn't change.
- **Show in Finder.** `$V key return --cmd`, then `$V opened`. The last line is `reveal<TAB><same path>`.
- **Open a folder.** `$V search open-me`. One row, `📁 sifs-verify-open-me ~/Documents · <date>`, with an empty size. `$V key return` records `open<TAB>--<TAB><run home>/Documents/sifs-verify-open-me`.
- **Escape.** `$V key escape`, then `$V read`. `query` is empty and `rows` is empty.

## Gotchas

- After Escape the footer still reads `0 results in N ms`, because Escape clears the rows but not the time. Observed on main at 6a45930. Assert on `rows` and `query`, not on the footer's absence.
- Results arrive after a 60 ms debounce. `search` waits until the rows and footer stop changing for 0.5 s, so use it rather than `type` plus a sleep.
- Open and reveal are captured by the open spy and never performed. They prove SIFS asked for the right path, not that Finder or another app opened it.
- Selection is kept per search. Every new `search` resets it to row 0.
- `name`, `folder` and `meta` are split from the row's text and assume an English (US) date. When they are null, assert on `text`.
