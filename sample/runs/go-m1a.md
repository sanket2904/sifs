# Go M1a extractor

The M1a extractor, round 3. It is rules only, with no model. It reads the sample's page text and writes the run file that `sample/FORMAT.md` describes. The code lives in `engine/`: the package `engine/extract` and the command `engine/cmd/m1a-extract`. The module imports the standard library only.

## Reproduce

```sh
sample/tools/go-m1a-runs.sh
```

The script builds the command and writes the four runs under `sample/runs/`. Each run gets the scorer's `.score.json` beside it, and the script prints each table. It exits non-zero when any run fails the bar. A run with the directory off is started without `-directory`, so the program never opens `banks.json`. Two runs in a row give byte-identical files.

## Round 3: output format 2

The runs are now in format 2 (`sample/FORMAT.md`). The extractor recognises the same text as in round 2. What changed is what it claims:

- Every identifier, link, statement fact, passport fact and `same_org` row has a tier. The tier comes from the proof the extractor recorded when it drew the conclusion, through one table in `engine/extract/tiers.go` that mirrors FORMAT.md's tables.
- Keys are spelt from text: `name:<match key>` for a printed name, `IFSC HDFC`, `BIC COBADEFF` or `IBAN DE 37040044` for a code. No key is a counter, and no two keys are merged. The joins the extractor used to make by union are now `same_org` rows.
- The IBAN length and bank code place come from `sample/iban-registry.json`, the table the scorer reads.

| Run | certain r/w | likely r/w | left out | same_org r/w/l | merges | round 2 measures r/w/l |
| --- | --- | --- | --- | --- | --- | --- |
| on, text | 73/0 | 134/0 | 8 | 20/0/0 | 0 | 248/0/5 |
| off, text | 23/0 | 171/0 | 16 | 7/0/11 | 0 | 248/0/5 |
| on, text-ocr | 66/0 | 136/0 | 8 | 19/0/0 | 0 | 244/0/5 |
| off, text-ocr | 19/0 | 171/0 | 15 | 7/0/10 | 0 | 244/0/5 |

Mentions and cards carry no tier, so certain and likely rows do not add up to every right row. The last column adds the measures that round 2 scored, so it compares with the round 2 tables below. The other left-out rows are three directory ids that only a merge used to give a card, and, with the directory off, the joins that only the directory can make.

## Result

Round 2 is not yet at the bar: that is for a set of documents no author has seen to say. On the sample, all four runs pass the bar. They have no wrong link, merge, card, attachment or answer. They also have no wrong item in any other measure.

| Run | links r/w/l/i | mentions r/w/l | identifiers r/w/l/i | statement facts r/w/l | passport facts r/w/l | answers r/w/l | splits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| on, text | 47/0/0/0 | 61/0/5 | 67/0/0/0 | 42/0/0 | 16/0/0 | 15/0/0 | 0 |
| off, text | 47/0/0/0 | 61/0/5 | 67/0/0/0 | 42/0/0 | 16/0/0 | 15/0/0 | 12 |
| on, text-ocr | 45/0/0/2 | 61/0/5 | 65/0/0/2 | 42/0/0 | 16/0/0 | 15/0/0 | 0 |
| off, text-ocr | 45/0/0/2 | 61/0/5 | 65/0/0/2 | 42/0/0 | 16/0/0 | 15/0/0 | 11 |

The letters are r right, w wrong, l left out and i impossible. Splits are not wrong. With the directory off, a bank seen by its name in one document and by its code in another gets two cards. The scorer's full tables follow at the end.

The text runs are byte-identical to round 1. On text-ocr, the scanned expired passport's four facts (holder, number, expiry, current) are now right instead of left out, so left-out rows fall from 9 to 5 there.

## Round 2: documents the authors had not seen

