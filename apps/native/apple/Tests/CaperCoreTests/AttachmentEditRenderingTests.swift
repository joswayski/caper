import AVFoundation
import CoreGraphics
import CoreMedia
import CoreVideo
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import CaperCore

/// Attachment edits painted with CoreGraphics (`AttachmentEditRenderer`) and
/// applied by the preparer, checked through the colours of known pixels.
final class AttachmentEditRenderingTests: XCTestCase {
    private let red: [UInt8] = [255, 0, 0, 255]
    private let green: [UInt8] = [0, 255, 0, 255]
    private let blue: [UInt8] = [0, 0, 255, 255]
    private let white: [UInt8] = [255, 255, 255, 255]
    private let black: [UInt8] = [0, 0, 0, 255]
    private let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!

    private func image(width: Int, height: Int, pixel: (Int, Int) -> [UInt8]) -> CGImage {
        var bytes: [UInt8] = []
        for y in 0..<height { for x in 0..<width { bytes += pixel(x, y) } }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4, space: sRGB,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }

    /// Red top-left, green top-right, blue bottom-left, white bottom-right.
    private func quadrants(width: Int, height: Int) -> CGImage {
        image(width: width, height: height) { x, y in
            switch (x < width / 2, y < height / 2) {
            case (true, true): return red
            case (false, true): return green
            case (true, false): return blue
            case (false, false): return white
            }
        }
    }

    private func encoded(_ image: CGImage, type: UTType, quality: Double = 1) throws -> Data {
        let output = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output as CFMutableData, type.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return output as Data
    }

