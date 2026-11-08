// sifs-ax: reads and drives the SIFS window of one sifs-verify run. See SKILL.md.
import AppKit
import ApplicationServices
import Darwin

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(("sifs-ax: " + message + "\n").data(using: .utf8)!)
    exit(1)
}

func refuse(_ why: String) -> Never { fail("refused: " + why) }

func isDigits(_ text: Substring) -> Bool { !text.isEmpty && text.allSatisfy { $0.isASCII && $0.isNumber } }

struct Identity: Equatable {
    let start: String
    let exe: String
    let home: String?
}

func identity(_ pid: pid_t) -> Identity? {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.size)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else { return nil }
    var path = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
    guard proc_pidpath(pid, &path, UInt32(path.count)) > 0 else { return nil }
    return Identity(start: String(format: "%llu.%06llu", info.pbi_start_tvsec, info.pbi_start_tvusec),
                    exe: String(cString: path), home: environment(pid, "HOME"))
}

/// KERN_PROCARGS2 is argc (an Int32), the executable path and its NUL
/// padding, then argv and the environment as NUL-terminated strings.
func environment(_ pid: pid_t, _ name: String) -> String? {
    var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
    var size = 0
    guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > 4 else { return nil }
    var bytes = [UInt8](repeating: 0, count: size)
    guard sysctl(&mib, 3, &bytes, &size, nil, 0) == 0, size > 4 else { return nil }
    let argc = bytes.withUnsafeBytes { $0.load(as: Int32.self) }
    var i = 4
    func next() -> String? {
        guard i < size, let end = bytes[i..<size].firstIndex(of: 0) else { return nil }
        defer { i = end + 1 }
        return String(decoding: bytes[i..<end], as: UTF8.self)
    }
    guard next() != nil else { return nil }
    while i < size, bytes[i] == 0 { i += 1 }
    for _ in 0..<max(0, Int(argc)) { guard next() != nil else { return nil } }
    while let variable = next(), !variable.isEmpty {
        if variable.hasPrefix(name + "=") { return String(variable.dropFirst(name.count + 1)) }
    }
    return nil
}

struct Run {
    let dir: String
    let pid: pid_t
    let start: String
    var expected: Identity { Identity(start: start, exe: dir + "/sifs", home: dir + "/home") }
}

func runDir(_ path: String) -> String {
    let name = (path as NSString).lastPathComponent
    guard let real = realpath(path, nil) else { refuse("\(path) is not a run folder") }
    let canonical = String(cString: real)
    free(real)
    guard canonical == path, (1...64).contains(name.count),
        name.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }),
        ((path as NSString).deletingLastPathComponent as NSString).lastPathComponent == "runs"
    else { refuse("\(path) is not a run folder: sifs-verify passes <workspace>/runs/<id>, canonical") }
    return canonical
}

func loadRun(_ path: String) -> Run {
    let dir = runDir(path)
    guard let text = try? String(contentsOfFile: dir + "/run.env", encoding: .utf8) else { refuse("\(dir)/run.env can't be read") }
    var lines = text.split(separator: "\n", omittingEmptySubsequences: false)
    if lines.last == "" { lines.removeLast() }
    var values: [Substring: Substring] = [:]
    for line in lines {
        guard let eq = line.firstIndex(of: "="), !line[..<eq].isEmpty,
            line[..<eq].allSatisfy({ $0.isASCII && ($0.isUppercase || $0.isNumber || $0 == "_") }),
            values.updateValue(line[line.index(after: eq)...], forKey: line[..<eq]) == nil
        else { refuse("\(dir)/run.env is malformed at: \(line)") }
    }
    let start = values["START"]?.split(separator: ".", omittingEmptySubsequences: false) ?? []
    guard let pidText = values["PID"], isDigits(pidText), let pid = pid_t(pidText), pid > 0,
        start.count == 2, isDigits(start[0]), isDigits(start[1]), start[1].count == 6
    else { refuse("\(dir)/run.env has no valid PID and START") }
    return Run(dir: dir, pid: pid, start: String(values["START"]!))
}

