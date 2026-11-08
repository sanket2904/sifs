# Search inside files

**Not on main yet. It lands with PR #5 (branch `phase3-step3`).** Until it merges, drive it from a build of the PR's head (`$V build --ref origin/phase3-step3`). When it merges, delete this notice and move the Gotchas' PR-specific notes into the recipe. The one search box finds files by name and by the words inside them. Names show first. About 120 ms after typing pauses, names and contents ranked together replace them. A content result adds a line under the folder: a label for where the words are (sheet name, page, slide), then a snippet with the matched words in bold.

## Sub-features

- `cs-text` finds a text or code file by a word inside it and shows a snippet.
- `cs-word` finds a .docx by a word inside it.
- `cs-sheet` finds an .xlsx by a cell's text, labelled with the sheet's name.
- `cs-phrase` finds a two-word query across a sheet.
- `cs-escape` keeps the box and rows empty when Escape is pressed while the content pass is pending.
- `cs-open` opens a content result with Enter.

## How to get to it (user POV)

- Type in `Search your files`, as for a name search, and pause.

## Driving it with sifs-verify

Preconditions:

- A run launched from a PR #5 build, for example `$V build --ref d470ac5` then `$V launch --build d470ac5`. Doctor is all `ok`, and launch has waited for `0 waiting` in `Reading inside files`.
- Use `search` with a settle of 1.5 s, so it waits for the content pass, not only the names.

- **Text and code.** `$V search pelican 1.5`. Three rows, each with a `found` line: `meeting-notes.txt` (`…We agreed the zanzibar timeline and the pelican budget.`), `project-brief.docx` (`Project Zanzibar launch plan The pelican milestone is due in March.`), `main.rs` (`fn main() { println!("pelican server starting"); }`). `$V shot pelican-snippets`. The screenshot shows `pelican` in bold in each snippet.
- **Word.** `$V search zanzibar 1.5`. `project-brief.docx` is among the rows, with a `found` line. `server.py` shows `def serve(): return "zanzibar"`.
- **Sheet.** `$V search marmalade 1.5`. One row, `quarterly-budget.xlsx`, `found` `Budget · Item marmalade imports 1200`. The label `Budget` is the sheet's name.
- **Phrase.** `$V search "finance team" 1.5`. One row, `quarterly-budget.xlsx`, `found` `Notes · Approved by the finance team`.
- **Escape while pending.** `$V focus-search`, `$V type pelican`, `$V key escape` at once, wait 1.5 s, then `$V read`. `query` is empty, `rows` is empty, and the footer has no count.
- **Open.** `$V search pelican 1.5`, `$V key down` twice, `$V key return`, then `$V opened`. The last line is `open<TAB>--<TAB><run home>/Desktop/sifs-demo-code/src/main.rs`.

## Gotchas

- The rows and footer are those of PR #5 at d470ac5. The branch has moved since: origin/phase3-step3 was 5054213 when this was written. Rerun before trusting these exact strings on a later head.
- Bold is visible only in the screenshot. Accessibility gives the snippet as plain text in `found`.
- A .docx without page breaks has no label, so its `found` starts straight with the snippet.
- At 6e11b57, an earlier PR #5 commit, Escape during the content pass brought the rows back under an empty box (headless report in `devos-work/astra/s3ui/REPORT.txt`). At d470ac5 the real window stays empty. Keep `cs-escape` in every rerun.
