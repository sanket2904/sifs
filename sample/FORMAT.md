# Extractor input and output

An extractor reads the sample's page text and writes one output file per run. `tools/score.mjs` compares that file with `labels.json`. This file is the contract between the two. Field names follow the label schema in `README.md` wherever the meaning is the same, so scoring is a comparison and not a translation.

An extractor never reads `spec.json`, `labels.json` or `files/`. It reads a text set, and in a run with the directory on it also reads `directory/banks.json`.

## Input

A text set is a folder with one JSON file per document, `<id>.json`, and an `index.json`.

| Folder | Pages |
| --- | --- |
| `text/` | The page text the labels' spans count in: the page's lines joined with a line feed (README, "Label schema"). A scanned page holds the text drawn on it, as a perfect reading would give. |
| `text-ocr/` | The same, except that each scanned page holds what Apple Vision reads from its picture, one recognised line per line, top to bottom. |

A document file holds:

| Field | Value |
| --- | --- |
| `id` | The document id. |
| `file` | The file name in `files/`. |
| `sha256` | The file's hash, as `files/SHA256SUMS` gives it. |
| `format` | `pdf`, `txt` or `html`. |
| `lang` | The document's language: `en`, `de`, `fr`, `hi` or `ja`. |
| `created` | The file date, `YYYY-MM-DDTHH:MM:SSZ`. It deliberately disagrees with statement periods. |
| `pages` | A list of `{"text", "scanned"}`. `scanned` is `true` for a page with no text layer. |

`index.json` holds `format` (1), `text_set` (the folder name), `reference_date` (the date that decides which passport is current, from `labels.json`), and `documents` (every id). In `text-ocr/` it also holds `ocr`: the macOS version, the Vision request and revision, the recognition level, language correction (off) and the pixel size each scanned page was drawn at. The committed set was read on macOS 26.4.1 with `VNRecognizeTextRequest` revision 3.

`node sample/tools/text.mjs` writes `text/` from `spec.json`, and `text-ocr/` from `spec.json` with each scanned page keeping its committed reading. `node sample/tools/text.mjs --ocr` reads the scanned pages again by running `tools/vision-text.swift`, macOS only. The OCR set is committed so a score does not depend on the machine that takes it. `node sample/tools/text.mjs --check` writes nothing and fails when a committed file in either set differs from what the tool writes, so text left behind by a change to `spec.json`, `labels.json` or `files/` fails `score.test.mjs`. For a scanned page it compares everything except the reading, which depends on the Mac.

## Output

One JSON file per run.

| Field | Value |
| --- | --- |
| `format` | `1`. |
| `extractor` | The extractor's name, for example `rust-m1a`. |
| `directory` | `on` or `off`. |
| `text_set` | `text` or `text-ocr`. |
| `organisations` | The extractor's own keys, each `{"name", "directory"}`. `directory` is a directory id, or `null`. With the directory off it is always `null`. |
| `documents` | One entry per document the extractor read. A document left out of the list has nothing found. |
| `answers` | `latest_statement`, a list of `{"account", "document"}`, and `current_passport`, a list of `{"holder", "document"}`. |

A key is any string. It is the extractor's identity for an organisation across all documents, as a local id is in the labels. The extractor never sees the labels' local ids.

A span is `page` (from 1), `start`, `end` (Unicode scalar values, end exclusive) and `text`, counted in the page text of the run's text set. Every span must read back its `text`, or the scorer rejects the file.

A document entry holds:

