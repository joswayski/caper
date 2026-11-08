// NEVER MERGE. Copied into ONLY the diagnostic CaperCore and UI-test targets.
#if os(macOS)
import AppKit
import CryptoKit

@MainActor enum ForwardNoteObservation {
    private static var sequence = 0
    private static let stream = UUID().uuidString
    private static let views = NSMapTable<NSTextView, NSString>.weakToStrongObjects()
    private static let responders = NSMapTable<NSResponder, NSString>.weakToStrongObjects()
    private static weak var note: NSTextView?
    private static let flags = NSMapTable<NSTextView, NSDictionary>.weakToStrongObjects()
    private static var monitor: Any?
    private static var ready = false
    private static var finished = false
    private static var testDirectory: URL?
    private static let canary = "TEST FIXTURE — cancel this note, not the channel draft"
    private static var directory: URL? {
        let env = ProcessInfo.processInfo.environment
        guard Bundle.main.bundleIdentifier == "chat.caper.appkit-development",
              env["CAPER_TEST_MODE"] == "parity",
              env["CAPER_API_BASE_URL"] == "http://127.0.0.1:3001",
              let path = env["CAPER_FORWARD_OBSERVATION"],
              URL(fileURLWithPath: path).lastPathComponent.hasPrefix("caper-forward-observation-") else { return nil }
        return URL(fileURLWithPath: path)
    }

