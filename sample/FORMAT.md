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
| `format` | `3`. Format 1 had no tiers. Format 2 let a printed name's key stand in every document and took an expiry's century from the zone. The scorer refuses both. |
| `extractor` | The extractor's name, for example `rust-m1a`. |
| `directory` | `on` or `off`. |
| `text_set` | `text` or `text-ocr`. |
| `organisations` | The extractor's own keys, each `{"name", "directory"}`. `name` is a name of the organisation, or `null`. `directory` is a directory id, or `null`. With the directory off it is always `null`. A key is an institution code's key or a document key, never both ("No merging"). |
| `documents` | One entry per document the extractor read. A document left out of the list has nothing found. |
| `answers` | `latest_statement`, a list of `{"account", "document"}`, and `current_passport`, a list of `{"holder", "document"}`. |

A key is the extractor's identity for an organisation. An institution code's key is the same in every document. A document key names one document and is used only there. The extractor never sees the labels' local ids. "No merging" and "Code keys" say how a key is spelt.

A span is `page` (from 1), `start`, `end` (Unicode scalar values, end exclusive) and `text`, counted in the page text of the run's text set. Every span must lie inside its page (`0 <= start <= end <=` the page's length) and read back its `text`, or the scorer rejects the file. One occurrence in the text is one row: two mentions, or two identifiers, with the same span in one document make the scorer reject the file.

A document entry holds:

