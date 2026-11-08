// Read-only capture metadata and offline pixel checks. Never linked into any control or Caper.
import AppKit
import AVFoundation
import CoreGraphics
import Foundation
import Vision

@main
struct Pixels {
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

    @MainActor static func main() async throws {
        let args = CommandLine.arguments
        let mode = args[1]
        if mode == "window" {
            guard CGPreflightScreenCaptureAccess(), let pid = Int(args[2]),
                  let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
                throw NSError(domain: "Pixels.windowPermissionOrInventory", code: 1)
            }
            let matches = windows.filter {
                ($0[kCGWindowOwnerPID as String] as? NSNumber)?.intValue == pid &&
                ($0[kCGWindowLayer as String] as? NSNumber)?.intValue == 0
            }
            guard matches.count == 1, let window = matches.first,
                  let id = window[kCGWindowNumber as String] as? NSNumber,
                  let bounds = window[kCGWindowBounds as String] as? [String: Any] else {
                throw NSError(domain: "Pixels.noUniqueLiveWindow", code: 1)
            }
            try emit(["pid": pid, "windowID": id.intValue, "bounds": bounds, "onScreen": true])
            return
        }
        if mode == "video" {
            let asset = AVURLAsset(url: URL(fileURLWithPath: args[2]))
            let duration = try await asset.load(.duration).seconds
            guard duration.isFinite, duration > 0 else { throw NSError(domain: "Pixels.videoDuration", code: 1) }
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            var observations: [[String: Any]] = []
            // Offline inspection, not repeated capture: nine fixed interior timestamps.
            for index in 1...9 {
                let seconds = duration * Double(index) / 10
                let result = try await generator.image(at: CMTime(seconds: seconds, preferredTimescale: 600))
                var observation = try inspect(result.image, mode: mode)
                observation["requestedSeconds"] = seconds
                observation["actualSeconds"] = result.actualTime.seconds
                observations.append(observation)
                if observation["passed"] as? Bool == true {
                    guard let png = NSBitmapImageRep(cgImage: result.image).representation(using: .png, properties: [:]) else {
                        throw NSError(domain: "Pixels.png", code: 1)
                    }
                    try png.write(to: URL(fileURLWithPath: args[3]))
                    try emit(["passed": true, "durationSeconds": duration, "observations": observations])
                    return
                }
            }
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