    private static func emit(_ event: String, _ fields: [String: Any] = [:]) {
        guard sequence <= 256 else { return }
        sequence += 1
        var data = fields
        data["event"] = sequence == 257 ? "observation.limit" : event
        data["sequence"] = sequence
        data["stream"] = stream
        data["pid"] = ProcessInfo.processInfo.processIdentifier
        data["uptimeNanoseconds"] = String(DispatchTime.now().uptimeNanoseconds)
        guard let bytes = try? JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]),
              let text = String(data: bytes, encoding: .utf8) else { return }
        FileHandle.standardError.write(Data(("CAPER_FORWARD_OBSERVATION " + text + "\n").utf8))
        if let root = testDirectory ?? directory {
            let path = root.appendingPathComponent("events-" + stream + ".jsonl")
            do {
                if !FileManager.default.fileExists(atPath: path.path) {
                    try Data().write(to: path, options: .withoutOverwriting)
                }
                let file = try FileHandle(forWritingTo: path)
                defer { try? file.close() }
                try file.seekToEnd()
                try file.write(contentsOf: Data((text + "\n").utf8))
            } catch {
                FileHandle.standardError.write(Data("CAPER_FORWARD_OBSERVATION_IO_ERROR\n".utf8))
            }
        }
    }

    static func prepare() -> URL? {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("caper-forward-observation-" + UUID().uuidString)
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
            testDirectory = url
            emit("test.prepared", ["directory": url.path])
            return url
        } catch { emit("test.prepare-error"); return nil }
    }

    static func begin(_ url: URL?, writeSucceeded: Bool, expected: String) {
        guard let url else { emit("test.missing-directory"); return }
        let board = NSPasteboard.general
        let count = board.changeCount
        // Only retain metadata for the just-written synthetic canary. An interference
        // or failed write never emits the unexpected contents or their digest/types.
        let readback = writeSucceeded ? board.string(forType: .string) : nil
        let types = board.types?.map(\.rawValue) ?? []
        let matches = readback == expected && types == [NSPasteboard.PasteboardType.string.rawValue] && board.changeCount == count
        var data: [String: Any] = ["writeSucceeded": writeSucceeded, "readbackMatches": matches,
            "changeCount": count, "started": String(DispatchTime.now().uptimeNanoseconds), "active": true]
        if matches, let readback {
            data["sha256"] = SHA256.hash(data: Data(readback.utf8)).map { String(format: "%02x", $0) }.joined()
            data["utf16Length"] = readback.utf16.count
            data["types"] = types
        }
        emit("test.pasteboard", data)
        do {
            try JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]).write(to: url.appendingPathComponent("marker.json"), options: .atomic)
        } catch { emit("test.marker-error") }
    }

    static func end(_ url: URL?) {
        guard let url else { return }
        do {
            try Data("{\"active\":false}".utf8).write(to: url.appendingPathComponent("marker.json"), options: .atomic)
            emit("test.end")
        } catch { emit("test.end-error") }
    }

    private static func active() -> Bool {
        guard !finished, let directory,
              let bytes = try? Data(contentsOf: directory.appendingPathComponent("marker.json")), bytes.count <= 4096,
              let marker = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any] else { return false }
        guard marker["active"] as? Bool == true else { finished = true; return false }
        guard let raw = marker["started"] as? String, let start = UInt64(raw) else { return false }
        let now = DispatchTime.now().uptimeNanoseconds
        guard now >= start, now - start <= 12_000_000_000 else {
            finished = true; emit("app.expired"); return false
        }
        return true
    }

    private static func identity(_ view: NSTextView) -> String {
        if let id = views.object(forKey: view) { return id as String }
        let id = UUID().uuidString
        views.setObject(id as NSString, forKey: view)
        return id
    }

    private static func snapshot(_ view: NSTextView) -> [String: Any] {
        let responder = NSApp.keyWindow?.firstResponder
        var data: [String: Any] = ["view": identity(view), "isEditable": view.isEditable,
            "isSelectable": view.isSelectable, "stringUTF16Length": view.string.utf16.count,
            "appActive": NSApp.isActive, "windowIsKey": view.window?.isKeyWindow ?? false,
            "windowNumber": view.window?.windowNumber ?? -1,
            "keyWindowNumber": NSApp.keyWindow?.windowNumber ?? -1,
            "firstResponderIsNote": responder === view,
            "firstResponderClass": responder.map { String(describing: type(of: $0)) } ?? "nil"]
        if let other = responder as? NSTextView, let id = views.object(forKey: other) { data["firstResponderView"] = id as String }
        if let responder {
            let id = responders.object(forKey: responder) ?? (UUID().uuidString as NSString)
            responders.setObject(id, forKey: responder)
            data["firstResponderID"] = id as String
        }
        for (key, value) in (flags.object(forKey: view) as? [String: Bool] ?? [:]) { data[key] = value }
        return data
    }

    static func update(_ event: String, _ view: NSTextView, id: String, editable: Bool, enabled: Bool, bindingLength: Int, resync: Bool) {
        guard directory != nil, id == "forward-note", !finished else { return }
        note = view
        _ = identity(view)
        flags.setObject(["swiftUIEditable": editable, "swiftUIEnabled": enabled] as NSDictionary, forKey: view)
        if !ready {
            ready = true
            // Returns the identical event; never consumes, synthesizes or redirects input.
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .keyDown]) { event in
                MainActor.assumeIsolated {
                    if let note, active() {
                        emit(event.type == .leftMouseDown ? "app.mouseDown.beforeRouting" : "app.keyDown.beforeRouting", snapshot(note))
                    }
                }
                return event
            }
            var data = snapshot(view)
            data["monitorInstalled"] = monitor != nil
            emit("app.ready", data)
        }
        observe(event, view, ["bindingUTF16Length": bindingLength, "wouldResync": resync])
    }

    static func observe(_ event: String, _ view: NSTextView, _ fields: [String: Any] = [:]) {
        guard directory != nil, views.object(forKey: view) != nil, active() else { return }
        var data = snapshot(view)
        for (key, value) in fields { data[key] = value }
        if event == "paste.enter" { data["pasteboard"] = pasteboardSnapshot() }
        emit(event, data)
    }

    private static func pasteboardSnapshot() -> [String: Any] {
        guard let directory,
              let bytes = try? Data(contentsOf: directory.appendingPathComponent("marker.json")), bytes.count <= 4096,
              let marker = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any],
              let originalCount = marker["changeCount"] as? Int else { return ["error": "marker-unavailable"] }
        let expectedTypes = [NSPasteboard.PasteboardType.string.rawValue]
        let expectedHash = SHA256.hash(data: Data(canary.utf8)).map { String(format: "%02x", $0) }.joined()
        let safe = marker["writeSucceeded"] as? Bool == true && marker["readbackMatches"] as? Bool == true
            && marker["sha256"] as? String == expectedHash && marker["types"] as? [String] == expectedTypes
            && marker["utf16Length"] as? Int == canary.utf16.count
        let board = NSPasteboard.general
        let count = board.changeCount
        var readback: String?
        var types: [String] = []
        var inspected = false
        // A changed count or failed original observation forbids content inspection.
        // The count/type/read sequence is not atomic; recheck and suppress any mismatch.
        if safe && count == originalCount {
            types = board.types?.map(\.rawValue) ?? []
            if types == expectedTypes && board.changeCount == count {
                inspected = true
                readback = board.string(forType: .string)
            }
        }
        let after = board.changeCount
        let changed = count != originalCount || after != originalCount
        let matches = safe && !changed && types == expectedTypes && readback?.utf8.elementsEqual(canary.utf8) == true
        var data: [String: Any] = ["markerChangeCount": originalCount, "changeCount": count, "changeCountAfter": after,
            "changeStatus": changed ? "changed" : "unchanged", "provenanceSafe": safe, "contentInspected": inspected,
            "readbackStatus": matches ? "matching-canary" : (safe && count == originalCount ? "readback-unavailable" : "skipped-unsafe-provenance")]
        if matches, let readback {
            data["sha256"] = SHA256.hash(data: Data(readback.utf8)).map { String(format: "%02x", $0) }.joined()
            data["utf16Length"] = readback.utf16.count
            data["types"] = types
        }
        return data
    }
}
#endif