| Field | Entries |
| --- | --- |
| `id` | The document id. |
| `orgs` | An organisation mention: a span, `org` (a key, or `null` for a name that is no organisation the extractor can settle), and `settled_by`: `name`, `mention` or `code` when the document settles it (the README's evidence kinds), `directory` when only a directory lookup does, or `null`. |
| `identifiers` | A span, `id` (the extractor's own, unique in the document), `kind` (a README identifier kind), `value` (as "Values" below gives it), `valid` (`true`, `false`, or `null` for a kind with no check), and `tier`, the tier of the kind ("Identifier tiers"). |
| `links` | Identifier held at organisation: `identifier` (an identifier `id`), `org` (a key), `reasons` (README link reasons), `via` (an identifier `id`), which a link with `paired-code` must give, and `tier`. |
| `statement` | `from`, `to` (`YYYY-MM-DD`), `accounts` (identifier ids that spell the statement's own account), `issuer` (a key, or `null`), and `tiers`: a tier for `period`, for `accounts` when it is not empty, and for `issuer` when it is not `null`. Absent when the document is not a statement. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids of the document, or `null`), `issued` (a date, or `null`), `expiry` (a date, `cannot-tell`, or `null`), `expiry_at` (the span of the printed expiry, or `null`; a certain expiry needs it), `current`, and `tiers`: a tier for each of `holder`, `number`, `expiry` and `current` that is not `null`. Absent when the document is not a passport. |
| `same_org` | Two keys that the document shows to be one organisation: `keys` (two different keys), `reason` and `tier` ("Same organisation"). |

Dates are `YYYY-MM-DD` and must be dates of the calendar. The scorer checks every passport field as the table gives it, and scores `holder`, `number`, `expiry` and `current`. It does not score `issued` or `mrz`; the machine-readable zone is scored as an identifier. It scores `expiry_at` only for a certain expiry, which must cite the printed date of expiry, and it reads that text again ("Tiers").

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
  "format": 3, "extractor": "go-m1a", "directory": "on", "text_set": "text",
  "organisations": {
    "name:hdfc-statement-sep:hdfcbank": {"name": "HDFC Bank Ltd", "directory": "Q631047"},
    "IFSC HDFC": {"name": null, "directory": "Q631047"}
  },
  "documents": [{
    "id": "hdfc-statement-sep",
    "orgs": [{"page": 1, "start": 60, "end": 73, "text": "HDFC Bank Ltd", "org": "name:hdfc-statement-sep:hdfcbank", "settled_by": "name"}],
    "identifiers": [
      {"id": "i1", "page": 1, "start": 193, "end": 207, "text": "50100417263958", "kind": "account", "value": "50100417263958", "valid": null, "tier": "likely"},
      {"id": "i2", "page": 1, "start": 214, "end": 225, "text": "HDFC0SAMPL1", "kind": "ifsc", "value": "HDFC0SAMPL1", "valid": true, "tier": "certain"}
    ],
    "links": [
      {"identifier": "i2", "org": "IFSC HDFC", "reasons": ["ifsc-prefix"], "tier": "certain"},
      {"identifier": "i1", "org": "IFSC HDFC", "reasons": ["paired-code"], "via": "i2", "tier": "likely"},
      {"identifier": "i1", "org": "name:hdfc-statement-sep:hdfcbank", "reasons": ["statement-header"], "tier": "likely"}
    ],
    "statement": {"from": "2026-09-01", "to": "2026-09-30", "accounts": ["i1"], "issuer": "name:hdfc-statement-sep:hdfcbank",
                  "tiers": {"period": "likely", "accounts": "likely", "issuer": "likely"}},
    "same_org": [{"keys": ["name:hdfc-statement-sep:hdfcbank", "IFSC HDFC"], "reason": "directory", "tier": "certain"}]
  }],
  "answers": {"latest_statement": [{"account": "50100417263958", "document": "hdfc-statement-sep"}], "current_passport": []}
}
```

The printed name and the IFSC have one key each. The name's key is a document key: it stands for `HDFC Bank Ltd` in this statement only, and the August statement gives its own name its own key. The IFSC is certain to be an IFSC, and its link certain, because the directory holds `HDFC` for the entry on its card. The account number is likely: nothing in its digits says it is an account. The account is held at both keys, and both links are likely: one is read from the line beside the code, the other from the header. The `same_org` row says the two keys are one bank, and it is certain because the name `HDFC Bank Ltd` looks up to exactly one directory entry and the code `HDFC` to that same entry. A person sees one HDFC Bank card.

With the directory off, both cards give `"directory": null`, the IFSC and its link are `likely`, and the `same_org` row is `likely` with the reason `spells-code`: the name begins with the code's four letters. A person then sees two cards and the row "may be the same organisation". A `…` in the input stands for lines left out here. `score.test.mjs` checks every JSON block in this file against the committed text and the scorer.

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
{"id": "natwest-statement", "identifiers": [{"id": "i4", "page": 1, "start": 148, "end": 156, "text": "60-16-13", "kind": "gb_sort_code", "value": "601613", "valid": true, "tier": "likely"}]}
```

The machine-readable zone in `passport-rohan`:

```json
{"id": "passport-rohan", "identifiers": [{"id": "i2", "page": 1, "start": 390, "end": 479, "text": "P<UTOKALPIT<<ROHAN<<<<<<<<<<<<<<<<<<<<<<<<<<\nSMP0029356UTO1609307M2801049<<<<<<<<<<<<<<<8", "kind": "mrz", "value": "P<UTOKALPIT<<ROHAN<<<<<<<<<<<<<<<<<<<<<<<<<<\nSMP0029356UTO1609307M2801049<<<<<<<<<<<<<<<8", "valid": true, "tier": "certain"}]}
```

## Tiers

Every identifier, link, statement fact, passport fact and `same_org` row carries a tier, `certain` or `likely`. The app shows a certain claim as a fact. It shows a likely claim as "may be", with its reason, for a person to confirm or reject.

A claim is `certain` only when the characters of an identifier prove it, or when the directory holds the exact code; a certain passport expiry also cites the printed date of expiry, whose four-digit year gives the century the zone leaves out. Nothing read from layout, headings, nearness or wording is certain, however strong it looks. Everything else the extractor wants to say is `likely`: a statement header's bank, a code beside an account, the issuer, the period, which passport is current, which statement is the latest, and which printed name goes with which code.

| Claim | Certain when |
| --- | --- |
| Link with `iban-bank-code` | The identifier is an IBAN that may be certain ("Identifier tiers"): its country is in the IBAN registry table, `iban-registry.json`, it has that country's length, and its check digits pass. The scorer checks all three again from `value`. Such an IBAN is held at the institution that has its bank code in its country, the bank identifier the table places, so the link goes to that code's key. The directory is not needed. For a country the table does not have, the link is likely, and its key is a document key ("No merging"). |
| Link with `ifsc-prefix` | The IFSC may be certain ("Identifier tiers"), and the directory entry that the key's card gives lists its first four letters. An IFSC has no check digit, so with the directory off the link is likely. |
| Link with `bic-institution` | The BIC may be certain ("Identifier tiers"), and the directory entry that the key's card gives lists a BIC with the same first eight characters, the institution ("Code keys"). A BIC has no check digit, so with the directory off the link is likely. |
| Link with `paired-code`, `same-account`, `inside-iban` or `statement-header` alone | Never. These reasons are read from the page's layout. |
| Statement `period`, `accounts`, `issuer` | Never. |
| Passport `holder`, `number`, `expiry` | The passport's `mrz` is a certain identifier that is a passport machine-readable zone (it starts with `P`) whose check digits all pass, and the fact is what that zone says: `number` is the zone's document number, and `holder` the zone's surname and given names (case and spacing ignored). The zone's date of expiry proves a day, a month and the last two digits of a year, never the century. So `expiry` is certain only when `expiry_at` is also the span of a date the page prints, whose text holds one four-digit year ending in the zone's two digits and, besides it, only the zone's day, or its day and month in either order, as digits. The certain `expiry` is that printed year with the zone's month and day. The span is the printed date of expiry and nothing else: not a longer text around it, not the same date in another field, and not a date on another page. The scorer reads the span's text again, as it checks an IBAN again from `value`, and compares the span with the labels' `expiry_at` ("Matching": by span, or on a page the OCR read by page and text); a certain expiry that cites any other span is a wrong certain row. The scorer reads digits only, in ASCII or full width, and years of the Gregorian calendar, so a date printed in another script's digits or in another calendar, such as a Thai Buddhist Era year, stays `likely`. Without such a printed date, an expiry may still be given as `likely`, or as `cannot-tell`. A visa's zone makes no passport fact certain. |
| Passport `current` | Never. It depends on other documents and on the reference date. |
| `same_org` row | Its reason is `directory` ("Same organisation" gives the test). |

A link with several reasons is certain when one of them passes its test. An output that calls a claim certain where a table here does not allow it breaks the format. Mentions and cards carry no tier: they are what the extractor read, and the claims above are built on them. The answers carry no tier and are always likely, because "latest" and "current" are judgements over many documents.

A claim is never certain when the identifier it rests on is only likely, except where the claim's own test proves the identifier. These are the exceptions, and the only ones. `iban-bank-code`, `ifsc-prefix` and `bic-institution` each include the identifier's own test, so a certain link's identifier always passes it, whatever tier the output gives the identifier. A passport's `holder`, `number` and `expiry` rest on the machine-readable zone, not on the printed number, whose kind has no check: a certain passport fact names the zone it rests on, that zone is itself certain, so a zone that is not the page's own fails the run as a wrong certain identifier, and the fact equals what the zone says. A certain `expiry` rests on the printed date in `expiry_at` as well, for the century the zone leaves out. No statement fact is certain.

### Identifier tiers

An identifier's tier is the tier of its kind: "this text is an IBAN", "this number is an account". The kind is certain only when the identifier's own check proves it, and the check is strong.

| Kind | Certain when |
| --- | --- |
| `iban` | Its country is in the IBAN registry table, `iban-registry.json`, it has that country's length, and its check digits pass (mod 97). The scorer checks all three again from `value`. An IBAN of a country the table does not have is likely, however its check digits come out. |
| `mrz` | It is a passport or visa zone whose check digits all pass. |
| `ifsc`, `bic` | It has the code's shape, the directory is on, and a directory entry holds its institution code exactly ("Code keys"). Neither code has a check digit. |
| `card` | Never. One string of digits in ten passes the Luhn check digit, so a pass proves little. |
| `account`, `gb_sort_code`, `in_pan`, `fr_bank_code`, `fr_branch_code`, `fr_rib_key`, `passport_number` | Never. These kinds have no check, or one only a shape or the digits beside them can pass. The word printed beside a number is wording. |

A wrong certain identifier fails the run. A wrong likely one is counted. `valid` still says whether the kind's check passes, as the labels give it.

### Cards on likely rows only

A card that no certain link and no certain `same_org` row stands on is built only from likely rows. Such a card, and any document attached to it, does not fail a run. In the app, nothing on it may be shown as confirmed: the card itself is a proposal until a certain row or a person's confirmation supports it. The scorer's table prints how many cards rest on likely rows only.

### No merging

An extractor never merges two organisations into one key. A key is one of two things, and nothing else:

- An institution code's key, spelt as "Code keys" says. It is the same in every document, every run and every extractor: two different codes are two keys, always.
- A document key. It stands for one printed name inside one document, or for one IBAN of a country that `iban-registry.json` does not have. It is spelt `name:<document id>:<name part>`, such as `name:hdfc-statement-sep:hdfcbank`. The name part is the match key of a text the page prints (`README.md`, last section): the text in Unicode compatibility form (NFKC), in lower case, with `株式会社` removed, split into words at every character that is not a letter, a mark or a number, with the words `the`, `ltd`, `limited`, `plc`, `inc`, `ag`, `sa` and `nv` dropped, and the remaining words joined with nothing between them, so `HDFC Bank Ltd` is `hdfcbank`. A printed name's key is spelt from the text of at least one mention in the document that uses the key; a mention that the document settles as a short form of the name ("Mentions, with the directory on and off" in `README.md`) may use the name's key. The key of an IBAN of a country the table does not have is spelt the same way from the IBAN's `value`, so `DZ56 0001 2000 3400 0005 6789 01` is `dz560001200034000005678901`, and it is that IBAN's code key in its document ("Code keys"). The key is never spelt from a counter or a position in the file, so it comes out the same on every run and no two documents share one.

The scorer rejects any other key: a key that is neither spelt as a code's key nor as a document key of a document in the text set, a document key used by a mention, link, issuer or `same_org` row of a document other than the one it names, and a printed name's key whose name part no mention on it spells.

The same printed name in two documents does not prove one organisation. `SMBK` in `credit-advice-smbk-ca` is a Canadian bank and `SMBK` in `payroll-release-smbk-gb` a British one, each settled by its own BIC. So the extractor writes nothing that says "this name in document A is that name in document B". Inside one document, a `same_org` row may join a name's key to a code's key, and a code's key is the same everywhere: that is the only way an output joins documents. Joining two documents' names is a judgement over many documents, like `latest_statement`. It belongs to the engine's rule "may be the same organisation", which a person confirms or rejects. So that this rule's work is known, the scorer reports, and never fails a run on, the pairs of document keys in different documents whose names are spelt the same, with how many pairs the labels say are one organisation and how many are two.

A key is a MERGE when its papers land on two labelled organisations ("Organisation keys"). Any merge fails the run. A key cannot hold two codes: the link of the second would not go to its own code's key, which breaks the format. A key cannot hold a code and a printed name either: a mention on a code's key breaks the format ("Code keys").

### Code keys

A key that stands for an institution code is spelt one way, the same in every document, every run and every extractor: the kind, the country where the kind needs one, and the code, separated by single spaces, in upper case.

| Kind | Institution code | Key |
| --- | --- | --- |
| IFSC | Its first four letters, the bank. | `IFSC HDFC` |
| BIC | Its first eight characters: the institution's four letters, its country and its two-character suffix. The three characters after them name a branch. | `BIC COBADEFF` |
| IBAN | Its country and its bank code, the bank identifier at the place `iban-registry.json` gives for the country. For a country the table does not have, the document key spelt from the IBAN ("No merging"). | `IBAN DE 37040044` |

An IFSC's code is four letters, a BIC's code six letters and then two letters or digits, and an IBAN's country two letters and its bank code as long as the table's place for the country. A key spelt otherwise is no code's key.

A BIC's institution is its first eight characters because in ISO 9362 those eight (the party prefix, the country and the party suffix) identify one business party, and the last three a branch of it. The two characters after the country belong to the party's identifier, not to a place inside it, so two parties can share the first six. Eight characters never put two institutions on one key; at worst one bank with two such codes has two keys, a split, which `same_org` rows join.

A link by an identifier's own code reason (`ifsc-prefix` for an IFSC, `bic-institution` for a BIC, `iban-bank-code` for an IBAN) goes to that identifier's code key, or the output breaks the format. `iban-registry.json` holds, for every country of the IBAN registry that SWIFT publishes as the ISO 13616 registration authority, the IBAN's length and where its bank identifier sits; its header names the source and the release. For an IBAN of a country the table does not have, the scorer reads no code: the IBAN and its link are likely, and its code key is the document key spelt from the IBAN, which the scorer treats as a code's key in that document.

A code's key stands for a code that an identifier of the same document gives. These rows, and no others, use it:

- A link by the identifier's own code reason, to that identifier's code key.
- Any other link in a document whose identifiers give the code, whatever its reasons, such as `paired-code` through the code, `inside-iban` or `same-account` from a part of an IBAN, or `statement-header`. Such a link is likely ("Tiers").
- A `same_org` row in such a document, which joins the code's key to a printed name's key or to another code's key.

A mention never uses a code's key, and neither does a statement's `issuer`: a printed name joins a code's key only by a `same_org` row. A code's key used in a document whose identifiers do not give the code, and every use this list does not name, breaks the format.

A claim's id is made from its keys ("Ids"), and a person's confirmation is kept by that id, so a key must come out the same when the extractor runs again over the same files. That is why a document key's name is spelt from the printed text.

"This name is that code's bank" and "these two keys are one organisation" are claims of their own, written as `same_org` rows. A code's key may still have a card name, such as the directory entry's name with the directory on.

### Same organisation

A `same_org` row sits in the document that shows the join. Both keys must be used in that document by a mention, a link or the statement's issuer. `reason` is one of these:

| Reason | The document or the directory shows |
| --- | --- |
| `directory` | The directory says the name is the code's institution. The row is certain only when the directory is on, one key is a printed name with mentions in the document, the other key holds a code linked to it in the document by the code's own reason, every one of those mentions' exact directory lookups (README, "Mentions, with the directory on and off": one entry holds its match key) gives exactly one entry, and the code's exact lookup gives exactly that entry. The scorer makes both lookups itself and never reads the ids the cards give. Only this reason can be certain. A name settled by wording, nearness, another mention or a code, such as a bare `HDFC` that two entries hold, keeps the row likely. |
| `spells-code` | The name begins with the code's letters, as `HDFC Bank Ltd` begins with the `HDFC` of `HDFC0SAMPL1`. |
| `statement-header` | The name heads the statement and the code is on the statement's account. |
| `printed-together` | The name and the code are on one line, or on lines next to each other. |

### Ids

Each claim can be named without its position in the file, so that a person's confirmation can be kept outside this format. A link's id is its document, its identifier's kind and value, and its key. A `same_org` row's id is its document and its two keys in sorted order. Rows with the same id are one claim. The claim holds the union of the rows' reasons, and it is certain when one of its rows is certain. Row order never changes a score. Because keys are spelt from codes and names ("Code keys"), ids stay the same from one run to the next.

### What the tiers settle

- A code-based link reason with the directory off: `iban-bank-code` is certain when the IBAN's country is in `iban-registry.json`, its length is that country's and its check digits pass, and the link goes to the IBAN's code key. `ifsc-prefix` and `bic-institution` are likely, because nothing has checked that the code is a bank's.
- A code that names no organisation printed in the document makes a key of its own, with `name` `null` when the directory is off. Its links are likely unless the table above makes them certain. A link from that code to a printed name's key is not allowed: the join is a `same_org` row.

## How the scorer decides

**Matching.** An output mention or identifier matches the label with the same span. On `text-ocr/`, a scanned page holds what the OCR read, so spans there cannot line up: on a scanned page a mention matches by page and text, and an identifier by page and value. Every other page of `text-ocr/` holds the same text as in `text/` and matches by span, exactly as on `text/`. This has a limit. On a scanned page the scorer checks an identifier's value, not where on the page it was cited, because the labels' spans count in the clean text. A citation that points at the wrong line of a scanned page is not caught there. The same extractor code is checked by span on every other page. Equal texts or values pair by position on the page (page, then start), first with first, whatever order the output lists them in.

**Mentions.** The right organisation for a labelled mention is the README's rule. With the directory off it is `resolved_by_document.org`, or none. With it on, the document still decides first, and a mention the document does not settle is the lookup's entry when the lookup is exact, and none otherwise. A mention with a key where the right answer is none is wrong. A mention with `settled_by: "directory"` in a run with the directory off is wrong. An output mention with a key that matches no labelled mention is wrong, whether it is a `not_orgs` name or any other text.

**Attachments.** An output mention with a key, exactly on a `not_orgs` span, is a WRONG ATTACHMENT unless its key maps to the organisation of a labelled mention on that page whose span contains it. The document would sit on a card because of text the labels say is no organisation. For example, `HDFC` inside `HDFC Life Insurance Company Limited` keyed to HDFC Bank is a wrong attachment, and so is a new `Bank of India` card made from the text inside `State Bank of India`. The same `HDFC` keyed to the HDFC Life card is not. A key maps to an organisation only through its mentions and links, never through its card, so a key that maps to no organisation, or to two, is always wrong here, whatever its card's name. Either way the mention itself is a wrong mention, because the span is not an organisation. A wrong attachment is listed with the document and the key, and counted. It does not fail the run by itself: which organisation a printed name is, is read from wording, so the claims built on it are likely. The rule does not depend on directory ids, so it holds the same with the directory on or off. A key on a labelled mention whose right answer is none, such as `BOI` in `household-notes`, is only a wrong mention.

**Report the whole name.** A mention's span is the whole name as the document writes it, such as `HDFC Bank Limited`, not a short form inside it, such as `HDFC`. A card that is only ever reported through a short form inside a longer name maps to no organisation, so its attachments and its links fail, even when its card names the right organisation. Only a link from an IFSC, a BIC or an IBAN can still map it, as "Organisation keys" says.

**Organisation keys.** Each key maps to the set of labelled organisations its matched mentions land on, across all documents. A key that no mention lands on, such as a bank known only by the IFSC beside an account, maps through its links from identifiers that carry their institution inside them: an IFSC, a BIC or an IBAN. It maps to the organisations the labels link those identifiers to. An account linked to the key never says who the key is, whatever the reason given. Then:

- A key whose set holds one organisation is that organisation.
- A key whose set holds two or more is a MERGE: two organisations' papers on one card. Every mention, link, issuer and `same_org` row that uses it is wrong, and the run fails.
- A labelled organisation that two or more keys land on is a SPLIT. Splits are listed and counted. They are not wrong. An organisation named in two documents always has two document keys, so it is always split, even when `same_org` rows join each of them to one code's key: a join is a claim between two keys, and both keys remain.
- A key whose set is empty is no labelled organisation. A link to it is wrong.

**Cards.** A key's entry in `organisations` states who the key is, and the scorer checks it against the organisation the key maps to. `name` may be `null`, for example for a bank known only by a code with the directory off. When given, it must be a name the labels give that organisation: its `name` or one of its `names` in `labels.json`, the text of a labelled mention of it, or, with the directory on, a name of its directory entry. Case and whitespace do not count, so `三菱 UFJ 銀行` is `三菱UFJ銀行`. Both names also go through Unicode compatibility folding (NFKC), so a full-width `＆` is `&` and full-width letters are ordinary letters. Nothing else is folded: `HDFC Ltd` is not `HDFC`, and punctuation counts. `directory`, when given, must be that organisation's directory id. A key whose card gives another name or another organisation's directory id is a WRONG CARD: the organisation's papers are on a card that says it is someone else. As with a merge, every mention, link, issuer and `same_org` row that uses the key is wrong. A wrong card fails the run through the certain rows on it. A missing directory id is left out, not wrong.

`org_directory` scores each mapped key's `directory`. With the directory on it must be the organisation's directory id, and `null` counts as left out. With it off it must be `null`.

**Identifiers.** A matched identifier is right when its `kind` and `value` equal the label's, and its `valid` too unless the label's `valid` is `null`. The label gives `null` for a kind with no check, and for the PAN `ABCPK0000F`, which a shape check accepts and a strict check rejects; there either answer is right. An output identifier that matches no label is wrong. When it sits on a `not_identifiers` entry it is reported as a decoy. Each identifier is counted under the tier the output gives it.

**Links.** A link is its identifier's kind and value and the organisation its key maps to. A link's identifier counts only when it matched a labelled identifier with the same kind and value. Rows in one document for the same link are one link: it holds the union of their reasons and `via` codes and is scored once, whatever order the rows are in. That includes rows at two keys of one organisation, such as an account held at a bank's name and at its code. The link is certain when one of its rows is certain. A link is right when the labels have that link and at least one of its reasons holds: it is among the label's reasons, or it is `iban-bank-code` on an IBAN whose kind may be certain ("Identifier tiers"): its country is in `iban-registry.json`, it has that country's length, and its check digits pass. Such an IBAN is held at the institution with its bank code, whatever reasons the label lists, so its link to its code key is right exactly when that key is the organisation the labels link the IBAN to. `paired-code` holds only when a `via` is an output identifier that matched the labelled `via` code. Every other link is wrong, with the first cause that applies: a merged, wrong-card or unmapped key, no such identifier in the labels, a labelled link with no reason that holds, a `not_links` entry, an identifier the labels mark invalid, an organisation the document does not settle, or nothing that ties them. A labelled link with no right output link is left out.

**Same organisation.** A `same_org` claim is right when both keys map to one labelled organisation. It is wrong when they map to two, or when either key maps to none, merges two organisations or has a wrong card. The labels have no `same_org` rows: they are scored against the labels' organisations. In each document, the keys that the document uses and that map to one labelled organisation must be joined by right claims, directly or through each other. Each join that is missing is left out. With the directory off, some of these joins only the directory could make, such as Commerzbank and the IBAN in `transfer-untied`, where three banks are named in one line; leaving them out is the right answer.

**Statements and passports.** `statement_period` is right when `from` and `to` equal the label's. `statement_account` is right when every account given spells the labelled account. `statement_issuer` is right when the key maps to the issuer mention's organisation. Passport holder (case and spacing ignored), number, expiry and current compare with the label. When the label's expiry is `cannot-tell`, only `cannot-tell` is right. A statement or passport on a document whose labels have none is wrong in every fact it gives.

**Answers.** A labelled answer is right when every output answer for that account or holder names the labelled document, wrong when one names another, and left out when none is given. An output answer for an account or holder the labels do not list is wrong. Every answer is likely.

**Tiers.** Each measure counts right and wrong per tier, from the tier the output gives. Left out and impossible have no tier. The table prints them with the precision of the likely rows, right divided by right and wrong. Each wrong row is listed with its tier, a certain one as `WRONG CERTAIN`. The table also prints how many cards rest on likely rows only, and the JSON lists their keys ("Cards on likely rows only").

**What OCR made impossible.** Impossible says what no extractor could have found. On `text-ocr/`, a labelled item that the output does not give, and whose text or value the page does not hold with any spacing, is counted as impossible instead of left out. A link is impossible when its identifier is, or when its only reason is `paired-code` through an impossible code. A claim the output does give is always compared with the labels, on either text set, in every measure: an extractor that repairs the OCR text and finds the value anyway scores it as right, and a wrong claim is wrong, so a wrong certain claim fails the run.

**Exit status.** 0 when no certain row is wrong and no key merges. 1 when one is wrong or one merges. 2 when the output breaks this format, including a root that is not an object, a format 1 or format 2 output, a missing tier, a certain claim the tables do not allow, a certain expiry whose `expiry_at` does not give its year and day, a code link to a key that is not the code's key, a key that is neither a code's key nor a document key, a document key used in another document or not spelt from a mention on it, a mention or an issuer on a code's key, and a code's key in a document whose identifiers do not give the code. A likely row and a left-out row never fail a run.

## How to score an extractor

1. Run the extractor four times: directory on and off, each on `text/` and on `text-ocr/`. In the off runs, do not give it `directory/banks.json`.
2. Score each output file:

   ```sh
   node sample/tools/score.mjs run-on-text.json
   ```

   It prints a table (right, wrong, left out and impossible for each measure, in total, per tier and per language), the merges, wrong cards, wrong attachments and splits, the same-name report ("No merging") with each pair the labels say is two organisations, and every wrong, impossible and left-out item. It writes the same as JSON to `run-on-text.score.json`, or to the path after `--json`.
3. The bar is no wrong certain row and no merge on all four runs. Leaving something out is allowed. The likely rows' precision says how often a "may be" is right.

`node sample/tools/reference.mjs <on|off> <text|text-ocr> <path>` writes the output a perfect extractor would give, made from the labels. It gives each organisation a document names a document key in that document, spelt from the first name the document prints for it, gives each institution code its code's key, and an IBAN of a country the table does not have its document key, and joins them with `same_org` rows. It scores full marks on `text/`, and on `text-ocr/` it loses only what OCR made impossible. With the directory off it leaves out the joins that only the directory could make. `node --test sample/tools/score.test.mjs` tests each scoring rule, and `node sample/tools/score-mutants.mjs` removes one rule at a time from `score.mjs` and shows that a test fails. A removal counts as caught only when the test run exits by itself and names a failing test; a run ended by a signal, a test file that does not load, or a failing exit that names no test is a runner error, and the runner exits 1. `node --test sample/tools/suite.test.mjs` tests those endings.

The sections below are a history. A passage marked "History" no longer holds as written; the rule it names holds.

## Changes in round 1

- A key is a non-empty string. A card's `name` may be `null`. A name or directory id that is not the organisation's makes a wrong card, which fails the run. (History: since round 7 a key is a code's key or a document key, "No merging".)
- A key no mention lands on is known only from an IFSC, BIC or IBAN linked to it, never from an account.
- A span outside its page, and a second mention or identifier on the same span, are format errors.
- A link counts only when its identifier, and for `paired-code` its `via`, matched a labelled identifier.
- Rows for the same link are scored once with the union of their reasons; row order never changes a score.
- "Values" gives `value` for every kind: a sort code is `601613`, and a machine-readable zone keeps its line feed.
- `valid` is not scored where the label's `valid` is `null`.
- Passport `number`, `mrz`, `issued`, `expiry` and `expiry_at` are checked; `issued`, `mrz` and `expiry_at` are not scored. (History: since round 7 a certain expiry's `expiry_at` is scored, "Tiers".)
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

- Output format 2. Every link, statement fact, passport fact and `same_org` row has a tier, `certain` or `likely`. The scorer refuses format 1 and any missing tier, and any certain claim that "Tiers" does not allow. (History: format 3 replaced it in round 7.)
- A key is one institution code or one printed name. Two codes on one key, or a code and a name, is a merge, and any merge fails the run. (History: since round 7 a key is a code's key or a document key, and a mention on a code's key breaks the format, "No merging" and "Code keys".)
- `same_org` rows say that two keys are one organisation. They are scored against the labels' organisations.
- A run fails when a certain row is wrong or a key merges. A wrong likely link, a wrong answer, a wrong card and a wrong attachment are counted and listed, and do not fail the run by themselves.

## Changes in round 5

- An identifier carries a tier. Its kind is certain only for an IBAN or a machine-readable zone whose check digits pass, or an IFSC or BIC the directory holds exactly. A card number, an account number and every other kind are likely. A wrong certain identifier fails the run.
- A key that stands for an institution code is spelt one way, such as `IFSC HDFC`, `BIC COBADEFF` or `IBAN DE 37040044`. A link by a code's own reason to any other key breaks the format, so no key can hold two codes. A printed name's key is spelt from the name, never from a counter. (History: since round 7 a document key's name part is the match key of a mention's text, "No merging".)
- A BIC's institution is its first eight characters, in this file, `README.md` and the scorer.
- A link by `iban-bank-code` on an IBAN whose check digits pass holds at the organisation the labels link the IBAN to, whatever reasons the label lists.
- A certain `same_org` row needs the printed name's exact directory lookup and the code's to give one entry each, the same one. The scorer makes both lookups and reads no card's directory id.
- A certain passport `number` must be the zone's document number.
- The scorer keys every pair it groups by in a form no two different pairs share.
- The table prints how many cards rest on likely rows only.

## Changes in round 6

- `iban-registry.json` gives every IBAN registry country's length and bank identifier place. An IBAN of any of them may be certain, and its key is spelt from that bank identifier. An IBAN of a country the table does not have is likely, with its link, and its key is free-form. (History: since round 7 that key is the document key spelt from the IBAN, "No merging".)
- A certain passport `holder`, `number` or `expiry` needs a certain zone, and must be what that zone says. (History: since round 7 a certain `expiry` also cites the printed date of expiry, "Tiers".)

## Changes in round 7

- Output format 3. The scorer refuses format 2 with a sentence that names the two changes below.
- A printed name's key is for one document. A key is an institution code's key or a document key, `name:<document id>:<name>`, and nothing else. A document key used in another document breaks the format. Format 2 let one key stand for a printed name in every document that printed it, so one short name on a Canadian receipt and on a British one, two banks, landed on one key. Only the engine's rule "may be the same organisation" joins names across documents. The scorer reports the pairs of document keys whose names are spelt the same, and how many the labels say are one organisation and how many two.
- A zone does not prove a century. A certain `expiry` cites in `expiry_at` a printed date with a four-digit year that ends in the zone's two digits, and the zone's day. Format 2 let an extractor write the zone's `98` as `2098` at certain, for a passport that expired in 1998.
- `credit-advice-smbk-ca`, `payroll-release-smbk-gb` and `passport-meera-1988` join the sample. They show the two faults.
- A claim the output gives is always compared with the labels. "Impossible" counts only a labelled item the output does not give, so a wrong certain claim fails the run on `text-ocr/` as on `text/`, in every measure.
- A certain expiry's `expiry_at` is the labelled printed date of expiry, and a certain expiry that cites any other span is a wrong certain row.
- A code's key is used only in a document whose identifiers give the code, and only by links and `same_org` rows, never by a mention or an issuer. An IFSC's key holds four letters and a BIC's six letters and two letters or digits.
- A document key's name part is the match key of a text the page prints, and a printed name's key is spelt from a mention on it.
- An IBAN of a country the table does not have goes to the document key spelt from the IBAN, which no mention uses.
