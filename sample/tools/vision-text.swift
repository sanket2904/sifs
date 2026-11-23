// Reads every page of each PDF named that has no text layer with Apple Vision,
// and prints JSON {"vision": {...settings}, "files": {file name: [page]}},
// where a page is its recognised lines, top to bottom, or null for a page
// PDFKit reads text from. macOS only; text.mjs --ocr runs it.
import CoreGraphics
import Foundation
import PDFKit
import Vision

// The scanned pages are drawn 1700 by 2200 pixels (sample/tools/lib/scan.mjs),
// so rendering at that size gives Vision every pixel of the picture.
let width = 1700
let height = 2200

func render(_ page: PDFPage) -> CGImage {
  let box = page.bounds(for: .mediaBox)
  let context = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
  context.setFillColor(gray: 1, alpha: 1)
  context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  context.scaleBy(x: CGFloat(width) / box.width, y: CGFloat(height) / box.height)
  page.draw(with: .mediaBox, to: context)
  return context.makeImage()!
}

func recognise(_ image: CGImage) throws -> [String] {
  let request = VNRecognizeTextRequest()
  request.recognitionLevel = .accurate
  // Identifiers are not words; correction would "fix" them.
  request.usesLanguageCorrection = false
  request.recognitionLanguages = ["en-US"]
  try VNImageRequestHandler(cgImage: image).perform([request])
  let found = (request.results ?? []).compactMap { obs -> (CGRect, String)? in
    guard let text = obs.topCandidates(1).first?.string else { return nil }
    return (obs.boundingBox, text)
  }
  // Vision's boxes have their origin at the bottom left. Pieces whose
  // vertical centres are within half a box height are one printed line.
  var lines: [(y: CGFloat, h: CGFloat, parts: [(CGFloat, String)])] = []
  for (box, text) in found.sorted(by: { $0.0.midY > $1.0.midY }) {
    if let i = lines.firstIndex(where: { abs($0.y - box.midY) < min($0.h, box.height) / 2 }) {
      lines[i].parts.append((box.minX, text))
    } else {
      lines.append((box.midY, box.height, [(box.minX, text)]))
    }
  }
  return lines.map { $0.parts.sorted { $0.0 < $1.0 }.map { $0.1 }.joined(separator: " ") }
}

var files: [String: [Any]] = [:]
for path in CommandLine.arguments.dropFirst() {
  guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else {
    FileHandle.standardError.write("PDFKit cannot open \(path)\n".data(using: .utf8)!)
    exit(1)
  }
  files[(path as NSString).lastPathComponent] = try (0..<doc.pageCount).map { i -> Any in
    let page = doc.page(at: i)!
    if !(page.string ?? "").isEmpty { return NSNull() }
    return try recognise(render(page))
  }
}
let os = ProcessInfo.processInfo.operatingSystemVersion
let vision: [String: Any] = [
  "macos": "\(os.majorVersion).\(os.minorVersion).\(os.patchVersion)",
  "request": "VNRecognizeTextRequest",
  "revision": VNRecognizeTextRequest.currentRevision,
  "recognition_level": "accurate",
  "language_correction": false,
  "languages": ["en-US"],
  "pixels": [width, height],
]
let data = try JSONSerialization.data(
  withJSONObject: ["vision": vision, "files": files], options: [.sortedKeys])
FileHandle.standardOutput.write(data)
