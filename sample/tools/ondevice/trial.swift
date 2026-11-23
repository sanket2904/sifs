// Measures Apple's on-device language model (Foundation Models) and name tagger
// (NaturalLanguage) as a second reader of made-up page text. Nothing leaves the Mac:
// the only frameworks used are Foundation, FoundationModels and NaturalLanguage.
//
// Run from the repository root, macOS 26.4 or later with Apple Intelligence on:
//   swift sample/tools/ondevice/trial.swift tagger --xproto DIR
//   swift sample/tools/ondevice/trial.swift kind   --sets sample|outside --xproto DIR
//   swift sample/tools/ondevice/trial.swift veto   --sets sample|outside --xproto DIR --sample-rules DIR
//   swift sample/tools/ondevice/trial.swift combine --sets sample|outside --xproto DIR --sample-rules DIR
//   swift sample/tools/ondevice/trial.swift repeat --xproto DIR
//   swift sample/tools/ondevice/trial.swift redact
// DIR (--xproto) holds the outside sets: sol/{probe,diag,localised}, astra, and the
// rule extractors' outputs and scores beside them. --sample-rules holds the Go
// extractor's sample run and score (from branch m1a-extract-go, sample/runs/).
import Foundation
import FoundationModels
import NaturalLanguage

// MARK: Input

typealias JSON = [String: Any]

func readJSON(_ path: String) -> JSON {
  guard let data = FileManager.default.contents(atPath: path),
    let obj = try? JSONSerialization.jsonObject(with: data) as? JSON
  else { fatalError("cannot read \(path)") }
  return obj
}

