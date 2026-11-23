# Rust M1a extractor

The goal: SIFS shows a card for an organisation with the accounts held there, and answers "when does my passport expire", from rules alone. Before any schema is built on it, the extractor must prove it makes no wrong link, merge or answer on the labelled sample, and must leave out what a document does not settle. This is the Rust one of the two independent prototypes.

The library is `src-tauri/src/findings/` (no Tauri; builds and tests with `--no-default-features`). The binary is `src-tauri/src/bin/m1a_extract.rs`.

## Reproduce

```sh
sample/tools/rust-m1a.sh
```

It builds the release binary, writes the four runs under `sample/runs/`, scores each with `sample/tools/score.mjs` (which writes the `.score.json` beside it), and exits with the worst scorer status. The directory-off runs are not given `directory/banks.json`. One run by hand: `m1a_extract <text-set-dir> <out.json> [--directory <banks.json>]`.

## Scores

All four runs exit 0: no wrong link, merge, card, attachment or answer. Every measure has 0 wrong in every run.

### Directory on, text

| Measure | Right | Wrong | Left out | Impossible | de r/w/l | en r/w/l | fr r/w/l | hi r/w/l | ja r/w/l |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- | --- |
| mentions | 61 | 0 | 5 | 0 | 6/0/0 | 45/0/5 | 1/0/0 | 4/0/0 | 5/0/0 |
| org_directory | 31 | 0 | 1 | 0 |  |  |  |  |  |
| identifier:account | 24 | 0 | 0 | 0 |  | 19/0/0 | 1/0/0 | 2/0/0 | 2/0/0 |
| identifier:bic | 4 | 0 | 0 | 0 | 1/0/0 | 1/0/0 | 1/0/0 |  | 1/0/0 |
| identifier:card | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |
| identifier:fr_bank_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_branch_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_rib_key | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:gb_sort_code | 1 | 0 | 0 | 0 |  | 1/0/0 |  |  |  |
| identifier:iban | 7 | 0 | 0 | 0 | 2/0/0 | 4/0/0 | 1/0/0 |  |  |
| identifier:ifsc | 13 | 0 | 0 | 0 |  | 11/0/0 |  | 2/0/0 |  |
| identifier:in_pan | 2 | 0 | 0 | 0 |  |  |  | 2/0/0 |  |
| identifier:mrz | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| identifier:passport_number | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| links | 47 | 0 | 0 | 0 | 2/0/0 | 33/0/0 | 5/0/0 | 4/0/0 | 3/0/0 |
| statement_period | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_account | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_issuer | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| passport_holder | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_number | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_expiry | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_current | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| answer_latest_statement | 12 | 0 | 0 | 0 | 1/0/0 | 9/0/0 |  | 1/0/0 | 1/0/0 |
| answer_current_passport | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |

Wrong merges 0, wrong cards 0, wrong attachments 0, splits 0; failed: false.

### Directory off, text

| Measure | Right | Wrong | Left out | Impossible | de r/w/l | en r/w/l | fr r/w/l | hi r/w/l | ja r/w/l |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- | --- |
| mentions | 61 | 0 | 5 | 0 | 6/0/0 | 45/0/5 | 1/0/0 | 4/0/0 | 5/0/0 |
| org_directory | 41 | 0 | 0 | 0 |  |  |  |  |  |
| identifier:account | 24 | 0 | 0 | 0 |  | 19/0/0 | 1/0/0 | 2/0/0 | 2/0/0 |
| identifier:bic | 4 | 0 | 0 | 0 | 1/0/0 | 1/0/0 | 1/0/0 |  | 1/0/0 |
| identifier:card | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |
| identifier:fr_bank_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_branch_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_rib_key | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:gb_sort_code | 1 | 0 | 0 | 0 |  | 1/0/0 |  |  |  |
| identifier:iban | 7 | 0 | 0 | 0 | 2/0/0 | 4/0/0 | 1/0/0 |  |  |
| identifier:ifsc | 13 | 0 | 0 | 0 |  | 11/0/0 |  | 2/0/0 |  |
| identifier:in_pan | 2 | 0 | 0 | 0 |  |  |  | 2/0/0 |  |
| identifier:mrz | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| identifier:passport_number | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| links | 47 | 0 | 0 | 0 | 2/0/0 | 33/0/0 | 5/0/0 | 4/0/0 | 3/0/0 |
| statement_period | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_account | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_issuer | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| passport_holder | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_number | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_expiry | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_current | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| answer_latest_statement | 12 | 0 | 0 | 0 | 1/0/0 | 9/0/0 |  | 1/0/0 | 1/0/0 |
| answer_current_passport | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |

