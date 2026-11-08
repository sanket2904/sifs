// sifs-ax: reads and drives one SIFS window through the accessibility API,
// addressed by process id. It never activates an app, never moves or clicks
// the mouse, and posts key events to that one process only, so the person
// using the Mac keeps their focus and their cursor.
import AppKit
import ApplicationServices

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(("sifs-ax: " + message + "\n").data(using: .utf8)!)
    exit(1)
}

func attr(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func string(_ element: AXUIElement, _ name: String) -> String {
    guard let value = attr(element, name) else { return "" }
    if let s = value as? String { return s }
    if let n = value as? NSNumber { return n.stringValue }
    return ""
}

func role(_ element: AXUIElement) -> String { string(element, kAXRoleAttribute) }

func children(_ element: AXUIElement) -> [AXUIElement] {
    (attr(element, kAXChildrenAttribute) as? [AXUIElement]) ?? []
}

func classes(_ element: AXUIElement) -> [String] {
    (attr(element, "AXDOMClassList") as? [String]) ?? []
}

func isTrue(_ element: AXUIElement, _ name: String) -> Bool {
    (attr(element, name) as? NSNumber)?.boolValue ?? false
}

func walk(_ element: AXUIElement, depth: Int = 0, _ visit: (AXUIElement, Int) -> Bool) {
    guard visit(element, depth) else { return }
    for child in children(element) { walk(child, depth: depth + 1, visit) }
}

func find(_ root: AXUIElement, _ match: (AXUIElement) -> Bool) -> [AXUIElement] {
    var found: [AXUIElement] = []
    walk(root) { element, _ in
        if match(element) { found.append(element) }
        return true
    }
    return found
}

/// The visible text under an element, in document order.
func text(_ element: AXUIElement) -> String {
    var pieces: [String] = []
    walk(element) { node, _ in
        if role(node) == kAXStaticTextRole { pieces.append(string(node, kAXValueAttribute)) }
        return true
    }
    return pieces.joined()
}

/// Accessibility lists a window in AXWindows only while it is on the Space
/// in front; AXMainWindow still reaches it from any Space.
func window(_ pid: pid_t) -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    let listed = (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
    let main = [kAXMainWindowAttribute, kAXFocusedWindowAttribute].compactMap { attr(app, $0) }.map { $0 as! AXUIElement }
    guard let found = (listed + main).first(where: { string($0, kAXTitleAttribute) == "SIFS" }) else {
        fail("process \(pid) shows no SIFS window to accessibility (closed to the menu bar? relaunch: sifs-verify cleanup, then launch)")
    }
    return found
}

/// WebKit builds the page's accessibility tree lazily, so a new window can
/// briefly show an empty web area.
func webArea(_ pid: pid_t) -> AXUIElement {
    for _ in 0..<50 {
        if let area = find(window(pid), { role($0) == "AXWebArea" }).first, !children(area).isEmpty { return area }
        usleep(100_000)
    }
    fail("the SIFS window of process \(pid) has no page content")
}

func searchBox(_ area: AXUIElement) -> AXUIElement {
    guard let box = find(area, { string($0, "AXPlaceholderValue") == "Search your files" }).first else {
        fail("no search box (placeholder \"Search your files\")")
    }
    return box
}

func dump(_ pid: pid_t) {
    walk(window(pid)) { node, depth in
        var line = String(repeating: "  ", count: depth) + role(node)
        let fields: [(String, String)] = [
            ("subrole", string(node, kAXSubroleAttribute)),
            ("id", string(node, "AXDOMIdentifier")),
            ("class", classes(node).joined(separator: ".")),
            ("title", string(node, kAXTitleAttribute)),
            ("desc", string(node, kAXDescriptionAttribute)),
            ("placeholder", string(node, "AXPlaceholderValue")),
            ("value", string(node, kAXValueAttribute)),
        ]
        for (key, value) in fields where !value.isEmpty { line += " \(key)=\(value.debugDescription)" }
        if isTrue(node, kAXSelectedAttribute) { line += " selected" }
        if isTrue(node, kAXFocusedAttribute) { line += " focused" }
        print(line)
        return true
    }
}

/// A result row. WebKit exposes each row (role="option") as one text whose
/// title is its visible text: kind icon, name, folder, size · date, then
/// for a content hit its label and snippet. The fields are split from that
/// text by the page's fixed layout; `text` is the row exactly as shown.
struct Row: Encodable {
    let index: Int
    let selected: Bool
    let text: String
    let name: String?
    let closeMatch: Bool?
    let folder: String?
    let meta: String?
    let found: String?
}

let metaPattern = try! NSRegularExpression(pattern: #" ((?:\d+(?:\.\d+)? (?:B|KB|MB|GB|TB) )?· [A-Z][a-z]{2} \d{1,2}, \d{4})(?: (.*))?$"#)

func row(_ element: AXUIElement) -> Row {
    let full = string(element, kAXTitleAttribute)
    let index = Int(string(element, "AXDOMIdentifier").dropFirst(4)) ?? -1
    let selected = classes(element).contains("selected") || isTrue(element, kAXSelectedAttribute)
    let ns = full as NSString
    guard let space = full.firstIndex(of: " "),
        let match = metaPattern.firstMatch(in: full, range: NSRange(location: 0, length: ns.length))
    else { return Row(index: index, selected: selected, text: full, name: nil, closeMatch: nil, folder: nil, meta: nil, found: nil) }
    let head = String(full[full.index(after: space)..<Range(match.range, in: full)!.lowerBound])
    let found = match.range(at: 2).location == NSNotFound ? nil : ns.substring(with: match.range(at: 2))
    // A name can't hold "/", so the folder starts at the first " ~/" or " /".
    let folderStart = [" ~/", " /"].compactMap { head.range(of: $0)?.lowerBound }.min()
        ?? (head.hasSuffix(" ~") ? head.index(head.endIndex, offsetBy: -2) : nil)
    var name = folderStart.map { String(head[..<$0]) } ?? head
    let closeMatch = name.hasSuffix("· close match")
    if closeMatch { name = String(name.dropLast("· close match".count)).trimmingCharacters(in: .whitespaces) }
    let folder = folderStart.map { String(head[head.index(after: $0)...]) }
    return Row(index: index, selected: selected, text: full, name: name, closeMatch: closeMatch, folder: folder,
               meta: ns.substring(with: match.range(at: 1)), found: found)
}

struct Screen: Encodable {
    let query: String
    let searchBoxFocused: Bool
    let rows: [Row]
    let footer: String
    /// Every line of the page in order (settings, errors, notices), buttons in [brackets].
    let lines: [String]
}

func isRow(_ element: AXUIElement) -> Bool { string(element, "AXDOMIdentifier").hasPrefix("hit-") }

func buttonTitle(_ button: AXUIElement) -> String {
    let title = string(button, kAXTitleAttribute)
    return title.isEmpty ? text(button) : title
}

/// Inline content (text, buttons) of one parent joins into a line; blocks
/// (headings, groups, fields, rows) start new lines. A line ends after a
/// button that text follows, which keeps each folder row on its own line.
func lines(_ element: AXUIElement) -> [String] {
    var out: [String] = []
    var line: [String] = []
    var lastWasButton = false
    func flush() {
        let joined = line.joined(separator: " ").split(separator: " ", omittingEmptySubsequences: true).joined(separator: " ")
        if !joined.isEmpty { out.append(joined) }
        line = []
    }
    for child in children(element) {
        let r = role(child)
        if isRow(child) {
            continue
        } else if r == kAXStaticTextRole || string(child, kAXSubroleAttribute) == "AXCodeStyleGroup" {
            if lastWasButton { flush() }
            line.append(text(child).isEmpty ? string(child, kAXValueAttribute) : text(child))
            lastWasButton = false
        } else if r == kAXButtonRole {
            line.append("[" + buttonTitle(child) + "]")
            lastWasButton = true
        } else if r == kAXTextFieldRole {
            flush()
            out.append("[search box: \(string(child, kAXValueAttribute))]")
        } else if r == kAXTextAreaRole {
            flush()
            out.append("[text area]")
            out += string(child, kAXValueAttribute).split(separator: "\n").map { "  " + $0 }
        } else if r == kAXHeadingRole {
            flush()
            out.append("# " + text(child))
        } else {
            flush()
            out += lines(child)
            lastWasButton = false
        }
    }
    flush()
    return out
}

func read(_ pid: pid_t) -> Screen {
    let area = webArea(pid)
    let box = searchBox(area)
    let rows = find(area, isRow).map(row).sorted { $0.index < $1.index }
    let footer = find(area, { string($0, kAXSubroleAttribute) == "AXSectionFooter" }).first.map {
        children($0).map(text).joined(separator: " ")
    } ?? ""
    return Screen(query: string(box, kAXValueAttribute), searchBoxFocused: isTrue(box, kAXFocusedAttribute),
                  rows: rows, footer: footer, lines: lines(area))
}

func printJSON(_ screen: Screen) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .withoutEscapingSlashes, .sortedKeys]
    print(String(data: try! encoder.encode(screen), encoding: .utf8)!)
}