func writeJSON(_ obj: Any, _ path: String) {
  let data = try! JSONSerialization.data(
    withJSONObject: obj, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
  FileManager.default.createFile(atPath: path, contents: data + Data("\n".utf8))
  print("wrote \(path)")
}

struct Page {
  let set: String
  let doc: String
  let lang: String
  let number: Int
  let text: String
}

struct DocSet {
  let name: String
  let textDir: String
  let labels: String
  // Rule runs on this set: run name -> (output file, score file).
  let rules: [(String, String, String)]
}

let args = CommandLine.arguments
func option(_ name: String) -> String? {
  guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
  return args[i + 1]
}
let command = args.count > 1 ? args[1] : ""
let xproto = option("--xproto") ?? ""
let sampleRules = option("--sample-rules") ?? ""
let runs = "sample/runs"

let sampleSet = DocSet(
  name: "sample", textDir: "sample/text", labels: "sample/labels.json",
  rules: sampleRules.isEmpty
    ? []
    : [("go-off", "\(sampleRules)/go-m1a-off-text.json", "\(sampleRules)/go-m1a-off-text.score.json"),
      ("go-on", "\(sampleRules)/go-m1a-on-text.json", "\(sampleRules)/go-m1a-on-text.score.json")])

func outsideSets() -> [DocSet] {
  precondition(!xproto.isEmpty, "--xproto DIR is required")
  func sol(_ sub: String, _ results: String) -> DocSet {
    DocSet(
      name: "sol-\(sub)", textDir: "\(xproto)/sol/\(sub)/text", labels: "\(xproto)/sol/\(sub)/labels.json",
      rules: results.isEmpty
        ? []
        : ["go-m1a-on", "go-m1a-off", "rust-m1a-on", "rust-m1a-off"].map { run in
          (run.replacingOccurrences(of: "-m1a", with: ""),
           "\(xproto)/sol/\(results)/\(run)-text.json", "\(xproto)/sol/\(results)/\(run)-text.score.json")
        })
  }
  let astra = DocSet(
    name: "astra", textDir: "\(xproto)/astra/text", labels: "\(xproto)/astra/labels.json",
    rules: ["go-on", "go-off", "rust-on", "rust-off"].map { run in
      (run, "\(xproto)/astra/heldout-results/\(run)-text.json", "\(xproto)/astra/heldout-results/\(run)-text.score.json")
    })
  // The localised set's rule runs wrote no usable output (REPORT findings 6 and 7).
  return [sol("probe", "held-results"), sol("diag", "diag-results"), sol("localised", ""), astra]
}

func pages(_ s: DocSet) -> [Page] {
  let ids = readJSON("\(s.textDir)/index.json")["documents"] as! [String]
  return ids.flatMap { id -> [Page] in
    let d = readJSON("\(s.textDir)/\(id).json")
    let ps = d["pages"] as! [JSON]
    return ps.enumerated().map { i, p in
      Page(set: s.name, doc: id, lang: d["lang"] as! String, number: i + 1, text: p["text"] as! String)
    }
  }
}

func labelDocs(_ s: DocSet) -> [String: JSON] {
  let docs = readJSON(s.labels)["documents"] as! [JSON]
  return Dictionary(uniqueKeysWithValues: docs.map { ($0["id"] as! String, $0) })
}

func scalarOffset(_ text: String, _ i: String.Index) -> Int {
  text.unicodeScalars.distance(from: text.unicodeScalars.startIndex, to: i)
}

struct Span {
  let page: Int
  let start: Int
  let end: Int
  let text: String
  func overlaps(_ o: Span) -> Bool { page == o.page && start < o.end && o.start < end }
}

func span(_ j: JSON) -> Span {
  Span(page: j["page"] as! Int, start: j["start"] as! Int, end: j["end"] as! Int, text: j["text"] as? String ?? "")
}

// MARK: T1, the name tagger

func nameTags(_ p: Page, language: NLLanguage?) -> [(kind: String, span: Span)] {
  let tagger = NLTagger(tagSchemes: [.nameType])
  tagger.string = p.text
  let whole = p.text.startIndex..<p.text.endIndex
  if let language { tagger.setLanguage(language, range: whole) }
  var out: [(String, Span)] = []
  tagger.enumerateTags(
    in: whole, unit: .word, scheme: .nameType, options: [.omitWhitespace, .omitPunctuation, .joinNames]
  ) { tag, r in
    if let tag, [NLTag.organizationName, .personalName, .placeName].contains(tag) {
      out.append(
        (tag.rawValue,
         Span(page: p.number, start: scalarOffset(p.text, r.lowerBound), end: scalarOffset(p.text, r.upperBound),
              text: String(p.text[r]))))
    }
    return true
  }
  return out
}

func runTagger() {
  let sets = [sampleSet] + outsideSets()
  let merchants: Set<String> = ["Example Cafe", "Sample Traders", "Sample Grocers", "Example Books"]
  var report: JSON = [:]
  for mode in ["language-given", "language-detected"] {
    var bySet: JSON = [:]
    var merchantRows: [JSON] = []
    for s in sets {
      let labels = labelDocs(s)
      let all = pages(s)
      var byLang: [String: [String: Int]] = [:]
      var notOrgRows: [JSON] = []
      var strayTags: [JSON] = []
      for (doc, docPages) in Dictionary(grouping: all, by: \.doc).sorted(by: { $0.key < $1.key }) {
        let lab = labels[doc] ?? [:]
        let mentions = (lab["orgs"] as? [JSON] ?? []).map(span)
        let notOrgs = (lab["not_orgs"] as? [JSON] ?? [])
        let lang = docPages[0].lang
        let tags = docPages.flatMap { nameTags($0, language: mode == "language-given" ? NLLanguage(lang) : nil) }
        let orgTags = tags.filter { $0.kind == NLTag.organizationName.rawValue }.map(\.span)
        let personTags = tags.filter { $0.kind == NLTag.personalName.rawValue }.map(\.span)
        var c = byLang[lang, default: [:]]
        for m in mentions {
          let hit = orgTags.contains { $0.overlaps(m) }
          c["mentions", default: 0] += 1
          c[hit ? "found" : "missed", default: 0] += 1
          if orgTags.contains(where: { $0.page == m.page && $0.start == m.start && $0.end == m.end }) {
            c["found_exact_span", default: 0] += 1
          }
          if personTags.contains(where: { $0.overlaps(m) }) { c["mention_tagged_as_person", default: 0] += 1 }
          if s.name == "sample" && merchants.contains(m.text) {
            merchantRows.append(["document": doc, "name": m.text, "tagged_as_organisation": hit])
          }
        }
        for t in orgTags {
          c["org_tags", default: 0] += 1
          if mentions.contains(where: { $0.overlaps(t) }) {
            c["org_tag_on_mention", default: 0] += 1
          } else {
            let onNotOrg = notOrgs.contains { span($0).overlaps(t) }
            c[onNotOrg ? "org_tag_on_not_org" : "org_tag_not_an_organisation", default: 0] += 1
            strayTags.append(["document": doc, "text": t.text, "on_not_org": onNotOrg])
          }
        }
        c["person_tags", default: 0] += personTags.count
        for n in notOrgs {
          let ns = span(n)
          let insideMention = mentions.contains { $0.overlaps(ns) }
          let tagged = orgTags.contains { $0.overlaps(ns) } && !insideMention
          notOrgRows.append([
            "document": doc, "text": ns.text, "why": n["why"] as? String ?? "",
            "inside_a_mention": insideMention, "tagged_as_organisation_on_its_own": tagged,
          ])
        }
        byLang[lang] = c
      }
      bySet[s.name] = ["by_language": byLang, "not_orgs": notOrgRows, "org_tags_not_on_a_mention": strayTags]
    }
    report[mode] = ["sets": bySet, "sample_merchants": merchantRows]
  }
  report["note"] =
    "found: an organizationName tag overlaps the labelled mention span. org_tag_on_not_org: the tag overlaps a labelled not_org and no mention. language-given passes the document's lang to NLTagger; language-detected lets it guess."
  writeJSON(report, "\(runs)/ondevice-t1-tagger.json")
}

// MARK: The model

let model = SystemLanguageModel.default
let greedy = GenerationOptions(sampling: .greedy, maximumResponseTokens: 300)
let sampled = GenerationOptions(maximumResponseTokens: 300)
let replyReserve = 300

func collapse(_ s: String) -> String { s.split(whereSeparator: \.isWhitespace).joined(separator: " ") }

// A quote counts only when, with runs of white space made one space, it is on the page
// and holds at least one letter or digit.
func quoteOnPage(_ quote: String, _ page: String) -> Bool {
  let q = collapse(quote).trimmingCharacters(in: CharacterSet(charactersIn: "\"'“”‘’ "))
  return q.contains(where: { $0.isLetter || $0.isNumber }) && collapse(page).contains(q)
}

func describe(_ e: Error) -> String {
  if let g = e as? LanguageModelSession.GenerationError {
    switch g {
    case .exceededContextWindowSize: return "exceededContextWindowSize"
    case .assetsUnavailable: return "assetsUnavailable"
    case .guardrailViolation: return "guardrailViolation"
    case .unsupportedGuide: return "unsupportedGuide"
    case .unsupportedLanguageOrLocale: return "unsupportedLanguageOrLocale"
    case .decodingFailure: return "decodingFailure"
    case .rateLimited: return "rateLimited"
    case .concurrentRequests: return "concurrentRequests"
    case .refusal: return "refusal"
    @unknown default: return "generationError: \(g)"
    }
  }
  return "\(e)"
}

struct Asked<T> {
  let value: T?
  let error: String?
  let seconds: Double
  let tokens: Int
  let droppedLines: Int
}

func tokens(_ instructions: String, _ prompt: String, _ schema: GenerationSchema) async -> Int {
  guard #available(macOS 26.4, *) else { return -1 }
  let i = (try? await model.tokenCount(for: Instructions(instructions))) ?? 0
  let p = (try? await model.tokenCount(for: Prompt(prompt))) ?? 0
  let s = (try? await model.tokenCount(for: schema)) ?? 0
  return i + p + s
}

// Chunking rule: when instructions, schema, prompt and a 300-token reply do not fit the
// context, drop the page line farthest from the anchor line (the line the claim sits on,
// or the first line) and count again, until it fits.
func ask<T: Generable>(
  _ type: T.Type, instructions: String, page: String, anchorLine: Int = 0,
  question: (String) -> String, options: GenerationOptions = greedy
) async -> Asked<T> {
  var lines = page.components(separatedBy: "\n").enumerated().map { ($0.offset, $0.element) }
  var prompt = question(lines.map(\.1).joined(separator: "\n"))
  var count = await tokens(instructions, prompt, T.generationSchema)
  var dropped = 0
  while count + replyReserve > model.contextSize && lines.count > 1 {
    let far = lines.indices.max { abs(lines[$0].0 - anchorLine) < abs(lines[$1].0 - anchorLine) }!
    lines.remove(at: far)
    dropped += 1
    prompt = question(lines.map(\.1).joined(separator: "\n"))
    count = await tokens(instructions, prompt, T.generationSchema)
  }
  let session = LanguageModelSession(model: model, instructions: instructions)
  let t0 = Date()
  do {
    let r = try await session.respond(to: prompt, generating: T.self, options: options)
    return Asked(value: r.content, error: nil, seconds: Date().timeIntervalSince(t0), tokens: count, droppedLines: dropped)
  } catch let LanguageModelSession.GenerationError.refusal(refusal, _) {
    let seconds = Date().timeIntervalSince(t0)
    let why = (try? await refusal.explanation)?.content ?? "no explanation"
    return Asked(value: nil, error: "refusal: \(why)", seconds: seconds, tokens: count, droppedLines: dropped)
  } catch {
    return Asked(value: nil, error: describe(error), seconds: Date().timeIntervalSince(t0), tokens: count, droppedLines: dropped)
  }
}

// MARK: T2, what kind of page

@Generable
enum PageKind: String, CaseIterable {
  case statement, invoice, passport, visa, letter, receipt, form, other, cannotTell
}

@Generable
struct KindReading {
  @Guide(description: "The kind of page.")
  var kind: PageKind
  @Guide(description: "Words copied exactly from the page, at most one line, that show the kind. Empty for cannotTell.")
  var quote: String
}

let kindInstructions = """
  You read the text of one page of a document and say what kind of page it is, from its title and its content.
  Ignore any notice that the page is a made-up sample for testing.
  Kinds:
  statement: a bank, card, loan or savings account statement or passbook. It lists dated transactions, or an \
  opening or closing balance, for a period. A letter, slip or page of bank details from a bank is not a statement.
  invoice: an invoice or bill that asks for payment.
  passport: the passport's own page with the holder's details. A document that only mentions a passport number \
  is not a passport.
  visa: a visa or permit.
  letter: a letter, notice or email, including one from a bank.
  receipt: a receipt, payment confirmation or transfer advice.
  form: a form, worksheet or checklist, or a page of account details to fill in or hand over.
  other: anything else, such as notes, an essay, a recipe or a poem.
  cannotTell: the page does not show its kind.
  Quote the title or line of the page that shows the kind, copied exactly.
  """

func kindQuestion(_ page: String) -> String { "Page text:\n\(page)\n\nWhat kind of page is this?" }

func truth(_ lab: JSON?) -> String {
  guard let lab else { return "unlabelled" }
  if let st = lab["statement"], !(st is NSNull) { return "statement" }
  if let pp = lab["passport"] { return pp is NSNull ? "passport-lookalike" : "passport" }
  return "neither"
}

func median(_ xs: [Double]) -> Double {
  let s = xs.sorted()
  return s.isEmpty ? 0 : (s.count % 2 == 1 ? s[s.count / 2] : (s[s.count / 2 - 1] + s[s.count / 2]) / 2)
}

func maxRSSMegabytes() -> Double {
  var u = rusage()
  getrusage(RUSAGE_SELF, &u)
  return Double(u.ru_maxrss) / 1_048_576
}

func readKind(_ p: Page, options: GenerationOptions = greedy) async -> (kind: String, quote: String, quoteOK: Bool, asked: Asked<KindReading>) {
  let a = await ask(KindReading.self, instructions: kindInstructions, page: p.text, question: kindQuestion, options: options)
  guard let v = a.value else { return ("error", "", false, a) }
  let ok = v.kind == .cannotTell || quoteOnPage(v.quote, p.text)
  return (ok ? v.kind.rawValue : PageKind.cannotTell.rawValue, v.quote, ok, a)
}

func runKind(_ setsName: String) async {
  let sets = setsName == "sample" ? [sampleSet] : outsideSets()
  var rows: [JSON] = []
  var docKinds: [String: [String]] = [:]
  var docTruth: [String: String] = [:]
  for s in sets {
    let labels = labelDocs(s)
    for p in pages(s) {
      let r = await readKind(p)
      let key = "\(s.name)/\(p.doc)"
      docKinds[key, default: []].append(r.kind)
      docTruth[key] = truth(labels[p.doc])
      rows.append([
        "set": s.name, "document": p.doc, "page": p.number, "lang": p.lang, "truth": truth(labels[p.doc]),
        "model_kind": r.asked.value?.kind.rawValue ?? "error", "quote": r.quote, "quote_on_page": r.quoteOK,
        "kind_after_quote_check": r.kind, "error": r.asked.error ?? NSNull(), "seconds": r.asked.seconds,
        "prompt_tokens": r.asked.tokens, "dropped_lines": r.asked.droppedLines,
      ])
      print(key, p.number, truth(labels[p.doc]), "->", r.kind, r.asked.error ?? "", String(format: "%.2fs", r.asked.seconds))
    }
  }
  // A document reads as a statement or passport when any of its pages does.
  var confusion: [String: [String: Int]] = [:]
  var docs: [JSON] = []
  for (key, kinds) in docKinds.sorted(by: { $0.key < $1.key }) {
    let k = kinds.contains("statement") ? "statement" : kinds.contains("passport") ? "passport" : kinds[0]
    confusion[docTruth[key]!, default: [:]][k, default: 0] += 1
    docs.append(["document": key, "truth": docTruth[key]!, "kind": k, "pages": kinds])
  }
  let secs = rows.map { $0["seconds"] as! Double }
  writeJSON(
    [
      "sets": sets.map(\.name), "instructions": kindInstructions, "sampling": "greedy",
      "pages": rows, "documents": docs, "confusion_truth_by_model_kind": confusion,
      "seconds_median": median(secs), "seconds_max": secs.max() ?? 0, "process_max_rss_mb": maxRSSMegabytes(),
    ], "\(runs)/ondevice-t2-kind-\(setsName).json")
}

// MARK: T3, a second opinion on each rule claim

@Generable
enum Answer: String { case yes, no, cannotTell }

@Generable
struct Verdict {
  @Guide(description: "Words copied exactly from the page, at most one line, that settle the question. Empty when none do.")
  var quote: String
  @Guide(description: "yes when the quoted words show the claim is true, no when they show it is false, cannotTell otherwise.")
  var answer: Answer
}

let vetoInstructions = """
  You check one claim that a program made about a page of a document. Read the page text and answer the \
  question with yes, no or cannotTell. Answer only from what the page says. Answer cannotTell when the page \
  does not settle it.
  Many pages carry a notice that they are made-up samples for testing, such as \
  "SAMPLE DOCUMENT. Made up for testing. Not a real statement." That notice does not change the answer; ignore it.
  For the quote, copy exactly, character for character, the few words of the page that show your answer.
  """

struct Claim {
  let set: String
  let doc: String
  let kind: String  // link, statement, passport, mention
  let key: String  // the same claim in two rule runs has the same key
  let question: String
  let anchor: Span?
  var reasons: [String] = []
}

let kindWords = [
  "account": "account", "iban": "IBAN", "card": "card", "bic": "bank code", "ifsc": "bank code",
  "passport_number": "passport number", "mrz": "machine-readable zone", "pan": "PAN",
]

func lineOf(_ text: String, _ s: Span) -> (Int, String) {
  let prefix = String(text.unicodeScalars.prefix(s.start))
  let n = prefix.components(separatedBy: "\n").count - 1
  return (n, text.components(separatedBy: "\n")[n])
}

// Returns the claims of one rule run and, per claim key, whether the scorer called it wrong.
func claims(_ s: DocSet, output: String, score: String, pageText: [String: [String]]) -> [(Claim, Bool)] {
  let out = readJSON(output)
  let orgNames = (out["organisations"] as? JSON ?? [:]).mapValues { ($0 as? JSON)?["name"] as? String }
  let wrong = (readJSON(score)["wrong"] as? [JSON] ?? []).map {
    ($0["measure"] as! String, $0["document"] as! String, $0["what"] as! String)
  }
  func isWrong(_ measure: (String) -> Bool, _ doc: String, _ what: (String) -> Bool) -> Bool {
    wrong.contains { measure($0.0) && $0.1 == doc && what($0.2) }
  }
  var result: [(Claim, Bool)] = []
  for d in out["documents"] as? [JSON] ?? [] {
    let doc = d["id"] as! String
    let orgs = d["orgs"] as? [JSON] ?? []
    let ids = Dictionary(uniqueKeysWithValues: (d["identifiers"] as? [JSON] ?? []).map { ($0["id"] as! String, $0) })
    let links = d["links"] as? [JSON] ?? []
    func orgName(_ key: String) -> String {
      if let m = orgs.first(where: { $0["org"] as? String == key }) { return m["text"] as! String }
      if let n = orgNames[key] ?? nil { return n }
      // An organisation known only from a code: name it by that code.
      let code = links.first { $0["org"] as? String == key && ($0["reasons"] as? [String] ?? []).contains { $0.hasSuffix("institution") || $0.hasSuffix("prefix") } }
      let codeText = code.flatMap { ids[$0["identifier"] as! String]?["text"] as? String } ?? "?"
      return "the bank that the code \(codeText) names"
    }
    for l in links {
      let id = ids[l["identifier"] as! String]!
      let org = l["org"] as! String
      let kind = id["kind"] as! String
      let word = kindWords[kind] ?? kind
      let name = orgName(org)
      let q = """
        Question: does this page say that the \(word) \(id["text"] as! String) is held at \(name), that is, that \
        \(name) keeps or issued this \(word) for its holder?
        """
      let value = id["value"] as! String
      let w = isWrong({ $0 == "links" }, doc, { $0.hasPrefix("\(kind) \(value) at \(org)") })
      result.append((Claim(set: s.name, doc: doc, kind: "link", key: "link|\(doc)|\(value)|\(name)", question: q, anchor: span(id), reasons: l["reasons"] as? [String] ?? []), w))
    }
    for m in orgs {
      let sp = span(m)
      let line = pageText[doc].map { lineOf($0[sp.page - 1], sp).1 } ?? ""
      let q = """
        On this page the text "\(sp.text)" appears in the line "\(line)". Question: is "\(sp.text)" there the \
        name of an organisation, such as a bank, company, shop, office or other institution?
        """
      let w = isWrong({ $0 == "mentions" }, doc, { $0.hasPrefix("\(sp.text) at \(sp.page):\(sp.start)") })
      result.append((Claim(set: s.name, doc: doc, kind: "mention", key: "mention|\(doc)|\(sp.page):\(sp.start)", question: q, anchor: sp), w))
    }
    if let st = d["statement"] as? JSON {
      let accts = (st["accounts"] as? [String] ?? []).compactMap { ids[$0]?["text"] as? String }
      let issuer = (st["issuer"] as? String).map(orgName) ?? "a bank"
      let q = """
        Question: is this page a statement from \(issuer) for the account \(accts.joined(separator: " / ")): a \
        statement or passbook that \(issuer) issued, giving this account's transactions or balance for a period?
        """
      let w = isWrong({ $0.hasPrefix("statement_") }, doc, { _ in true })
      let anchor = (st["accounts"] as? [String])?.first.flatMap { ids[$0] }.map(span)
      result.append((Claim(set: s.name, doc: doc, kind: "statement", key: "statement|\(doc)|\(accts.joined(separator: ","))|\(issuer)", question: q, anchor: anchor), w))
    }
    if let pp = d["passport"] as? JSON, let h = pp["holder"] as? JSON {
      let holder = "\(h["surname"] as? String ?? "?"), \(h["given"] as? String ?? "?")"
      let q = """
        Question: is this page a passport, the page of the passport itself that shows its holder's details, \
        whose holder is \(holder)? A visa, permit, form or letter that only mentions a passport is not a passport.
        """
      let w = isWrong({ ["passport_holder", "passport_number", "passport_expiry"].contains($0) }, doc, { $0.contains("not a passport") })
      result.append((Claim(set: s.name, doc: doc, kind: "passport", key: "passport|\(doc)|\(holder)", question: q, anchor: nil), w))
    }
  }
  return result
}

func runVeto(_ setsName: String) async {
  let sets = setsName == "sample" ? [sampleSet] : outsideSets()
  var rows: [JSON] = []
  var tally: [String: [String: [String: Int]]] = [:]  // run -> kind -> counter
  for s in sets where !s.rules.isEmpty {
    let ps = pages(s)
    let pageText = Dictionary(grouping: ps, by: \.doc).mapValues { $0.sorted { $0.number < $1.number }.map(\.text) }
    var asked: [String: (String, String, Bool, Asked<Verdict>)] = [:]
    for (run, output, score) in s.rules {
      for (c, wrong) in claims(s, output: output, score: score, pageText: pageText) {
        if asked[c.key] == nil {
          let pageNumber = c.anchor?.page ?? 1
          let text = pageText[c.doc]![pageNumber - 1]
          let anchorLine = c.anchor.map { lineOf(text, $0).0 } ?? 0
          let a = await ask(
            Verdict.self, instructions: vetoInstructions, page: text, anchorLine: anchorLine,
            question: { "Page text:\n\($0)\n\n\(c.question)" })
          let raw = a.value?.answer.rawValue ?? "error"
          let quote = a.value?.quote ?? ""
          let ok = raw == "cannotTell" || quoteOnPage(quote, text)
          asked[c.key] = (raw, quote, ok, a)
          print(s.name, c.key, "->", raw, ok ? "" : "(quote not on page)", a.error ?? "", String(format: "%.2fs", a.seconds))
        }
        let (raw, quote, ok, a) = asked[c.key]!
        // The model may only remove a row: a "no" with its quote on the page removes it;
        // anything else keeps the row (counted as not judged when the model gave no verdict).
        let removed = raw == "no" && ok
        let judged = (raw == "yes" || raw == "no") && ok
        let keptOnlyOnYes = raw == "yes" && ok
        for k in [c.kind, "all"] {
          var t = tally[run, default: [:]][k, default: [:]]
          t[wrong ? "wrong_rows" : "right_rows", default: 0] += 1
          if removed { t[wrong ? "wrong_removed" : "right_removed", default: 0] += 1 }
          if !judged { t[wrong ? "wrong_not_judged" : "right_not_judged", default: 0] += 1 }
          if !keptOnlyOnYes { t[wrong ? "wrong_dropped_if_only_yes_keeps" : "right_dropped_if_only_yes_keeps", default: 0] += 1 }
          tally[run, default: [:]][k] = t
        }
        rows.append([
          "set": s.name, "run": run, "document": c.doc, "claim": c.kind, "key": c.key, "rules_row_wrong": wrong,
          "model_answer": raw, "quote": quote, "quote_on_page": ok, "removed": removed, "error": a.error ?? NSNull(),
          "seconds": a.seconds, "prompt_tokens": a.tokens, "dropped_lines": a.droppedLines,
        ])
      }
    }
  }
  let secs = rows.map { $0["seconds"] as! Double }
  writeJSON(
    [
      "sets": sets.map(\.name), "instructions": vetoInstructions, "sampling": "greedy", "rows": rows,
      "by_run": tally, "seconds_median_per_row": median(secs), "process_max_rss_mb": maxRSSMegabytes(),
      "policy": "removed: answer no with its quote on the page. not judged: cannotTell, an error, or a quote not on the page; the row is kept. dropped_if_only_yes_keeps: the stricter policy that keeps a row only on a checked yes.",
    ], "\(runs)/ondevice-t3-veto-\(setsName).json")
}

// MARK: T2 and T3 together

// No model call: reads the T2 page kinds already written and vetoes a statement or passport
// claim when T2 read the document as another kind with a checked quote, and a link whose
// only reason is the statement header of a vetoed statement. Errors and cannotTell keep the row.
func runCombine(_ setsName: String) {
  let sets = setsName == "sample" ? [sampleSet] : outsideSets()
  let t2 = readJSON("\(runs)/ondevice-t2-kind-\(setsName).json")["documents"] as! [JSON]
  let kindOf = Dictionary(uniqueKeysWithValues: t2.map { ($0["document"] as! String, $0["kind"] as! String) })
  let abstain: Set<String> = ["cannotTell", "error"]
  var tally: [String: [String: [String: Int]]] = [:]
  var removedRows: [JSON] = []
  for s in sets where !s.rules.isEmpty {
    let ps = pages(s)
    let pageText = Dictionary(grouping: ps, by: \.doc).mapValues { $0.sorted { $0.number < $1.number }.map(\.text) }
    for (run, output, score) in s.rules {
      for (c, wrong) in claims(s, output: output, score: score, pageText: pageText) {
        let kind = kindOf["\(s.name)/\(c.doc)"] ?? "error"
        let notStatement = !abstain.contains(kind) && kind != "statement"
        let removed: Bool
        switch c.kind {
        case "statement": removed = notStatement
        case "passport": removed = !abstain.contains(kind) && kind != "passport"
        case "link": removed = notStatement && c.reasons == ["statement-header"]
        default: removed = false
        }
        for k in [c.kind, "all"] {
          var t = tally[run, default: [:]][k, default: [:]]
          t[wrong ? "wrong_rows" : "right_rows", default: 0] += 1
          if removed { t[wrong ? "wrong_removed" : "right_removed", default: 0] += 1 }
          tally[run, default: [:]][k] = t
        }
        if removed { removedRows.append(["run": run, "key": c.key, "rules_row_wrong": wrong, "page_kind": kind]) }
      }
    }
  }
  writeJSON(["sets": sets.map(\.name), "by_run": tally, "removed": removedRows,
             "policy": "statement claim removed when T2 read the page as a kind other than statement; passport claim when other than passport; a link whose only reason is statement-header follows its statement. cannotTell and errors keep the row. Chosen after the outside T2 and T3 runs: a hypothesis, not a held-out result."],
            "\(runs)/ondevice-t2t3-combined-\(setsName).json")
}

// MARK: T4, repeatability and languages

func runRepeat() async {
  let all = pages(sampleSet) + outsideSets().flatMap(pages)
  let picks = [
    "hdfc-statement-sep", "visa-asha", "passport-asha-current", "commerzbank-kontoauszug", "rib-fr",
    "mizuho-statement-ja", "essay-ja", "hdfc-letter-hindi", "sbi-passbook-hindi", "recipe-hindi",
    "explicitly-not-statement", "service-invoice", "devanagari-date-proof",
  ]
  var rows: [JSON] = []
  for id in picks {
    guard let p = all.first(where: { $0.doc == id }) else { continue }
    for (name, opts) in [("greedy", greedy), ("default-sampling", sampled)] {
      var kinds: [String] = []
      var errors: [String] = []
      var secs: [Double] = []
      for _ in 0..<5 {
        let r = await readKind(p, options: opts)
        kinds.append(r.kind)
        if let e = r.asked.error { errors.append(e) }
        secs.append(r.asked.seconds)
      }
      print(id, name, kinds)
      rows.append([
        "document": id, "set": p.set, "lang": p.lang, "sampling": name, "kinds": kinds,
        "same_all_five": Set(kinds).count == 1, "errors": errors, "seconds": secs,
      ])
    }
  }
  writeJSON(
    [
      "rows": rows, "process_max_rss_mb": maxRSSMegabytes(), "context_size": model.contextSize,
      "supports_hi": model.supportsLocale(Locale(identifier: "hi")),
    ], "\(runs)/ondevice-t4-repeat.json")
}

// MARK: Redaction of the outside sets

// The outside sets' text stays out of this repository: only counts and document names
// are kept. Strips quotes, tagged text and claim keys (which hold names and numbers)
// from every row whose set is not the sample, in place. Keep the full files elsewhere.
func runRedact() {
  var keyIds: [String: String] = [:]
  func redactKey(_ row: JSON) -> String {
    let key = row["key"] as! String
    if let k = keyIds[key] { return k }
    let parts = key.split(separator: "|", maxSplits: 2).map(String.init)
    let n = keyIds.values.filter { $0.hasPrefix("\(parts[0])|\(parts[1])|") }.count + 1
    keyIds[key] = "\(parts[0])|\(parts[1])|#\(n)"
    return keyIds[key]!
  }
  func scrub(_ rows: [JSON], isOutside: (JSON) -> Bool) -> [JSON] {
    rows.map { r in
      guard isOutside(r) else { return r }
      var r = r
      if r["quote"] != nil { r["quote"] = "(redacted: outside set)" }
      if r["text"] != nil { r["text"] = "(redacted: outside set)" }
      if r["key"] != nil { r["key"] = redactKey(r) }
      return r
    }
  }
  let outside: (JSON) -> Bool = { ($0["set"] as? String ?? "") != "sample" }
  for f in ["ondevice-t2-kind-outside", "ondevice-t3-veto-outside", "ondevice-t2t3-combined-outside", "ondevice-t4-repeat", "ondevice-t1-tagger"] {
    let path = "\(runs)/\(f).json"
    guard FileManager.default.fileExists(atPath: path) else { continue }
    var j = readJSON(path)
    if let rows = j["pages"] as? [JSON] { j["pages"] = scrub(rows, isOutside: outside) }
    if let rows = j["rows"] as? [JSON] { j["rows"] = scrub(rows, isOutside: outside) }
    if let rows = j["removed"] as? [JSON] { j["removed"] = scrub(rows, isOutside: { _ in true }) }
    for mode in ["language-given", "language-detected"] {
      guard var m = j[mode] as? JSON, var sets = m["sets"] as? JSON else { continue }
      for (name, v) in sets where name != "sample" {
        var v = v as! JSON
        for field in ["not_orgs", "org_tags_not_on_a_mention"] {
          v[field] = scrub(v[field] as? [JSON] ?? [], isOutside: { _ in true })
        }
        sets[name] = v
      }
      m["sets"] = sets
      j[mode] = m
    }
    writeJSON(j, path)
  }
}

// MARK: Main

guard case .available = model.availability else {
  if command != "tagger" && command != "redact" {
    print("The on-device model is not available: \(model.availability). Only `tagger` runs.")
    exit(2)
  }
  command == "redact" ? runRedact() : runTagger()
  exit(0)
}
switch command {
case "tagger": runTagger()
case "redact": runRedact()
case "kind": await runKind(option("--sets") ?? "sample")
case "veto": await runVeto(option("--sets") ?? "sample")
case "repeat": await runRepeat()
case "combine": runCombine(option("--sets") ?? "sample")
default:
  print("usage: swift sample/tools/ondevice/trial.swift <tagger|kind|veto|combine|repeat|redact> [--sets sample|outside] --xproto DIR [--sample-rules DIR]")
  exit(2)
}
