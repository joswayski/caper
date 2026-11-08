// NEVER MERGE. One session-scoped transaction on a disposable hosted VM only.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 5 else { exit(64) }
let action = CommandLine.arguments[1]
let stateURL = URL(fileURLWithPath: CommandLine.arguments[2])
let snapshotURL = URL(fileURLWithPath: CommandLine.arguments[3])
let traceURL = URL(fileURLWithPath: CommandLine.arguments[4])
var trace: [String: Any] = ["action": action, "passed": false, "scope": "forSession"]

enum Rejected: Error { case reason(String) }
func require(_ condition: Bool, _ reason: String) throws {
    if !condition { throw Rejected.reason(reason) }
}
func write(_ value: [String: Any], to url: URL) throws {
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]).write(to: url, options: .atomic)
}
func read(_ url: URL) throws -> [String: Any] {
    guard let value = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any] else {
        throw Rejected.reason("Expected JSON object")
    }
    return value
}
func same(_ lhs: [String: Any], _ rhs: [String: Any]) -> Bool {
    NSDictionary(dictionary: lhs).isEqual(NSDictionary(dictionary: rhs))
}
func modeRecord(_ mode: CGDisplayMode) -> [String: Any] {
    ["width": mode.width, "height": mode.height, "pixelWidth": mode.pixelWidth, "pixelHeight": mode.pixelHeight,
     "refreshRate": mode.refreshRate, "ioDisplayModeID": mode.ioDisplayModeID, "ioFlags": mode.ioFlags,
     "usableForDesktopGUI": mode.isUsableForDesktopGUI()]
}
func display(_ snapshot: [String: Any]) throws -> [String: Any] {
    guard snapshot["complete"] as? Bool == true,
          let displays = snapshot["displays"] as? [[String: Any]], displays.count == 1,
          let screens = snapshot["screens"] as? [[String: Any]], screens.count == 1 else {
        throw Rejected.reason("Incomplete or non-single-display snapshot")
    }
    return displays[0]
}
func perform() throws {
    try require(action == "setup" || action == "restore", "Unknown action")
    #if !arch(arm64)
    throw Rejected.reason("ARM64 hosted VM only")
    #endif
    let env = ProcessInfo.processInfo.environment
    let keys = ["GITHUB_RUN_ID", "GITHUB_SHA", "GITHUB_REF", "GITHUB_RUN_ATTEMPT", "RUNNER_ENVIRONMENT", "ImageVersion", "DEVELOPER_DIR"]
    let identity = Dictionary(uniqueKeysWithValues: keys.map { ($0, env[$0] ?? "") })
    try require(identity.values.allSatisfy { !$0.isEmpty }, "Missing hosted identity")
    try require(env["RUNNER_ENVIRONMENT"] == "github-hosted" && env["GITHUB_RUN_ATTEMPT"] == "1"
        && env["GITHUB_REF"] == "refs/heads/apple-arm64-health-sidebar-run"
        && env["ImageVersion"] == "20261102.0473.1"
        && env["DEVELOPER_DIR"] == "/Applications/Xcode_26.6.app/Contents/Developer", "Wrong hosted identity")
    trace["identity"] = identity
    let snapshot = try read(snapshotURL)
    let observed = try display(snapshot)
    guard let number = observed["displayID"] as? NSNumber,
          let observedMode = observed["currentMode"] as? [String: Any] else {
        throw Rejected.reason("Missing observed display/mode")
    }
    let id = number.uint32Value
    var ids = [CGDirectDisplayID](repeating: 0, count: 2)
    var count: UInt32 = 0
    let enumeration = CGGetOnlineDisplayList(2, &ids, &count)
    trace["enumerationStatus"] = enumeration.rawValue
    trace["onlineIDs"] = Array(ids.prefix(Int(min(count, 2))))
    try write(trace, to: traceURL)
    try require(enumeration == .success && count == 1 && ids[0] == id && id == CGMainDisplayID()
        && CGDisplayIsActive(id) != 0 && CGDisplayIsAsleep(id) == 0 && CGDisplayMirrorsDisplay(id) == 0,
        "Display topology changed or is unsupported")
    guard let current = CGDisplayCopyDisplayMode(id),
          let modes = CGDisplayCopyAllDisplayModes(id, nil) as? [CGDisplayMode], modes.count <= 2048 else {
        throw Rejected.reason("Missing/bounded current or advertised modes")
    }
    trace["displayID"] = id
    trace["currentMode"] = modeRecord(current)
    trace["advertisedModes"] = modes.map(modeRecord)
    try write(trace, to: traceURL)
    try require(same(modeRecord(current), observedMode), "Mode changed since fresh inventory")
    let matches: [CGDisplayMode]
    if action == "setup" {
        try require(!FileManager.default.fileExists(atPath: stateURL.path), "Original state already exists; no repeat setup")
        matches = modes.filter { $0.width == 1600 && $0.height == 1200 && $0.pixelWidth == 1600
            && $0.pixelHeight == 1200 && $0.isUsableForDesktopGUI() }
        try require(modes.filter { same(modeRecord($0), observedMode) }.count == 1, "Original mode not uniquely restorable")
    } else {
        let state = try read(stateURL)
        guard let original = state["original"] as? [String: Any],
              let savedIdentity = state["identity"] as? [String: Any],
              let selected = state["selected"] as? [String: Any],
              let originalMode = try display(original)["currentMode"] as? [String: Any] else {
            throw Rejected.reason("Invalid original state")
        }
        try require(same(savedIdentity, identity), "Restoration belongs to another run")
        try require((try display(original)["displayID"] as? NSNumber)?.uint32Value == id, "Original display changed")
        try require(same(observedMode, selected) || same(observedMode, originalMode), "Unexpected third-party mode; not overwritten")
        matches = modes.filter { same(modeRecord($0), originalMode) }
    }
    try require(matches.count == 1, "Required advertised mode absent or ambiguous; no fallback")
    let target = matches[0]
    trace["targetMode"] = modeRecord(target)
    if action == "setup" {
        // Save original state atomically BEFORE any API can change the session.
        try write(["identity": identity, "original": snapshot, "selected": modeRecord(target)], to: stateURL)
    }
    if same(modeRecord(current), modeRecord(target)) {
        trace["switchAttempted"] = false
        return
    }
    trace["switchAttempted"] = true
    try write(trace, to: traceURL)
    var config: CGDisplayConfigRef?
    let begin = CGBeginDisplayConfiguration(&config)
    defer {
        // Completion invalidates its token even on error; cancel only a pending transaction.
        if begin == .success, let pending = config {
            trace["cancelStatus"] = CGCancelDisplayConfiguration(pending).rawValue
            try? write(trace, to: traceURL)
        }
    }
    trace["beginStatus"] = begin.rawValue
    try write(trace, to: traceURL)
    try require(begin == .success && config != nil, "Begin transaction failed")
    let configure = CGConfigureDisplayWithDisplayMode(config, id, target, nil)
    trace["configureStatus"] = configure.rawValue
    try write(trace, to: traceURL)
    try require(configure == .success, "Configure transaction failed")
    let complete = CGCompleteDisplayConfiguration(config, .forSession)
    config = nil
    trace["completeStatus"] = complete.rawValue
    try write(trace, to: traceURL)
    try require(complete == .success, "Complete transaction failed")
}
do {
    try perform()
    trace["passed"] = true // API result only; fresh usable-point admission is outside this process.
} catch {
    trace["error"] = String(describing: error)
}
do {
    try write(trace, to: traceURL)
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: trace, options: [.prettyPrinted, .sortedKeys]))
    FileHandle.standardOutput.write(Data([10]))
} catch {
    FileHandle.standardError.write(Data("Trace write failed: \(error)\n".utf8))
    exit(1)
}
exit(trace["passed"] as? Bool == true ? 0 : 1)