/// Presses the n-th button with this title, or with `after`, the first one
/// that follows the text `after` (a folder's path, say) in the page.
func press(_ pid: pid_t, _ title: String, nth: Int, after: String?) {
    var ordered: [AXUIElement] = []
    walk(webArea(pid)) { node, _ in
        ordered.append(node)
        return role(node) != kAXButtonRole
    }
    var candidates = ordered
    if let after {
        guard let start = ordered.firstIndex(where: { role($0) == kAXStaticTextRole && string($0, kAXValueAttribute) == after }) else {
            fail("no text \"\(after)\" in the page")
        }
        candidates = Array(ordered[start...])
    }
    let all = candidates.filter { role($0) == kAXButtonRole }
    let matching = all.filter { buttonTitle($0) == title }
    guard nth < matching.count else {
        let names = all.map { "\"" + buttonTitle($0) + "\"" }.joined(separator: ", ")
        fail("no button #\(nth + 1) titled \"\(title)\" (buttons: \(names))")
    }
    let result = AXUIElementPerformAction(matching[nth], kAXPressAction as CFString)
    guard result == .success else { fail("pressing \"\(title)\" failed: \(result.rawValue)") }
}

func sheet(_ pid: pid_t) -> AXUIElement {
    for _ in 0..<50 {
        if let found = children(window(pid)).first(where: { role($0) == kAXSheetRole }) { return found }
        usleep(100_000)
    }
    fail("no dialog is open on the SIFS window")
}