    private func decoded(_ url: URL) throws -> CGImage {
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(url as CFURL, nil))
        return try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
    }

    /// RGBA of one pixel, rows from the top.
    private func pixel(_ image: CGImage, _ x: Int, _ y: Int) throws -> [UInt8] {
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        let offset = (y * pixels.width + x) * 4
        return Array(pixels.straight[offset..<offset + 4])
    }

    private func assertColor(_ image: CGImage, _ x: Int, _ y: Int, _ expected: [UInt8], tolerance: Int = 3, _ note: String = "",
                             file: StaticString = #filePath, line: UInt = #line) throws {
        let actual = try pixel(image, x, y)
        let close = zip(actual, expected).allSatisfy { abs(Int($0) - Int($1)) <= tolerance }
        XCTAssertTrue(close, "\(note) (\(x), \(y)) is \(actual), not \(expected)", file: file, line: line)
    }

    private func render(_ source: CGImage?, width: Int, height: Int, _ edit: AttachmentEdit, outputWidth: Int? = nil, outputHeight: Int? = nil) throws -> CGImage {
        let region = edit.cropPixels(width: width, height: height)
        return try XCTUnwrap(AttachmentEditRenderer.render(source, width: width, height: height, edit: edit, region: region,
                                                           outputWidth: outputWidth ?? region.width, outputHeight: outputHeight ?? region.height,
                                                           space: sRGB))
    }

    func testQuarterTurnsAreClockwise() throws {
        let source = quadrants(width: 16, height: 8)
        let turned = try render(source, width: 16, height: 8, AttachmentEdit(rotation: 90))
        XCTAssertEqual(turned.width, 8)
        XCTAssertEqual(turned.height, 16)
        try assertColor(turned, 1, 1, blue, "the bottom-left corner turns to the top-left")
        try assertColor(turned, 6, 1, red)
        try assertColor(turned, 6, 14, green)
        try assertColor(turned, 1, 14, white)
        let upsideDown = try render(source, width: 16, height: 8, AttachmentEdit(rotation: 180))
        try assertColor(upsideDown, 1, 1, white)
        try assertColor(upsideDown, 14, 1, blue)
        try assertColor(upsideDown, 1, 6, green)
        try assertColor(upsideDown, 14, 6, red)
        let left = try render(source, width: 16, height: 8, AttachmentEdit.original.rotatedLeft())
        XCTAssertEqual(left.width, 8)
        try assertColor(left, 1, 1, green, "a left turn brings the top-right corner to the top-left")
        try assertColor(left, 6, 1, white)
        try assertColor(left, 1, 14, red)
        try assertColor(left, 6, 14, blue)
    }

    func testCropsKeepTheirRegionAtNativePixels() throws {
        let source = quadrants(width: 16, height: 8)
        let corner = try render(source, width: 16, height: 8, AttachmentEdit(crop: EditRect(x: 0.5, y: 0, width: 0.5, height: 0.5)))
        XCTAssertEqual(corner.width, 8)
        XCTAssertEqual(corner.height, 4)
        for (x, y) in [(0, 0), (7, 0), (0, 3), (7, 3)] { try assertColor(corner, x, y, green, "the top-right quadrant") }
        // The crop is of the turned picture: its top-left quadrant is the original's bottom-left.
        let turned = try render(source, width: 16, height: 8, AttachmentEdit(rotation: 90, crop: EditRect(x: 0, y: 0, width: 0.5, height: 0.5)))
        XCTAssertEqual(turned.width, 4)
        XCTAssertEqual(turned.height, 8)
        for (x, y) in [(0, 0), (3, 0), (0, 7), (3, 7)] { try assertColor(turned, x, y, blue) }
        let large = quadrants(width: 64, height: 32)
        let scaled = try render(large, width: 64, height: 32, AttachmentEdit(crop: EditRect(x: 0, y: 0.5, width: 0.5, height: 0.5)),
                                outputWidth: 8, outputHeight: 4)
        XCTAssertEqual(scaled.width, 8)
        try assertColor(scaled, 4, 2, blue, "scaled to the output size")
    }

    func testStrokesHaveRoundCapsAndATapIsADot() throws {
        let source = image(width: 100, height: 100) { _, _ in white }
        let dot = StrokeMark(color: "#000000", width: 0.1, points: [0.25, 0.75])
        let dotted = try render(source, width: 100, height: 100, AttachmentEdit(marks: [.stroke(dot)]))
        try assertColor(dotted, 25, 75, black, "the dot's centre, measured from the top")
        try assertColor(dotted, 25, 25, white, "not mirrored")
        try assertColor(dotted, 32, 75, white, "a 10-pixel dot")
        let line = StrokeMark(color: "#E5484D", width: 0.04, points: [0.1, 0.5, 0.9, 0.5])
        let drawn = try render(source, width: 100, height: 100, AttachmentEdit(marks: [.stroke(line)]))
        try assertColor(drawn, 50, 49, [229, 72, 77, 255], "the line's colour")
        try assertColor(drawn, 9, 49, [229, 72, 77, 255], "a round cap reaches past the end point")
        try assertColor(drawn, 50, 44, white, "4 pixels wide")
        try assertColor(drawn, 4, 49, white)
    }

    func testStrokesScaleWithThePicturesLongerEdge() throws {
        let source = image(width: 200, height: 100) { _, _ in white }
        let dot = StrokeMark(color: "#000000", width: 0.1, points: [0.5, 0.5])
        let drawn = try render(source, width: 200, height: 100, AttachmentEdit(marks: [.stroke(dot)]))
        try assertColor(drawn, 108, 50, black, "20 pixels across: a tenth of 200")
        try assertColor(drawn, 113, 50, white)
    }

    func testTextStaysUprightAndCentredWhenThePictureTurns() throws {
        let source = image(width: 100, height: 200) { _, _ in white }
        let text = TextMark(color: "#000000", size: 0.2, x: 0.5, y: 0.5, text: "———")
        let drawn = try render(source, width: 100, height: 200, AttachmentEdit(rotation: 90, marks: [.text(text)]))
        XCTAssertEqual(drawn.width, 200)
        XCTAssertEqual(drawn.height, 100)
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(drawn))
        var minX = Int.max, maxX = Int.min, minY = Int.max, maxY = Int.min
        for y in 0..<pixels.height {
            for x in 0..<pixels.width where pixels.straight[(y * pixels.width + x) * 4] < 128 {
                minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        XCTAssertLessThan(minX, maxX, "the text was painted")
        XCTAssertGreaterThan(maxX - minX, 3 * (maxY - minY), "the dashes stay horizontal")
        XCTAssertEqual(Double(minX + maxX) / 2, 100, accuracy: 4, "centred on its point")
        XCTAssertEqual(Double(minY + maxY) / 2, 50, accuracy: 12)
    }

    func testOverlaysPaintOnlyTheMarksAtTheOutputSize() throws {
        let dot = StrokeMark(color: "#FFFFFF", width: 0.2, points: [0.5, 0.5])
        let overlay = try render(nil, width: 100, height: 100, AttachmentEdit(marks: [.stroke(dot)]), outputWidth: 50, outputHeight: 50)
        XCTAssertEqual(overlay.width, 50)
        try assertColor(overlay, 25, 25, white)
        XCTAssertEqual(try pixel(overlay, 2, 2)[3], 0, "transparent outside the marks")
        try assertColor(overlay, 33, 25, [0, 0, 0, 0], tolerance: 0, "the 20-pixel dot is 10 pixels at half size")
    }

    // MARK: Preparing edited files

    func testEditedPNGUploadsTurnedAndLosslessAtFullSize() async throws {
        let file = try AttachmentStaging.stage(data: try encoded(quadrants(width: 16, height: 8), type: .png), filename: "shot.png")
        defer { AttachmentStaging.remove(file.url) }
        let canEdit = await AttachmentPreparer.canEdit(file)
        XCTAssertTrue(canEdit)
        let prepared = try await AttachmentPreparer.prepare(file, edit: AttachmentEdit(rotation: 90), settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertNotEqual(prepared.fileURL, file.url)
        XCTAssertTrue(["image/png", "image/webp"].contains(prepared.contentType), prepared.contentType)
        XCTAssertEqual((prepared.name as NSString).deletingPathExtension, "shot")
        XCTAssertEqual(prepared.width, 8)
        XCTAssertEqual(prepared.height, 16)
        let image = try decoded(prepared.fileURL)
        try assertColor(image, 1, 1, blue, tolerance: 0, "lossless")
        try assertColor(image, 6, 14, green, tolerance: 0)
    }

    func testEditedPhotosAlwaysReencodeEvenWhenLarger() async throws {
        // A tiny low-quality JPEG: re-encoding at the server quality is larger, and still used.
        let jpeg = try encoded(quadrants(width: 64, height: 32), type: .jpeg, quality: 0.1)
        let file = try AttachmentStaging.stage(data: jpeg, filename: "IMG_1.JPG")
        defer { AttachmentStaging.remove(file.url) }
        let edit = AttachmentEdit(crop: EditRect(x: 0.5, y: 0, width: 0.5, height: 1))
        let prepared = try await AttachmentPreparer.prepare(file, edit: edit, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertNotEqual(prepared.fileURL, file.url, "the original is never the result")
        XCTAssertTrue(["image/webp", "image/jpeg", "image/avif", "image/png"].contains(prepared.contentType), prepared.contentType)
        XCTAssertEqual((prepared.name as NSString).deletingPathExtension, "IMG_1")
        XCTAssertEqual(prepared.width, 32)
        XCTAssertEqual(prepared.height, 32)
        let image = try decoded(prepared.fileURL)
        try assertColor(image, 16, 4, green, tolerance: 40, "the right half")
    }

    func testUneditedFilesPrepareAsBefore() async throws {
        let file = try AttachmentStaging.stage(data: try encoded(quadrants(width: 16, height: 8), type: .png), filename: "shot.png")
        defer { AttachmentStaging.remove(file.url) }
        let plain = await AttachmentPreparer.prepare(file, settings: AttachmentCompression())
        let unedited = try await AttachmentPreparer.prepare(file, edit: .original, settings: AttachmentCompression())
        defer { [plain, unedited].forEach { if $0.fileURL != file.url { AttachmentStaging.remove($0.fileURL) } } }
        XCTAssertEqual(unedited.contentType, plain.contentType)
        XCTAssertEqual(unedited.byteSize, plain.byteSize)
        XCTAssertEqual(unedited.width, plain.width)
    }

    func testEditsThatCantBeAppliedFailInsteadOfUploadingTheOriginal() async throws {
        let text = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        let broken = try AttachmentStaging.stage(data: Data("not a picture".utf8), filename: "broken.png")
        defer { [text, broken].forEach { AttachmentStaging.remove($0.url) } }
        let edit = AttachmentEdit(rotation: 180)
        for file in [text, broken] {
            let canEdit = await AttachmentPreparer.canEdit(file)
            XCTAssertFalse(canEdit, file.name)
            do {
                _ = try await AttachmentPreparer.prepare(file, edit: edit, settings: AttachmentCompression())
                XCTFail("\(file.name) prepared without its edit")
            } catch {
                XCTAssertEqual(error.localizedDescription, AttachmentEdit.failureMessage)
            }
        }
    }

    /// A one-second H.264 clip, red on the left half and blue on the right.
    private func writeClip(width: Int, height: Int) async throws -> URL {
        let url = try AttachmentStaging.newDirectory().appendingPathComponent("clip.mov")
        let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
        ])
        input.expectsMediaDataInRealTime = false
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
        ])
        XCTAssertTrue(writer.canAdd(input))
        writer.add(input)
        try XCTSkipUnless(writer.startWriting(), "this Mac can't encode H.264")
        writer.startSession(atSourceTime: .zero)
        for index in 0..<30 {
            while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(5)) }
            var created: CVPixelBuffer?
            CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA,
                                [kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any]()] as CFDictionary, &created)
            let buffer = try XCTUnwrap(created)
            CVPixelBufferLockBaseAddress(buffer, [])
            let base = try XCTUnwrap(CVPixelBufferGetBaseAddress(buffer)).assumingMemoryBound(to: UInt8.self)
            let stride = CVPixelBufferGetBytesPerRow(buffer)
            for y in 0..<height {
                for x in 0..<width {
                    let pixel = base + y * stride + x * 4 // BGRA
                    pixel[0] = x < width / 2 ? 0 : 255
                    pixel[1] = 0
                    pixel[2] = x < width / 2 ? 255 : 0
                    pixel[3] = 255
                }
            }
            CVPixelBufferUnlockBaseAddress(buffer, [])
            XCTAssertTrue(adaptor.append(buffer, withPresentationTime: CMTime(value: CMTimeValue(index), timescale: 30)))
        }
        input.markAsFinished()
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            writer.finishWriting { continuation.resume() }
        }
        XCTAssertEqual(writer.status, .completed)
        return url
    }

    func testEditedVideosBakeTheTurnCropAndMarksIntoEveryFrame() async throws {
        let url = try await writeClip(width: 128, height: 64)
        defer { AttachmentStaging.remove(url) }
        let file = LocalAttachmentFile(url: url, name: "clip.mov", contentType: "video/quicktime",
                                       size: try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
        let canEdit = await AttachmentPreparer.canEdit(file)
        XCTAssertTrue(canEdit)
        // Turned clockwise the red half is on top and the blue below; keep the
        // bottom (blue) half, with a white dot near its top.
        let dot = StrokeMark(color: "#FFFFFF", width: 0.1, points: [0.5, 0.6])
        let edit = AttachmentEdit(rotation: 90, crop: EditRect(x: 0, y: 0.5, width: 1, height: 0.5), marks: [.stroke(dot)])
        let prepared = try await AttachmentPreparer.prepare(file, edit: edit, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertEqual(prepared.contentType, "video/mp4")
        XCTAssertEqual(prepared.name, "clip.mp4")
        XCTAssertEqual(prepared.width, 64)
        XCTAssertEqual(prepared.height, 64)
        XCTAssertNotNil(prepared.preview, "the poster comes from the edited video")
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: prepared.fileURL))
        generator.appliesPreferredTrackTransform = true
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero
        let frame = try await generator.image(at: CMTime(value: 15, timescale: 30)).image
        XCTAssertEqual(frame.width, 64)
        XCTAssertEqual(frame.height, 64)
        try assertColor(frame, 8, 40, blue, tolerance: 48, "the crop is the turned picture's bottom half")
        try assertColor(frame, 32, 52, blue, tolerance: 48)
        try assertColor(frame, 32, 12, white, tolerance: 48, "the dot, 12.8 pixels from the crop's top")
    }

    func testOnlyStillsThatAreNotAnimatedAreEdited() async throws {
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/avif"), .photo, "AVIF edits take the photo path")
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/heic"), .photo)
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/png"), .lossless)
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/webp", webP: .lossy), .photo)
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/webp", webP: .animated), .unchanged)
        XCTAssertEqual(AttachmentPolicy.editedStillSource(contentType: "image/gif"), .unchanged)
        let gif = try AttachmentStaging.stage(data: try encoded(quadrants(width: 8, height: 8), type: .gif), filename: "still.gif")
        defer { AttachmentStaging.remove(gif.url) }
        let canEdit = await AttachmentPreparer.canEdit(gif)
        XCTAssertFalse(canEdit, "GIFs keep their frames")
    }
}
