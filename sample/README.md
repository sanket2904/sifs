# Labelled sample documents

Forty made-up documents, the right answers for each, and a bank directory built from Wikidata. An extractor that finds banks, accounts and passports in text is scored against these labels before anything is built on its output. The hard part is not finding the string `HDFC`. It is saying which account belongs to which bank, and whose passport is the current one, without guessing.

Every document says on its face that it is a made-up sample. People, accounts, addresses and amounts are invented. Banks are real, named in plain text only. Passports use the ICAO specimen state Utopia (`UTO`). Card numbers come from published test ranges.

Nothing here depends on one language's test runner. The tools need only Node.js (version 22 or later) and no packages.

## Check the labels

```sh
node sample/tools/check.mjs
```

The check regenerates every document in memory and compares it byte for byte with `files/` and `files/SHA256SUMS`. Then it proves each label against the text the document was generated from:

- Every span reads back the labelled text.
- Every identifier marked valid passes its check, and every one marked invalid fails it.
- Every organisation id is in the directory, and its match against the directory is what `by_name` says.
- Every link's identifier is in the same document, and each reason holds: the IBAN's bank code, the BIC, or the IFSC prefix belongs to that bank. A paired code sits on the same line or the next, and a statement header names exactly one organisation. A link lists every reason that holds, and every valid IBAN, BIC or IFSC whose code names a directory entry is linked to it.
- Every statement period and passport expiry is read from its span, and `current` and the answers follow from the dates.
- Every directory name and every identifier-shaped number on a page is covered by a label, so nothing findable is left unlabelled.

On macOS, `node sample/tools/check.mjs --pdfkit` also reads every PDF through PDFKit and requires its text to equal the spec text exactly. Image pages must read as empty.

`node sample/tools/mutants.mjs` breaks one label or file at a time in a copy of this folder and shows that the check reports each break.

## Change a document

1. Edit `spec.json`.
2. Run `node sample/tools/generate.mjs`. It rewrites `files/`, `files/SHA256SUMS`, and each file's modified time.
3. Edit `labels.json`. For a new span, give `page` and `text` (and `nth` for a later occurrence) and run `node sample/tools/locate.mjs` to fill in `start` and `end`.
4. Run the check.

Git does not keep modified times. Run `generate.mjs` after a checkout if a test needs the file dates, which deliberately disagree with the statement periods.

## Files

| Path | What it is |
| --- | --- |
| `spec.json` | One entry per document: `id`, `file`, `format` (`pdf`, `txt` or `html`), `lang`, `title`, `created` (the file date), `about`, and `pages`. A page is a list of `lines`, with `"image": true` for a scanned page. |
| `files/` | The generated documents and `SHA256SUMS` (readable by `shasum -a 256 -c`). |
| `labels.json` | The right answers. |
| `directory/banks.json` | The bank directory. |
| `directory/build.mjs` | Builds `banks.json` from Wikidata and `codes.json`. |
| `directory/codes.json` | Bank codes and names that do not come from Wikidata, with their sources. |
| `tools/` | `generate.mjs`, `check.mjs`, `locate.mjs`, `mutants.mjs`, and `pdfkit-text.swift`. |

A text PDF shows a line in Helvetica when all its characters are Latin-1, so the line is visible. Other lines use a font with no glyphs and a ToUnicode map, as `src-tauri/src/extract/readers/macos/pdf/sample.rs` does. They read correctly but show as dots. A PDF text page has no blank lines and no runs of spaces, because PDFKit drops both. A scanned page is a 1-bit picture of upper-case text in a 5 by 7 pixel font, with no text layer.

## Label schema

A span is `page` (from 1), `start`, `end` and `text`. `start` and `end` count Unicode scalar values, end exclusive, in the page text: the page's lines joined with a line feed. For a txt file the page text is the file without its final line feed. For HTML it is the text of the `<p>` elements, one per line.

Each entry in `documents` has an `id` from `spec.json`, an optional `note`, and either `"nothing_to_find": true` or these lists:

| Field | Entries |
| --- | --- |
| `orgs` | An organisation mentioned: a span, `id`, `org` (a directory id, or `null`), and `by_name`. `exact` means the text's match key belongs to that entry only. `clash` means two or more entries share the key: `org` is `null` and `candidates` lists them. `none` means the text is not a directory name. |
| `not_orgs` | A directory name in the text that is not a mention of that entry: a span, `org`, and `why`. |
| `identifiers` | A span, `id`, `kind`, `value` (the text with spaces removed), and `valid` (`true` or `false`, or `null` for a kind with no check). |
| `not_identifiers` | Text shaped like an identifier that is not one: a span, `looks_like` (a kind), and `why`. |
| `links` | Identifier held at organisation: `identifier`, `org`, `mention` (the `orgs` id that names the bank, or `null`), `reasons`, and `via` for `paired-code`. |
| `not_links` | A pair that must not be linked: `identifier`, `org`, and `why`. |
| `statement` | A span holding the period, `accounts` (identifier ids of the statement's own account), `from` and `to`. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids), `issued`, `expiry` (a date, or `cannot-tell` with `expiry_candidates`), `expiry_at` (the span of the printed expiry), `current`, and `why` when needed. `null` for a document that looks like a passport and is not one. |

`answers` lists the latest statement of every account, by the statement's own period, and the current passport of every holder. `reference_date` (2026-11-22) decides what is current.

Score links by identifier kind, value, and organisation. The same account in two places in one document is one link.

Identifier kinds are `iban`, `bic`, `card` (Luhn), `ifsc` (shape), `in_pan` (shape, Indian PAN), `mrz` (passport TD3 or visa MRV-A, all check digits), `account`, and `passport_number`. The last two have no check.

### Link reasons

| Reason | Holds when |
| --- | --- |
| `iban-bank-code` | The IBAN's bank code is one of the bank's `iban_bank_code` entries. |
| `bic-institution` | The BIC's first six characters match one of the bank's BICs. |
| `ifsc-prefix` | The IFSC's first four letters are the bank's prefix. |
| `paired-code` | An account or IBAN sits on the same line as, or the line next to, an IFSC or BIC that is itself linked to the bank. `via` names that code. |
| `statement-header` | The identifier is the statement's account, and the document names exactly one organisation. |

A link lists every reason that holds. An extractor that gives any one of them is right.

### Why codes

| Field | Codes |
| --- | --- |
| `not_links` | `no-tie`: nothing in the document ties them. `code-names-other-bank`: a code ties the identifier to a different bank. `invalid-identifier`: the identifier fails its check, so it is not linked anywhere. `name-not-this-bank`: the bank's name appears only inside another name, so the bank is not mentioned. |
| `not_orgs` | `inside-other-name`: the name sits inside a longer mention of another organisation. `word-not-name`: the text is an ordinary word, such as the field label `BIC`, which is also the name of Société BIC. |
| `not_identifiers` | `phone-number`, `invoice-number`, `reference-number`, `policy-number`, `consumer-number`, `tax-account-number`, `visa-number`. |

## Hard cases

| Case | Documents |
| --- | --- |
| IBAN whose bank code is the bank's | `commerzbank-kontoauszug`, `deutsche-bank-brief`, `natwest-statement`, `abn-amro-statement` |
| Indian account with its bank's IFSC | `hdfc-statement-sep`, `sbi-passbook-hindi`, `scan-bob-letter`, `scan-pnb-deposit`, `axis-statement-scanned-annex` |
| IFSC prefix that is not the bank's name (`UTIB` for Axis Bank) | `axis-statement-scanned-annex` |
| Account tied only by the code beside it; the bank is never named | `utility-bill`, `pan-form-hindi`, `rib-bnp-fr` |
| Statement header with one bank and one account, no code | `kotak-statement`, `mizuho-statement-ja` |
| Two banks and two accounts, each tied by its IFSC | `neft-receipt-tied` |
| Three banks and two accounts; one account tied to none | `transfer-untied` |
| A bank and no account | `hdfc-branch-letter`, `hdfc-letter-hindi` |
| An account and no bank | `rent-receipt` |
| Two banks whose names share words | `school-fee-banks`, `deutsche-bank-brief`, `mufg-remittance-ja`, `sbi-passbook-hindi` |
| Not a bank, with a bank's name inside | `hdfc-life-receipt`, `icici-lombard-notice`, `hdfc-ltd-loan-2019` |
| Alias held by two directory entries: no organisation | `household-notes` (`BOI`, `SBI`, `HDFC`), `neft-receipt-tied` (`HDFC`), `hdfc-ltd-loan-2019` (`HDFC Ltd`) |
| A field label that is a directory name | `natwest-statement`, `commerzbank-kontoauszug`, `rib-bnp-fr` |
| Spellings of HDFC Bank | `hdfc-statement-sep` (`HDFC Bank Ltd`), `hdfc-branch-letter` (`HDFC Bank Limited`), `email-fee-payment` (`hdfcbank`, `HDFC BANK`), `hdfc-letter-hindi` (`एचडीएफसी बैंक`) |
| IBAN with a wrong check digit | `rechnung-falsche-iban`, `invoice-lookalike-numbers` |
| Card number that fails Luhn | `invoice-lookalike-numbers` |
| Numbers shaped like accounts that are not | `rent-receipt`, `rechnung-falsche-iban`, `invoice-lookalike-numbers`, `neft-receipt-tied`, `email-fee-payment`, `hdfc-life-receipt`, `icici-lombard-notice`, `utility-bill` |
| PAN, malformed PAN, and a TAN shaped like one | `pan-form-hindi` |
| Passports of two people in one family | `passport-asha-current`, `passport-rohan` |
| Expired passport and its renewal | `passport-asha-expired` (scan), `passport-asha-current` |
| Visa page with dates and a passport number | `visa-asha` |
| Machine-readable zone with a wrong check digit: expiry cannot be told | `passport-bad-mrz` |
| One account, three months, file dates in reverse order | `hdfc-statement-jul`, `hdfc-statement-aug`, `hdfc-statement-sep` |
| Nothing to find | `recipe-lemon-rice`, `poem`, `recipe-hindi`, `essay-ja` |
| Image-only pages | `scan-bob-letter`, `scan-pnb-deposit`, `passport-asha-expired`, page 2 of `axis-statement-scanned-annex` |

## Bank directory

`directory/banks.json` version 1 was built on 2026-11-22 by `node sample/directory/build.mjs`. It holds 397 entries: Indian banks other than branches, every Wikidata item with a BIC in Great Britain, Ireland, the Netherlands, Japan, France, Spain, and Switzerland, and named seeds such as look-alikes. Each entry has `id` (the Wikidata id), `name`, `kind` (`bank` when Wikidata classes it under bank, otherwise `other`), `country`, `dissolved`, `names`, and `codes`. Each name has its `text`, its `langs`, `type` (`label` or `alias`), `source`, and its match `key`. `kind` comes from Wikidata's classes and has errors. For example, Mizuho Bank is `other`.

Sources and licences:

- Names, countries and BICs come from Wikidata, under CC0 1.0. The selection is in `build.mjs`, and the request sends only a generic client name.
- IFSC prefixes come from `src/banknames.json` of github.com/razorpay/ifsc, under the MIT licence. That file compiles the Reserve Bank of India's IFSC list.
- German bank codes come from the Deutsche Bundesbank's Bankleitzahlendatei. The download states no licence, and only single codes, which are facts, are copied.
- For Great Britain, Ireland and the Netherlands, the IBAN bank code is the first four letters of the bank's BIC. `build.mjs` derives those codes.
- Wikidata puts some banks' BICs on their customers as well, for example ABN AMRO's `ABNANL2A` on an energy company. A BIC whose first six characters are also held by an entry classed as a bank stays only on the bank entries. `dropped_codes` lists the 15 BICs removed this way.
- One name is curated: `HDFC` for HDFC Bank. Wikidata gives that alias only to Housing Development Finance Corporation, which merged into HDFC Bank in 2023.

The match key is the name in NFKC, in lower case, with `株式会社` removed, split on every character that is not a letter, mark or digit, with the words `the`, `ltd`, `limited`, `plc`, `inc`, `ag`, `sa` and `nv` dropped, and the rest joined. `HDFC Bank Ltd`, `HDFC BANK` and `hdfcbank` share the key `hdfcbank`. A key held by two or more entries is a clash. `clashes` lists each one, and each name in a clash carries `clash_with`. A clash means no link: an extractor leaves such a name without an organisation. Labels record the directory version they were checked against. When you commit a rebuilt directory, raise `VERSION` in `build.mjs` and run the check, because Wikidata changes daily.
