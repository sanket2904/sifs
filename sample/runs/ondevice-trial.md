# Trial: Apple's on-device model as a second reader

This is a measurement, not a feature. No app code changes.

The goal is that typing `HDFC` shows a card for that organisation, with the accounts held there and the documents that mention it, each with its source, and no wrong link. A rules-only extractor scored no wrong row on the 43 sample documents and then made wrong links and answers on 36 documents it had never seen. In each case a matched pattern was taken as the document saying so. The rule is now that nothing leaves the Mac. This trial asks whether the reader Apple ships on the Mac should get a job beside the rules, and which job.

Machine: macOS 26.4.1 (25E253), Apple M5 Pro. All documents are made up. "Sample" means the 43 documents in this folder. "Outside" means the two review sets written by outside reviewers (sol: 23 documents, astra: 18), which are not in this repository. Only their counts and document names appear here.

## What Apple has on this Mac

| Tool | What it is | Limits measured here |
| --- | --- | --- |
| Foundation Models (`SystemLanguageModel`) | The language model behind Apple Intelligence, running on the Mac. Guided generation fills a Swift type, such as an enum. | Available. Context 4,096 tokens. 23 languages. Hindi is not one of them, so every Hindi page fails at once with `unsupportedLanguageOrLocale`. It refused 3 of 91 pages (two insurance pages and a passport-intake form), and whether it refuses depends on harmless wording in the instructions. |
| `NLTagger`, name type | Tags person, place and organisation names, with no language model. | Finds known big names. Misses invented ones. Tags words such as `Canada`, `Invoice`, `IBAN` and `BIC` as organisations. |
| `NSDataDetector` | Finds dates, addresses and phone numbers. | Not measured. The rules already read these fields. |

## T1. The name tagger (no model)

An organisation mention counts as found when an organisation tag overlaps its labelled span. The document's language is passed to the tagger. Letting the tagger guess the language changes little (see the JSON).

| Set | Mentions | Found | Organisation tags | On a mention | On a labelled not-organisation | Not an organisation |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Sample | 66 | 42 | 74 | 42 | 3 | 29 |
| Sol (probe, diag, localised) | 14 | 4 | 24 | 4 | 2 | 18 |
| Astra | 11 | 0 | 12 | 0 | 1 | 11 |

By language on the sample, it found 37 of 50 English, 5 of 6 German, 0 of 1 French, 0 of 4 Hindi and 0 of 5 Japanese mentions. Of the five merchants that the rules miss, it found one (`Sample Grocers`). It missed `Example Books`, `Example Cafe` and both `Sample Traders`. On the knitting sheet (`ch-stitch-code`) it tagged `UBS` as an organisation both times. That is the wrong attachment the rules made.

## T2. The model reads what kind of page this is

There is one closed question per page. The model answers with one of nine kinds and quotes a line of the page. The program checks that the quote is on the page. If it is not, the answer becomes "cannot tell". I wrote the prompt on the sample in three tries, then ran it once on the outside sets. A document reads as a statement or a passport when any of its pages does.

| Labels say | Sample (prompt written here) | Outside (one run) |
| --- | --- | --- |
| Statement | 13 of 14 read as statement, 1 error (Hindi) | 10 of 10 |
| Passport | 4 of 4 | 9 of 9 |
| Looks like a passport, is not | 1 of 1 read as visa | 1 of 1 read as visa |
| Neither | 24 documents: 1 read as statement (`scan-pnb-deposit`), 1 as passport (`rib-fr`), 5 errors, 17 other kinds | 21 documents: 5 read as statement, 7 cannot tell, 2 errors, 7 other kinds |

Out of 15 outside "statement" answers, 5 were wrong (`quebec-credit`, `india-outbound`, `spain-sandbox`, `swiss-code-only`, `madrid-press`). A quote on the page proves only that the line exists. It does not prove that the line shows the kind.

On the pages where the rules went wrong, the model read the following. `explicitly-not-statement` read as form, and `eligibility-audit` as form. `service-invoice` read as invoice, and `visa-not-passport` as visa. `ch-stitch-code` read as form. For `alaska-forms`, `number-decoys` and `bare-short-name` it could not tell. It read `swiss-code-only` as a statement, which is wrong. It read `two-live-previous`, `passport-void` and `ca-oct` as passport, passport and statement. Those kinds are right, but those rule errors are about whether a passport is current and about an account number grouped with spaces, which the page's kind does not settle. `devanagari-date-proof` gave an error because Hindi is unsupported.

