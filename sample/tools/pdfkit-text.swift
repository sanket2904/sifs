// Prints the text PDFKit reads from each page of each PDF named, as JSON
// {file name: [page text]}. macOS only; check.mjs --pdfkit runs it.
import Foundation
import PDFKit

var out: [String: [String]] = [:]
for path in CommandLine.arguments.dropFirst() {
  guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else {
    FileHandle.standardError.write("PDFKit cannot open \(path)\n".data(using: .utf8)!)
    exit(1)
  }
  out[(path as NSString).lastPathComponent] = (0..<doc.pageCount).map { doc.page(at: $0)?.string ?? "" }
}
let data = try JSONSerialization.data(withJSONObject: out, options: [.sortedKeys])
FileHandle.standardOutput.write(data)
