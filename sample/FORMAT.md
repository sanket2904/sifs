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
| `format` | `2`. Format 1 had no tiers, and the scorer refuses it. |
| `extractor` | The extractor's name, for example `rust-m1a`. |
| `directory` | `on` or `off`. |
| `text_set` | `text` or `text-ocr`. |
| `organisations` | The extractor's own keys, each `{"name", "directory"}`. `name` is a name of the organisation, or `null`. `directory` is a directory id, or `null`. With the directory off it is always `null`. A key is one institution code or one printed name, never both ("No merging"). |
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
| `links` | Identifier held at organisation: `identifier` (an identifier `id`), `org` (a key), `reasons` (README link reasons), `via` (an identifier `id`), which a link with `paired-code` must give, and `tier`. |
| `statement` | `from`, `to` (`YYYY-MM-DD`), `accounts` (identifier ids that spell the statement's own account), `issuer` (a key, or `null`), and `tiers`: a tier for `period`, for `accounts` when it is not empty, and for `issuer` when it is not `null`. Absent when the document is not a statement. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids of the document, or `null`), `issued` (a date, or `null`), `expiry` (a date, `cannot-tell`, or `null`), `expiry_at` (a span, or `null`), `current`, and `tiers`: a tier for each of `holder`, `number`, `expiry` and `current` that is not `null`. Absent when the document is not a passport. |
| `same_org` | Two keys that the document shows to be one organisation: `keys` (two different keys), `reason` and `tier` ("Same organisation"). |

Dates are `YYYY-MM-DD` and must be dates of the calendar. The scorer checks every passport field as the table gives it, and scores `holder`, `number`, `expiry` and `current`. It does not score `issued`, `mrz` or `expiry_at`; the machine-readable zone is scored as an identifier.

The answers use `reference_date` from `index.json`. `account` is any spelling of the account. `holder` is `SURNAME, GIVEN NAMES`.

### Example

The text set gives `text/hdfc-statement-sep.json`:

```json
{"id": "hdfc-statement-sep", "file": "kalpit-statement-a.pdf",
 "sha256": "e57558fcf1d2f7d941793f90a8dd71b67ad9bc055fc987b620db7d4d1e544c34",
 "format": "pdf", "lang": "en", "created": "2026-08-03T09:00:00Z",
 "pages": [{"text": "SAMPLE DOCUMENT. Made up for testing. Not a real statement.\nHDFC Bank Ltd\n…\nAccount number: 50100417263958\nIFSC: HDFC0SAMPL1\n…\nStatement period: 01 Sep 2026 to 30 Sep 2026\n…", "scanned": false}]}
```

A run with the directory on writes, among the other documents:

```json
{
  "format": 2, "extractor": "go-m1a", "directory": "on", "text_set": "text",
  "organisations": {
    "name:hdfcbank": {"name": "HDFC Bank Ltd", "directory": "Q631047"},
    "ifsc HDFC": {"name": null, "directory": "Q631047"}
  },
  "documents": [{
    "id": "hdfc-statement-sep",
    "orgs": [{"page": 1, "start": 60, "end": 73, "text": "HDFC Bank Ltd", "org": "name:hdfcbank", "settled_by": "name"}],
    "identifiers": [
      {"id": "i1", "page": 1, "start": 193, "end": 207, "text": "50100417263958", "kind": "account", "value": "50100417263958", "valid": null},
      {"id": "i2", "page": 1, "start": 214, "end": 225, "text": "HDFC0SAMPL1", "kind": "ifsc", "value": "HDFC0SAMPL1", "valid": true}
    ],
    "links": [
      {"identifier": "i2", "org": "ifsc HDFC", "reasons": ["ifsc-prefix"], "tier": "certain"},
      {"identifier": "i1", "org": "ifsc HDFC", "reasons": ["paired-code"], "via": "i2", "tier": "likely"},
      {"identifier": "i1", "org": "name:hdfcbank", "reasons": ["statement-header"], "tier": "likely"}
    ],
    "statement": {"from": "2026-09-01", "to": "2026-09-30", "accounts": ["i1"], "issuer": "name:hdfcbank",
                  "tiers": {"period": "likely", "accounts": "likely", "issuer": "likely"}},
    "same_org": [{"keys": ["name:hdfcbank", "ifsc HDFC"], "reason": "directory", "tier": "certain"}]
  }],
  "answers": {"latest_statement": [{"account": "50100417263958", "document": "hdfc-statement-sep"}], "current_passport": []}
}
```

The printed name and the IFSC have one key each. The IFSC link is certain because the directory holds `HDFC` for the entry on its card. The account is held at both keys, and both links are likely: one is read from the line beside the code, the other from the header. The `same_org` row says the two keys are one bank, and it is certain because both cards give the directory id that the name and the code look up to. A person sees one HDFC Bank card.

With the directory off, both cards give `"directory": null`, the IFSC link is `likely`, and the `same_org` row is `likely` with the reason `spells-code`: the name begins with the code's four letters. A person then sees two cards and the row "may be the same organisation". A `…` in the input stands for lines left out here. `score.test.mjs` checks every JSON block in this file against the committed text and the scorer.

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

## Tiers

Every link, statement fact, passport fact and `same_org` row carries a tier, `certain` or `likely`. The app shows a certain claim as a fact. It shows a likely claim as "may be", with its reason, for a person to confirm or reject.

A claim is `certain` only when the characters of an identifier prove it, or when the directory holds the exact code. Nothing read from layout, headings, nearness or wording is certain, however strong it looks. Everything else the extractor wants to say is `likely`: a statement header's bank, a code beside an account, the issuer, the period, which passport is current, which statement is the latest, and which printed name goes with which code.

| Claim | Certain when |
| --- | --- |
| Link with `iban-bank-code` | The identifier is an IBAN whose check digits pass. The scorer computes them again from `value`. An IBAN that passes is held at the institution that has its bank code in its country, so the link goes to that code's key. The directory is not needed. |
| Link with `ifsc-prefix` | The directory is on, and the directory entry that the key's card gives lists the IFSC's first four letters. An IFSC has no check digit, so with the directory off the link is likely. |
| Link with `bic-institution` | The directory is on, and the directory entry that the key's card gives lists a BIC with the same first eight characters (bank, country and location). A BIC has no check digit, so with the directory off the link is likely. |
| Link with `paired-code`, `same-account`, `inside-iban` or `statement-header` alone | Never. These reasons are read from the page's layout. |
| Statement `period`, `accounts`, `issuer` | Never. |
| Passport `holder`, `number`, `expiry` | The passport's `mrz` is a passport machine-readable zone (it starts with `P`) whose check digits all pass. A visa's zone makes no passport fact certain. |
| Passport `current` | Never. It depends on other documents and on the reference date. |
| `same_org` row | Its reason is `directory`, the directory is on, and both cards give the same directory id. |

A link with several reasons is certain when one of them passes its test. An output that calls a claim certain where this table does not allow it breaks the format. Mentions, identifiers and cards carry no tier: they are what the extractor read, and the claims above are built on them. An identifier's `valid` already says whether its check digits pass. The answers carry no tier and are always likely, because "latest" and "current" are judgements over many documents.

### No merging

An extractor never merges two organisations into one key. A key is one of these:

- An institution code. The key is that code: two different codes are two keys, always. The institution code of an IFSC is its first four letters. The institution code of a BIC is its first six characters (bank and country). The institution code of an IBAN is its country and bank code. The scorer knows where the bank code sits for CH, DE, ES, FR, GB, IE and NL; for an IBAN of another country it reads no code, and only the labels can find a merge.
- A printed name. The key stands for that name, and the extractor may use it in every document that prints the name.

A key holds the institution code of every IFSC, BIC or IBAN linked to it with that code's own reason (`ifsc-prefix`, `bic-institution` or `iban-bank-code`). A key is a MERGE when it holds two institution codes, when it holds an institution code and a mention is keyed to it, or when its papers land on two labelled organisations ("Organisation keys"). Any merge fails the run.

"This name is that code's bank" and "these two keys are one organisation" are claims of their own, written as `same_org` rows. A code's key may still have a card name, such as the directory entry's name with the directory on.

### Same organisation

A `same_org` row sits in the document that shows the join. Both keys must be used in that document by a mention, a link or the statement's issuer. `reason` is one of these:

| Reason | The document or the directory shows |
| --- | --- |
| `directory` | Both cards give the same directory id: the name's exact lookup and the code's entry are one entry. Only this reason can be certain, and only with the directory on. |
| `spells-code` | The name begins with the code's letters, as `HDFC Bank Ltd` begins with the `HDFC` of `HDFC0SAMPL1`. |
| `statement-header` | The name heads the statement and the code is on the statement's account. |
| `printed-together` | The name and the code are on one line, or on lines next to each other. |

### Ids

Each claim can be named without its position in the file, so that a person's confirmation can be kept outside this format. A link's id is its document, its identifier's kind and value, and its key. A `same_org` row's id is its document and its two keys in sorted order. Rows with the same id are one claim. The claim holds the union of the rows' reasons, and it is certain when one of its rows is certain. Row order never changes a score. A key spelt from its code or name, such as `ifsc HDFC`, keeps ids the same from one run to the next; a counter does not.

### What the tiers settle

- A code-based link reason with the directory off: `iban-bank-code` is certain when the check digits pass, and the link goes to the IBAN's code key. `ifsc-prefix` and `bic-institution` are likely, because nothing has checked that the code is a bank's.
- A code that names no organisation printed in the document makes a key of its own, with `name` `null` when the directory is off. Its links are likely unless the table above makes them certain. A link from that code to a printed name's key is not allowed: the join is a `same_org` row.

## How the scorer decides

**Matching.** An output mention or identifier matches the label with the same span. On `text-ocr/`, a scanned page holds what the OCR read, so spans there cannot line up: on a scanned page a mention matches by page and text, and an identifier by page and value. Every other page of `text-ocr/` holds the same text as in `text/` and matches by span, exactly as on `text/`. This has a limit. On a scanned page the scorer checks an identifier's value, not where on the page it was cited, because the labels' spans count in the clean text. A citation that points at the wrong line of a scanned page is not caught there. The same extractor code is checked by span on every other page. Equal texts or values pair by position on the page (page, then start), first with first, whatever order the output lists them in.

**Mentions.** The right organisation for a labelled mention is the README's rule. With the directory off it is `resolved_by_document.org`, or none. With it on, the document still decides first, and a mention the document does not settle is the lookup's entry when the lookup is exact, and none otherwise. A mention with a key where the right answer is none is wrong. A mention with `settled_by: "directory"` in a run with the directory off is wrong. An output mention with a key that matches no labelled mention is wrong, whether it is a `not_orgs` name or any other text.

**Attachments.** An output mention with a key, exactly on a `not_orgs` span, is a WRONG ATTACHMENT unless its key maps to the organisation of a labelled mention on that page whose span contains it. The document would sit on a card because of text the labels say is no organisation. For example, `HDFC` inside `HDFC Life Insurance Company Limited` keyed to HDFC Bank is a wrong attachment, and so is a new `Bank of India` card made from the text inside `State Bank of India`. The same `HDFC` keyed to the HDFC Life card is not. A key maps to an organisation only through its mentions and links, never through its card, so a key that maps to no organisation, or to two, is always wrong here, whatever its card's name. Either way the mention itself is a wrong mention, because the span is not an organisation. A wrong attachment is listed with the document and the key, and counted. It does not fail the run by itself: which organisation a printed name is, is read from wording, so the claims built on it are likely. The rule does not depend on directory ids, so it holds the same with the directory on or off. A key on a labelled mention whose right answer is none, such as `BOI` in `household-notes`, is only a wrong mention.

**Report the whole name.** A mention's span is the whole name as the document writes it, such as `HDFC Bank Limited`, not a short form inside it, such as `HDFC`. A card that is only ever reported through a short form inside a longer name maps to no organisation, so its attachments and its links fail, even when its card names the right organisation. Only a link from an IFSC, a BIC or an IBAN can still map it, as "Organisation keys" says.

**Organisation keys.** Each key maps to the set of labelled organisations its matched mentions land on, across all documents. A key that no mention lands on, such as a bank known only by the IFSC beside an account, maps through its links from identifiers that carry their institution inside them: an IFSC, a BIC or an IBAN. It maps to the organisations the labels link those identifiers to. An account linked to the key never says who the key is, whatever the reason given. Then:

- A key whose set holds one organisation is that organisation.
- A key whose set holds two or more is a MERGE: two organisations' papers on one card. Every mention, link, issuer and `same_org` row that uses it is wrong, and the run fails. The other two merges of "No merging", two codes or a code and a name on one key, fail the run as well, but the rows on the key are scored as the organisation the labels give it.
- A labelled organisation that two or more keys land on is a SPLIT. Splits are listed and counted. They are not wrong.
- A key whose set is empty is no labelled organisation. A link to it is wrong.

**Cards.** A key's entry in `organisations` states who the key is, and the scorer checks it against the organisation the key maps to. `name` may be `null`, for example for a bank known only by a code with the directory off. When given, it must be a name the labels give that organisation: its `name` or one of its `names` in `labels.json`, the text of a labelled mention of it, or, with the directory on, a name of its directory entry. Case and whitespace do not count, so `三菱 UFJ 銀行` is `三菱UFJ銀行`. Both names also go through Unicode compatibility folding (NFKC), so a full-width `＆` is `&` and full-width letters are ordinary letters. Nothing else is folded: `HDFC Ltd` is not `HDFC`, and punctuation counts. `directory`, when given, must be that organisation's directory id. A key whose card gives another name or another organisation's directory id is a WRONG CARD: the organisation's papers are on a card that says it is someone else. As with a merge, every mention, link, issuer and `same_org` row that uses the key is wrong. A wrong card fails the run through the certain rows on it. A missing directory id is left out, not wrong.

`org_directory` scores each mapped key's `directory`. With the directory on it must be the organisation's directory id, and `null` counts as left out. With it off it must be `null`.

**Identifiers.** A matched identifier is right when its `kind` and `value` equal the label's, and its `valid` too unless the label's `valid` is `null`. The label gives `null` for a kind with no check, and for the PAN `ABCPK0000F`, which a shape check accepts and a strict check rejects; there either answer is right. An output identifier that matches no label is wrong. When it sits on a `not_identifiers` entry it is reported as a decoy.

**Links.** A link is its identifier's kind and value and the organisation its key maps to. A link's identifier counts only when it matched a labelled identifier with the same kind and value. Rows in one document for the same link are one link: it holds the union of their reasons and `via` codes and is scored once, whatever order the rows are in. That includes rows at two keys of one organisation, such as an account held at a bank's name and at its code. The link is certain when one of its rows is certain. A link is right when the labels have that link and at least one of its reasons is among the label's reasons. `paired-code` holds only when a `via` is an output identifier that matched the labelled `via` code. Every other link is wrong, with the first cause that applies: a merged, wrong-card or unmapped key, no such identifier in the labels, a labelled link with no reason that holds, a `not_links` entry, an identifier the labels mark invalid, an organisation the document does not settle, or nothing that ties them. A labelled link with no right output link is left out.

**Same organisation.** A `same_org` claim is right when both keys map to one labelled organisation. It is wrong when they map to two, or when either key maps to none, merges two organisations or has a wrong card. The labels have no `same_org` rows: they are scored against the labels' organisations. In each document, the keys that the document uses and that map to one labelled organisation must be joined by right claims, directly or through each other. Each join that is missing is left out. With the directory off, some of these joins only the directory could make, such as Commerzbank and the IBAN in `transfer-untied`, where three banks are named in one line; leaving them out is the right answer.

**Statements and passports.** `statement_period` is right when `from` and `to` equal the label's. `statement_account` is right when every account given spells the labelled account. `statement_issuer` is right when the key maps to the issuer mention's organisation. Passport holder (case and spacing ignored), number, expiry and current compare with the label. When the label's expiry is `cannot-tell`, only `cannot-tell` is right. A statement or passport on a document whose labels have none is wrong in every fact it gives.

**Answers.** A labelled answer is right when every output answer for that account or holder names the labelled document, wrong when one names another, and left out when none is given. An output answer for an account or holder the labels do not list is wrong. Every answer is likely.

**Tiers.** Each measure counts right and wrong per tier, from the tier the output gives. Left out and impossible have no tier. The table prints them with the precision of the likely rows, right divided by right and wrong. Each wrong row is listed with its tier, a certain one as `WRONG CERTAIN`.

**What OCR made impossible.** On `text-ocr/`, a labelled item whose text or value the page does not hold, with any spacing, is counted as impossible instead of left out. A link is impossible when its identifier is, or when its only reason is `paired-code` through an impossible code. An extractor that repairs the OCR text and finds the value anyway scores it as right.

**Exit status.** 0 when no certain row is wrong and no key merges. 1 when one is wrong or one merges. 2 when the output breaks this format, including a root that is not an object, a format 1 output and a missing tier. A likely row and a left-out row never fail a run.

## How to score an extractor

1. Run the extractor four times: directory on and off, each on `text/` and on `text-ocr/`. In the off runs, do not give it `directory/banks.json`.
2. Score each output file:

   ```sh
   node sample/tools/score.mjs run-on-text.json
   ```

   It prints a table (right, wrong, left out and impossible for each measure, in total, per tier and per language), the merges, wrong cards, wrong attachments and splits, and every wrong, impossible and left-out item. It writes the same as JSON to `run-on-text.score.json`, or to the path after `--json`.
3. The bar is no wrong certain row and no merge on all four runs. Leaving something out is allowed. The likely rows' precision says how often a "may be" is right.

`node sample/tools/reference.mjs <on|off> <text|text-ocr> <path>` writes the output a perfect extractor would give, made from the labels. It gives each printed name and each institution code a key of its own and joins them with `same_org` rows. It scores full marks on `text/`, and on `text-ocr/` it loses only what OCR made impossible. With the directory off it leaves out the joins that only the directory could make. `node --test sample/tools/score.test.mjs` tests each scoring rule, and `node sample/tools/score-mutants.mjs` removes one rule at a time from `score.mjs` and shows that a test fails.

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
- A keyed mention exactly on a `not_orgs` span is a wrong attachment, which fails the run, unless its key maps to the organisation of a labelled mention on the same page whose span contains that span.
- A wrong attachment does not depend on directory ids. It holds with the directory on or off, and for a key that maps to no labelled organisation, such as a "Bank of India" card made from the text inside "State Bank of India".
- A card name ignores all whitespace, not only repeated spaces, and folds nothing else.
- A root that is not an object, and a `paired-code` link with no `via`, are format errors.

## Changes in round 3

- "Attachments" states the rule the scorer applies: an exact `not_orgs` span is a wrong attachment unless its key maps to the organisation of a labelled mention that contains it.
- "Report the whole name" says that a card reported only through a short form inside a longer name maps to no organisation, so its attachments and links fail.
- "Matching" states that on a scanned page an identifier is checked by value, not by where it was cited.
- "Cards" states that card names also go through NFKC folding, so `＆` is `&`.

## Changes in round 4

- Output format 2. Every link, statement fact, passport fact and `same_org` row has a tier, `certain` or `likely`. The scorer refuses format 1 and any missing tier, and any certain claim that "Tiers" does not allow.
- A key is one institution code or one printed name. Two codes on one key, or a code and a name, is a merge, and any merge fails the run.
- `same_org` rows say that two keys are one organisation. They are scored against the labels' organisations.
- A run fails when a certain row is wrong or a key merges. A wrong likely link, a wrong answer, a wrong card and a wrong attachment are counted and listed, and do not fail the run by themselves.