## T3. The model as a second opinion on each rule claim

For each link, statement, passport and organisation mention that the rules emitted on the outside sets, the model answers yes, no or cannot tell, with a checked quote. It may only remove a row. A "no" with its quote on the page removes the row. Anything else keeps it. Whether a row is right or wrong is the scorer's verdict for that run.

| Rules run | Wrong rows | Wrong removed | Right rows | Right removed | Not judged (wrong / right) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Go, directory off | 6 | 0 | 47 | 0 | 5 / 11 |
| Go, directory on | 5 | 0 | 48 | 0 | 5 / 10 |
| Rust, directory off | 9 | 1 | 49 | 0 | 8 / 10 |
| Rust, directory on | 7 | 1 | 49 | 0 | 6 / 9 |
| Go on the sample (prompt written here) | 0 | 0 | 125 | 1 | 0 / 38 |

The model almost never says "no". On the "Not a statement" worksheet it answered "cannot tell" while quoting the line that says it is not a statement. It said yes to `LUM` being an organisation. It said yes to `UBS` on the knitting sheet, and it quoted the line with `ü` written as `ue`, so the quote check failed. The only removal was the visa read as a passport (Rust). A stricter policy that keeps a row only on a checked "yes" would remove 5 of Go's 6 wrong rows, and also 11 of its 47 right rows (directory off; with it on, 5 of 5 and 10 of 48).

The answers depend on how the question is put. With the answer field before the quote and a sentence that said when to answer no, the model said no to 100 of the 125 right sample rows. Putting the quote first and dropping that sentence brought it down to 1 of 125.

