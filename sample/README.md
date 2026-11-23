# Labelled sample documents

Forty-six made-up documents, the right answers for each, and a bank directory built from Wikidata. An extractor that finds organisations, accounts and passports in text is scored against these labels before anything is built on its output. The hard part is not finding the string `HDFC`. It is saying which account belongs to which organisation, and whose passport is the current one, without guessing.

An organisation exists because a document shows it: a statement header, a code inside an account number, a name in the text. The directory is a hint that adds a public id and other spellings. It is never the source of truth, so the labels give a right answer with the directory switched on and with it switched off.

Every document says on its face that it is a made-up sample. People, addresses and amounts are invented. Real banks are named in plain text only; the other organisations, such as Sample Traders and Sample Nagar Sahakari Bank, are invented. Passports use the ICAO specimen state Utopia (`UTO`).

Which numbers could belong to someone. These are public test values: the IBANs `GB29 NWBK 6016 1331 9268 19` (with sort code `60-16-13` and account `31926819`), `NL91 ABNA 0417 1643 00`, `DE89 3704 0044 0532 0130 00` and `FR14 2004 1010 0505 0001 3M02 606` (with bank code `20041`, branch code `01005`, account `0500013M026` and key `06`) are the examples of the SWIFT IBAN Registry, as compiled in `registry.txt` of github.com/globalcitizen/php-iban (LGPL 3.0; only single values are copied); the cards `4242 4242 4242 4242` and `5555 5555 5555 4444` are Stripe's Visa and Mastercard test cards (docs.stripe.com/testing, "Simulate a payment by card brand"); `L898902C3` is the specimen passport number of ICAO Doc 9303; and the phone numbers `020 7946 0123` and `+44 7700 900xxx` are in Ofcom's drama ranges and `+49 30 23125 471` in the Bundesnetzagentur's drama numbers for Berlin (India has no documented range, so no Indian phone number appears). These fail their checksum: `DE88…` and `GB28…` (registry examples with the check digits changed), the card `4111 1111 1111 1112` (Luhn), and the machine-readable zone of `passport-bad-mrz`. These break a rule the issuer publishes, so they cannot be assigned: the PAN `ABCPK0000F`, because the Income Tax Department issues serials 0001 to 9999 ("How PAN is formed", incometaxindia.gov.in), which is why its `valid` is `null` (a shape check accepts it, a strict check rejects it); the misspelt PAN `ABCDK0000F` and the TAN-field value `PNESX2345A`, whose fourth letter is not one of the holder statuses that page lists, and which has five letters, not the four of the Department's TAN form `AAAA99999A`; and the Mizuho account `12345678`, which has eight digits where the Zengin transfer format holds a seven-digit account number (field 口座番号 N(7) in MUFG BizSTATION's 総合振込 record format). These are invented digit strings under a real institution's name: the accounts `1234567890` (Kotak Mahindra Bank), `50100417263958` and `57500000123456` (HDFC Bank), `002101234567` (ICICI Bank), `30123456789` (SBI), `12340100005678` (Bank of Baroda), `0123000100456789` (PNB), `917010012345678` (Axis Bank), `612345678` (HDFC Ltd) and `0123456` (MUFG Bank), and the policy numbers `21345678` (HDFC Life) and `3001/123456789/00/000` (ICICI Lombard). An Indian account number has no national format and no check digit, and no bank publishes a reserved range, so a number under an Indian bank's name cannot be shown to be unassigned. The IFSC codes beside these accounts use the invented branch part `SAMPL…`, and each page says it is a sample. `0123456` has the valid Japanese length and could carry an eighth digit like the Mizuho account. The BICs `SMBKCAT0XXX` and `SMBKGB20XXX` of two invented banks have a location code whose second character is `0`, which ISO 9362 gives to test BICs, not to BICs used on the live network. All other numbers stand under invented names or none.

Nothing here depends on one language's test runner. The tools need only Node.js (version 22 or later) and no packages.

## Check the labels

```sh
node sample/tools/check.mjs
```

The check regenerates every document in memory and compares it byte for byte with `files/` and `files/SHA256SUMS`. Then it proves each label against the text the document was generated from:

- Every span reads back the labelled text, and an organisation or identifier label holds a whole token, never part of one.
- Every organisation and identifier that `spec.json` lists under `planted` is labelled with exactly its span and as the kind `spec.json` planted it, and every label is a planted token. Every organisation-shaped word (`Bank`, `Ltd`, `GmbH`, `Office`, `銀行`, `बैंक` and others) and every identifier-shaped number on a page lies inside a planted token or an `ignore` entry. `node sample/tools/planted.mjs` prints this proof per document.
- Every mention's `directory` field is what the directory gives for its spelling, and every `resolved_by_document` holds by the evidence it cites. A mention is `null` only when the document gives no evidence for it: not its own full name, not a short form of another mention, not a code that spells it. No not_org overlaps a mention of the same entry.
- Every link's identifier is in the same document, and each reason holds. A link lists every reason that holds, and every valid IBAN, BIC or IFSC whose code names a directory entry is linked to it.
- Every held identifier has a link or a not_link to every organisation the document names or ties it to, and no not_link pair has a reason that would make it a link.
- Every statement period is the statement's own period field. Every passport's `issued` is its date of issue field and its `expiry_at` is its date of expiry field, on the label's line or as the whole line below a label that stands alone. Its `expiry` is the date the zone gives, whose century only the printed date gives, and `current` and the answers follow from the dates.
- Every directory name on a page is covered, for each entry that holds the name, by a mention of that entry or a not_org of it. Every identifier-shaped number lies whole inside one label.

On macOS, `node sample/tools/check.mjs --pdfkit` also reads every PDF through PDFKit and requires its text to equal the spec text exactly. Image pages must read as empty.

`node sample/tools/mutants.mjs` breaks one label or file at a time in a copy of this folder and shows that the check reports each break.

## Change a document

1. Edit `spec.json`, including the document's `planted` list.
2. Run `node sample/tools/generate.mjs`. It rewrites `files/`, `files/SHA256SUMS`, and each file's modified time.
3. Edit `labels.json`. For a new span, give `page` and `text` (and `nth` for a later occurrence) and run `node sample/tools/locate.mjs` to fill in `start` and `end`.
4. Run the check.

Git does not keep modified times. Run `generate.mjs` after a checkout if a test needs the file dates, which deliberately disagree with the statement periods.

## Files

| Path | What it is |
| --- | --- |
| `spec.json` | One entry per document: `id`, `file`, `format` (`pdf`, `txt` or `html`), `lang`, `title`, `created` (the file date), `about`, `planted`, and `pages`. A page is a list of `lines`, with `"image": true` for a scanned page. `planted` lists the `orgs` and `identifiers` the author put in the text, and `ignore` entries (`text`, `why`) for words or numbers that look like one and are out of scope. Each planted identifier is `text` with the `kind` it was planted as, or, for a decoy, the kind it `looks_like` and why it is `not` one; its label must agree. |
| `files/` | The generated documents and `SHA256SUMS` (readable by `shasum -a 256 -c`). |
| `labels.json` | The right answers, format version 2. |
| `directory/banks.json` | The bank directory. |
| `directory/build.mjs` | Builds `banks.json` from Wikidata and `codes.json`. |
| `directory/codes.json` | Bank codes and names that do not come from Wikidata, with their sources. |
| `tools/` | `generate.mjs`, `check.mjs`, `planted.mjs`, `locate.mjs`, `mutants.mjs`, and `pdfkit-text.swift`. |
| `tools/fixtures/` | `format-2-shared-name-key.json`, a format 2 output for `credit-advice-smbk-ca` and `payroll-release-smbk-gb`, two receipts that print one short name for two banks, with one key for that name in both, as format 2 allowed. `score.test.mjs` shows that the scorer refuses it. |

A text PDF shows a line in Helvetica when all its characters are Latin-1, so the line is visible. Other lines use a font with no glyphs and a ToUnicode map, as `src-tauri/src/extract/readers/macos/pdf/sample.rs` does. They read correctly but show as dots. A PDF text page has no blank lines and no runs of spaces, because PDFKit drops both. A scanned page is a 1-bit picture of upper-case text in a 5 by 7 pixel font, with no text layer.

## Label schema

A span is `page` (from 1), `start`, `end` and `text`. `start` and `end` count Unicode scalar values, end exclusive, in the page text: the page's lines joined with a line feed. For a txt file the page text is the file without its final line feed. For HTML it is the text of the `<p>` elements, one per line.

`organisations` maps a local id to an organisation: `name`, `directory` (its directory id, or `null` when no directory entry holds it), `names` (spellings that name it by themselves; two spellings with the same match key are one name), and, for an invented bank that no directory holds, `bic` (the BICs the sample gives it, so that its `bic-institution` links can be checked). The local id is the labels' identity for the organisation, in every mode and across documents, so the labels know `hdfc-bank` is one organisation whatever a document calls it. It is not an extractor's key: an extractor keys each document's printed name on its own (`FORMAT.md`, "No merging"), and only the engine's rule "may be the same organisation" puts two documents' names on one card.

Each entry in `documents` has an `id` from `spec.json`, an optional `note`, and either `"nothing_to_find": true` or these lists:

| Field | Entries |
| --- | --- |
| `orgs` | An organisation mentioned: the full span of its name, `id`, `resolved_by_document`, and `directory`. |
| `not_orgs` | A directory name in the text that is not a mention of that entry: a span, `entry` (a directory id), and `why`. A name that sits inside a longer mention has one not_org for each entry holding it, other than the mention's own. |
| `identifiers` | A span, `id`, `kind`, `value` (the text with its spaces removed; a sort code also has its hyphens removed, so `60-16-13` is `601613`; a machine-readable zone keeps the line feed between its two lines), `valid` (`true` or `false`; `null` for a kind with no check, and for a PAN with serial `0000`, which has the shape but is never issued), and `same_account_as` for a domestic account number whose IBAN is in the same document. |
| `not_identifiers` | Text shaped like an identifier that is not one: a span, `looks_like` (a kind), and `why`. |
| `links` | Identifier held at organisation: `identifier`, `org` (a local id), `mention` (the `orgs` id that names it, or `null`), `reasons`, and `via` for `paired-code`. |
| `not_links` | A pair that must not be linked: `identifier`, `org` (a local id), and `why`. |
| `statement` | A span holding the period field's value, `accounts` (identifier ids that spell the statement's own account), `issuer` (the `orgs` id of the header), `from` and `to`. |
| `passport` | `holder` (`surname`, `given`), `number` and `mrz` (identifier ids), `issued`, `expiry` (a date, or `cannot-tell` with `expiry_candidates`), `expiry_at` (the span of the printed expiry), `current`, and `why` when needed. `null` for a document that looks like a passport and is not one. |

