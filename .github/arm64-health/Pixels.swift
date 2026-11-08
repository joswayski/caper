// Read-only capture metadata and offline pixel checks. Never linked into any control or Caper.
import AppKit
import AVFoundation
import CoreGraphics
import Foundation
import Vision

@main
struct Pixels {
    // Video-only diagnostics. Synchronous metadata logging adds overhead, not retries.
    @MainActor static var videoEvents: [[String: Any]] = []
    @MainActor static var videoStage = "not-started"

    static func timeFields(_ time: CMTime) -> [String: Any] {
        ["value": time.value, "timescale": time.timescale, "epoch": time.epoch,
         "flags": time.flags.rawValue, "seconds": time.seconds.isFinite ? time.seconds as Any : NSNull()]
    }

    static func errorChain(_ error: NSError) -> [String: Any] {
        var entries: [[String: Any]] = []
        var seen: Set<ObjectIdentifier> = []
        var current: NSError? = error
        while let item = current {
            guard seen.insert(ObjectIdentifier(item)).inserted else {
                return ["entries": entries, "end": "cycle"]
            }
            guard entries.count < 4 else { return ["entries": entries, "end": "depth-limit"] }
            var entry: [String: Any] = ["domain": String(item.domain.prefix(128)), "code": item.code,
                "message": String(item.localizedDescription.prefix(512))]
            if let reason = item.localizedFailureReason { entry["failureReason"] = String(reason.prefix(512)) }
            entries.append(entry)
            guard let underlying = item.userInfo[NSUnderlyingErrorKey] else {
                return ["entries": entries, "end": "no-underlying-error"]
            }
            guard let nested = underlying as? NSError else {
                return ["entries": entries, "end": "non-NSError-underlying"]
            }
            current = nested
        }
        return ["entries": entries, "end": "no-underlying-error"]
    }

    @MainActor static func videoEvent(_ stage: String, _ fields: [String: Any] = [:]) {
        videoStage = stage
        // Nine requests produce at most 44 records, including a final error.
        guard videoEvents.count < 64 else { return }
        var event = fields
        event["stage"] = stage
        event["sequence"] = videoEvents.count + 1
        event["uptimeNanoseconds"] = String(DispatchTime.now().uptimeNanoseconds)
        videoEvents.append(event)
        // stderr is incremental; stdout keeps the existing final-result JSON contract.
        if let data = try? JSONSerialization.data(withJSONObject: event, options: [.sortedKeys]) {
            FileHandle.standardError.write(Data("PIXELS_VIDEO ".utf8) + data + Data([10]))
        } else {
            FileHandle.standardError.write(Data("PIXELS_VIDEO {\"diagnosticEncodingFailed\":true}\n".utf8))
        }
    }

    static func emit(_ value: [String: Any]) throws {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }

    static func text(_ image: CGImage) throws -> [String] {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.recognitionLanguages = ["en-US"]
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
        return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    }

    static func normalized(_ text: [String]) -> String {
        text.joined().lowercased().filter { $0.isLetter || $0.isNumber }
    }

    static func inspect(_ image: CGImage, mode: String) throws -> [String: Any] {
        guard image.width >= 100, image.height >= 60 else { throw NSError(domain: "Pixels.tooSmall", code: 1) }
        if mode == "hosting" || mode == "video" {
            let lines = try text(image)
            let expected = mode == "hosting" ? "intelgraphicscontrol" : "standalonetextfield"
            return ["passed": normalized(lines).contains(expected), "recognizedText": lines,
                    "width": image.width, "height": image.height, "expected": expected]
        }
        guard mode == "hosting-canvas" || mode == "hosting-blur" else { throw NSError(domain: "Pixels.mode", code: 1) }
        let bitmap = NSBitmapImageRep(cgImage: image)
        var green = 0, samples = 0, left = image.width, right = -1
        for y in stride(from: 0, to: image.height, by: 2) {
            for x in stride(from: 0, to: image.width, by: 2) {
                samples += 1
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if color.greenComponent > 0.35 && color.greenComponent > color.redComponent * 1.3 && color.greenComponent > color.blueComponent * 1.2 {
                    green += 1; left = min(left, x); right = max(right, x)
                }
            }
        }
        let fraction = Double(green) / Double(samples)
        let span = Double(max(0, right - left + 1)) / Double(image.width)
        // Distinguish the fixture's wide Canvas ellipse from its small blurred circle.
        let shape = mode == "hosting-canvas" ? span > 0.7 : span > 0.15 && span < 0.6
        return ["passed": fraction > 0.05 && fraction < 0.95 && shape, "greenFraction": fraction,
                "greenWidthFraction": span, "width": image.width, "height": image.height]
    }