Combined, without new model calls: a statement or passport claim is dropped when T2 read the page as another kind, and so is a link whose only reason is that statement's header. Outside, this removes 2 of Go's 6 wrong rows with the directory off and 2 of its 5 with it on (the worksheet's link and statement) and 7 of Rust's 9 (off) and 7 of 7 (on), with 0 of 47 to 49 right rows removed. On the sample it removes 0 of 125 right rows. I chose this rule after seeing the outside results, so it is a hypothesis to test on new documents, not a held-out result.

## T4. Cost and repeatability

| Measure | Value |
| --- | --- |
| Seconds per question | median 0.56 (T2, 83 answered pages) and 0.50 (T3, 208 claims). The slowest was 1.82 s, on a passport page. |
| Pages that did not fit 4,096 tokens | None. The largest prompt was 737 tokens. The tool's rule for a page that does not fit is to drop the page line farthest from the claim's line (or from the first line) until it fits. It never fired. |
| Same page asked 5 times, greedy sampling | The same answer 5 of 5 times on all 13 pages tried (4 of them Hindi, which fail the same way each time). |
| Same page asked 5 times, default sampling | It changed on 3 of 13 pages (`rib-fr`, `essay-ja`, `explicitly-not-statement`). All runs above use greedy sampling. |
| Memory | 271 MB peak for the script process, mostly the Swift interpreter. While it ran, the inference service `TGOnDeviceInferenceProviderService` held 188 MB and 224 MB resident in two processes (`ps`). The model weights' own memory is not visible without root. |
| Languages | Japanese, German and French pages get answers. The Japanese statement, essay and remittance notice read as statement, other and letter. The German and French statements read as statements, and the French bank-details page (`rib-fr`) read as a passport. Hindi fails on every call: 5 pages in T2 and 8 claims in T3, 0 answers. |

## Nothing left the Mac

The code imports only Foundation, FoundationModels and NaturalLanguage, and makes no network call of its own. `SystemLanguageModel` is the on-device model. This API has no option to use Private Cloud Compute. During the T4 run, `lsof -i` showed no socket open in the script process, in `TGOnDeviceInferenceProviderService`, `modelmanagerd`, `generativeexperiencesd` or the safety inference provider. None of them appeared in `nettop`'s list of processes with network traffic. I did not switch the network off, because changing settings was out of scope.

## Recommendation

Give the name tagger no job. It found 4 of 25 organisations outside the sample, and it tags countries and field words as organisations. Do not let the model veto claims one by one: it removed 0 of the Go extractor's wrong rows. The one job worth trying next is "the model reads the page's kind, and a statement or passport claim on a page it reads as another kind is dropped". That removed the worksheet error and the Rust invoice and visa errors, at no cost to right rows here.

The numbers are too small to carry this. There were 6 wrong Go rows and 9 wrong Rust rows with the directory off (5 and 7 with it on), on 41 outside documents. The combined rule was picked after seeing them. And the model read 5 of 21 outside non-statements as statements, so it can also miss a page that the rules get right. The answer would change with a few hundred new labelled pages run once under the combined rule, with Hindi support from Apple (a Hindi page gets no second reader today), and with a fix for refusals, which drop pages without warning.

## The clean test, run afterwards

The write-up above asked for a test on new documents. It was run afterwards by an outside tester. The kind reader and the drop rule were exactly as committed here, run once. They ran on two further sets of 28 made-up documents each (56 documents, 57 pages). Outside reviewers wrote these sets after the Go extractor was frozen. Neither the model's prompt nor the rule had been tried on them.

| Set | Directory | Wrong rows before | Wrong rows removed | Right rows before | Right rows removed |
| --- | --- | ---: | ---: | ---: | ---: |
| A | on | 10 | 0 | 101 | 0 |
| A | off | 12 | 0 | 101 | 0 |
| B | on | 25 | 4 | 99 | 0 |
| B | off | 25 | 4 | 101 | 0 |

All four removed rows come from one document. It is a service invoice that the rules had read as a statement. Its wrong "latest statement" answer stayed, because the rule does not reach answers.

The reader gave no usable answer on 7 of 28 pages of set A and 15 of 29 pages of set B. The causes, for set A and set B:

- Language not supported: 1 and 6.
- The model declined, or a guardrail stopped it: 1 and 6.
- The quote was not on the page: 5 and 2.
- The model itself said it cannot tell: 0 and 1.

All eight labelled passports got no usable answer.

The reader also read as a statement 5 of 11 pages of set A and 2 of 13 pages of set B that are neither a statement nor a passport. It read one residence permit as a passport.

Some errors page kind cannot settle, whatever the reader's quality:

- Which of two named parties is the issuer and which is the customer.
- Sender against receiver in a transfer.
- Two organisations merged into one.
- A passport that a later line cancels.
- A number cut short.

Conclusion. On new documents the page-kind rule is a narrow filter. It removed 8 of 72 wrong rows across the four runs and no right row. It does not bring the rule-based extractor to "no wrong row". With this reader it should not be given that job. The directory-on and directory-off runs are the same documents twice, so they are not independent evidence.

## Run it

macOS 26.4 or later, with Apple Intelligence on. These scripts run by hand on a Mac only. The hosted CI runners cannot run the on-device model, so CI does not run them. `DIR` is the folder holding the two outside sets and the rule extractors' results on them.

```sh
swift sample/tools/ondevice/availability.swift                            # T0
swift sample/tools/ondevice/trial.swift tagger --xproto DIR               # T1
swift sample/tools/ondevice/trial.swift kind --sets sample                # T2, prompt written here
swift sample/tools/ondevice/trial.swift kind --sets outside --xproto DIR  # T2, run once
mkdir -p /tmp/go-sample && for f in go-m1a-off-text go-m1a-on-text; do
  git show origin/m1a-extract-go:sample/runs/$f.json > /tmp/go-sample/$f.json
  git show origin/m1a-extract-go:sample/runs/$f.score.json > /tmp/go-sample/$f.score.json; done
swift sample/tools/ondevice/trial.swift veto --sets sample --sample-rules /tmp/go-sample  # T3
swift sample/tools/ondevice/trial.swift veto --sets outside --xproto DIR                  # T3, run once
swift sample/tools/ondevice/trial.swift combine --sets outside --xproto DIR               # T2 with T3
swift sample/tools/ondevice/trial.swift repeat --xproto DIR                               # T4
swift sample/tools/ondevice/trial.swift redact   # strips outside-set text from the JSON before commit
```

Raw results are in `sample/runs/ondevice-*.json`. In the outside files, quotes, tagged words and claim keys are redacted, so that no text from those documents enters this repository.