Wrong merges 0, wrong cards 0, wrong attachments 0, splits 9; failed: false.

### Directory on, text-ocr

| Measure | Right | Wrong | Left out | Impossible | de r/w/l | en r/w/l | fr r/w/l | hi r/w/l | ja r/w/l |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- | --- |
| mentions | 61 | 0 | 5 | 0 | 6/0/0 | 45/0/5 | 1/0/0 | 4/0/0 | 5/0/0 |
| org_directory | 31 | 0 | 1 | 0 |  |  |  |  |  |
| identifier:account | 24 | 0 | 0 | 0 |  | 19/0/0 | 1/0/0 | 2/0/0 | 2/0/0 |
| identifier:bic | 4 | 0 | 0 | 0 | 1/0/0 | 1/0/0 | 1/0/0 |  | 1/0/0 |
| identifier:card | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |
| identifier:fr_bank_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_branch_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_rib_key | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:gb_sort_code | 1 | 0 | 0 | 0 |  | 1/0/0 |  |  |  |
| identifier:iban | 7 | 0 | 0 | 0 | 2/0/0 | 4/0/0 | 1/0/0 |  |  |
| identifier:ifsc | 12 | 0 | 0 | 1 |  | 10/0/0 |  | 2/0/0 |  |
| identifier:in_pan | 2 | 0 | 0 | 0 |  |  |  | 2/0/0 |  |
| identifier:mrz | 4 | 0 | 0 | 1 |  | 4/0/0 |  |  |  |
| identifier:passport_number | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| links | 45 | 0 | 0 | 2 | 2/0/0 | 31/0/0 | 5/0/0 | 4/0/0 | 3/0/0 |
| statement_period | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_account | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_issuer | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| passport_holder | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_number | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_expiry | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_current | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| answer_latest_statement | 12 | 0 | 0 | 0 | 1/0/0 | 9/0/0 |  | 1/0/0 | 1/0/0 |
| answer_current_passport | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |

Wrong merges 0, wrong cards 0, wrong attachments 0, splits 0; failed: false.

### Directory off, text-ocr

