# SIFS verification map

This folder is the maintained source for verifying what a SIFS user can do. Read this index, then drive the matching feature file as the recipe.

## Baseline preconditions

- A run started with `sifs-verify launch` from the build under test, and `sifs-verify doctor` all `ok`.
- The run's fake home holds the seeded files from `seed-home.sh`, and SIFS searches `~/Documents` and `~/Desktop` (launch adds both).
- Settings are closed and the search box is empty (launch leaves it so).
- The owner's SIFS, if running, is listed by doctor as "not ours" and is never addressed.

## Driving conventions

- `V=.pi/skills/verify/sifs-verify`. Every drive goes through `$V`, which addresses the test instance through its run, through accessibility, without activating it or touching the mouse.
- Name buttons by their visible title (`⚙︎`, `+ ~/Downloads`, `Remove`, `Save and re-index`, `show`). Name a folder's Remove with `--after "<folder as shown>"`.
- Assert on read-back JSON (`rows[].text`, `rows[].name`, `footer`, `lines`), not on screenshots. Screenshots are for the reviewer.
- A feature that changes state (folders, skipped patterns) runs last in a run, or in its own run. Recipes say what they change.

## Proof and skip reporting

- For each sub-feature, keep the `search` or `read` output that shows the result and a `shot` of that state. Both land in the run's evidence folder.
- A side effect is proven by a second, independent view: a search after the change, `$V opened`, or `doctor`.
- Report the build id and commit (`run.env` in the evidence folder) with every claim.
- A sub-feature you couldn't reach is reported as not verified, with the command you ran and the reason. Never report it as verified through another entry point.

## Feature entry contract

Each feature file starts with an H1 and one paragraph on what the user sees. Then exactly four H2s: `Sub-features`, `How to get to it (user POV)`, `Driving it with sifs-verify` (starting with `Preconditions:`), `Gotchas`.

## Features

- [Name search](./name-search.md): typing, ranking, close matches, selection, Escape, open and show in Finder.
- [Folders](./folders.md): adding a suggested folder, removing one, the Add a folder… dialog.
- [Skipped patterns](./skipped-patterns.md): the default rules and editing them with Save and re-index.
- [Reading inside files](./reading-inside-files.md): the counts line and the panel of files that couldn't be read or were skipped.
- [Search inside files](./content-search.md): content results with label and snippet. Lands with PR #5 (branch `phase3-step3`); not on main.
