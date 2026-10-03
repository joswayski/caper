import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// A picked file copied into the app's temporary staging area.
public struct LocalAttachmentFile: Equatable, Sendable {
    public let url: URL
    public let name: String
    public let contentType: String
    public let size: Int
}

public struct AttachmentPreviewImage: Equatable, Sendable {
    public let data: Data
    public let contentType: String
}

/// What will be uploaded after compression, measurement and preview drawing.
public struct PreparedAttachment: Equatable, Sendable {
    public var fileURL: URL
    public var name: String
    public var contentType: String
    public var kind: AttachmentKind
    public var byteSize: Int
    public var sourceSize: Int
    public var width: Int?
    public var height: Int?
    public var durationMs: Int?
    public var preview: AttachmentPreviewImage?

    public init(original file: LocalAttachmentFile) {
        fileURL = file.url; name = file.name; contentType = file.contentType
        kind = AttachmentKind(contentType: file.contentType); byteSize = file.size; sourceSize = file.size
    }
}

public enum AttachmentStaging {
    static var root: URL { FileManager.default.temporaryDirectory.appendingPathComponent("caper-attachments", isDirectory: true) }

    static func newDirectory() throws -> URL {
        let directory = root.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    public static func contentType(forFilename name: String) -> String {
        let ext = (name as NSString).pathExtension
        guard !ext.isEmpty, let type = UTType(filenameExtension: ext) else { return "application/octet-stream" }
        return type.preferredMIMEType ?? "application/octet-stream"
    }

    /// Copies a picked, dropped or imported file so it outlives the picker's
    /// access window. Security-scoped URLs (file importer) are opened here.
    public static func stage(copying source: URL, filename: String? = nil) throws -> LocalAttachmentFile {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let name = sanitized(filename ?? source.lastPathComponent)
        let destination = try newDirectory().appendingPathComponent(name)
        try FileManager.default.copyItem(at: source, to: destination)
        return try file(at: destination, name: name)
    }

    public static func stage(data: Data, filename: String) throws -> LocalAttachmentFile {
        let name = sanitized(filename)
        let destination = try newDirectory().appendingPathComponent(name)
        try data.write(to: destination)
        return try file(at: destination, name: name)
    }

    static func file(at url: URL, name: String) throws -> LocalAttachmentFile {
        let size = (try url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        return LocalAttachmentFile(url: url, name: name, contentType: contentType(forFilename: name), size: size)
    }

    /// Removes a staged file's private directory; ignores files elsewhere.
    public static func remove(_ url: URL) {
        let directory = url.deletingLastPathComponent()
        guard directory.deletingLastPathComponent().standardizedFileURL.path == root.standardizedFileURL.path else { return }
        try? FileManager.default.removeItem(at: directory)
    }

    static func sanitized(_ name: String) -> String {
        let cleaned = name.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return cleaned.isEmpty || cleaned == "." || cleaned == ".." ? "file" : String(cleaned.prefix(200))
    }
}

/// Applies the server's compression settings before upload. The API verifies
/// stored bytes independently; this only saves storage and bandwidth.
public enum AttachmentPreparer {
    public static func prepare(_ file: LocalAttachmentFile, settings: AttachmentCompression) async -> PreparedAttachment {
        let type = file.contentType.lowercased()
        if type.hasPrefix("image/") { return prepareImage(file, settings: settings) }
        let kind = AttachmentKind(contentType: type)
        if kind == .video || type.hasPrefix("video/") { return await prepareVideo(file, settings: settings) }
        if kind == .audio { return await prepareAudio(file) }
        return PreparedAttachment(original: file)
    }

    // MARK: Stills

    static func prepareImage(_ file: LocalAttachmentFile, settings: AttachmentCompression) -> PreparedAttachment {
        var prepared = PreparedAttachment(original: file)
        guard let source = CGImageSourceCreateWithURL(file.url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let pixelWidth = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue,
              let pixelHeight = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue,
              pixelWidth > 0, pixelHeight > 0 else { return prepared }
        let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
        let rotated = (5...8).contains(orientation)
        prepared.width = rotated ? pixelHeight : pixelWidth
        prepared.height = rotated ? pixelWidth : pixelHeight

        if AttachmentPolicy.compressible(file.contentType), pixelWidth * pixelHeight <= AttachmentPolicy.maxCompressPixels {
            let longest = max(pixelWidth, pixelHeight)
            let target = settings.imageMaxEdge > 0 ? min(longest, settings.imageMaxEdge) : longest
            if let image = thumbnail(source, maxPixelSize: target), let encoded = encodeStill(image, originalType: file.contentType, settings: settings),
               AttachmentPolicy.keepReencoded(originalType: file.contentType, originalSize: file.size, encodedSize: encoded.data.count),
               let directory = try? AttachmentStaging.newDirectory() {
                // Re-encoding also drops EXIF metadata such as photo GPS coordinates.
                let name = AttachmentPolicy.renamed(file.name, contentType: encoded.contentType)
                let destination = directory.appendingPathComponent(AttachmentStaging.sanitized(name))
                if (try? encoded.data.write(to: destination)) != nil {
                    prepared.fileURL = destination
                    prepared.name = name
                    prepared.contentType = encoded.contentType
                    prepared.kind = .image
                    prepared.byteSize = encoded.data.count
                    prepared.width = image.width
                    prepared.height = image.height
                }
            }
        }
        if AttachmentPolicy.needsPreview(kind: prepared.kind, width: prepared.width, height: prepared.height, byteSize: prepared.byteSize, settings: settings),
           let image = thumbnail(source, maxPixelSize: settings.previewEdge) {
            prepared.preview = previewImage(image)
        }
        return prepared
    }

    /// Indexed PNG when the palette fits, otherwise lossy WebP/JPEG. HEIC is
    /// never emitted; originals that cannot render inline always convert.
    static func encodeStill(_ image: CGImage, originalType: String, settings: AttachmentCompression) -> AttachmentPreviewImage? {
        var palette: IndexedPNG.Palette?
        if settings.paletteColors > 0, let pixels = rgbaPixels(image) {
            palette = IndexedPNG.palette(width: image.width, height: image.height, rgba: pixels, maxColors: settings.paletteColors)
        }
        switch AttachmentPolicy.stillEncoding(colorCount: palette?.colors.count, settings: settings) {
        case .indexedPNG:
            if let palette, let data = IndexedPNG.encode(palette) { return AttachmentPreviewImage(data: data, contentType: "image/png") }
            return settings.imageQuality < 100 ? encodeLossy(image, quality: Double(settings.imageQuality) / 100) : nil
        case .lossy(let quality):
            return encodeLossy(image, quality: quality)
        case .none:
            return AttachmentKind(contentType: originalType) == .image ? nil : encodeLossy(image, quality: 1)
        }
    }

    static func previewImage(_ image: CGImage) -> AttachmentPreviewImage? {
        guard let encoded = encodeLossy(image, quality: AttachmentPolicy.previewQuality),
              (1...AttachmentPolicy.previewMaxBytes).contains(encoded.data.count) else { return nil }
        return encoded
    }

    static func thumbnail(_ source: CGImageSource, maxPixelSize: Int) -> CGImage? {
        guard maxPixelSize > 0 else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// Straight-alpha RGBA8 pixels in sRGB, rows from the top.
    static func rgbaPixels(_ image: CGImage) -> [UInt8]? {
        let width = image.width, height = image.height
        guard width > 0, height > 0, let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: space,
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
            context.interpolationQuality = .none
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drawn else { return nil }
        for offset in stride(from: 0, to: pixels.count, by: 4) {
            let alpha = Int(pixels[offset + 3])
            guard alpha > 0, alpha < 255 else { continue }
            for channel in 0..<3 { pixels[offset + channel] = UInt8(min(255, (Int(pixels[offset + channel]) * 255 + alpha / 2) / alpha)) }
        }
        return pixels
    }

    static let webPEncodable: Bool = {
        let identifiers = (CGImageDestinationCopyTypeIdentifiers() as? [String]) ?? []
        return identifiers.contains(UTType.webP.identifier)
    }()

    /// WebP when ImageIO can encode it, else JPEG flattened onto white.
    static func encodeLossy(_ image: CGImage, quality: Double) -> AttachmentPreviewImage? {
        if webPEncodable, let data = encode(image, type: UTType.webP.identifier, quality: quality) {
            return AttachmentPreviewImage(data: data, contentType: "image/webp")
        }
        guard let flat = flattened(image), let data = encode(flat, type: UTType.jpeg.identifier, quality: quality) else { return nil }
        return AttachmentPreviewImage(data: data, contentType: "image/jpeg")
    }

    static func encode(_ image: CGImage, type: String, quality: Double) -> Data? {
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(output as CFMutableData, type as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return output as Data
    }

    static func flattened(_ image: CGImage) -> CGImage? {
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        let rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
        context.fill(rect)
        context.draw(image, in: rect)
        return context.makeImage()
    }

    // MARK: Timed media

    static func prepareVideo(_ file: LocalAttachmentFile, settings: AttachmentCompression) async -> PreparedAttachment {
        var prepared = PreparedAttachment(original: file)
        let asset = AVURLAsset(url: file.url)
        guard let measured = await measureVideo(asset) else { return prepared }
        prepared.durationMs = measured.durationMs
        prepared.width = measured.width
        prepared.height = measured.height
        var finalAsset = asset

        if let preset = AttachmentPolicy.videoPreset(width: measured.width, height: measured.height, maxHeight: settings.videoMaxHeight),
           let directory = try? AttachmentStaging.newDirectory(),
           let session = AVAssetExportSession(asset: asset, presetName: preset) {
            let name = AttachmentPolicy.renamed(file.name, contentType: "video/mp4")
            let output = directory.appendingPathComponent(AttachmentStaging.sanitized(name))
            session.outputURL = output
            session.outputFileType = .mp4
            session.shouldOptimizeForNetworkUse = true
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                session.exportAsynchronously { continuation.resume() }
            }
            let size = (try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            // Keep only a smaller result, unless the original cannot play inline.
            if session.status == .completed, size > 0, size < file.size || AttachmentKind(contentType: file.contentType) != .video {
                let transcoded = AVURLAsset(url: output)
                let transcodedSize = await measureVideo(transcoded)
                prepared.fileURL = output
                prepared.name = name
                prepared.contentType = "video/mp4"
                prepared.kind = .video
                prepared.byteSize = size
                prepared.width = transcodedSize?.width ?? prepared.width
                prepared.height = transcodedSize?.height ?? prepared.height
                prepared.durationMs = transcodedSize?.durationMs ?? prepared.durationMs
                finalAsset = transcoded
            } else {
                // Transcoding is optional: fall back to the original file.
                try? FileManager.default.removeItem(at: directory)
            }
        }

        if prepared.kind == .video, settings.previewEdge > 0 {
            let generator = AVAssetImageGenerator(asset: finalAsset)
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: settings.previewEdge, height: settings.previewEdge)
            // A poster from just after the start, past common black first frames.
            let seconds = Double(prepared.durationMs ?? 1000) / 1000
            let time = CMTime(seconds: min(0.5, seconds / 2), preferredTimescale: 600)
            if let poster = try? await generator.image(at: time).image { prepared.preview = previewImage(poster) }
        }
        return prepared
    }

    static func measureVideo(_ asset: AVURLAsset) async -> (width: Int, height: Int, durationMs: Int?)? {
        guard let track = try? await asset.loadTracks(withMediaType: .video).first,
              let geometry = try? await track.load(.naturalSize, .preferredTransform) else { return nil }
        let display = geometry.0.applying(geometry.1)
        let width = Int(abs(display.width).rounded()), height = Int(abs(display.height).rounded())
        guard width > 0, height > 0 else { return nil }
        let duration = await durationMs(asset)
        return (width, height, duration)
    }

    static func durationMs(_ asset: AVURLAsset) async -> Int? {
        guard let duration = try? await asset.load(.duration) else { return nil }
        let seconds = CMTimeGetSeconds(duration)
        guard seconds.isFinite, seconds >= 0, seconds * 1000 < Double(Int32.max) else { return nil }
        return Int((seconds * 1000).rounded())
    }

    static func prepareAudio(_ file: LocalAttachmentFile) async -> PreparedAttachment {
        var prepared = PreparedAttachment(original: file)
        prepared.durationMs = await durationMs(AVURLAsset(url: file.url))
        return prepared
    }
}

/// Reserve, upload straight to storage, then confirm (web's `uploadPrepared`).
public enum AttachmentUploader {
    public static func upload(_ file: PreparedAttachment, channelID: String, api: APIClient,
                              progress: @escaping @Sendable (Double) -> Void) async throws -> ChatAttachment {
        func dimension(_ value: Int?) -> Int? { value.flatMap { (1...32_768).contains($0) ? $0 : nil } }
        let input = AssetCreateInput(
            channelId: channelID, filename: file.name, contentType: file.contentType, byteSize: file.byteSize,
            sourceByteSize: file.sourceSize > 0 ? file.sourceSize : nil,
            width: dimension(file.width), height: dimension(file.height),
            durationMs: file.durationMs.flatMap { (0...86_400_000).contains($0) ? $0 : nil },
            preview: file.preview.map { AssetCreateInput.Preview(contentType: $0.contentType, byteSize: $0.data.count) }
        )
        let reservation = try await api.createAsset(input)
        let previewSize = Double(file.preview?.data.count ?? 0)
        let total = max(1, previewSize + Double(file.byteSize))
        if let preview = file.preview, let previewUpload = reservation.previewUpload {
            try await api.putToStorage(previewUpload, body: .data(preview.data))
        }
        let originalSize = Double(file.byteSize)
        try await api.putToStorage(reservation.upload, body: .file(file.fileURL)) { fraction in
            progress((previewSize + fraction * originalSize) / total)
        }
        try Task.checkCancellation()
        let attachment = try await api.completeAsset(id: reservation.id)
        progress(1)
        return attachment
    }
}