| Measure | Right | Wrong | Left out | Impossible | de r/w/l | en r/w/l | fr r/w/l | hi r/w/l | ja r/w/l |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- | --- |
| mentions | 61 | 0 | 5 | 0 | 6/0/0 | 45/0/5 | 1/0/0 | 4/0/0 | 5/0/0 |
| org_directory | 40 | 0 | 0 | 0 |  |  |  |  |  |
| identifier:account | 24 | 0 | 0 | 0 |  | 19/0/0 | 1/0/0 | 2/0/0 | 2/0/0 |
| identifier:bic | 4 | 0 | 0 | 0 | 1/0/0 | 1/0/0 | 1/0/0 |  | 1/0/0 |
| identifier:card | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |
| identifier:fr_bank_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_branch_code | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:fr_rib_key | 1 | 0 | 0 | 0 |  |  | 1/0/0 |  |  |
| identifier:gb_sort_code | 1 | 0 | 0 | 0 |  | 1/0/0 |  |  |  |
| identifier:iban | 7 | 0 | 0 | 0 | 2/0/0 | 4/0/0 | 1/0/0 |  |  |
| identifier:ifsc | 12 | 0 | 0 | 1 |  | 10/0/0 |  | 2/0/0 |  |
| identifier:in_pan | 2 | 0 | 0 | 0 |  |  |  | 2/0/0 |  |
| identifier:mrz | 4 | 0 | 0 | 1 |  | 4/0/0 |  |  |  |
| identifier:passport_number | 5 | 0 | 0 | 0 |  | 5/0/0 |  |  |  |
| links | 45 | 0 | 0 | 2 | 2/0/0 | 31/0/0 | 5/0/0 | 4/0/0 | 3/0/0 |
| statement_period | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_account | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| statement_issuer | 14 | 0 | 0 | 0 | 1/0/0 | 11/0/0 |  | 1/0/0 | 1/0/0 |
| passport_holder | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_number | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_expiry | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| passport_current | 4 | 0 | 0 | 0 |  | 4/0/0 |  |  |  |
| answer_latest_statement | 12 | 0 | 0 | 0 | 1/0/0 | 9/0/0 |  | 1/0/0 | 1/0/0 |
| answer_current_passport | 3 | 0 | 0 | 0 |  | 3/0/0 |  |  |  |

Wrong merges 0, wrong cards 0, wrong attachments 0, splits 8; failed: false.

## Rules

Identifiers (`ids.rs`). Each is found by its shape, valid by its check, and named by where its institution sits.

- IBAN: country, two digits, then exactly the country's length (table of 12 countries; others are left out), spaces allowed between characters. Valid by ISO 7064 mod 97 and the length. Its bank code (per-country position) is its institution key.
- Card: four groups of four digits, or 13 to 19 digits on a line with the word "card". Valid by Luhn.
- IFSC: four letters, `0`, six letters or digits. Valid by shape. On a scanned page with an `IFSC` label, a read `O` or `Ø` in the fixed fifth place becomes `0`. Its first four letters are its institution key.
- BIC: six letters and two letters or digits, optional three more, only on a line that says BIC or SWIFT. Valid by shape. Its first six characters are its institution key.
- PAN: five letters, four digits, a letter, only on a line labelled PAN or पैन. Valid when the fourth letter is a holder status the Income Tax Department lists and the serial is not 0000.
- Sort code: `NN-NN-NN` on a line that says "sort code". Valid by shape.
- French bank code, branch code: five digits after `Code banque :` and `Code guichet :`. Valid by shape.
- Clé RIB: two digits after `Clé RIB :`. Valid when it equals the key recomputed from the bank code, branch code and account on its line; unchecked (`null`) when any is missing.
- Passport number: six to nine letters or digits after "Passport No." or "Passport number". No check.
- Machine-readable zone: two lines of 44 `[A-Z0-9<]`, the first starting `P` (passport) or `V` (visa). Valid when every check digit holds (TD3: number, birth, expiry, personal number, composite; MRV-A: the first three).
- Account: a number of at least six digits after an account label (`account`, `account no.`, `a/c`, `Konto`, `numéro de compte`, `口座`, `खाता`), with at most one short word between (`普通`). Also the long digit runs in the rows of a table whose header has an `Account` column. No check.

Organisation mentions (`orgs.rs`). A name is an organisation because of what it is built from or the field it fills; nothing is looked up to find it.