func verify(_ run: Run) {
    guard let now = identity(run.pid) else { refuse("process \(run.pid) of \(run.dir) is gone") }
    guard now == run.expected else {
        refuse("process \(run.pid) is not the one \(run.dir) started: it runs \(now.exe), HOME \(now.home ?? "unknown"), "
            + "started \(now.start); the run recorded \(run.expected.exe), HOME \(run.expected.home!), started \(run.start)")
    }
}

func errnoText() -> String { String(cString: strerror(errno)) }

let controlSocket = "control.sock"

func withAddress<T>(_ path: String, _ body: (UnsafePointer<sockaddr>, socklen_t) -> T) -> T {
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
    withUnsafeMutableBytes(of: &address.sun_path) { buffer in
        for (i, byte) in path.utf8.enumerated() { buffer[i] = byte }
    }
    return withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { body($0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
}

func readAll(_ fd: Int32) -> String {
    var bytes: [UInt8] = []
    var buffer = [UInt8](repeating: 0, count: 512)
    while true {
        let n = Darwin.read(fd, &buffer, buffer.count)
        if n < 0 && errno == EINTR { continue }
        if n <= 0 { break }
        bytes += buffer[0..<n]
        if bytes.count > 4096 { break }
    }
    return String(decoding: bytes, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
}

func send(_ fd: Int32, _ text: String) {
    let bytes = Array((text + "\n").utf8)
    _ = bytes.withUnsafeBytes { Darwin.write(fd, $0.baseAddress, $0.count) }
}

/// The child stays a zombie until waitpid reaps it. WNOWAIT looks without
/// reaping, so its pid can't go to another process.
func exited(_ pid: pid_t) -> Bool {
    var info = siginfo_t()
    return waitid(P_PID, id_t(pid), &info, WEXITED | WNOHANG | WNOWAIT) == 0 && info.si_pid == pid
}

func stopChild(_ pid: pid_t) -> String {
    if exited(pid) {
        waitpid(pid, nil, 0)
        return "stopped pid \(pid): it had already exited"
    }
    kill(pid, SIGTERM)
    for _ in 0..<25 {
        if waitpid(pid, nil, WNOHANG) == pid { return "stopped pid \(pid) with TERM" }
        usleep(200_000)
    }
    kill(pid, SIGKILL)
    waitpid(pid, nil, 0)
    return "stopped pid \(pid): it ignored TERM for 5 s, so it got KILL"
}

/// Reaps the child only on a stop request, so until then the app's pid,
/// alive or exited, can't name another process. Once control.sock is gone,
/// deleted with the run folder, nothing can reach it, so it stops the child
/// and exits.
func supervise(_ dir: String, _ childArgs: [String]) -> Never {
    signal(SIGHUP, SIG_IGN)
    signal(SIGPIPE, SIG_IGN)
    _ = setsid()
    guard chdir(dir) == 0 else { fail("can't enter \(dir): \(errnoText())") }
    let server = socket(AF_UNIX, SOCK_STREAM, 0)
    guard server >= 0, fcntl(server, F_SETFD, FD_CLOEXEC) == 0,
        withAddress(controlSocket, { Darwin.bind(server, $0, $1) }) == 0, listen(server, 8) == 0
    else { fail("can't listen on \(dir)/\(controlSocket): \(errnoText())") }
    var bound = stat()
    guard lstat(controlSocket, &bound) == 0 else { fail("can't stat \(dir)/\(controlSocket): \(errnoText())") }
    func socketGone() -> Bool {
        var now = stat()
        return lstat(controlSocket, &now) != 0 || now.st_dev != bound.st_dev || now.st_ino != bound.st_ino
    }

    let exe = dir + "/sifs", home = dir + "/home"
    var env = ProcessInfo.processInfo.environment
    let spy = env.removeValue(forKey: "SIFS_VERIFY_SPY")
    env["DYLD_INSERT_LIBRARIES"] = spy
    env["HOME"] = home
    env["CFFIXED_USER_HOME"] = home
    env["SIFS_VERIFY_BACKGROUND"] = "1"
    var actions: posix_spawn_file_actions_t?
    posix_spawn_file_actions_init(&actions)
    posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0)
    posix_spawn_file_actions_addopen(&actions, 1, "app.log", O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
    posix_spawn_file_actions_adddup2(&actions, 1, 2)
    let argv = ([exe] + childArgs).map { strdup($0) } + [nil]
    let envp = env.map { strdup("\($0.key)=\($0.value)") } + [nil]
    var pid: pid_t = 0
    let spawned = posix_spawn(&pid, exe, &actions, nil, argv, envp)
    guard spawned == 0 else { fail("starting \(exe): \(String(cString: strerror(spawned)))") }
    guard let started = identity(pid), started.exe == exe, started.home == home else {
        print(stopChild(pid))
        fail("process \(pid) didn't start as \(exe) with HOME \(home)")
    }

    while true {
        var ready = pollfd(fd: server, events: Int16(POLLIN), revents: 0)
        let polled = poll(&ready, 1, 1000)
        if polled <= 0 {
            if polled == 0 && socketGone() {
                _ = stopChild(pid)
                exit(0)
            }
            continue
        }
        let client = accept(server, nil, nil)
        guard client >= 0 else {
            if errno == EINTR { continue }
            fail("accept: \(errnoText())")
        }
        var buffer = [UInt8](repeating: 0, count: 16)
        let n = Darwin.read(client, &buffer, buffer.count)
        switch String(decoding: buffer[0..<max(n, 0)], as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) {
        case "status":
            send(client, "\(exited(pid) ? "exited" : "running") \(pid) \(started.start)")
        case "stop":
            let how = stopChild(pid)
            unlink(controlSocket)
            send(client, how)
            close(client)
            exit(0)
        default:
            send(client, "unknown request")
        }
        close(client)
    }
}

func ask(_ dir: String, _ request: String) -> String? {
    guard chdir(dir) == 0 else { return nil }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0, withAddress(controlSocket, { connect(fd, $0, $1) }) == 0 else { return nil }
    send(fd, request)
    let reply = readAll(fd)
    close(fd)
    return reply.isEmpty ? nil : reply
}

#if SIFS_AX_TEST
enum FakeTree {
    static let nodes: [String: [String: Any]] = {
        guard let path = ProcessInfo.processInfo.environment["SIFS_AX_TEST_TREE"],
            let data = FileManager.default.contents(atPath: path),
            let tree = try? JSONSerialization.jsonObject(with: data) as? [String: [String: Any]]
        else { fail("the test build needs SIFS_AX_TEST_TREE, a JSON accessibility tree") }
        return tree
    }()
    static let names = nodes.keys.filter { $0 != "app" }.sorted()
    static func element(_ name: String) -> AXUIElement {
        guard let i = names.firstIndex(of: name) else { fail("the fake tree has no node \(name)") }
        return AXUIElementCreateApplication(pid_t(1_000_000 + i))
    }
    static func node(_ element: AXUIElement) -> [String: Any] {
        var pid: pid_t = 0
        AXUIElementGetPid(element, &pid)
        let i = Int(pid) - 1_000_000
        return names.indices.contains(i) ? nodes[names[i]]! : nodes["app"] ?? [:]
    }
}

func copyAttribute(_ element: AXUIElement, _ name: String) -> (AXError, AnyObject?) {
    switch FakeTree.node(element)[name] {
    case nil: return (.attributeUnsupported, nil)
    case let value as [String: Any]:
        if let code = value["error"] as? Int { return (AXError(rawValue: Int32(code))!, nil) }
        if let name = value["el"] as? String { return (.success, FakeTree.element(name)) }
        if let names = value["els"] as? [String] { return (.success, names.map(FakeTree.element) as NSArray) }
        fail("bad fake value for \(name)")
    case let value: return (.success, value as AnyObject)
    }
}
#else
func copyAttribute(_ element: AXUIElement, _ name: String) -> (AXError, AnyObject?) {
    var value: AnyObject?
    let result = AXUIElementCopyAttributeValue(element, name as CFString, &value)
    return (result, value)
}
#endif

func attr(_ element: AXUIElement, _ name: String) -> AnyObject? {
    let (result, value) = copyAttribute(element, name)
    return result == .success ? value : nil
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
func windows(_ pid: pid_t) -> [AXUIElement] {
    let app = AXUIElementCreateApplication(pid)
    let listed = (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? []
    return listed + [kAXMainWindowAttribute, kAXFocusedWindowAttribute].compactMap { attr(app, $0) }.map { $0 as! AXUIElement }
}

func window(_ pid: pid_t) -> AXUIElement {
    guard let found = windows(pid).first(where: { string($0, kAXTitleAttribute) == "SIFS" }) else {
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
        children($0).map(text).joined(separator: " ").split(separator: " ").joined(separator: " ")
    } ?? ""
    return Screen(query: string(box, kAXValueAttribute), searchBoxFocused: isTrue(box, kAXFocusedAttribute),
                  rows: rows, footer: footer, lines: lines(area))
}

func printJSON(_ screen: Screen) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .withoutEscapingSlashes, .sortedKeys]
    print(String(data: try! encoder.encode(screen), encoding: .utf8)!)
}

enum Input {
    case key(CGKeyCode, CGEventFlags)
    case text
    case press
    case focus
    case cancelDialog
}

/// While a native dialog is open only Escape and the dialog's Cancel get
/// through. The folder picker opens in the owner's real home, so Return, a
/// typed newline or a press there could add his real folders.
func refusal(_ input: Input, dialog: String?) -> String? {
    guard let dialog else { return nil }
    if case .cancelDialog = input { return nil }
    if case let .key(code, flags) = input, code == keys["escape"], flags.isEmpty { return nil }
    return "a native dialog is open on the SIFS window, or one can't be ruled out (\(dialog)); "
        + "until that changes only dialog-tree, cancel-dialog and key escape work"
}

enum Checked {
    case value(AnyObject)
    case absent
    case failed(String)
}

func checked(_ element: AXUIElement, _ name: String) -> Checked {
    let (result, value) = copyAttribute(element, name)
    switch result {
    case .success: return value.map { .value($0) } ?? .absent
    case .noValue, .attributeUnsupported: return .absent
    default: return .failed("reading \(name) failed (\(result.rawValue))")
    }
}

func asElement(_ value: AnyObject) -> AXUIElement? {
    CFGetTypeID(value) == AXUIElementGetTypeID() ? (value as! AXUIElement) : nil
}

func asElements(_ value: AnyObject) -> [AXUIElement]? {
    guard let list = value as? [AnyObject] else { return nil }
    let elements = list.compactMap(asElement)
    return elements.count == list.count ? elements : nil
}

/// Why a native dialog is open or can't be ruled out; nil when none is. A
/// dialog (the Add a folder… picker, an alert) shows as a sheet on the SIFS
/// window or as a window of its own.
func dialogOpen(_ pid: pid_t) -> String? {
    let app = AXUIElementCreateApplication(pid)
    var windows: [AXUIElement] = []
    for name in [kAXWindowsAttribute, kAXMainWindowAttribute, kAXFocusedWindowAttribute] {
        switch checked(app, name) {
        case .absent: continue
        case let .failed(why): return why
        case let .value(value):
            guard let found = name == kAXWindowsAttribute ? asElements(value) : asElement(value).map({ [$0] }) else {
                return "\(name) is not a window list"
            }
            windows += found
        }
    }
    guard !windows.isEmpty else { return "the app shows no window to accessibility" }
    for window in windows {
        guard case let .value(roleValue) = checked(window, kAXRoleAttribute), let windowRole = roleValue as? String,
            case let .value(titleValue) = checked(window, kAXTitleAttribute), let title = titleValue as? String
        else { return "a window's role or title can't be read" }
        guard windowRole == kAXWindowRole, title == "SIFS" else { return "a window other than SIFS is open: \(windowRole) \(title.debugDescription)" }
        guard case let .value(childList) = checked(window, kAXChildrenAttribute), let children = asElements(childList) else {
            return "the SIFS window's children can't be read"
        }
        for child in children {
            guard case let .value(childRole) = checked(child, kAXRoleAttribute), let role = childRole as? String else {
                return "a child of the SIFS window has no readable role"
            }
            var subrole = ""
            switch checked(child, kAXSubroleAttribute) {
            case .absent: break
            case let .failed(why): return why
            case let .value(value):
                guard let text = value as? String else { return "a child of the SIFS window has a subrole that isn't text" }
                subrole = text
            }
            if role == kAXSheetRole || [kAXDialogSubrole, kAXSystemDialogSubrole].contains(subrole) {
                return "\([role, subrole].filter { !$0.isEmpty }.joined(separator: " ")) is on the SIFS window"
            }
        }
    }
    return nil
}

/// Only admit makes one, and every function that sends input takes one.
struct Admitted { let run: Run }

func admit(_ run: Run, _ input: Input) -> Admitted {
    verify(run)
    if let why = refusal(input, dialog: dialogOpen(run.pid)) { refuse(why) }
    return Admitted(run: run)
}

func perform(_ admitted: Admitted, _ element: AXUIElement, _ what: String) {
    verify(admitted.run)
    #if SIFS_AX_TEST
    print("pressed \(what)")
    #else
    let result = AXUIElementPerformAction(element, kAXPressAction as CFString)
    guard result == .success else { fail("pressing \(what) failed: \(result.rawValue)") }
    #endif
}

/// Presses the n-th button with this title, or with `after`, the first one
/// that follows the text `after` (a folder's path, say) in the page.
func press(_ admitted: Admitted, _ title: String, nth: Int, after: String?) {
    var ordered: [AXUIElement] = []
    walk(webArea(admitted.run.pid)) { node, _ in
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
    perform(admitted, matching[nth], "\"\(title)\"")
}

func sheet(_ pid: pid_t) -> AXUIElement {
    for _ in 0..<50 {
        if let found = children(window(pid)).first(where: { role($0) == kAXSheetRole }) { return found }
        usleep(100_000)
    }
    fail("no dialog is open on the SIFS window")
}

func cancelDialog(_ admitted: Admitted) {
    guard let cancel = find(sheet(admitted.run.pid), { role($0) == kAXButtonRole && string($0, kAXTitleAttribute) == "Cancel" }).first else {
        fail("the dialog has no Cancel button")
    }
    perform(admitted, cancel, "the dialog's Cancel")
}

func focus(_ admitted: Admitted, _ element: AXUIElement, _ what: String) {
    verify(admitted.run)
    #if SIFS_AX_TEST
    print("focused \(what)")
    #else
    let result = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    guard result == .success else { fail("focusing \(what) failed: \(result.rawValue)") }
    #endif
}

/// CGEvent.postToPid delivers to this process only, whichever app is in
/// front, and without bringing this one forward.
func post(_ admitted: Admitted, key: CGKeyCode, flags: CGEventFlags = [], unicode: [UniChar]? = nil) {
    #if SIFS_AX_TEST
    verify(admitted.run)
    print("sent key \(key) flags \(flags.rawValue)" + (unicode.map { " text " + String(utf16CodeUnits: $0, count: $0.count) } ?? ""))
    #else
    let source = CGEventSource(stateID: .privateState)
    for down in [true, false] {
        guard let event = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: down) else { fail("no key event") }
        event.flags = flags
        if let unicode { event.keyboardSetUnicodeString(stringLength: unicode.count, unicodeString: unicode) }
        verify(admitted.run)
        event.postToPid(admitted.run.pid)
        usleep(12_000)
    }
    #endif
}

let keys: [String: CGKeyCode] = [
    "down": 125, "up": 126, "left": 123, "right": 124, "return": 36, "escape": 53,
    "tab": 48, "delete": 51, "a": 0, "space": 49,
]

func type(_ admitted: Admitted, _ text: String) {
    for unit in text.utf16 {
        if unit == 10 || unit == 13 { post(admitted, key: keys["return"]!) } else { post(admitted, key: 0, unicode: [unit]) }
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
func search(_ admitted: Admitted, _ query: String, settle: Double) -> Screen {
    let pid = admitted.run.pid
    let box = searchBox(webArea(pid))
    focus(admitted, box, "the search box")
    for _ in string(box, kAXValueAttribute).utf16 { post(admitted, key: keys["delete"]!) }
    type(admitted, query)
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
    usage: sifs-ax <command> <run> [args]
    <run> is a run folder, <workspace>/runs/<id>, as sifs-verify passes it. Every
    command but supervise, status and stop refuses unless the run's pid is still
    the process the run started.
      supervise <run> [args]        start <run>/sifs as a child, hold it, and serve status and stop
      status <run>                  the supervisor's answer: running|exited <pid> <start>
      stop <run>                    ask the supervisor to stop its child; nothing is signalled without one
      check <run>                   the run's process, if it is still the one the run started
      wininfo <run>                 window number and on-screen flag (window server only)
      tree <run>                    the window's accessibility tree
      read <run>                    query, rows, footer and every page line as JSON
      search <run> <query> [settle] type a query, wait for settled results, print JSON
      wait <run> <regex> [seconds]  wait until a page line, row or the footer matches
      press <run> <title> [n | --after <text>]  AXPress the n-th button with this title, or the first after a text
      cancel-dialog <run>           press the open dialog's Cancel
      dialog-tree <run>             the open dialog's accessibility tree
      focus-search <run>            focus the search box
      focus-textarea <run>          focus the skipped-patterns text area
      type <run> <text>             post the text as key events ("\\n" is Return)
      key <run> <name> [--cmd] [--shift]
    While a native dialog is open, only dialog-tree, cancel-dialog, key escape and
    the commands that only read work.
    """
guard args.count >= 2 else { fail(usage) }
let command = args[0]
switch command {
case "supervise":
    supervise(runDir(args[1]), Array(args.dropFirst(2)))
case "status", "stop":
    let dir = runDir(args[1])
    guard let reply = ask(dir, command) else { refuse("no supervisor answers for \(dir), so nothing was signalled") }
    print(reply)
    exit(0)
default: break
}
let run = loadRun(args[1])
verify(run)

var input: Input?
switch command {
case "check":
    print("pid \(run.pid), started \(run.start), runs \(run.expected.exe) with HOME \(run.expected.home!)")
    exit(0)
case "wininfo":
    guard let (id, onscreen) = windowInfo(run.pid) else { fail("process \(run.pid) has no SIFS window") }
    print("\(id) \(onscreen ? "onscreen" : "offscreen")")
    exit(0)
case "key":
    guard args.count >= 3, let code = keys[args[2]] else { fail("key needs one of: \(keys.keys.sorted().joined(separator: " "))") }
    var flags: CGEventFlags = []
    if args.contains("--cmd") { flags.insert(.maskCommand) }
    if args.contains("--shift") { flags.insert(.maskShift) }
    input = .key(code, flags)
case "type", "search": input = .text
case "press": input = .press
case "focus-search", "focus-textarea": input = .focus
case "cancel-dialog": input = .cancelDialog
default: break
}
#if !SIFS_AX_TEST
guard AXIsProcessTrusted() else { fail("this terminal lacks Accessibility permission (System Settings > Privacy & Security > Accessibility)") }
#endif
let admitted = input.map { admit(run, $0) }

switch (command, admitted) {
case ("tree", _):
    dump(run.pid)
case ("read", _):
    printJSON(read(run.pid))
case let ("search", admitted?):
    guard args.count >= 3 else { fail(usage) }
    printJSON(search(admitted, args[2], settle: args.count > 3 ? Double(args[3]) ?? 0.5 : 0.5))
case ("wait", _):
    guard args.count >= 3 else { fail(usage) }
    waitFor(run.pid, args[2], seconds: args.count > 3 ? Double(args[3]) ?? 30 : 30)
case let ("press", admitted?):
    guard args.count >= 3 else { fail(usage) }
    if args.count > 4 && args[3] == "--after" {
        press(admitted, args[2], nth: 0, after: args[4])
    } else {
        press(admitted, args[2], nth: args.count > 3 ? max((Int(args[3]) ?? 1) - 1, 0) : 0, after: nil)
    }
case let ("cancel-dialog", admitted?):
    cancelDialog(admitted)
case ("dialog-tree", _):
    walk(sheet(run.pid)) { node, depth in
        print(String(repeating: "  ", count: depth) + role(node) + " " + string(node, kAXTitleAttribute) + " " + string(node, kAXValueAttribute))
        return true
    }
case let ("focus-search", admitted?):
    focus(admitted, searchBox(webArea(run.pid)), "the search box")
case let ("focus-textarea", admitted?):
    guard let area = find(webArea(run.pid), { role($0) == kAXTextAreaRole }).first else {
        fail("no skipped-patterns text area (open settings first: press ⚙︎)")
    }
    focus(admitted, area, "the skipped-patterns text area")
case let ("type", admitted?):
    guard args.count >= 3 else { fail(usage) }
    type(admitted, args[2])
case let ("key", admitted?):
    guard case let .key(code, flags)? = input else { fail(usage) }
    post(admitted, key: code, flags: flags)
default:
    fail(usage)
}