Two outside reviewers wrote documents and labels: sol's 18, sol's 4 diagnostic documents, sol's one-page Hindi input, and astra's 18 with its own directory (the sample's plus two invented entries). Each set was scored by a private copy of `score.mjs` pointed at that set's labels. The totals add every measure except `org_directory`; i is impossible.

| Set | Directory | Round 1 r/w/l | Round 2 r/w/l (i) | Links r/w/l | Answers r/w/l | Score exit |
| --- | --- | --- | --- | --- | --- | --- |
| sample text | on | 248/0/5 | 248/0/5 | 47/0/0 | 15/0/0 | 0 |
| sample text | off | 248/0/5 | 248/0/5 | 47/0/0 | 15/0/0 | 0 |
| sample text-ocr | on | 240/0/9 (4) | 244/0/5 (4) | 45/0/0 (2) | 15/0/0 | 0 |
| sample text-ocr | off | 240/0/9 (4) | 244/0/5 (4) | 45/0/0 (2) | 15/0/0 | 0 |
| sol, 18 | on | 45/3/16 | 57/0/4 | 5/0/0 | 4/0/0 | 0 |
| sol, 18 | off | 46/3/15 | 58/2/3 | 5/2/0 | 4/0/0 | 1 |
| sol, 4 | on | 15/6/1 | 17/0/0 | 0/0/0 | 1/0/0 | 0 |
| sol, 4 | off | 15/6/1 | 17/0/0 | 0/0/0 | 1/0/0 | 0 |
| sol, Hindi page | on | format error | 0/0/0 | 0/0/0 | 0/0/0 | 0 (was 2) |
| sol, Hindi page | off | format error | 0/0/0 | 0/0/0 | 0/0/0 | 0 (was 2) |
| astra, 18 | on | 74/3/7 | 79/0/3 | 9/0/0 | 7/0/0 | 0 |
| astra, 18 | off | 73/4/7 | 80/0/2 | 9/0/0 | 7/0/0 | 0 |

Every round 1 wrong row, and what the program does now:

- `explicitly-not-statement` (sol, 4): the title says "Not a statement". There is no statement, header link or answer now. The account is an identifier with no link.
- `two-live-previous` (sol, 4): the older of two unexpired passports of one holder was current. The renewal now supersedes it: `current` is false, and the renewal is the answer.
- `ch-stitch-code` (sol, 18): `Bank: UBS` on a knitting sheet was a settled organisation, and with the directory on, a real bank's card. The bare field word is now a mention with no key, in both modes.
- `swiss-code-only` (sol, 18), directory on: a BIC no entry holds made a nameless card with the account on it. Now there is no card and no link. With the directory off the two links remain; see the disputed rows.
- `number-decoys` (astra): `Invoice number: 4242 4242 4242 4242` and the 15-digit one were cards. The field label now beats the shape; no identifier is reported.
- `bare-short-name` (astra), directory off: `Bank: LUM` was settled by name. It is now a mention with no key. With the directory on it was right through the alias and is now left out.
- `ca-oct` (astra): `Account: 8800 1234 01` was not read, and September was answered as the latest. The grouped number is now one account, and October is the answer.
- The Hindi page (sol): nothing found wrote `"documents": null`. It writes `[]` now.

Round 1 rows that were left out and are right now: both Glass Tamarack statements, their header links and the latest answer (sol), because `:: Name ::` is no longer read as a field label and the grouped account is read.

## Rows I dispute

- **sol, 18, `swiss-code-only`, directory off: `links account 11839654220` and `links bic ZZZQCH9X`, "key lands on no labelled organisation".** The labels say an invented, unassigned BIC names no organisation. With the directory off, nothing in the text tells this code from the sample's own codes in the same shape: a valid code beside an account, under a printed bank name the code does not spell (the French RIB, the Japanese remittance, both scanned Indian letters). The sample's labels expect those links, and ruling R1 keeps code-only cards where they do. With the directory off, 27 right links in 12 sample documents land on nameless code cards (counted on the committed `off, text` run); a rule strict enough to leave this row out would leave those out too. I left the program as it is; the directory-on run shows the rule that applies once a directory can say a code is unassigned.

No label of astra's set or of sol's diagnostic sets is disputed.

## Round 2 rules

Each new rule, as one general statement:

- **Denied role.** A document that says it is not a statement or not a passport (a negation right before the role word, or right after it in Hindi and Japanese) is none. A specimen disclaimer ("not a real statement", "kein echter Kontoauszug") denies nothing.
- **Colon runs.** One colon ends a field label; a run of two or more is decoration.
- **Grouped account.** After an account label, digits in groups split by single spaces are one account when the groups run to the end of the field. The value drops the spaces; the span covers the whole written form.
- **Unread later statement.** No latest-statement answer for an account when a statement of the same issuer with a later period has no account the program could read.
- **Label beats shape.** A grouped number whose own field label names something other than a card is not a card.
- **Bare field word.** A bank field's one-word value is an organisation only when a tie in the document settles it. Neither the label nor the directory does.
- **Code alone.** With the directory on, a code that no entry holds and no mention names makes no card, and nothing links to it.
- **Renewal.** Of one holder's passports that are current by their dates, only the one issued last is current. When issue dates cannot order them, none can be told current.
- **Cancellation stamp.** A line that is only cancellation words (or a label and only cancellation words), in capitals, cancels the passport. The words in a sentence ("VOID IF ALTERED") do not.
- **Scanned labels.** On a scanned page, a passport field label of six or more characters at the line start may have one misread character, its colon counted.
- **Empty run.** A run that finds nothing still writes `"documents": []`.

`TestOddInputs` in `engine/cmd/m1a-extract` feeds the command empty pages, only digits, only colons, Devanagari, full-width and Arabic-Indic digits, mixed scripts, very long lines and zone-shaped junk, and requires a run file that the scorer's own format check accepts. Dates and numbers in non-ASCII digits are left out, not parsed.

## Rules

Each rule says what it finds, what makes it valid, and what ties it to an organisation.

Identifiers (`identifiers.go`, `checks.go`):

- **IBAN.** Two letters and two digits, then the registry length for that country, with single spaces allowed, ending at a word end. Valid when mod 97 is 1. It ties by its bank code, for the countries whose bank-code position the registry table holds.
- **BIC.** Eight or eleven characters, taken only after a `BIC` or `SWIFT` label on the line. Valid when its country part is an ISO 3166 code. It ties by its first six characters.
- **IFSC.** Four letters, the digit 0, then six characters. Shape only. On a scanned page, an `O` or `Ø` in the fifth place is read as 0, because the scheme fixes that digit. It ties by its first four letters.
- **Card.** A 16-digit number in groups of four (or 4-6-5), when the line says card or the number passes Luhn, and its own field label, if it has one, says card. Valid by Luhn. A card ties only through a statement header.
- **PAN.** Five letters, four digits, a letter, on a line whose field says PAN (`PAN`, `पैन`). Valid when the fourth letter is a holder status and the serial is not 0000. The same shape in a TAN field is not taken.
- **Sort code.** `NN-NN-NN` after a `Sort code` label. Shape only. It ties as that part of a GB or IE IBAN in the document.
- **Code banque and code guichet.** Five digits after their labels. Shape only. Each ties as that part of an FR or MC IBAN.
- **Clé RIB.** Two digits after `Clé RIB`. Valid when it equals the key recomputed from the bank code, branch code and account on its own line. With any of the three missing, `valid` is null.
- **Passport number.** Six to nine characters after a passport-number label (`Passport No.`, `Reisepass-Nr.`, `N° de passeport`, `旅券番号`, `पासपोर्ट संख्या`). No check.
- **Machine-readable zone.** Two consecutive lines of 44 characters from `[A-Z0-9<]`, starting `P` (TD3) or `V` (MRV-A). Valid when every check digit holds.
- **Account (labelled).** A number of at least six digits, with at most two letters, right after a field label that names an account (`account`, `a/c`, `konto`, `compte`, `खाता`, `口座`). Digits in groups split by single spaces are one account when the groups run to the end of the field. One account-type word may sit between the label and the number (`受取口座: 普通 0123456`). The label must not also say card, policy, consumer, customer, invoice, reference or transaction.
- **Account (transfer row).** A bare run of 9 to 18 digits on a line that holds exactly one IFSC.

Ties from an identifier to an organisation (`document.go`):

- **ifsc-prefix, bic-institution, iban-bank-code.** A valid code ties to the institution its code names. The key is the code itself (`ifsc:HDFC`), or with the directory on, the one directory entry holding that code.
- **paired-code.** An account or IBAN ties to the IFSC or BIC that is the only code on its line, when the line holds exactly one account. When its line holds nothing else, it ties to the only identifier on the next or previous line, if that identifier is a code. The tie is never by distance in characters.
- **inside-iban, same-account.** A sort code, a French bank or branch code, or a domestic account number that is that part of a valid IBAN in the document ties to the IBAN's institution. Leading zeros are ignored for accounts.
- **statement-header.** Every account of a statement ties to its issuer, unless a code ties that account to an organisation the document does not tie to the issuer.
- **No other ties.** A name and a number on one page with nothing above between them make no link. An invalid identifier is never linked.

Organisations (`orgs.go`, `document.go`, `run.go`):

- **Names around an organisation word.** An organisation word is a bank word (`Bank`, `Banque`, `Banco`, `Sparkasse`, `बैंक`, `銀行`), a word ending in `bank` (`Commerzbank`), a type word (`Company`, `Corporation`, `Office`, `School`, `Credit Union`, `Building Society`), or a legal form (`Ltd`, `GmbH`, `AG`, `N.V.`, `लिमिटेड`). These are words for kinds of organisation, not names. The name grows left over capitalised words and `&`. It grows right over legal forms, over `of` or `de` plus capitalised words, and, after a Romance bank noun, over its adjectives (`La Banque Postale`). It stops at punctuation, and it drops capitalised function words such as `The`.
- **Devanagari names.** Devanagari has no capitals. So a name grows left to the start of its phrase, at most four words, and is not taken when it does not reach the start of the phrase.
- **Japanese names.** A name ending in `銀行` runs left over kanji, katakana and Latin letters. A run of hiragana joins it only when the run reaches the start of the phrase (`みずほ銀行`). That is why `お問い合わせは` stays out.
- **Field labels are not names.** A bank word followed by a colon is a field label (`Bank:`, `支払銀行:`). A run of two colons is decoration, not a field separator. So is any organisation word followed by a colon with fewer than two name words before it.
- **Bank fields.** The value of a field whose label says bank (`Bank:`, `Kreditinstitut:`, `送金銀行:`, `Banks involved…:`) is split at commas. Each item of two or more words shaped like a name is a name (`Alpha Group`, `ABC Trust`). A one-word item is a bare word: it is an organisation only when a tie in the document settles it, the same way an acronym is.
- **Settled by name.** A whole name holding an organisation word gets the key `name:<match key>`. The match key is the directory's own algorithm.
- **Acronyms need a tie.** An all-capitals acronym with only a legal form (`HDFC Ltd`) is no name by itself. So is a bare acronym (`HDFC`). Either one is settled only by a tie inside the document: its first word is the institution code of an IFSC or BIC there (code), or the words are the leading words of a longer name there spelt the same way, or the acronym is that name's initials (mention). When a tie fits two organisations, or none, the acronym name gets no key, and a bare acronym is not reported.
- **Name tied to a code.** A bank name whose first word is an IFSC's or BIC's institution code (`HDFC Bank` beside `HDFC0…`) joins that code's key, when only one name in the document fits. An IBAN joins the BIC paired with it, when they have the same country.
- **Directory, when on.** A key joins a directory entry only when exactly one entry holds its name or code. A join that would put two entries on one card is refused. An acronym name with a legal form that the document does not settle takes an exact entry, settled by directory; a bare field word does not. A code that no entry holds and no mention names makes no card, and nothing links to it. With the directory off, the program holds no organisation names at all, and a code keeps a nameless card.
- **Cards.** A card is named by its directory entry when it has one. Otherwise it takes the most-used name the documents settle by name. A card known only by codes has a null name.

Facts (`facts.go`, `dates.go`):

- **Statement.** A statement needs four things. The document calls itself one (`statement`, `passbook`, `Kontoauszug`, `relevé de compte`, `明細`, `पासबुक`) above a period field. The period field (`Period`, `Zeitraum`, `Période`, `期間`, `अवधि`) holds exactly two dates. The accounts above that field spell one account (one number, or an IBAN with its domestic number). The first organisation the document names has a key. A note such as "your next statement will cover…" is no period field. Without all four, there is no statement. A document that denies the role ("Not a statement") has none; a specimen disclaimer ("not a real statement") denies nothing.
- **Dates.** The parser reads day-month-year with month names in en, de, fr and hi, month-first with names, `年月日`, ISO, and numeric dates. Two numeric dates in a period must read in the same order. When both day-first and month-first work, the period is left out.
- **Passport.** A page with a passport zone is a passport. So is a page with no zone at all, titled `PASSPORT`, that gives a passport number. A visa zone means no passport. The holder comes from the `Surname` and `Given names` fields, else from a valid zone. Expiry is the zone's date when the zone is valid and agrees with the printed date. Otherwise it is `cannot-tell`. A page that denies being a passport is none. On a scanned page a field label may have one misread character. `current` is false when a line is a cancellation stamp (`CANCELLED`, `VOID`, `ANNULÉ` and their like, alone on the line or after a label). Otherwise it is true or false when the expiry, or every date the expiry could be, is on one side of the reference date. When the dates fall on both sides, it is null.
- **Answers.** The latest statement of an account is chosen by the statement's own period, never the file date. Statements are one account when they share a spelling. Two documents with the same latest period give no answer. No latest answer is given when a statement of the same issuer with a later period has no readable account. Of one holder's passports current by their dates, only the one issued last is current; when issue dates cannot order them, none is. A holder's current passport is answered only when exactly one of theirs is current and none is unsettled.

## Left out, by cause

- **Names with no organisation word (5 mentions in every run).** These are the merchants `Example Cafe`, `Sample Grocers` and `Example Books`, and the counterparty `Sample Traders` (twice). Nothing in the text marks them as organisations, except that a reader knows a café is a business. Catching them would need a named-entity tagger. The Mac app has one and Go does not.
- **OCR damage on scanned pages (text-ocr only).** The scorer counts the PNB IFSC, its two links and the expired zone as impossible. The expired passport's misread field labels are read again since round 2.
- **Directory id for an inflected name (directory on).** `Deutschen Bundesbank` is settled by the document, but no directory spelling matches the inflected form, so `org_directory` is left out once.
- **Splits (directory off).** With the directory off, a bank named in one document and known only by its code in another has two keys: `name:axisbank` and `ifsc:UTIB`, for example. The only code-to-name join I allow is a bank name whose first word is the code. That join fires for HDFC and for no other bank in the sample, so 12 splits remain on text and 11 on OCR. Splits are not wrong, and the directory removes all of them.

## Size, dependencies, time

- **Code.** 2,811 lines in the package and command (round 1: 2,511). About 150 of those are output types.
- **Tests.** 962 lines (round 1: 617): table tests for every validator, every tie rule, every round 2 rule and Unicode spans, on text written in the tests, plus the odd-input test of the command. Mutation probe: I broke seven rules one at a time and the tests failed for each one.
- **Dependencies.** None: the Go 1.26 standard library only.
- **Time.** Round 1, not measured again: I measured on an Apple M5 Pro with `BenchmarkRun` (in `bench_test.go`), 200 iterations, 3 counts. Extracting all 43 documents takes 8.5 to 8.7 ms with the directory off and 8.7 to 9.6 ms with it on, about 0.2 ms per document. The CPU profile shows `regexp` backtracking as 52% of the time, from about 15 scheme patterns run over every line. A whole command run, with process start, file reads and directory load, takes 10 to 20 ms wall time.

## Go: easy and hard

- **Easy.** The scheme checks (`math/big` for mod 97, Luhn, ICAO weights, the RIB key) and table-driven rules are short in Go. Each identifier scheme is one row in a table. JSON with nullable fields is pointers and struct tags. `[]rune` makes scalar-value spans straightforward, and the tests prove them on Hindi and Japanese text. Builds are instant, and the module needs no token.
- **Hard.** The standard library has no Unicode normalisation, because NFKC is in `golang.org/x/text`, outside the standard library. `fold` implements only the part the sample needs: full-width ASCII, the ideographic space, and the Devanagari nukta letters that NFKC decomposes. On this directory, every one of its names gets the stored key (`TestMatchKeyAgreesWithDirectory`). A wider directory would hit cases this fold does not cover, such as half-width katakana and ligatures. `regexp` is RE2, so it has no look-behind, and its `\b` is ASCII-only. Rules that look at the field label before a value are written by hand (`fieldLabel`). Go has no script-aware word breaker, so Japanese names use a hand-written rule.

## Crossing a process boundary

If this ran beside SIFS as a separate process, I measured on the 43 sample documents what SIFS would send and get back.

- **Send per document.** `id`, `lang` and `pages` (`text`, `scanned`). Mean 507 bytes, median 466, maximum 922, 21,789 in total.
- **Get back per document.** The document entry plus the cards its keys use. Mean 610 bytes, median 612, maximum 1,338, 26,235 in total. That is 4.4 rows on average (mentions, identifiers, links, statement, passport), and at most 12.
- **Once per session.** The directory, 355,167 bytes, if the extractor does not read it from disk itself.
- **Not per document.** Cross-document keys and the answers need every document's tokens. So either the extractor keeps state across calls, or SIFS keeps the tokens and joins them itself.

At about 1 KB each way and 0.2 ms of work per document, the bytes do not matter. What does matter: an extra process to ship, sign and supervise in a Mac app; a JSON schema both sides must version; and the span unit. Spans count Unicode scalar values. Rust's `str` counts bytes and Swift's `String` counts grapheme clusters, so each side must convert at the boundary.

**What a Go extractor cannot reach.** It cannot use Apple's on-device frameworks: NaturalLanguage's `NLTagger` with the `.organizationName` tag, and Foundation's `NSDataDetector`. These live in the Mac app's process and come through Swift or Objective-C, with no cgo-free path from a portable Go binary. The merchants above (`Example Cafe`, `Sample Grocers`) are exactly what a name tagger finds. The Go side would need SIFS to run the tagger and send its spans as a hint, and that adds a field per document.

## Where FORMAT.md or the scorer was unclear

- **`current` when the expiry is `cannot-tell`.** FORMAT.md gives no type for this case. The labels expect `true` for `passport-bad-mrz`; I learned that from a left-out line in the scorer's output. I chose a general rule: it is current when every date the expiry could be is after the reference date.
- **`paired-code` and "a code of its own".** It is not clear whether a sort code or a French bank code counts as a code. I treat any other identifier on the account's line as blocking an adjacent-line pair.
- **A statement with no issuer.** The format has no null `issuer`. I leave the statement out.
- **`iban-bank-code` with the directory off.** The README defines it through "the bank's `iban_bank_code` entries", which only a directory holds. With the directory off, I give it wherever the IBAN's country has a bank-code position. Where a BIC pairs with the IBAN, I also give `paired-code` on the same row. For the French IBAN, only `paired-code` holds in the labels.
- **`valid` for a BIC and for PAN serial 0000.** A BIC has no check digit, so I report a valid country code as `true`. For serial 0000 I report `false`; the scorer accepts either.
- **Unmatched mentions with no key.** FORMAT.md counts a mention as wrong when it has a key and matches no label. So an output could list any text with `org: null` at no cost. This extractor reports a keyless mention only for an acronym name with a legal form and for a bare bank field word.

## The scorer's tables

These are copied from the scorer's output for the committed runs. Each language column gives right/wrong/left out.

```text
go-m1a, directory on, text set text
measure                     right  wrong  left out  impossible  de r/w/l  en r/w/l  fr r/w/l  hi r/w/l  ja r/w/l
mentions                       61      0         5           0     6/0/0    45/0/5     1/0/0     4/0/0     5/0/0
org_directory                  31      0         1           0                                                  
identifier:account             24      0         0           0              19/0/0     1/0/0     2/0/0     2/0/0
identifier:bic                  4      0         0           0     1/0/0     1/0/0     1/0/0               1/0/0
identifier:card                 3      0         0           0               3/0/0                              
identifier:fr_bank_code         1      0         0           0                         1/0/0                    
identifier:fr_branch_code       1      0         0           0                         1/0/0                    
identifier:fr_rib_key           1      0         0           0                         1/0/0                    
identifier:gb_sort_code         1      0         0           0               1/0/0                              
identifier:iban                 7      0         0           0     2/0/0     4/0/0     1/0/0                    
identifier:ifsc                13      0         0           0              11/0/0               2/0/0          
identifier:in_pan               2      0         0           0                                   2/0/0          
identifier:mrz                  5      0         0           0               5/0/0                              
identifier:passport_number      5      0         0           0               5/0/0                              
links                          47      0         0           0     2/0/0    33/0/0     5/0/0     4/0/0     3/0/0
statement_period               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_account              14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_issuer               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
passport_holder                 4      0         0           0               4/0/0                              
passport_number                 4      0         0           0               4/0/0                              
passport_expiry                 4      0         0           0               4/0/0                              
passport_current                4      0         0           0               4/0/0                              
answer_latest_statement        12      0         0           0     1/0/0     9/0/0               1/0/0     1/0/0
answer_current_passport         3      0         0           0               3/0/0                              
go-m1a, directory off, text set text
measure                     right  wrong  left out  impossible  de r/w/l  en r/w/l  fr r/w/l  hi r/w/l  ja r/w/l
mentions                       61      0         5           0     6/0/0    45/0/5     1/0/0     4/0/0     5/0/0
org_directory                  46      0         0           0                                                  
identifier:account             24      0         0           0              19/0/0     1/0/0     2/0/0     2/0/0
identifier:bic                  4      0         0           0     1/0/0     1/0/0     1/0/0               1/0/0
identifier:card                 3      0         0           0               3/0/0                              
identifier:fr_bank_code         1      0         0           0                         1/0/0                    
identifier:fr_branch_code       1      0         0           0                         1/0/0                    
identifier:fr_rib_key           1      0         0           0                         1/0/0                    
identifier:gb_sort_code         1      0         0           0               1/0/0                              
identifier:iban                 7      0         0           0     2/0/0     4/0/0     1/0/0                    
identifier:ifsc                13      0         0           0              11/0/0               2/0/0          
identifier:in_pan               2      0         0           0                                   2/0/0          
identifier:mrz                  5      0         0           0               5/0/0                              
identifier:passport_number      5      0         0           0               5/0/0                              
links                          47      0         0           0     2/0/0    33/0/0     5/0/0     4/0/0     3/0/0
statement_period               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_account              14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_issuer               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
passport_holder                 4      0         0           0               4/0/0                              
passport_number                 4      0         0           0               4/0/0                              
passport_expiry                 4      0         0           0               4/0/0                              
passport_current                4      0         0           0               4/0/0                              
answer_latest_statement        12      0         0           0     1/0/0     9/0/0               1/0/0     1/0/0
answer_current_passport         3      0         0           0               3/0/0                              
go-m1a, directory on, text set text-ocr
measure                     right  wrong  left out  impossible  de r/w/l  en r/w/l  fr r/w/l  hi r/w/l  ja r/w/l
mentions                       61      0         5           0     6/0/0    45/0/5     1/0/0     4/0/0     5/0/0
org_directory                  31      0         1           0                                                  
identifier:account             24      0         0           0              19/0/0     1/0/0     2/0/0     2/0/0
identifier:bic                  4      0         0           0     1/0/0     1/0/0     1/0/0               1/0/0
identifier:card                 3      0         0           0               3/0/0                              
identifier:fr_bank_code         1      0         0           0                         1/0/0                    
identifier:fr_branch_code       1      0         0           0                         1/0/0                    
identifier:fr_rib_key           1      0         0           0                         1/0/0                    
identifier:gb_sort_code         1      0         0           0               1/0/0                              
identifier:iban                 7      0         0           0     2/0/0     4/0/0     1/0/0                    
identifier:ifsc                12      0         0           1              10/0/0               2/0/0          
identifier:in_pan               2      0         0           0                                   2/0/0          
identifier:mrz                  4      0         0           1               4/0/0                              
identifier:passport_number      5      0         0           0               5/0/0                              
links                          45      0         0           2     2/0/0    31/0/0     5/0/0     4/0/0     3/0/0
statement_period               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_account              14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_issuer               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
passport_holder                 4      0         0           0               4/0/0                              
passport_number                 4      0         0           0               4/0/0                              
passport_expiry                 4      0         0           0               4/0/0                              
passport_current                4      0         0           0               4/0/0                              
answer_latest_statement        12      0         0           0     1/0/0     9/0/0               1/0/0     1/0/0
answer_current_passport         3      0         0           0               3/0/0                              
go-m1a, directory off, text set text-ocr
measure                     right  wrong  left out  impossible  de r/w/l  en r/w/l  fr r/w/l  hi r/w/l  ja r/w/l
mentions                       61      0         5           0     6/0/0    45/0/5     1/0/0     4/0/0     5/0/0
org_directory                  45      0         0           0                                                  
identifier:account             24      0         0           0              19/0/0     1/0/0     2/0/0     2/0/0
identifier:bic                  4      0         0           0     1/0/0     1/0/0     1/0/0               1/0/0
identifier:card                 3      0         0           0               3/0/0                              
identifier:fr_bank_code         1      0         0           0                         1/0/0                    
identifier:fr_branch_code       1      0         0           0                         1/0/0                    
identifier:fr_rib_key           1      0         0           0                         1/0/0                    
identifier:gb_sort_code         1      0         0           0               1/0/0                              
identifier:iban                 7      0         0           0     2/0/0     4/0/0     1/0/0                    
identifier:ifsc                12      0         0           1              10/0/0               2/0/0          
identifier:in_pan               2      0         0           0                                   2/0/0          
identifier:mrz                  4      0         0           1               4/0/0                              
identifier:passport_number      5      0         0           0               5/0/0                              
links                          45      0         0           2     2/0/0    31/0/0     5/0/0     4/0/0     3/0/0
statement_period               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_account              14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
statement_issuer               14      0         0           0     1/0/0    11/0/0               1/0/0     1/0/0
passport_holder                 4      0         0           0               4/0/0                              
passport_number                 4      0         0           0               4/0/0                              
passport_expiry                 4      0         0           0               4/0/0                              
passport_current                4      0         0           0               4/0/0                              
answer_latest_statement        12      0         0           0     1/0/0     9/0/0               1/0/0     1/0/0
answer_current_passport         3      0         0           0               3/0/0                              
```