- Latin names: the run of capitalised words (and `of`, `&`, `de`) up to its last kind word (Bank, Union, Insurance, Corporation, Company, Office, School, Sparkasse), legal form (Ltd, Limited, GmbH, AG, Plc, N.V., Pvt) or word ending in `bank`, with `of …` after a kind word, legal forms after it, and the rest of the run after a Romance kind word (Banque, Banco, Compagnie). It needs one word that is not a kind or legal word. Settled by name; key is the directory's match key of the text.
- A token with digits breaks a run; a title-case word before a name set in capitals is dropped (`The HDFC BANK app`).
- Lower-case one-word compounds ending in `bank` (`hdfcbank`) are names.
- Devanagari: up to four words before बैंक or लिमिटेड back to the last delimiter, with `ऑफ़ …` after बैंक.
- Japanese: kanji, katakana and Latin capitals before 銀行 (or 信用金庫, 信用組合), plus a leading kana run when it starts at a delimiter. 銀行 followed by a colon is a label.
- Field values: the comma-separated values after a label holding a bank word (`Bank:`, `Banks involved …:`, `Banque :`, `Kreditinstitut:`, `…銀行:`). Settled by name, except one bare word (`Bank: ZB`), which could be any of several banks.
- By code: a four-capital word that is the first four letters of exactly one IFSC or BIC in the document takes that code's key (settled by code).
- By mention: a run of two or more words that starts exactly one longer mention's words, or a one-word form (with or without a legal word) that is the initials of exactly one longer mention, takes that mention's key (settled by mention). A form that fits two organisations stays unsettled and is not reported.

Links (`doc.rs`). A link is written only with a reason the README names.

- `ifsc-prefix`, `bic-institution`, `iban-bank-code`: a valid code to its own institution key. With the directory on, only when some directory entry holds the code (the README links a code only when its code names an entry); with it off, the code itself is the evidence.
- `paired-code`: an account or IBAN to the code on its line when the line holds one code and one account, else to the one code on the line above or below when that line holds no account and the account's line holds no code. Two candidates give no link.
- `inside-iban`: a sort code, French bank or branch code that is that part of a valid IBAN in the document.
- `same-account`: an account that is the account part of a valid IBAN in the document.
- `statement-header`: the statement's account to the first organisation the document names.
- An identifier that fails its check is linked nowhere.

Statements, passports, keys and answers.

- Statement: a line-start period field (`Statement period`, `Period`, `Zeitraum`, `Période`, `अवधि`, `期間`) holding a whole date range. The accounts are the account, IBAN and card identifiers above it when they spell one account (one value, or an IBAN and its domestic part), and their repeats; otherwise none. Numeric dates where both orders make a date are read day first only outside English documents.
- Passport: a `P` zone, or a passport number with a printed expiry. Holder from the Surname and Given names fields, else the zone. Expiry: the zone's when its check digits hold and it agrees with the printed date; `cannot-tell` when they disagree or the zone fails its check; the printed date when there is no zone. Current when every expiry the page prints (the failed zone's included) is on or after the reference date, not current when all are before it or the page says CANCELLED, `null` otherwise. On a scanned page a field label may have one misread character, its colon included.
- Organisation keys: raw keys join when the document ties them. An IBAN joins the BIC it is paired with, and a statement's issuer joins the codes its own account is linked through. Two different codes of one scheme never join. With the directory on, a group takes the one entry its codes and spellings name exactly (none when they name two), and groups naming the same entry are one. A card's name is a mention's text, from a clean page and settled by name first.
- Latest statement: statements that share any spelling are one account; the one with the latest period end, with the IBAN as the spelling when there is one. Two that end the same day leave the account out.
- Current passport: per holder, the one current passport; two leave the holder out.

## What is left out, and why

- Names with no word that says they are an organisation. These are `Example Cafe` and `Example Books` (merchants), `Sample Grocers`, and `Sample Traders` (twice, a transfer party). That is 5 mentions in every run. Only a list of names would find them, and the rules may not hold one.
- The directory id of `Deutschen Bundesbank` (directory-on runs, 1 `org_directory`). The inflected spelling's match key is in no entry, and the rules do no fuzzy matching.
- OCR (text-ocr runs, counted as impossible, not left out). The PNB IFSC is read as `IFSCEPUNBOSHMPLT`, so the IFSC and both of its links are impossible. The expired passport's zone is unreadable. Its holder, number and expiry still come from the printed fields.
- Splits, not wrong, in the directory-off runs (9 on text, 8 on text-ocr). The same bank appears under a name key and a code key that no in-document reason ties (`ICICI Bank` beside `ICIC0…` in a row), or in another script (`State Bank of India`, `स्टेट बैंक ऑफ़ इंडिया`). The directory-on runs have none.

