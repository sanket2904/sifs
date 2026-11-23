# Go M1a extractor

The Go one of the two M1a extractor prototypes. It is rules only, with no model. It reads the sample's page text and writes the run file that `sample/FORMAT.md` describes. The code lives in `engine/`: the package `engine/extract` and the command `engine/cmd/m1a-extract`. The module imports the standard library only.

## Reproduce

```sh
sample/tools/go-m1a-runs.sh
```

The script builds the command and writes the four runs under `sample/runs/`. Each run gets the scorer's `.score.json` beside it, and the script prints each table. It exits non-zero when any run fails the bar. A run with the directory off is started without `-directory`, so the program never opens `banks.json`. Two runs in a row give byte-identical files.

## Result

All four runs pass the bar. They have no wrong link, merge, card, attachment or answer. They also have no wrong item in any other measure.

| Run | links r/w/l/i | mentions r/w/l | identifiers r/w/l/i | statement facts r/w/l | passport facts r/w/l | answers r/w/l | splits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| on, text | 47/0/0/0 | 61/0/5 | 67/0/0/0 | 42/0/0 | 16/0/0 | 15/0/0 | 0 |
| off, text | 47/0/0/0 | 61/0/5 | 67/0/0/0 | 42/0/0 | 16/0/0 | 15/0/0 | 12 |
| on, text-ocr | 45/0/0/2 | 61/0/5 | 65/0/0/2 | 42/0/0 | 12/0/4 | 15/0/0 | 0 |
| off, text-ocr | 45/0/0/2 | 61/0/5 | 65/0/0/2 | 42/0/0 | 12/0/4 | 15/0/0 | 11 |

The letters are r right, w wrong, l left out and i impossible. Splits are not wrong. With the directory off, a bank seen by its name in one document and by its code in another gets two cards. The scorer's full tables follow at the end.

## Rules

Each rule says what it finds, what makes it valid, and what ties it to an organisation.

Identifiers (`identifiers.go`, `checks.go`):

- **IBAN.** Two letters and two digits, then the registry length for that country, with single spaces allowed, ending at a word end. Valid when mod 97 is 1. It ties by its bank code, for the countries whose bank-code position the registry table holds.
- **BIC.** Eight or eleven characters, taken only after a `BIC` or `SWIFT` label on the line. Valid when its country part is an ISO 3166 code. It ties by its first six characters.
- **IFSC.** Four letters, the digit 0, then six characters. Shape only. On a scanned page, an `O` or `Ø` in the fifth place is read as 0, because the scheme fixes that digit. It ties by its first four letters.
- **Card.** A 16-digit number in groups of four (or 4-6-5), when the line says card or the number passes Luhn. Valid by Luhn. A card ties only through a statement header.
- **PAN.** Five letters, four digits, a letter, on a line whose field says PAN (`PAN`, `पैन`). Valid when the fourth letter is a holder status and the serial is not 0000. The same shape in a TAN field is not taken.
- **Sort code.** `NN-NN-NN` after a `Sort code` label. Shape only. It ties as that part of a GB or IE IBAN in the document.
- **Code banque and code guichet.** Five digits after their labels. Shape only. Each ties as that part of an FR or MC IBAN.
- **Clé RIB.** Two digits after `Clé RIB`. Valid when it equals the key recomputed from the bank code, branch code and account on its own line. With any of the three missing, `valid` is null.
- **Passport number.** Six to nine characters after a passport-number label (`Passport No.`, `Reisepass-Nr.`, `N° de passeport`, `旅券番号`, `पासपोर्ट संख्या`). No check.
- **Machine-readable zone.** Two consecutive lines of 44 characters from `[A-Z0-9<]`, starting `P` (TD3) or `V` (MRV-A). Valid when every check digit holds.
- **Account (labelled).** A number of at least six digits, with at most two letters, right after a field label that names an account (`account`, `a/c`, `konto`, `compte`, `खाता`, `口座`). One account-type word may sit between the label and the number (`受取口座: 普通 0123456`). The label must not also say card, policy, consumer, customer, invoice, reference or transaction.
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
- **Field labels are not names.** A bank word followed by a colon is a field label (`Bank:`, `受取銀行:`). So is any organisation word followed by a colon with fewer than two name words before it.
- **Bank fields.** The value of a field whose label says bank (`Bank:`, `Kreditinstitut:`, `送金銀行:`, `Banks involved…:`) is split at commas. Each item shaped like a name is a name (`BNP Paribas`, `ABN AMRO`).
- **Settled by name.** A whole name holding an organisation word gets the key `name:<match key>`. The match key is the directory's own algorithm.
- **Acronyms need a tie.** An all-capitals acronym with only a legal form (`HDFC Ltd`) is no name by itself. So is a bare acronym (`HDFC`). Either one is settled only by a tie inside the document: its first word is the institution code of an IFSC or BIC there (code), or the words are the leading words of a longer name there spelt the same way, or the acronym is that name's initials (mention). When a tie fits two organisations, or none, the acronym name gets no key, and a bare acronym is not reported.
- **Name tied to a code.** A bank name whose first word is an IFSC's or BIC's institution code (`HDFC Bank` beside `HDFC0…`) joins that code's key, when only one name in the document fits. An IBAN joins the BIC paired with it, when they have the same country.
- **Directory, when on.** A key joins a directory entry only when exactly one entry holds its name or code. A join that would put two entries on one card is refused. An acronym name the document does not settle takes an exact entry, settled by directory. With the directory off, the program holds no organisation names at all.
- **Cards.** A card is named by its directory entry when it has one. Otherwise it takes the most-used name the documents settle by name. A card known only by codes has a null name.