### Mentions, with the directory on and off

`resolved_by_document` is what the document itself settles: `{"org": <local id>, "evidence": [...]}`, or `null` when nothing in the document settles it. Each evidence entry is one of these:

| Evidence | Holds when |
| --- | --- |
| `{"by": "name"}` | The mention's text is one of the organisation's `names`. |
| `{"by": "mention", "mention": <id>}` | The text is a short form (a prefix of the match key, or the initials) of another mention of the organisation that is settled by name or code, and of no mention of another organisation. `HDFC Ltd` is the initials of `Housing Development Finance Corporation Limited`. |
| `{"by": "code", "identifier": <id>}` | The text spells the first four letters of an IFSC or BIC in the document that is linked to the organisation. `HDFC` beside `HDFC0SAMPL1` is HDFC Bank. |

`directory` is what a lookup of the spelling alone gives: `{"lookup": "exact", "id": …}` when one entry holds its match key, `{"lookup": "clash", "candidates": […]}` when two or more do, and `{"lookup": "none"}` when none does.

The right answer for a mention:

- With the directory off, the mention is `resolved_by_document.org`, or no organisation when that is `null`.
- With the directory on, the document still decides first. A mention the document settles is that organisation, with its `directory` id when it has one, even when the lookup is a clash (`HDFC` in `neft-receipt-tied`) or finds nothing (`Deutschen Bundesbank`). A mention the document does not settle is the lookup's entry when the lookup is exact, and no organisation otherwise. The three short names in `household-notes` are no organisation either way.

