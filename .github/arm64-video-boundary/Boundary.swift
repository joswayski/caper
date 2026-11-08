// NEVER MERGE. One fixed timestamp, one generator request, one fresh process. No OCR.
import AppKit
import AVFoundation
import Foundation

@main
struct Boundary {
    @MainActor static var events: [[String: Any]] = []
    @MainActor static var stage = "arguments"
    @MainActor static var generated = false

    static func timeFields(_ time: CMTime) -> [String: Any] {
        ["value": time.value, "timescale": time.timescale, "epoch": time.epoch,
         "flags": time.flags.rawValue, "seconds": time.seconds.isFinite ? time.seconds as Any : NSNull()]
    }

    static func errorChain(_ error: NSError) -> [String: Any] {
        var entries: [[String: Any]] = []
        var seen: Set<ObjectIdentifier> = []
        var current = error
        while true {
            guard seen.insert(ObjectIdentifier(current)).inserted else {
                return ["entries": entries, "end": "cycle"]
            }
            guard entries.count < 16 else { return ["entries": entries, "end": "depth-limit"] }
            let message = current.localizedDescription
            let reason = current.localizedFailureReason
            var entry: [String: Any] = ["domain": String(current.domain.prefix(128)), "code": current.code,
                "message": String(message.prefix(512)),
                "textTruncated": current.domain.count > 128 || message.count > 512 || (reason?.count ?? 0) > 512]
            if let reason { entry["failureReason"] = String(reason.prefix(512)) }
            entries.append(entry)
            guard let underlying = current.userInfo[NSUnderlyingErrorKey] else {
                return ["entries": entries, "end": "no-underlying-error"]
            }
            guard let nested = underlying as? NSError else {
                return ["entries": entries, "end": "non-NSError-underlying"]
            }
            current = nested
        }
    }

    @MainActor static func event(_ next: String, _ fields: [String: Any] = [:]) throws {
        stage = next
        var value = fields
        value["stage"] = next
        value["sequence"] = events.count + 1
        value["uptimeNanoseconds"] = String(DispatchTime.now().uptimeNanoseconds)
        events.append(value)
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        FileHandle.standardError.write(Data("BOUNDARY_VIDEO ".utf8) + data + Data([10]))
    }

    @MainActor static func emit(_ fields: [String: Any]) throws {
        var result = fields
        result["imageGenerationSucceeded"] = generated
        result["contentAcceptance"] = "not-evaluated"
        result["events"] = events
        let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
        FileHandle.standardOutput.write(data + Data([10]))
    }

    @MainActor static func main() async {
        do {
            let args = CommandLine.arguments
            guard args.count == 4, ["369", "370", "1720"].contains(args[2]), let tick = Int64(args[2]) else {
                throw NSError(domain: "Boundary.arguments", code: 1)
            }
            let requested = CMTime(value: tick, timescale: 600)
            try event("asset.create", ["requestedTime": timeFields(requested), "pid": ProcessInfo.processInfo.processIdentifier])
            let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
            try event("duration.load.start")
            let duration = try await asset.load(.duration)
            try event("duration.load.result", ["duration": timeFields(duration)])
            try event("duration.validate")
            guard duration.seconds.isFinite, duration.seconds > 0 else {
                throw NSError(domain: "Boundary.videoDuration", code: 1)
            }
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            generator.requestedTimeToleranceBefore = .zero
            generator.requestedTimeToleranceAfter = .zero
            try event("image.request", ["requestedTime": timeFields(requested)])
            let result = try await generator.image(at: requested)
            generated = true
            try event("image.result", ["actualTime": timeFields(result.actualTime),
                "width": result.image.width, "height": result.image.height])
            try event("png.encode")
            guard let png = NSBitmapImageRep(cgImage: result.image).representation(using: .png, properties: [:]) else {
                throw NSError(domain: "Boundary.png", code: 1)
            }
            try event("png.write")
            try png.write(to: URL(fileURLWithPath: args[3]))
            try event("output.success")
            try emit(["status": "complete", "imageOutputComplete": true])
        } catch {
            let failedStage = stage
            do {
                try event("error", ["failedStage": failedStage, "errorChain": errorChain(error as NSError)])
                try emit(["status": "error", "failedStage": failedStage, "imageOutputComplete": false])
            } catch {
                FileHandle.standardError.write(Data("BOUNDARY_REPORT_ENCODING_FAILED\n".utf8))
            }
            exit(1)
        }
    }
}
