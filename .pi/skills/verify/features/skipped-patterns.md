# Skipped patterns

SIFS skips hidden files and folders, `~/Library`, `node_modules`, build folders and similar by default. The list is editable in settings under `Skipped`, one glob per line, where `~` is the home folder and `**/` matches any folder. Save and re-index rescans every folder, so newly skipped files leave the results and unskipped ones join.

## Sub-features

- `sk-defaults` keeps `node_modules` and hidden files out of the results with the default list.
- `sk-add-rule` adds a pattern, and after Save and re-index the matching files leave the results.

## How to get to it (user POV)

- ⚙︎, then the `Skipped` text area, then `Save and re-index`.

## Driving it with sifs-verify

Preconditions:

- Baseline run. `sk-add-rule` changes the rules and rescans, so run it last or in its own run.

- **Defaults.** `$V search left-pad` and `$V search env.local`. Both show `Nothing found`, although `~/Desktop/sifs-demo-code/node_modules/left-pad/index.js` and `~/Desktop/sifs-demo-code/.env.local` exist.
- **Before the rule.** `$V search bundle-output`. One row: `bundle-output.js`, folder `~/Desktop/sifs-demo-code/generated`.
- **Add a rule.** `$V press "⚙︎"`, `$V focus-textarea`, `$V key down --cmd` (to the end), `$V type $'\n**/generated'`. `$V read` shows `  **/generated` as the last line under `[text area]`. Then `$V press "Save and re-index"`.
- **Rescanned.** `$V wait '^~/Desktop [0-9]+ items' 30`. The count drops from `10 items` to `8 items` (the `generated` folder and its file).
- **After the rule.** `$V search bundle-output`. `rows` is empty, and `lines` holds `Nothing found for “bundle-output”.`. `$V shot skipped-rule`.

## Gotchas

- `type` sends `\n` as Return, which is a new line in the text area. ⌘↓ moves to the end, so the recipe appends rather than selecting all (⌘A was not tried).
- A bad pattern should show its error in a `lines` entry under the search box and save nothing. That is read from the page's code; it was not driven.
- Rules persist in the run's index. A fresh `launch` starts from the defaults again.