Links and not_links use local ids, so the same links are right in both modes. The reasons say what ties each one.

`answers` lists the latest statement of every account, by the statement's own period, and the current passport of every holder. An account's `account` is its IBAN when a statement gives one, and `spellings` lists every way its statements write it; an extractor that finds any spelling has found the account. `reference_date` (2026-11-22) decides what is current.

Score links by identifier kind, value, and organisation. The same account in two places in one document is one link.

Identifier kinds are `iban`, `bic`, `card` (Luhn), `ifsc` (shape), `in_pan` (shape and holder status, Indian PAN; serial `0000` gives `null`), `mrz` (passport TD3 or visa MRV-A, all check digits), `gb_sort_code` (shape), `fr_bank_code` and `fr_branch_code` (shape), `fr_rib_key` (recomputed from the bank code, branch code and account on its line), `account`, and `passport_number`. The last two have no check.

### Link reasons

| Reason | Holds when | Highest tier in an output |
| --- | --- | --- |
| `iban-bank-code` | The IBAN's bank code is one of the bank's `iban_bank_code` entries. | `certain` when the IBAN's country is in `iban-registry.json`, its length is that country's and its check digits pass, with the directory on or off; `likely` for a country the table does not have. The scorer also holds it for any such IBAN at the organisation the labels link the IBAN to, whatever reasons the label lists. |
| `bic-institution` | The BIC's first eight characters, its institution (`FORMAT.md`, "Code keys"), match one of the bank's BICs: its directory entry's, or for an invented bank the `bic` its label gives. | `certain` only with the directory on, when the card's directory entry lists the BIC's first eight characters. Otherwise `likely`. |
| `ifsc-prefix` | The IFSC's first four letters are the bank's prefix. | `certain` only with the directory on, when the card's directory entry lists the prefix. Otherwise `likely`. |
| `paired-code` | An account or IBAN sits on the same line as an IFSC or BIC that is itself linked to the bank, or on the next or previous line when neither line holds a code or an account of its own. `via` names that code. | `likely` |
| `inside-iban` | A sort code, French bank code or French branch code is that part of an IBAN in the document that is linked to the bank. | `likely` |
| `same-account` | The account is the account part of an IBAN in the document (`same_account_as`) that is linked to the bank. | `likely` |
| `statement-header` | The identifier is the statement's account, and the organisation is the statement's issuer, the first organisation the document names. Merchants in the transactions do not count. | `likely` |