func cancelDialog(_ pid: pid_t) {
    guard let cancel = find(sheet(pid), { role($0) == kAXButtonRole && string($0, kAXTitleAttribute) == "Cancel" }).first else {
        fail("the dialog has no Cancel button")
    }
    AXUIElementPerformAction(cancel, kAXPressAction as CFString)
}

func focus(_ element: AXUIElement, _ what: String) {
    let result = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    guard result == .success else { fail("focusing \(what) failed: \(result.rawValue)") }
}

/// CGEvent.postToPid delivers to this process only, whichever app is in
/// front, and without bringing this one forward.
func post(_ pid: pid_t, key: CGKeyCode, flags: CGEventFlags = [], unicode: [UniChar]? = nil) {
    let source = CGEventSource(stateID: .privateState)
    for down in [true, false] {
        guard let event = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: down) else { fail("no key event") }
        event.flags = flags
        if let unicode { event.keyboardSetUnicodeString(stringLength: unicode.count, unicodeString: unicode) }
        event.postToPid(pid)
        usleep(12_000)
    }
}

let keys: [String: CGKeyCode] = [
    "down": 125, "up": 126, "left": 123, "right": 124, "return": 36, "escape": 53,
    "tab": 48, "delete": 51, "a": 0, "g": 5, "space": 49,
]

func type(_ pid: pid_t, _ text: String) {
    for scalar in text.utf16 {
        if scalar == 10 { post(pid, key: keys["return"]!) } else { post(pid, key: 0, unicode: [scalar]) }
    }
}

/// The window's number for `screencapture -l`, and whether it is on screen,
/// from the window server alone (no accessibility, nothing activated).
func windowInfo(_ pid: pid_t) -> (Int, Bool)? {
    let list = (CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]]) ?? []
    let mine = list.first {
        ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int).map { $0 <= 0 } == true
            && ($0[kCGWindowName as String] as? String) == "SIFS"
    }
    guard let mine, let id = mine[kCGWindowNumber as String] as? Int else { return nil }
    return (id, (mine[kCGWindowIsOnscreen as String] as? Bool) ?? false)
}

func waitFor(_ pid: pid_t, _ pattern: String, seconds: Double) {
    let regex = (try? NSRegularExpression(pattern: pattern)) ?? { fail("bad pattern \(pattern)") }()
    let deadline = Date().addingTimeInterval(seconds)
    var last: [String] = []
    while Date() < deadline {
        let screen = read(pid)
        last = screen.lines + screen.rows.map(\.text) + [screen.footer]
        if let hit = last.first(where: { regex.firstMatch(in: $0, range: NSRange(location: 0, length: ($0 as NSString).length)) != nil }) {
            print(hit)
            return
        }
        usleep(250_000)
    }
    fail("no line matched /\(pattern)/ within \(Int(seconds)) s. Last screen:\n" + last.joined(separator: "\n"))
}