| Field | Entries |
| --- | --- |
| `id` | The document id. |
| `orgs` | An organisation mention: a span, `org` (a key, or `null` for a name that is no organisation the extractor can settle), and `settled_by`: `name`, `mention` or `code` when the document settles it (the README's evidence kinds), `directory` when only a directory lookup does, or `null`. |
| `identifiers` | A span, `id` (the extractor's own, unique in the document), `kind` (a README identifier kind), `value` (README normalisation: spaces removed, hyphens kept in a sort code), and `valid` (`true`, `false`, or `null` for a kind with no check). |
| `links` | Identifier held at organisation: `identifier` (an identifier `id`), `org` (a key), `reasons` (README link reasons), and `via` (an identifier `id`) with `paired-code`. |
| `statement` | `from`, `to` (`YYYY-MM-DD`), `accounts` (identifier ids that spell the statement's own account), and `issuer` (a key). Absent when the document is not a statement. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids), `issued`, `expiry` (a date, or `cannot-tell`), `expiry_at` (a span) and `current`. Absent when the document is not a passport. |

The answers use `reference_date` from `index.json`. `account` is any spelling of the account. `holder` is `SURNAME, GIVEN NAMES`.

### Example

The text set gives `text/kotak-statement.json`:

```json
{"id": "kotak-statement", "file": "statement-oct-udaharan.pdf", "format": "pdf", "lang": "en",
 "created": "2026-11-02T08:30:00Z",
 "pages": [{"text": "SAMPLE DOCUMENT. Made up for testing. Not a real statement.\nKotak Mahindra Bank\n…\nAccount No. 4512369874\n…\nPeriod: 01 Oct 2026 - 31 Oct 2026\n…", "scanned": false}]}
```

A run with the directory on writes, among the other documents:

```json
{
  "format": 1, "extractor": "rust-m1a", "directory": "on", "text_set": "text",
  "organisations": {"org-17": {"name": "Kotak Mahindra Bank", "directory": "Q2040404"}},
  "documents": [{
    "id": "kotak-statement",
    "orgs": [{"page": 1, "start": 60, "end": 79, "text": "Kotak Mahindra Bank", "org": "org-17", "settled_by": "name"}],
    "identifiers": [{"id": "i1", "page": 1, "start": 142, "end": 152, "text": "4512369874", "kind": "account", "value": "4512369874", "valid": null}],
    "links": [{"identifier": "i1", "org": "org-17", "reasons": ["statement-header"]}],
    "statement": {"from": "2026-10-01", "to": "2026-10-31", "accounts": ["i1"], "issuer": "org-17"}
  }],
  "answers": {"latest_statement": [{"account": "4512369874", "document": "kotak-statement"}], "current_passport": []}
}
```

With the directory off, the same run gives `"directory": null` for `org-17`.

## How the scorer decides

**Matching.** On `text/`, an output mention or identifier matches the label with the same span. On `text-ocr/`, spans cannot line up, so a mention matches by page and text, and an identifier by page and value. Equal texts pair in order of occurrence.

**Mentions.** The right organisation for a labelled mention is the README's rule. With the directory off it is `resolved_by_document.org`, or none. With it on, the document still decides first, and a mention the document does not settle is the lookup's entry when the lookup is exact, and none otherwise. A mention with a key where the right answer is none is wrong. A mention with `settled_by: "directory"` in a run with the directory off is wrong. An output mention with a key that matches no labelled mention is wrong, whether it is a `not_orgs` name or any other text.

**Organisation keys.** Each key maps to the set of labelled organisations its matched mentions land on, across all documents. A key that no mention lands on, such as a bank known only by the IFSC beside an account, maps to the organisations the labels link its linked identifiers to. Then:

- A key whose set holds one organisation is that organisation.
- A key whose set holds two or more is a WRONG MERGE: two organisations' papers on one card. Every mention, link and issuer that uses it is wrong, and the run fails.
- A labelled organisation that two or more keys land on is a SPLIT. Splits are listed and counted. They are not wrong.
- A key whose set is empty is no labelled organisation. A link to it is wrong.

`org_directory` scores each mapped key's `directory`. With the directory on it must be the organisation's directory id, and `null` counts as left out. With it off it must be `null`.

**Identifiers.** A matched identifier is right when its `kind`, `value` and `valid` all equal the label's. An output identifier that matches no label is wrong. When it sits on a `not_identifiers` entry it is reported as a decoy.

**Links.** A link is its identifier's kind and value and the organisation its key maps to. The same link twice in one document counts once. A link is right when the labels have that link and at least one of the output's reasons is among the label's reasons. `paired-code` holds only through the labelled `via` code. Every other link is wrong, with the first cause that applies: a merged or unmapped key, a labelled link with no reason that holds, a `not_links` entry, no such identifier in the labels, an identifier the labels mark invalid, an organisation the document does not settle, or nothing that ties them. A labelled link with no right output link is left out.

**Statements and passports.** `statement_period` is right when `from` and `to` equal the label's. `statement_account` is right when every account given spells the labelled account. `statement_issuer` is right when the key maps to the issuer mention's organisation. Passport holder (case and spacing ignored), number, expiry and current compare with the label. When the label's expiry is `cannot-tell`, only `cannot-tell` is right. A statement or passport on a document whose labels have none is wrong in every fact it gives.

**Answers.** A labelled answer is right when every output answer for that account or holder names the labelled document, wrong when one names another, and left out when none is given. An output answer for an account or holder the labels do not list is wrong.

**What OCR made impossible.** On `text-ocr/`, a labelled item whose text or value the page does not hold, with any spacing, is counted as impossible instead of left out. A link is impossible when its identifier is, or when its only reason is `paired-code` through an impossible code. An extractor that repairs the OCR text and finds the value anyway scores it as right.

**Exit status.** 0 when no link, merge or answer is wrong. 1 when one is. 2 when the output breaks this format. A left-out item never fails a run.

## How to score an extractor

1. Run the extractor four times: directory on and off, each on `text/` and on `text-ocr/`. In the off runs, do not give it `directory/banks.json`.
2. Score each output file:

   ```sh
   node sample/tools/score.mjs run-on-text.json
   ```

   It prints a table (right, wrong, left out and impossible for each measure, in total and per language), the merges and splits, and every wrong, impossible and left-out item. It writes the same as JSON to `run-on-text.score.json`, or to the path after `--json`.
3. The bar is no wrong link, no wrong merge and no wrong answer on all four runs. Leaving something out is allowed.

`node sample/tools/reference.mjs <on|off> <text|text-ocr> <path>` writes the output a perfect extractor would give, made from the labels. It scores full marks on `text/`, and on `text-ocr/` it loses only what OCR made impossible. `node --test sample/tools/score.test.mjs` tests each scoring rule, and `node sample/tools/score-mutants.mjs` removes one rule at a time from `score.mjs` and shows that a test fails.
