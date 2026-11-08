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
    private static var routingObserved = false
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
        let after = board.changeCount
        let present = readback != nil
        let exactMatch = readback == expected
        let typePolicy = types == [NSPasteboard.PasteboardType.string.rawValue]
        let stable = after == count
        let matches = exactMatch && typePolicy && stable
        var data: [String: Any] = ["writeSucceeded": writeSucceeded, "readbackMatches": matches,
            "readbackPresent": present, "exactCanaryMatch": exactMatch, "typePolicyMatches": typePolicy,
            "countStable": stable, "changeCountAfter": after,
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
                        var data = snapshot(note)
                        if event.type == .leftMouseDown {
                            data["mouseEvent"] = mouseEvent(event)
                            if !routingObserved {
                                routingObserved = true
                                data["routing"] = routingSnapshot(note, event)
                            } else { data["routing"] = ["status": "already-observed", "queryPerformed": false] }
                        }
                        emit(event.type == .leftMouseDown ? "app.mouseDown.beforeRouting" : "app.keyDown.beforeRouting", data)
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

    private static func routingIdentity(_ responder: NSResponder) -> String {
        let id = responders.object(forKey: responder) ?? (UUID().uuidString as NSString)
        responders.setObject(id, forKey: responder)
        return id as String
    }

    private static func point(_ value: NSPoint) -> [String: CGFloat] { ["x": value.x, "y": value.y] }
    private static func rect(_ value: NSRect) -> [String: CGFloat] {
        ["x": value.origin.x, "y": value.origin.y, "width": value.width, "height": value.height]
    }

    private static func mouseEvent(_ event: NSEvent) -> [String: Any] {
        ["windowID": event.window.map { routingIdentity($0) } ?? "nil",
         "windowNumber": event.windowNumber, "eventNumber": event.eventNumber,
         "timestampSeconds": event.timestamp, "windowPoint": point(event.locationInWindow)]
    }

    // This is the real handler boundary, unlike the monitor's predictive hitTest query.
    // The overlay always calls super exactly once with the original event, even when unarmed.
    static func mouseDown(_ phase: String, _ view: NSTextView, _ event: NSEvent) {
        guard directory != nil, views.object(forKey: view) != nil, event.type == .leftMouseDown, active() else { return }
        var data = snapshot(view)
        data["mouseEvent"] = mouseEvent(event)
        emit(phase, data)
    }

    private static func routingSnapshot(_ view: NSTextView, _ event: NSEvent) -> [String: Any] {
        var data: [String: Any] = ["status": "unavailable", "queryPerformed": false,
            "coordinateSpace": "event-window-base", "predictionOnly": true]
        guard let window = event.window, view.window === window else {
            data["reason"] = "missing-or-different-note-window"; return data
        }
        guard let root = window.contentView, let scroll = view.enclosingScrollView else {
            data["reason"] = "missing-content-or-scroll-view"; return data
        }
        let clip = scroll.contentView
        data["noteID"] = routingIdentity(view)
        data["scrollID"] = routingIdentity(scroll)
        data["clipID"] = routingIdentity(clip)
        data["rootID"] = routingIdentity(root)
        data["noteVisibleWindow"] = rect(view.convert(view.visibleRect, to: nil))
        data["clipVisibleWindow"] = rect(clip.convert(clip.visibleRect, to: nil))
        data["noteLocalPoint"] = point(view.convert(event.locationInWindow, from: nil))
        data["notePointRoundTripWindow"] = point(view.convert(view.convert(event.locationInWindow, from: nil), to: nil))
        // AppKit hitTest expects the RECEIVER'S SUPERVIEW coordinate space, not
        // receiver-local coordinates. nil conversion uses window-base coordinates.
        let queryPoint = root.superview?.convert(event.locationInWindow, from: nil) ?? event.locationInWindow
        data["queryPoint"] = point(queryPoint)
        data["querySpaceID"] = root.superview.map { routingIdentity($0) } ?? "window-base"
        data["queryPointRoundTripWindow"] = point(root.superview?.convert(queryPoint, to: nil) ?? queryPoint)
        // Identify hosting ancestors by an explicit class-name heuristic, not an AX label.
        var ancestor: NSView? = view
        var host: NSView?
        var depth = 0
        while let current = ancestor, depth < 16 {
            if String(describing: type(of: current)).hasPrefix("NSHostingView") { host = current; break }
            ancestor = current.superview; depth += 1
        }
        data["hostID"] = host.map { routingIdentity($0) } ?? "nil"
        data["hostIdentification"] = "nearest-NSHostingView-class-prefix"
        data["hostSearchTruncated"] = host == nil && ancestor != nil
        // Exactly one predictive query. It may invoke framework code; it does not
        // establish which view the subsequent event dispatch will actually call.
        let recipient = root.hitTest(queryPoint)
        data["queryPerformed"] = true
        data["status"] = "observed"
        data["recipientID"] = recipient.map { routingIdentity($0) } ?? "nil"
        var chain: [[String: Any]] = []
        var current = recipient
        while let item = current, chain.count < 16 {
            chain.append(["id": routingIdentity(item), "class": String(describing: type(of: item)),
                          "isNote": item === view, "isScroll": item === scroll,
                          "isClip": item === clip, "isHost": item === host, "isRoot": item === root])
            current = item.superview
        }
        data["ancestry"] = chain
        data["ancestryTruncated"] = current != nil
        return data
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