/// Types the query the way a user does and waits until the window shows its
/// results and stops changing (a content pass can land after the names).
func search(_ pid: pid_t, _ query: String, settle: Double) -> Screen {
    let box = searchBox(webArea(pid))
    focus(box, "the search box")
    for _ in string(box, kAXValueAttribute).utf16 { post(pid, key: keys["delete"]!) }
    type(pid, query)
    let deadline = Date().addingTimeInterval(15)
    var last = read(pid)
    var stableSince = Date()
    while Date() < deadline {
        usleep(100_000)
        let now = read(pid)
        let shown = now.query == query && (now.footer.range(of: #"^\d+ results in"#, options: .regularExpression) != nil)
        if now.rows.map(\.text) != last.rows.map(\.text) || now.footer != last.footer { stableSince = Date() }
        last = now
        if shown && Date().timeIntervalSince(stableSince) >= settle { return now }
    }
    fail("the window didn't settle on results for \"\(query)\". Footer: \(last.footer), box: \(last.query)")
}

let args = Array(CommandLine.arguments.dropFirst())
let usage = """
    usage: sifs-ax <command> <pid> [args]
      wininfo <pid>                 window number and on-screen flag (window server only)
      tree <pid>                    the window's accessibility tree
      read <pid>                    query, rows, footer and every page line as JSON
      search <pid> <query> [settle] type a query, wait for settled results, print JSON
      wait <pid> <regex> [seconds]  wait until a page line, row or the footer matches
      press <pid> <title> [n | --after <text>]  AXPress the n-th button with this title, or the first after a text
      cancel-dialog <pid>           press the open dialog's Cancel
      dialog-tree <pid>             the open dialog's accessibility tree
      focus-search <pid>            focus the search box
      focus-textarea <pid>          focus the skipped-patterns text area
      type <pid> <text>             post the text as key events ("\\n" is Return)
      key <pid> <name> [--cmd] [--shift]
    """
guard args.count >= 2, let pid = pid_t(args[1]) else { fail(usage) }
guard AXIsProcessTrusted() else { fail("this terminal lacks Accessibility permission (System Settings > Privacy & Security > Accessibility)") }
guard NSRunningApplication(processIdentifier: pid) != nil else { fail("no app with process id \(pid)") }

switch args[0] {
case "wininfo":
    guard let (id, onscreen) = windowInfo(pid) else { fail("process \(pid) has no SIFS window") }
    print("\(id) \(onscreen ? "onscreen" : "offscreen")")
case "tree":
    dump(pid)
case "read":
    printJSON(read(pid))
case "search":
    guard args.count >= 3 else { fail(usage) }
    printJSON(search(pid, args[2], settle: args.count > 3 ? Double(args[3]) ?? 0.5 : 0.5))
case "wait":
    guard args.count >= 3 else { fail(usage) }
    waitFor(pid, args[2], seconds: args.count > 3 ? Double(args[3]) ?? 30 : 30)
case "press":
    guard args.count >= 3 else { fail(usage) }
    if args.count > 4 && args[3] == "--after" {
        press(pid, args[2], nth: 0, after: args[4])
    } else {
        press(pid, args[2], nth: args.count > 3 ? max((Int(args[3]) ?? 1) - 1, 0) : 0, after: nil)
    }
case "cancel-dialog":
    cancelDialog(pid)
case "dialog-tree":
    walk(sheet(pid)) { node, depth in
        print(String(repeating: "  ", count: depth) + role(node) + " " + string(node, kAXTitleAttribute) + " " + string(node, kAXValueAttribute))
        return true
    }
case "focus-search":
    focus(searchBox(webArea(pid)), "the search box")
case "focus-textarea":
    guard let area = find(webArea(pid), { role($0) == kAXTextAreaRole }).first else {
        fail("no skipped-patterns text area (open settings first: press ⚙︎)")
    }
    focus(area, "the skipped-patterns text area")
case "type":
    guard args.count >= 3 else { fail(usage) }
    type(pid, args[2])
case "key":
    guard args.count >= 3, let code = keys[args[2]] else { fail("key needs one of: \(keys.keys.sorted().joined(separator: " "))") }
    var flags: CGEventFlags = []
    if args.contains("--cmd") { flags.insert(.maskCommand) }
    if args.contains("--shift") { flags.insert(.maskShift) }
    post(pid, key: code, flags: flags)
default:
    fail(usage)
}
