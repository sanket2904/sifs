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

`node sample/tools/text.mjs` writes `text/` from `spec.json`, and `text-ocr/` from `spec.json` with each scanned page keeping its committed reading. `node sample/tools/text.mjs --ocr` reads the scanned pages again by running `tools/vision-text.swift`, macOS only. The OCR set is committed so a score does not depend on the machine that takes it. `node sample/tools/text.mjs --check` writes nothing and fails when a committed file in either set differs from what the tool writes, so text left behind by a change to `spec.json`, `labels.json` or `files/` fails `score.test.mjs`. It hashes each file in `files/` itself, so a changed file fails the check even when `files/SHA256SUMS` still gives the old hash. For a scanned page it compares everything except the reading, which depends on the Mac.

## Output

One JSON file per run.

| Field | Value |
| --- | --- |
| `format` | `1`. |
| `extractor` | The extractor's name, for example `rust-m1a`. |
| `directory` | `on` or `off`. |
| `text_set` | `text` or `text-ocr`. |
| `organisations` | The extractor's own keys, each `{"name", "directory"}`. `name` is a name of the organisation, or `null`. `directory` is a directory id, or `null`. With the directory off it is always `null`. |
| `documents` | One entry per document the extractor read. A document left out of the list has nothing found. |
| `answers` | `latest_statement`, a list of `{"account", "document"}`, and `current_passport`, a list of `{"holder", "document"}`. |

A key is any non-empty string. It is the extractor's identity for an organisation across all documents, as a local id is in the labels. The extractor never sees the labels' local ids.

A span is `page` (from 1), `start`, `end` (Unicode scalar values, end exclusive) and `text`, counted in the page text of the run's text set. Every span must lie inside its page (`0 <= start <= end <=` the page's length) and read back its `text`, or the scorer rejects the file. One occurrence in the text is one row: two mentions, or two identifiers, with the same span in one document make the scorer reject the file.

A document entry holds:

| Field | Entries |
| --- | --- |
| `id` | The document id. |
| `orgs` | An organisation mention: a span, `org` (a key, or `null` for a name that is no organisation the extractor can settle), and `settled_by`: `name`, `mention` or `code` when the document settles it (the README's evidence kinds), `directory` when only a directory lookup does, or `null`. |
| `identifiers` | A span, `id` (the extractor's own, unique in the document), `kind` (a README identifier kind), `value` (as "Values" below gives it), and `valid` (`true`, `false`, or `null` for a kind with no check). |
| `links` | Identifier held at organisation: `identifier` (an identifier `id`), `org` (a key), `reasons` (README link reasons), and `via` (an identifier `id`), which a link with `paired-code` must give. |
| `statement` | `from`, `to` (`YYYY-MM-DD`), `accounts` (identifier ids that spell the statement's own account), and `issuer` (a key). Absent when the document is not a statement. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids of the document, or `null`), `issued` (a date, or `null`), `expiry` (a date, `cannot-tell`, or `null`), `expiry_at` (a span, or `null`) and `current`. Absent when the document is not a passport. |

Dates are `YYYY-MM-DD` and must be dates of the calendar. The scorer checks every passport field as the table gives it, and scores `holder`, `number`, `expiry` and `current`. It does not score `issued`, `mrz` or `expiry_at`; the machine-readable zone is scored as an identifier.

The answers use `reference_date` from `index.json`. `account` is any spelling of the account. `holder` is `SURNAME, GIVEN NAMES`.

### Example

The text set gives `text/kotak-statement.json`:

```json
{"id": "kotak-statement", "file": "statement-oct-udaharan.pdf",
 "sha256": "dc8f482debfe6c586235c305fb9e57ed824557a7e59899175ade89c9feb64427",
 "format": "pdf", "lang": "en", "created": "2026-11-02T08:30:00Z",
 "pages": [{"text": "SAMPLE DOCUMENT. Made up for testing. Not a real statement.\nKotak Mahindra Bank\n…\nAccount No. 1234567890\n…\nPeriod: 01 Oct 2026 - 31 Oct 2026\n…", "scanned": false}]}
```

A run with the directory on writes, among the other documents:

```json
{
  "format": 1, "extractor": "rust-m1a", "directory": "on", "text_set": "text",
  "organisations": {"org-17": {"name": "Kotak Mahindra Bank", "directory": "Q2040404"}},
  "documents": [{
    "id": "kotak-statement",
    "orgs": [{"page": 1, "start": 60, "end": 79, "text": "Kotak Mahindra Bank", "org": "org-17", "settled_by": "name"}],
    "identifiers": [{"id": "i1", "page": 1, "start": 142, "end": 152, "text": "1234567890", "kind": "account", "value": "1234567890", "valid": null}],
    "links": [{"identifier": "i1", "org": "org-17", "reasons": ["statement-header"]}],
    "statement": {"from": "2026-10-01", "to": "2026-10-31", "accounts": ["i1"], "issuer": "org-17"}
  }],
  "answers": {"latest_statement": [{"account": "1234567890", "document": "kotak-statement"}], "current_passport": []}
}
```

With the directory off, the same run gives `"directory": null` for `org-17`. A `…` in the input stands for lines left out here. `score.test.mjs` checks every JSON block in this file against the committed text and the scorer.

### Values

An identifier's `value` is written as the labels write it, whichever text set the run reads:

| Kind | `value` |
| --- | --- |
| `iban`, `card`, `account` | The text with its spaces removed. |
| `gb_sort_code` | The six digits, with the hyphens removed: `60-16-13` is `601613`. |
| `mrz` | The zone's two lines as printed, joined with one line feed. |
| `bic`, `ifsc`, `in_pan`, `fr_bank_code`, `fr_branch_code`, `fr_rib_key`, `passport_number` | The text as printed. |

On `text-ocr/` the value is still what the paper prints, so an extractor that repairs the OCR text gives the repaired value. The sort code in `natwest-statement`:

```json
{"id": "natwest-statement", "identifiers": [{"id": "i4", "page": 1, "start": 148, "end": 156, "text": "60-16-13", "kind": "gb_sort_code", "value": "601613", "valid": true}]}
```

The machine-readable zone in `passport-rohan`:

```json
{"id": "passport-rohan", "identifiers": [{"id": "i2", "page": 1, "start": 390, "end": 479, "text": "P<UTOKALPIT<<ROHAN<<<<<<<<<<<<<<<<<<<<<<<<<<\nSMP0029356UTO1609307M2801049<<<<<<<<<<<<<<<8", "kind": "mrz", "value": "P<UTOKALPIT<<ROHAN<<<<<<<<<<<<<<<<<<<<<<<<<<\nSMP0029356UTO1609307M2801049<<<<<<<<<<<<<<<8", "valid": true}]}
```

## How the scorer decides

**Matching.** An output mention or identifier matches the label with the same span. On `text-ocr/`, a scanned page holds what the OCR read, so spans there cannot line up: on a scanned page a mention matches by page and text, and an identifier by page and value. Every other page of `text-ocr/` holds the same text as in `text/` and matches by span, exactly as on `text/`. Equal texts or values pair by position on the page (page, then start), first with first, whatever order the output lists them in.

**Mentions.** The right organisation for a labelled mention is the README's rule. With the directory off it is `resolved_by_document.org`, or none. With it on, the document still decides first, and a mention the document does not settle is the lookup's entry when the lookup is exact, and none otherwise. A mention with a key where the right answer is none is wrong. A mention with `settled_by: "directory"` in a run with the directory off is wrong. An output mention with a key that matches no labelled mention is wrong, whether it is a `not_orgs` name or any other text.

**Attachments.** A `not_orgs` entry names, by its `entry`, the directory entry its text is not. The organisation it excludes is the one in `labels.json` whose `directory` is that id. An output mention with a key, on a `not_orgs` span, where the key maps to the organisation that entry excludes, is a WRONG ATTACHMENT: the document is on that organisation's card because of a name the labels say is not it, as with `HDFC` inside `HDFC Life Insurance Company Limited` keyed to HDFC Bank. It is listed with the document and the key, counted, and the run fails. The rule reads the labels, not the run's directory, so with the directory off the same hit by a name the extractor holds itself fails the same way. An organisation with no directory id is excluded by no entry, so its key on a `not_orgs` span is only a wrong mention. A key on a labelled mention whose right answer is none, such as `BOI` in `household-notes`, is only a wrong mention too.

**Organisation keys.** Each key maps to the set of labelled organisations its matched mentions land on, across all documents. A key that no mention lands on, such as a bank known only by the IFSC beside an account, maps through its links from identifiers that carry their institution inside them: an IFSC, a BIC or an IBAN. It maps to the organisations the labels link those identifiers to. An account linked to the key never says who the key is, whatever the reason given. Then:

- A key whose set holds one organisation is that organisation.
- A key whose set holds two or more is a WRONG MERGE: two organisations' papers on one card. Every mention, link and issuer that uses it is wrong, and the run fails.
- A labelled organisation that two or more keys land on is a SPLIT. Splits are listed and counted. They are not wrong.
- A key whose set is empty is no labelled organisation. A link to it is wrong.

**Cards.** A key's entry in `organisations` states who the key is, and the scorer checks it against the organisation the key maps to. `name` may be `null`, for example for a bank known only by a code with the directory off. When given, it must be a name the labels give that organisation: its `name` or one of its `names` in `labels.json`, the text of a labelled mention of it, or, with the directory on, a name of its directory entry. Case and whitespace do not count, so `三菱 UFJ 銀行` is `三菱UFJ銀行`. Nothing else is folded: `HDFC Ltd` is not `HDFC`, and punctuation counts. `directory`, when given, must be that organisation's directory id. A key whose card gives another name or another organisation's directory id is a WRONG CARD: the organisation's papers are on a card that says it is someone else. As with a merge, every mention, link and issuer that uses the key is wrong, and the run fails. A missing directory id is left out, not wrong.

`org_directory` scores each mapped key's `directory`. With the directory on it must be the organisation's directory id, and `null` counts as left out. With it off it must be `null`.

**Identifiers.** A matched identifier is right when its `kind` and `value` equal the label's, and its `valid` too unless the label's `valid` is `null`. The label gives `null` for a kind with no check, and for the PAN `ABCPK0000F`, which a shape check accepts and a strict check rejects; there either answer is right. An output identifier that matches no label is wrong. When it sits on a `not_identifiers` entry it is reported as a decoy.

**Links.** A link is its identifier's kind and value and the organisation its key maps to. A link's identifier counts only when it matched a labelled identifier with the same kind and value. Rows in one document for the same link are one link: it holds the union of their reasons and `via` codes and is scored once, whatever order the rows are in. A link is right when the labels have that link and at least one of its reasons is among the label's reasons. `paired-code` holds only when a `via` is an output identifier that matched the labelled `via` code. Every other link is wrong, with the first cause that applies: a merged, wrong-card or unmapped key, no such identifier in the labels, a labelled link with no reason that holds, a `not_links` entry, an identifier the labels mark invalid, an organisation the document does not settle, or nothing that ties them. A labelled link with no right output link is left out.

**Statements and passports.** `statement_period` is right when `from` and `to` equal the label's. `statement_account` is right when every account given spells the labelled account. `statement_issuer` is right when the key maps to the issuer mention's organisation. Passport holder (case and spacing ignored), number, expiry and current compare with the label. When the label's expiry is `cannot-tell`, only `cannot-tell` is right. A statement or passport on a document whose labels have none is wrong in every fact it gives.

**Answers.** A labelled answer is right when every output answer for that account or holder names the labelled document, wrong when one names another, and left out when none is given. An output answer for an account or holder the labels do not list is wrong.

**What OCR made impossible.** On `text-ocr/`, a labelled item whose text or value the page does not hold, with any spacing, is counted as impossible instead of left out. A link is impossible when its identifier is, or when its only reason is `paired-code` through an impossible code. An extractor that repairs the OCR text and finds the value anyway scores it as right.

**Exit status.** 0 when no link, merge, card, attachment or answer is wrong. 1 when one is. 2 when the output breaks this format, including a root that is not an object. A left-out item never fails a run.

## How to score an extractor

1. Run the extractor four times: directory on and off, each on `text/` and on `text-ocr/`. In the off runs, do not give it `directory/banks.json`.
2. Score each output file:

   ```sh
   node sample/tools/score.mjs run-on-text.json
   ```

   It prints a table (right, wrong, left out and impossible for each measure, in total and per language), the merges, wrong cards, wrong attachments and splits, and every wrong, impossible and left-out item. It writes the same as JSON to `run-on-text.score.json`, or to the path after `--json`.
3. The bar is no wrong link, no wrong merge, no wrong card, no wrong attachment and no wrong answer on all four runs. Leaving something out is allowed.

`node sample/tools/reference.mjs <on|off> <text|text-ocr> <path>` writes the output a perfect extractor would give, made from the labels. It scores full marks on `text/`, and on `text-ocr/` it loses only what OCR made impossible. `node --test sample/tools/score.test.mjs` tests each scoring rule, and `node sample/tools/score-mutants.mjs` removes one rule at a time from `score.mjs` and shows that a test fails.

## Changes in round 1

- A key is a non-empty string. A card's `name` may be `null`. A name or directory id that is not the organisation's makes a wrong card, which fails the run.
- A key no mention lands on is known only from an IFSC, BIC or IBAN linked to it, never from an account.
- A span outside its page, and a second mention or identifier on the same span, are format errors.
- A link counts only when its identifier, and for `paired-code` its `via`, matched a labelled identifier.
- Rows for the same link are scored once with the union of their reasons; row order never changes a score.
- "Values" gives `value` for every kind: a sort code is `601613`, and a machine-readable zone keeps its line feed.
- `valid` is not scored where the label's `valid` is `null`.
- Passport `number`, `mrz`, `issued`, `expiry` and `expiry_at` are checked; `issued`, `mrz` and `expiry_at` are not scored.
- The worked example gives the Kotak account that the text holds, `1234567890`.
- The tools run when started through a symlinked path, such as one under `/tmp` on macOS.

## Changes in round 2

- On `text-ocr/`, only a scanned page matches by page and text or value. Every other page matches by span, as on `text/`.
- A keyed mention on a `not_orgs` span, where the key maps to the organisation the entry excludes, is a wrong attachment, which fails the run.
- A card name ignores all whitespace, not only repeated spaces, and folds nothing else.
- A root that is not an object, and a `paired-code` link with no `via`, are format errors.
