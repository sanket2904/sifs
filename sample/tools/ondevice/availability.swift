// Prints whether Apple's on-device language model is usable on this Mac.
// Run: swift sample/tools/ondevice/availability.swift
import Foundation
import FoundationModels

let model = SystemLanguageModel.default
let availability: String
switch model.availability {
case .available: availability = "available"
case .unavailable(.deviceNotEligible): availability = "unavailable: deviceNotEligible"
case .unavailable(.appleIntelligenceNotEnabled): availability = "unavailable: appleIntelligenceNotEnabled"
case .unavailable(.modelNotReady): availability = "unavailable: modelNotReady"
case .unavailable(let other): availability = "unavailable: \(other)"
}
let languages = model.supportedLanguages.map { lang in
    [lang.languageCode?.identifier, lang.script?.identifier, lang.region?.identifier].compactMap { $0 }.joined(separator: "-")
}.sorted()
let os = ProcessInfo.processInfo.operatingSystemVersionString
var out: [String: Any] = [
    "os": os,
    "availability": availability,
    "contextSize": model.contextSize,
    "supportedLanguages": languages,
]
for code in ["en", "hi", "ja", "de", "fr"] {
    out["supportsLocale_\(code)"] = model.supportsLocale(Locale(identifier: code))
}
let data = try JSONSerialization.data(withJSONObject: out, options: [.prettyPrinted, .sortedKeys])
print(String(data: data, encoding: .utf8)!)