## Size, dependencies, time

- Lines (by `wc -l` after rustfmt; tests are the `#[cfg(test)]` modules): `ids.rs` 513 code + 149 tests, `orgs.rs` 487 + 121, `mod.rs` 464 + 120, `doc.rs` 369 + 153, `dates.rs` 225 + 74, binary 61. Total 2,119 lines of code and 617 of tests in 37 unit tests. That is more than "a few hundred lines" of rules. The per-scheme identifier finders, the three name finders (Latin, Devanagari, Japanese) and the cross-document keys are most of it.
- Dependencies: only `regex`, `serde`, `serde_json` and `anyhow`, which the crate already had. No new crate.
- Time: the binary measures 11 to 17 ms to extract all 43 documents in one run (release build, Apple Silicon, three runs). The whole process takes 20 to 30 ms wall time, including reading the directory. Most of that time is the first use of each regex, which compiles it.

## Rust: easy and hard

- Easy. Unicode-aware `regex` classes (`\p{L}`, word boundaries in every script). One enum of identifier kinds with serde names, matched exhaustively. Lazily built static regexes. The speed, which needed no tuning.
- Hard. Spans count Unicode scalar values while `regex` gives byte offsets, so every match is converted and the code has to keep track of which offset is which. The standard library has no NFKC and the crate has no `unicode-normalization`, so the match key folds only full-width ASCII. A compatibility form outside that range gives a different key, and the mention is left out, not wrong. The union-find over keys, mutated inside iterator closures, had to be restructured to satisfy the borrow checker.

## The macOS on-device detectors

Not tried. The NaturalLanguage name tagger (`NLTagger` with `.nameType`, which tags organisation names) is in `objc2-natural-language`. That crate is not in `Cargo.toml` or `Cargo.lock`, so using it is a new dependency, which the bounds rule out. `NSDataDetector` is in Foundation, but the pinned `objc2-foundation` has its `NSDataDetector` and `NSTextCheckingResult` features off, and turning them on changes `Cargo.toml` beyond registering the module. It also detects dates, addresses, links and phone numbers, not organisations or account numbers. Here it could only confirm the phone numbers the account rules already skip, because they require a label. The tagger is the one detector that could help with the left-out group above (names with no organisation word). The open question is whether it tags `Sample Traders` or `Example Cafe` as organisations without also tagging people and places, and only a run can answer it. On the evidence here, its likely gain is about five mentions, which carry no link.

## Where FORMAT.md or the scorer is unclear

- Code links with the directory off. The README defines `ifsc-prefix`, `bic-institution` and `iban-bank-code` by the bank's own codes, and links "every valid IBAN, BIC or IFSC whose code names a directory entry". A directory-off run cannot know whether a code names an entry. So a code of a bank in no directory would be linked in an off run and, by the labels, wrongly. I link every valid code with the directory off and only codes the directory holds with it on. All codes in the sample are real, so the sample cannot tell the two readings apart.
- `passport.current` when the expiry is `cannot-tell`. FORMAT does not give its type. The labels score `passport-bad-mrz` as current, which matches "every candidate is after the reference date". I write `null` when candidates disagree.
- `statement.accounts`. FORMAT does not say whether every spelling, such as a domestic account beside the IBAN, must be listed. I list all of them, and the scorer accepts that.
- `settled_by`. FORMAT says only that `directory` in an off run is wrong. Whether `name`, `mention` and `code` are compared with the label's evidence is not stated.
- Numeric dates such as `01-08-2026` in an English document. Neither file says which order to assume. I leave a range out when both orders make dates and nothing in it forces one.