Facts (`facts.go`, `dates.go`):

- **Statement.** A statement needs four things. The document calls itself one (`statement`, `passbook`, `Kontoauszug`, `relevé de compte`, `明細`, `पासबुक`) above a period field. The period field (`Period`, `Zeitraum`, `Période`, `期間`, `अवधि`) holds exactly two dates. The accounts above that field spell one account (one number, or an IBAN with its domestic number). The first organisation the document names has a key. A note such as "your next statement will cover…" is no period field. Without all four, there is no statement.
- **Dates.** The parser reads day-month-year with month names in en, de, fr and hi, month-first with names, `年月日`, ISO, and numeric dates. Two numeric dates in a period must read in the same order. When both day-first and month-first work, the period is left out.
- **Passport.** A page with a passport zone is a passport. So is a page with no zone at all, titled `PASSPORT`, that gives a passport number. A visa zone means no passport. The holder comes from the `Surname` and `Given names` fields, else from a valid zone. Expiry is the zone's date when the zone is valid and agrees with the printed date. Otherwise it is `cannot-tell`. `current` is false when the page says `CANCELLED`. Otherwise it is true or false when the expiry, or every date the expiry could be, is on one side of the reference date. When the dates fall on both sides, it is null.
- **Answers.** The latest statement of an account is chosen by the statement's own period, never the file date. Statements are one account when they share a spelling. Two documents with the same latest period give no answer. A holder's current passport is answered only when exactly one of theirs is current and none is unsettled.

## Left out, by cause

- **Names with no organisation word (5 mentions in every run).** These are the merchants `Example Cafe`, `Sample Grocers` and `Example Books`, and the counterparty `Sample Traders` (twice). Nothing in the text marks them as organisations, except that a reader knows a café is a business. Catching them would need a named-entity tagger. The Mac app has one and Go does not.
- **OCR damage on scanned pages (text-ocr only).** In `passport-asha-expired`, Vision read the field labels as `SURHAME` and `GIVEN NAMESE` and broke the zone. The passport's holder, number, expiry and current are all left out: 4 facts. The scorer counts the PNB IFSC, its two links and the expired zone as impossible.
- **Directory id for an inflected name (directory on).** `Deutschen Bundesbank` is settled by the document, but no directory spelling matches the inflected form, so `org_directory` is left out once.
- **Splits (directory off).** With the directory off, a bank named in one document and known only by its code in another has two keys: `name:axisbank` and `ifsc:UTIB`, for example. The only code-to-name join I allow is a bank name whose first word is the code. That join fires for HDFC and for no other bank in the sample, so 12 splits remain on text and 11 on OCR. Splits are not wrong, and the directory removes all of them.

## Size, dependencies, time

- **Code.** 2,511 lines in the package and command (2,149 not blank and not comment). About 150 of those are output types.
- **Tests.** 617 lines (559 not blank and not comment): table tests for every validator, every tie rule and Unicode spans, on text written in the tests. Mutation probe: I broke seven rules one at a time and the tests failed for each one.
- **Dependencies.** None: the Go 1.26 standard library only.
- **Time.** I measured on an Apple M5 Pro with `BenchmarkRun` (in `bench_test.go`), 200 iterations, 3 counts. Extracting all 43 documents takes 8.5 to 8.7 ms with the directory off and 8.7 to 9.6 ms with it on, about 0.2 ms per document. The CPU profile shows `regexp` backtracking as 52% of the time, from about 15 scheme patterns run over every line. A whole command run, with process start, file reads and directory load, takes 10 to 20 ms wall time.

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
- **Unmatched mentions with no key.** FORMAT.md counts a mention as wrong when it has a key and matches no label. So an output could list any text with `org: null` at no cost. This extractor reports a keyless mention only for an acronym name with a legal form.

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
passport_holder                 3      0         1           0               3/0/1                              
passport_number                 3      0         1           0               3/0/1                              
passport_expiry                 3      0         1           0               3/0/1                              
passport_current                3      0         1           0               3/0/1                              
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
passport_holder                 3      0         1           0               3/0/1                              
passport_number                 3      0         1           0               3/0/1                              
passport_expiry                 3      0         1           0               3/0/1                              
passport_current                3      0         1           0               3/0/1                              
answer_latest_statement        12      0         0           0     1/0/0     9/0/0               1/0/0     1/0/0
answer_current_passport         3      0         0           0               3/0/0
```