A link lists every reason that holds. An extractor that gives any one of them is right. The labels have no tiers: the third column is what an extractor's output may claim (`FORMAT.md`, "Tiers").

### Why codes

| Field | Codes |
| --- | --- |
| `not_links` | `no-tie`: nothing in the document ties them. `code-names-other-bank`: a code ties the identifier to a different organisation. `held-at-other`: the identifier is held at a different organisation, tied by the statement header and no code. `invalid-identifier`: the identifier fails its check, so it is not linked anywhere. `name-not-this-bank`: the bank's name appears only inside another name, so the bank is not mentioned. |
| `not_orgs` | `inside-other-name`: the name sits inside a longer mention of another organisation. `word-not-name`: the text is an ordinary word, such as the field label `BIC`, which is also the name of Société BIC. |
| `not_identifiers` | `phone-number`, `invoice-number`, `reference-number`, `policy-number`, `consumer-number`, `tax-account-number`, `visa-number`. |

## Hard cases

| Case | Documents |
| --- | --- |
| IBAN whose bank code is the bank's | `commerzbank-kontoauszug`, `natwest-statement`, `abn-amro-statement`, `transfer-untied` |
| Domestic codes that are parts of the IBAN (sort code; code banque, code guichet, clé RIB) and the domestic account spelling of the IBAN's account | `natwest-statement`, `rib-fr` |
| Indian account with its bank's IFSC | `hdfc-statement-sep`, `sbi-passbook-hindi`, `scan-bob-letter`, `scan-pnb-deposit`, `axis-statement-scanned-annex` |
| IFSC prefix that is not the bank's name (`UTIB` for Axis Bank) | `axis-statement-scanned-annex` |
| Account tied only by the code beside it; the bank is never named | `utility-bill`, `pan-form-hindi`, `transfer-rows` |
| IBAN tied through the BIC beside it, because the directory has no French bank codes | `rib-fr` |
| Statement header with one bank and one account, no code | `kotak-statement`, `mizuho-statement-ja`, `sahakari-statement`, `credit-union-statement` |
| An organisation that holds an account and is in no directory | `sahakari-statement` (Sample Nagar Sahakari Bank), `credit-union-statement` (Sampleton Credit Union) |
| Organisations in no directory that issue a document or appear in one | `rechnung-falsche-iban`, `invoice-lookalike-numbers`, `neft-receipt-tied`, `transfer-rows`, `natwest-statement`, `hdfc-card-statement`, `school-fee-banks`, `pan-form-hindi`, `utility-bill`, `passport-asha-current`, `passport-rohan`, `passport-meera-1988`, `credit-advice-smbk-ca`, `payroll-release-smbk-gb` |
| Statement header plus merchants in the transactions | `natwest-statement`, `hdfc-card-statement` |
| Two banks and two accounts, each tied by its IFSC on the same line | `neft-receipt-tied` |
| Two rows where the first row's account is nearer the second row's code, so matching the nearest code fails: only the row decides | `transfer-rows` |
| Three banks and two accounts; one account tied to none | `transfer-untied` |
| A bank and no account | `hdfc-branch-letter`, `hdfc-letter-hindi`, `deutsche-bank-brief` |
| An account and no bank | `rent-receipt` |
| Two banks whose names share words | `school-fee-banks`, `deutsche-bank-brief`, `mufg-remittance-ja`, `sbi-passbook-hindi` |
| Inflected name that is no directory spelling (`der Deutschen Bundesbank`) | `deutsche-bank-brief` |
| Not a bank, with a bank's name inside | `hdfc-life-receipt`, `icici-lombard-notice`, `hdfc-ltd-loan-2019` |
| Alias held by two directory entries, settled by the document | `neft-receipt-tied` (`HDFC`, by the HDFC Bank name and IFSC on the debit row), `hdfc-ltd-loan-2019` (`HDFC Ltd`, by the full name it abbreviates) |
| Alias held by two directory entries, settled by nothing | `household-notes` (`BOI`, `SBI`, `HDFC`) |
| A field label that is a directory name | `natwest-statement`, `commerzbank-kontoauszug`, `rib-fr` |
| Spellings of HDFC Bank | `hdfc-statement-sep` (`HDFC Bank Ltd`), `hdfc-branch-letter` (`HDFC Bank Limited`), `email-fee-payment` (`hdfcbank`, `HDFC BANK`), `hdfc-letter-hindi` (`एचडीएफसी बैंक`) |
| IBAN with a wrong check digit | `rechnung-falsche-iban`, `invoice-lookalike-numbers` |
| Card number that fails Luhn | `invoice-lookalike-numbers` |
| Numbers shaped like accounts that are not | `rent-receipt`, `rechnung-falsche-iban`, `invoice-lookalike-numbers`, `neft-receipt-tied`, `transfer-untied`, `email-fee-payment`, `hdfc-life-receipt`, `icici-lombard-notice`, `utility-bill` |
| A statement that names a later period in a note | `hdfc-statement-sep` |
| PAN, malformed PAN, and a value in the TAN field shaped like a PAN | `pan-form-hindi` |
| Passports of three people in one family | `passport-asha-current`, `passport-rohan`, `passport-meera-1988` |
| Expired passport and its renewal | `passport-asha-expired` (scan), `passport-asha-current` |
| Visa page with dates and a passport number | `visa-asha` |
| Machine-readable zone with a wrong check digit: expiry cannot be told | `passport-bad-mrz` |
| Zone year `98` with the full date printed on the line below its label: only the page gives the century | `passport-meera-1988` |
| One short name printed for two banks in two countries, each settled by its own BIC: the same name in two documents is two organisations | `credit-advice-smbk-ca`, `payroll-release-smbk-gb` |
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

The match key is the name in NFKC, in lower case, with `株式会社` replaced by a word break, not deleted in place (so `桜銀行株式会社Ltd` is `桜銀行`, not `桜銀行ltd`), split on every character that is not a letter, mark or digit, with the words `the`, `ltd`, `limited`, `plc`, `inc`, `ag`, `sa` and `nv` dropped, and the rest joined. `HDFC Bank Ltd`, `HDFC BANK` and `hdfcbank` share the key `hdfcbank`. A key held by two or more entries is a clash. `clashes` lists each one, and each name in a clash carries `clash_with`. With the directory alone, a clash means no organisation; a document can still settle the mention. Labels record the directory version they were checked against. When you commit a rebuilt directory, raise `VERSION` in `build.mjs` and run the check, because Wikidata changes daily.