    static func window(pid: pid_t, inventoryURL: URL) throws -> [String: Any] {
        let started = ProcessInfo.processInfo.systemUptime
        // Leave time for process startup/reporting inside the unchanged external 2s cap.
        let budget = 1.5
        let interval = 0.05
        var observations: [[String: Any]] = []
        var report: [String: Any] = ["passed": false, "complete": false, "pid": pid,
            "status": "observing", "readinessBudgetSeconds": budget, "pollIntervalSeconds": interval]
        func save() throws {
            report["elapsedSeconds"] = ProcessInfo.processInfo.systemUptime - started
            report["observations"] = observations
            try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
                .write(to: inventoryURL, options: .atomic)
        }
        func alive() -> Bool {
            // Signal 0 observes existence/access only; it sends no signal to the fixture.
            if kill(pid, 0) == 0 { return true }
            let code = errno
            report["status"] = code == ESRCH ? "process-exited" : "process-check-failed"
            report["processCheckErrno"] = Int(code)
            return false
        }
        try save()
        while true {
            guard alive() else { break }
            guard ProcessInfo.processInfo.systemUptime - started < budget else {
                report["status"] = "timeout"; break
            }
            guard CGPreflightScreenCaptureAccess() else {
                report["status"] = "capture-permission-unavailable"; break
            }
            guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
                report["status"] = "inventory-unavailable"; break
            }
            let owned = windows.filter { ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid }
            var eligible: [[String: Any]] = []
            var candidates: [[String: Any]] = []
            var malformed = false
            for window in owned {
                let layer = window[kCGWindowLayer as String] as? NSNumber
                let id = window[kCGWindowNumber as String] as? NSNumber
                let bounds = window[kCGWindowBounds as String] as? [String: Any]
                var missing: [String] = []
                if layer == nil { missing.append("layer") }
                if layer?.intValue == 0 {
                    if id == nil || id!.int64Value <= 0 { missing.append("windowID") }
                    for key in ["X", "Y", "Width", "Height"] {
                        guard let value = bounds?[key] as? NSNumber, value.doubleValue.isFinite,
                              (key != "Width" && key != "Height") || value.doubleValue > 0 else {
                            missing.append("bounds." + key); continue
                        }
                    }
                }
                let valid = layer?.intValue == 0 && missing.isEmpty
                var candidate: [String: Any] = ["ownerPID": pid,
                    "layer": window[kCGWindowLayer as String] ?? NSNull(),
                    "windowID": window[kCGWindowNumber as String] ?? NSNull(),
                    "bounds": window[kCGWindowBounds as String] ?? NSNull(), "missingOrInvalidFields": missing,
                    "classification": valid ? "eligible" : (missing.isEmpty ? "nonzero-layer" : "missing-field")]
                candidate["onScreen"] = window[kCGWindowIsOnscreen as String] ?? NSNull()
                candidates.append(candidate)
                malformed = malformed || !missing.isEmpty
                if valid { eligible.append(candidate) }
            }
            observations.append(["elapsedSeconds": ProcessInfo.processInfo.systemUptime - started,
                "candidates": candidates, "eligibleCount": eligible.count,
                "status": malformed ? "missing-field" : (eligible.isEmpty ? "no-window" : (eligible.count > 1 ? "multiple-window" : "ready"))])
            // Preserve each PID-specific snapshot even if the external watchdog kills us.
            try save()
            guard alive() else { break }
            guard ProcessInfo.processInfo.systemUptime - started < budget else {
                report["status"] = "timeout"; break
            }
            if malformed { report["status"] = "missing-field"; break }
            if eligible.count > 1 { report["status"] = "multiple-window"; break }
            if let window = eligible.first {
                report["passed"] = true
                report["status"] = "ready"
                report["windowID"] = window["windowID"]
                report["bounds"] = window["bounds"]
                report["onScreen"] = true
                break
            }
            report["status"] = "no-window"
            Thread.sleep(forTimeInterval: min(interval, max(0, budget - (ProcessInfo.processInfo.systemUptime - started))))
        }
        report["complete"] = true
        try save()
        return report
    }

    @MainActor static func main() async {
        do {
            try await run(CommandLine.arguments)
        } catch {
            let error = error as NSError
            do {
                var report: [String: Any] = ["passed": false, "status": "error", "mode": CommandLine.arguments.dropFirst().first ?? "",
                    "error": ["domain": error.domain, "code": error.code, "message": error.localizedDescription]]
                if CommandLine.arguments.dropFirst().first == "video" {
                    let failedStage = videoStage
                    videoEvent("error", ["failedStage": failedStage, "errorChain": errorChain(error)])
                    report["videoDiagnostic"] = ["failedStage": failedStage, "events": videoEvents]
                }
                try emit(report)
            } catch {
                FileHandle.standardOutput.write(Data("{\"passed\":false,\"status\":\"error-report-encoding-failed\"}\n".utf8))
            }
            exit(1)
        }
    }

    @MainActor static func run(_ args: [String]) async throws {
        guard args.count >= 3 else { throw NSError(domain: "Pixels.arguments", code: 1) }
        let mode = args[1]
        if mode == "window" {
            guard args.count == 4, let pid = pid_t(args[2]), pid > 0 else { throw NSError(domain: "Pixels.arguments", code: 1) }
            let report = try window(pid: pid, inventoryURL: URL(fileURLWithPath: args[3]))
            try emit(report)
            if report["passed"] as? Bool != true { exit(1) }
            return
        }
        if mode == "video" {
            guard args.count == 4 else { throw NSError(domain: "Pixels.arguments", code: 1) }
            videoEvent("asset.create")
            let asset = AVURLAsset(url: URL(fileURLWithPath: args[2]))
            videoEvent("duration.load.start")
            let durationTime = try await asset.load(.duration)
            let duration = durationTime.seconds
            videoEvent("duration.load.result", ["duration": timeFields(durationTime)])
            videoEvent("duration.validate")
            guard duration.isFinite, duration > 0 else { throw NSError(domain: "Pixels.videoDuration", code: 1) }
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            // Default tolerances are infinite; decode frame-accurately at the fixed samples.
            generator.requestedTimeToleranceBefore = .zero
            generator.requestedTimeToleranceAfter = .zero
            var observations: [[String: Any]] = []
            // Offline inspection, not repeated capture: nine fixed interior timestamps.
            for index in 1...9 {
                let seconds = duration * Double(index) / 10
                let requested = CMTime(seconds: seconds, preferredTimescale: 600)
                videoEvent("image.request", ["index": index, "requestedSeconds": seconds, "requestedTime": timeFields(requested)])
                let result = try await generator.image(at: requested)
                videoEvent("image.result", ["index": index, "actualTime": timeFields(result.actualTime)])
                videoEvent("inspection.start", ["index": index])
                var observation = try inspect(result.image, mode: mode)
                observation["requestedSeconds"] = seconds
                observation["actualSeconds"] = result.actualTime.seconds
                observations.append(observation)
                videoEvent("inspection.result", ["index": index, "observation": observation])
                if observation["passed"] as? Bool == true {
                    videoEvent("png.encode", ["index": index])
                    guard let png = NSBitmapImageRep(cgImage: result.image).representation(using: .png, properties: [:]) else {
                        throw NSError(domain: "Pixels.png", code: 1)
                    }
                    videoEvent("png.write", ["index": index])
                    try png.write(to: URL(fileURLWithPath: args[3]))
                    videoEvent("output.success", ["index": index])
                    try emit(["passed": true, "durationSeconds": duration, "observations": observations])
                    return
                }
            }
            videoEvent("output.no-content")
            try emit(["passed": false, "durationSeconds": duration, "observations": observations])
            exit(1)
        }
        guard let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: args[2]))),
              let image = bitmap.cgImage else { throw NSError(domain: "Pixels.imageDecode", code: 1) }
        let report = try inspect(image, mode: mode)
        try emit(report)
        if report["passed"] as? Bool != true { exit(1) }
    }
}
