import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ImageIO
import UniformTypeIdentifiers
// Implementation-only so importers of CaperCore (apps, tests) need no libwebp
// or libavif headers.
@_implementationOnly import libavif
@_implementationOnly import libwebp

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

/// Applies the server's compression settings before upload
/// (`docs/media.md`, "Client compression and previews"). Every step is
/// optional: when a rule cannot be met the original is uploaded instead of a
/// worse file. The API verifies stored bytes independently.
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
        guard let source = CGImageSourceCreateWithURL(file.url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { return prepared }
        let index = CGImageSourceGetPrimaryImageIndex(source)
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
              let pixelWidth = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue,
              let pixelHeight = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue,
              pixelWidth > 0, pixelHeight > 0 else { return prepared }
        let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
        let rotated = (5...8).contains(orientation)
        prepared.width = rotated ? pixelHeight : pixelWidth
        prepared.height = rotated ? pixelWidth : pixelHeight

        let baseType = AttachmentPolicy.baseType(file.contentType)
        var webP: AttachmentPolicy.WebPFormat?
        if baseType == "image/webp", let handle = try? FileHandle(forReadingFrom: file.url) {
            webP = AttachmentPolicy.webPFormat((try? handle.read(upToCount: 65_536)) ?? Data())
            try? handle.close()
        }
        var stillSource = AttachmentPolicy.stillSource(contentType: file.contentType, webP: webP)
        // Animated PNG/WebP keep their frames; HEIC may hold several images but
        // only its primary image is a photo.
        if stillSource == .lossless, CGImageSourceGetCount(source) > 1 { stillSource = .unchanged }

        if stillSource != .unchanged, pixelWidth * pixelHeight <= AttachmentPolicy.maxCompressPixels,
           let encoded = encodeStill(source, index: index, contentType: file.contentType, stillSource: stillSource,
                                     orientation: orientation, longestEdge: max(pixelWidth, pixelHeight), settings: settings),
           AttachmentPolicy.keepReencoded(originalType: file.contentType, originalSize: file.size, encodedSize: encoded.data.count,
                                          lossless: encoded.lossless),
           let destination = writeStaged(encoded.data, name: AttachmentPolicy.renamed(file.name, contentType: encoded.contentType)) {
            // Re-encoding applies the orientation and drops EXIF/GPS metadata.
            prepared.fileURL = destination
            prepared.name = destination.lastPathComponent
            prepared.contentType = encoded.contentType
            prepared.kind = .image
            prepared.byteSize = encoded.data.count
            prepared.width = encoded.width
            prepared.height = encoded.height
        } else {
            stripMetadata(&prepared, baseType: baseType, orientation: orientation)
        }
        if AttachmentPolicy.needsPreview(kind: prepared.kind, width: prepared.width, height: prepared.height, byteSize: prepared.byteSize, settings: settings),
           let image = thumbnail(source, index: index, maxPixelSize: settings.previewEdge) {
            prepared.preview = previewImage(image)
        }
        return prepared
    }

    /// A new staging directory holding `data` as `name`.
    static func writeStaged(_ data: Data, name: String) -> URL? {
        guard let directory = try? AttachmentStaging.newDirectory() else { return nil }
        let destination = directory.appendingPathComponent(AttachmentStaging.sanitized(name))
        guard (try? data.write(to: destination)) != nil else {
            try? FileManager.default.removeItem(at: directory)
            return nil
        }
        return destination
    }

    /// Originals that upload unchanged lose their location and other
    /// metadata losslessly (JPEG APPn segments, PNG text/eXIf chunks).
    static func stripMetadata(_ prepared: inout PreparedAttachment, baseType: String, orientation: Int) {
        guard prepared.fileURL.isFileURL, let data = try? Data(contentsOf: prepared.fileURL) else { return }
        let stripped: Data?
        switch baseType {
        case "image/jpeg", "image/jpg", "image/pjpeg": stripped = AttachmentMetadata.strippedJPEG(data, orientation: orientation)
        case "image/png": stripped = AttachmentMetadata.strippedPNG(data)
        default: stripped = nil
        }
        guard let stripped, let destination = writeStaged(stripped, name: prepared.name) else { return }
        prepared.fileURL = destination
        prepared.byteSize = stripped.count
    }

    struct EncodedStill { let data: Data; let contentType: String; let width: Int; let height: Int; let lossless: Bool }

    /// The best re-encoding of a still, or nil to keep the original. Lossless
    /// sources only ever become an exact indexed PNG or lossless WebP (the
    /// smaller one that decodes to identical pixels); photos may also be
    /// encoded lossily.
    static func encodeStill(_ source: CGImageSource, index: Int, contentType: String, stillSource: AttachmentPolicy.StillSource,
                            orientation: Int, longestEdge: Int, settings: AttachmentCompression) -> EncodedStill? {
        let image: CGImage?
        if stillSource == .lossless {
            // Exact decode at full size: scaling or rotating would resample
            // pixels, so a rotated, deep-colour or non-RGB lossless file stays
            // as it is. `imageMaxEdge` applies to photos only.
            guard orientation == 1 else { return nil }
            image = CGImageSourceCreateImageAtIndex(source, index, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        } else {
            let target = settings.imageMaxEdge > 0 ? min(longestEdge, settings.imageMaxEdge) : longestEdge
            image = thumbnail(source, index: index, maxPixelSize: target)
        }
        guard let image else { return nil }
        if stillSource == .lossless, image.bitsPerComponent > 8 || nativeRGBSpace(image) == nil { return nil }

        var palette: IndexedPNG.Palette?
        var pixels: RGBAPixels?
        let needsPixels = stillSource == .lossless || settings.paletteColors > 0
        if needsPixels, image.width * image.height <= AttachmentPolicy.maxPalettePixels, let drawn = rgbaPixels(image) {
            pixels = drawn
            if settings.paletteColors > 0 {
                palette = IndexedPNG.palette(width: image.width, height: image.height, rgba: drawn.straight, maxColors: settings.paletteColors)
            }
        }
        let candidates = AttachmentPolicy.stillCandidates(contentType: contentType, source: stillSource, colorCount: palette?.colors.count,
                                                          settings: settings, avifEncodable: avifEncodable)
        // Lossless means lossless: every lossless result must decode to the same pixels.
        var best: EncodedStill?
        for candidate in candidates {
            var data: Data?
            var type = ""
            switch candidate {
            case .indexedPNG:
                if let palette, let pixels { data = IndexedPNG.encode(palette, iccProfile: pixels.iccProfile); type = "image/png" }
            case .losslessWebP:
                if let pixels { data = encodeLosslessWebP(pixels); type = "image/webp" }
            case .avif, .lossy:
                continue
            }
            guard let data, let pixels, data.count < (best?.data.count ?? Int.max), reproduces(data, pixels) else { continue }
            best = EncodedStill(data: data, contentType: type, width: image.width, height: image.height, lossless: true)
        }
        if let best { return best }
        for candidate in candidates {
            switch candidate {
            case .avif(let quality):
                // Any AVIF failure falls through to the WebP/JPEG candidate.
                if let data = encodeAVIF(pixels ?? rgbaPixels(image), quality: quality) {
                    return EncodedStill(data: data, contentType: "image/avif", width: image.width, height: image.height, lossless: false)
                }
            case .lossy(let quality):
                guard let encoded = encodeLossy(image, quality: quality) else { return nil }
                return EncodedStill(data: encoded.data, contentType: encoded.contentType, width: image.width, height: image.height, lossless: false)
            case .indexedPNG, .losslessWebP:
                continue
            }
        }
        return nil
    }

    static func previewImage(_ image: CGImage) -> AttachmentPreviewImage? {
        guard let encoded = encodeLossy(image, quality: AttachmentPolicy.previewQuality),
              (1...AttachmentPolicy.previewMaxBytes).contains(encoded.data.count) else { return nil }
        return encoded
    }

    /// Decoded, orientation-applied image whose longest edge is at most
    /// `maxPixelSize` (never enlarged).
    static func thumbnail(_ source: CGImageSource, index: Int = 0, maxPixelSize: Int) -> CGImage? {
        guard maxPixelSize > 0 else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary)
    }

    /// RGBA8 pixels drawn in the image's own RGB colour space (so no colour
    /// conversion changes values), rows from the top.
    struct RGBAPixels {
        let width: Int
        let height: Int
        /// Straight (non-premultiplied) alpha, for the palette.
        let straight: [UInt8]
        let space: CGColorSpace
        /// Embedded in an indexed PNG; nil means sRGB.
        let iccProfile: Data?
    }

    /// The image's own RGB space (or the base of its indexed space), so
    /// drawing into it converts nothing; nil for grey, CMYK and others.
    static func nativeRGBSpace(_ image: CGImage) -> CGColorSpace? {
        guard let space = image.colorSpace else { return nil }
        let rgb = space.model == .indexed ? space.baseColorSpace : space
        guard let rgb, rgb.model == .rgb else { return nil }
        return rgb
    }

    /// Where to draw for encoding: the native RGB space, else sRGB.
    static func drawingSpace(_ image: CGImage) -> CGColorSpace? {
        nativeRGBSpace(image) ?? CGColorSpace(name: CGColorSpace.sRGB)
    }

    static func rgbaPixels(_ image: CGImage) -> RGBAPixels? {
        guard let space = drawingSpace(image), var pixels = draw(image, in: space) else { return nil }
        for offset in stride(from: 0, to: pixels.count, by: 4) {
            let alpha = Int(pixels[offset + 3])
            guard alpha > 0, alpha < 255 else { continue }
            for channel in 0..<3 { pixels[offset + channel] = UInt8(min(255, (Int(pixels[offset + channel]) * 255 + alpha / 2) / alpha)) }
        }
        let isSRGB = space.name.map { ($0 as String) == (CGColorSpace.sRGB as String) } ?? false
        let profile = isSRGB ? nil : space.copyICCData().map { $0 as Data }
        return RGBAPixels(width: image.width, height: image.height, straight: pixels, space: space, iccProfile: profile)
    }

    static func draw(_ image: CGImage, in space: CGColorSpace) -> [UInt8]? {
        let width = image.width, height = image.height
        guard width > 0, height > 0 else { return nil }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: space,
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
            context.interpolationQuality = .none
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        return drawn ? pixels : nil
    }

    /// The encoded PNG decodes (through ImageIO, in the same colour space) to
    /// the same pixels: alpha everywhere and colour of every opaque pixel
    /// exactly. Translucent colour is limited by CoreGraphics' premultiplied
    /// drawing on both sides, so it is not compared.
    static func reproduces(_ png: Data, _ expected: RGBAPixels) -> Bool {
        guard let source = CGImageSourceCreateWithData(png as CFData, nil),
              let decoded = CGImageSourceCreateImageAtIndex(source, 0, nil),
              decoded.width == expected.width, decoded.height == expected.height,
              let actual = draw(decoded, in: expected.space), actual.count == expected.straight.count else { return false }
        return actual.withUnsafeBufferPointer { actual in
            expected.straight.withUnsafeBufferPointer { expected in
                var offset = 0
                while offset < actual.count {
                    let alpha = expected[offset + 3]
                    if actual[offset + 3] != alpha { return false }
                    if alpha == 255, actual[offset] != expected[offset] || actual[offset + 1] != expected[offset + 1] || actual[offset + 2] != expected[offset + 2] {
                        return false
                    }
                    offset += 4
                }
                return true
            }
        }
    }

    /// Lossless WebP through libwebp's simple API (`WebPEncodeLosslessRGBA`:
    /// lossless, method 4). ImageIO cannot encode WebP. A non-sRGB colour
    /// space is kept by adding its ICC profile in the extended format.
    static func encodeLosslessWebP(_ pixels: RGBAPixels) -> Data? {
        guard (1...AttachmentPolicy.webPMaxDimension).contains(pixels.width), (1...AttachmentPolicy.webPMaxDimension).contains(pixels.height),
              pixels.straight.count == pixels.width * pixels.height * 4 else { return nil }
        var output: UnsafeMutablePointer<UInt8>?
        let size = pixels.straight.withUnsafeBufferPointer { buffer -> Int in
            guard let base = buffer.baseAddress else { return 0 }
            return Int(WebPEncodeLosslessRGBA(base, Int32(pixels.width), Int32(pixels.height), Int32(pixels.width * 4), &output))
        }
        defer { if let output { WebPFree(output) } }
        guard size > 0, let encoded = output else { return nil }
        let webP = Data(bytes: encoded, count: size)
        guard let profile = pixels.iccProfile else { return webP }
        var hasAlpha = false
        for offset in stride(from: 3, to: pixels.straight.count, by: 4) where pixels.straight[offset] != 255 { hasAlpha = true; break }
        return AttachmentPolicy.webPAddingICCProfile(webP, profile: profile, width: pixels.width, height: pixels.height, hasAlpha: hasAlpha)
    }

    /// libavif's encoder speed (0 slowest ... 10 fastest). At 6 libavif runs
    /// aom in its all-intra still-image mode with constant-quality rate
    /// control, so `quality` sets the quantizer exactly as `avifenc -q` does.
    /// About 1.8 s for a 6-14 MP photo on 4 x86-64 cores with SIMD; speed 8
    /// halves that for 0.7% more bytes and 0.17 lower SSIMULACRA2.
    static let avifSpeed: Int32 = 6

    /// aom tuning for the colour planes. libavif 1.3+ with aom 3.13+ defaults
    /// stills to `tune=iq`, whose different quality-to-quantizer table makes
    /// quality 85 about 2 SSIMULACRA2 points better and 2-3% larger (and
    /// slower) than the `avifenc -q 85` the server's `avifQuality` was chosen
    /// with; `ssim` keeps that scale (and matches Android). Measured with this
    /// exact libavif 1.4.2/aom 3.15.1 build (x86-64 SIMD, 4 threads, quality
    /// 85, speed 6) on the five 6-14 MP benchmark photos: mean SSIMULACRA2
    /// 83.35 and 6.33 MB against 83.27 and 6.32 MB for `avifenc -q 85 -s 6
    /// -y 420` (libavif 1.0.4, aom 3.8.2), in about 0.9x its time.
    static let avifColorTune = "ssim"

    /// The bundled encoder's versions, e.g. "libavif 1.4.2 aom [enc]:v3.15.1".
    static let avifLibraryVersions: String = {
        var codecs = [CChar](repeating: 0, count: 256)
        avifCodecVersions(&codecs)
        let codecVersions = codecs.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
        return "libavif \(String(cString: avifVersion())) \(codecVersions)"
    }()

    /// libavif was built with an AV1 encoder (aom, by joswayski/libavif-apple).
    /// ImageIO decodes AVIF (iOS 16 / macOS 13 and later) but offers no AVIF
    /// destination, and any future one would take a 0...1 quality unrelated
    /// to libavif's scale, so photos are always encoded with libavif.
    static let avifEncodable: Bool = avifCodecName(AVIF_CODEC_CHOICE_AUTO, avifCodecFlags(AVIF_CODEC_FLAG_CAN_ENCODE.rawValue)) != nil

    /// A photo as AVIF through libavif's C API: 8-bit YUV 4:2:0, full range,
    /// BT.601 matrix, `quality` passed straight to libavif (the same scale as
    /// `avifenc -q`), no Exif/XMP. Colour matches the lossless WebP path: the
    /// pixels are in the image's own RGB space with its ICC profile, or sRGB
    /// signalled as CICP 1/13/6 (what avifenc writes for an untagged PNG).
    /// HDR (PQ/HLG) photos return nil and take the WebP/JPEG path, like any
    /// encoder error.
    static func encodeAVIF(_ pixels: RGBAPixels?, quality: Int) -> Data? {
        guard avifEncodable, let pixels, (1...100).contains(quality), pixels.width > 0, pixels.height > 0,
              pixels.straight.count == pixels.width * pixels.height * 4, !CGColorSpaceUsesITUR_2100TF(pixels.space),
              let image = avifImageCreate(UInt32(pixels.width), UInt32(pixels.height), 8, AVIF_PIXEL_FORMAT_YUV420) else { return nil }
        defer { avifImageDestroy(image) }
        image.pointee.yuvRange = AVIF_RANGE_FULL
        image.pointee.matrixCoefficients = avifMatrixCoefficients(AVIF_MATRIX_COEFFICIENTS_BT601)
        if let profile = pixels.iccProfile {
            let set = profile.withUnsafeBytes { bytes -> avifResult in
                avifImageSetProfileICC(image, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
            }
            guard set == AVIF_RESULT_OK else { return nil }
        } else {
            image.pointee.colorPrimaries = avifColorPrimaries(AVIF_COLOR_PRIMARIES_BT709)
            image.pointee.transferCharacteristics = avifTransferCharacteristics(AVIF_TRANSFER_CHARACTERISTICS_SRGB)
        }
        var opaque = true
        for offset in stride(from: 3, to: pixels.straight.count, by: 4) where pixels.straight[offset] != 255 { opaque = false; break }
        var rgb = avifRGBImage()
        avifRGBImageSetDefaults(&rgb, image)
        rgb.depth = 8
        rgb.format = AVIF_RGB_FORMAT_RGBA
        rgb.alphaPremultiplied = avifBool(0)
        // An opaque photo gets no alpha plane at all.
        rgb.ignoreAlpha = avifBool(opaque ? 1 : 0)
        rgb.rowBytes = UInt32(pixels.width * 4)
        let converted = pixels.straight.withUnsafeBufferPointer { buffer -> avifResult in
            rgb.pixels = UnsafeMutablePointer(mutating: buffer.baseAddress)
            defer { rgb.pixels = nil }
            return avifImageRGBToYUV(image, &rgb)
        }
        guard converted == AVIF_RESULT_OK, let encoder = avifEncoderCreate() else { return nil }
        defer { avifEncoderDestroy(encoder) }
        encoder.pointee.quality = Int32(quality)
        encoder.pointee.speed = avifSpeed
        encoder.pointee.maxThreads = Int32(max(1, ProcessInfo.processInfo.activeProcessorCount))
        // "c:" sets the colour planes only; a photo's alpha plane keeps libavif's default.
        guard avifEncoderSetCodecSpecificOption(encoder, "c:tune", avifColorTune) == AVIF_RESULT_OK else { return nil }
        var output = avifRWData()
        defer { avifRWDataFree(&output) }
        guard avifEncoderWrite(encoder, image, &output) == AVIF_RESULT_OK, let bytes = output.data, output.size > 0 else { return nil }
        return Data(bytes: bytes, count: Int(output.size))
    }

    static let webPEncodable: Bool = {
        let identifiers = (CGImageDestinationCopyTypeIdentifiers() as? [String]) ?? []
        return identifiers.contains(UTType.webP.identifier)
    }()

    /// WebP when ImageIO can encode it, else JPEG flattened onto white. Only
    /// the pixels are written: no EXIF, GPS or orientation metadata.
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

    /// Opaque copy on white in the image's own RGB space (keeping wide-gamut
    /// photos wide; the encoder embeds the profile).
    static func flattened(_ image: CGImage) -> CGImage? {
        guard let space = drawingSpace(image),
              let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        let rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
        context.fill(rect)
        context.draw(image, in: rect)
        return context.makeImage()
    }

    // MARK: Video

    /// Facts about one video file, read through AVFoundation.
    struct VideoProbe {
        var facts: AttachmentPolicy.VideoFacts
        var durationMs: Int?
        var hasAudio: Bool
    }

    static func probeVideo(_ asset: AVURLAsset, contentType: String, fileSize: Int) async -> VideoProbe? {
        guard let track = try? await asset.loadTracks(withMediaType: .video).first,
              let geometry = try? await track.load(.naturalSize, .preferredTransform) else { return nil }
        let display = geometry.0.applying(geometry.1)
        let width = Int(abs(display.width).rounded()), height = Int(abs(display.height).rounded())
        guard width > 0, height > 0 else { return nil }
        let formats = (try? await track.load(.formatDescriptions)) ?? []
        let characteristics = (try? await track.load(.mediaCharacteristics)) ?? []
        let isH264 = !formats.isEmpty && formats.allSatisfy { CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_H264 }
        let isHDR = characteristics.contains(.containsHDRVideo) || formats.contains(where: isHDRFormat)
        let duration = await durationMs(asset)
        var bitrate: Double?
        if let rate = try? await track.load(.estimatedDataRate), rate > 0 {
            bitrate = Double(rate) / 1000
        } else if let duration, duration > 0 {
            bitrate = Double(fileSize) * 8 / Double(duration) // bits per millisecond = kbps
        }
        let hasAudio = !((try? await asset.loadTracks(withMediaType: .audio)) ?? []).isEmpty
        let facts = AttachmentPolicy.VideoFacts(width: width, height: height, isH264: isH264, isHDR: isHDR, bitrateKbps: bitrate,
                                                playableContainer: AttachmentKind(contentType: contentType) == .video)
        return VideoProbe(facts: facts, durationMs: duration, hasAudio: hasAudio)
    }

    /// HLG (iPhone HDR and Dolby Vision 8.4 base layers) or PQ (HDR10).
    static func isHDRFormat(_ format: CMFormatDescription) -> Bool {
        guard let transfer = CMFormatDescriptionGetExtension(format, extensionKey: kCMFormatDescriptionExtension_TransferFunction) as? String else { return false }
        return transfer == (kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG as String)
            || transfer == (kCMFormatDescriptionTransferFunction_SMPTE_ST_2084_PQ as String)
    }

    static func prepareVideo(_ file: LocalAttachmentFile, settings: AttachmentCompression) async -> PreparedAttachment {
        var prepared = PreparedAttachment(original: file)
        let asset = AVURLAsset(url: file.url)
        guard let probe = await probeVideo(asset, contentType: file.contentType, fileSize: file.size) else { return prepared }
        prepared.durationMs = probe.durationMs
        prepared.width = probe.facts.width
        prepared.height = probe.facts.height
        var finalAsset = asset

        let plan = AttachmentPolicy.videoPlan(probe.facts, settings: settings)
        if plan != .keep, let output = await export(asset, plan: plan, name: file.name) {
            let transcoded = AVURLAsset(url: output.url)
            let result = await probeVideo(transcoded, contentType: "video/mp4", fileSize: output.size)
            let required: Bool
            switch plan {
            case .transcode(_, let isRequired): required = isRequired
            case .remux, .keep: required = true
            }
            if let result, AttachmentPolicy.keepTranscoded(required: required, originalSize: file.size, outputSize: output.size,
                                                            outputIsH264: result.facts.isH264, outputIsHDR: result.facts.isHDR,
                                                            sourceHasAudio: probe.hasAudio, outputHasAudio: result.hasAudio) {
                prepared.fileURL = output.url
                prepared.name = output.url.lastPathComponent
                prepared.contentType = "video/mp4"
                prepared.kind = .video
                prepared.byteSize = output.size
                prepared.width = result.facts.width
                prepared.height = result.facts.height
                prepared.durationMs = result.durationMs ?? prepared.durationMs
                finalAsset = transcoded
            } else {
                // Optional step: anything wrong (still HDR, audio lost, not
                // smaller) keeps the original file.
                AttachmentStaging.remove(output.url)
            }
        }
        // Location and other user metadata never upload: an export is patched
        // in place, an unchanged original is replaced by a patched copy.
        if prepared.fileURL != file.url {
            AttachmentMetadata.patchQuickTimeMetadata(at: prepared.fileURL)
        } else if let stripped = AttachmentMetadata.strippedQuickTimeCopy(of: file.url, name: file.name) {
            prepared.fileURL = stripped
            prepared.name = stripped.lastPathComponent
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

    /// Runs one export into a new staging directory; nil on any failure.
    ///
    /// Transcodes use an H.264/AAC size preset with a video composition whose
    /// colour properties are explicitly BT.709. With SDR colour properties the
    /// built-in compositor converts HDR (HLG/PQ) sources to SDR before
    /// compositing, and Apple recommends the H.264 presets for HDR→SDR
    /// conversion (WWDC20 10009 and 10010). The caller still re-probes the
    /// output and discards anything that is not SDR H.264.
    static func export(_ asset: AVURLAsset, plan: AttachmentPolicy.VideoPlan, name: String) async -> (url: URL, size: Int)? {
        let presetName: String
        var composition: AVMutableVideoComposition?
        switch plan {
        case .keep: return nil
        case .remux: presetName = AVAssetExportPresetPassthrough
        case .transcode(let preset, _):
            presetName = preset
            // Instructions that apply each track's preferred transform at its
            // natural frame rate, then a mutable copy to set the colour space.
            guard let base = try? await AVMutableVideoComposition.videoComposition(withPropertiesOf: asset),
                  let built = base.mutableCopy() as? AVMutableVideoComposition else { return nil }
            built.colorPrimaries = AVVideoColorPrimaries_ITU_R_709_2
            built.colorTransferFunction = AVVideoTransferFunction_ITU_R_709_2
            built.colorYCbCrMatrix = AVVideoYCbCrMatrix_ITU_R_709_2
            composition = built
        }
        guard let session = AVAssetExportSession(asset: asset, presetName: presetName),
              let directory = try? AttachmentStaging.newDirectory() else { return nil }
        let outputName = AttachmentStaging.sanitized(AttachmentPolicy.renamed(name, contentType: "video/mp4"))
        let output = directory.appendingPathComponent(outputName)
        session.outputURL = output
        session.outputFileType = .mp4
        session.shouldOptimizeForNetworkUse = true
        // Drops location and personal metadata (the output is also byte-patched).
        session.metadataItemFilter = AVMetadataItemFilter.forSharing()
        if let composition { session.videoComposition = composition }
        await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                session.exportAsynchronously { continuation.resume() }
            }
        } onCancel: {
            session.cancelExport()
        }
        let size = (try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        guard session.status == .completed, size > 0, !Task.isCancelled else {
            try? FileManager.default.removeItem(at: directory)
            return nil
        }
        return (output, size)
    }

    static func durationMs(_ asset: AVURLAsset) async -> Int? {
        guard let duration = try? await asset.load(.duration) else { return nil }
        let seconds = CMTimeGetSeconds(duration)
        guard seconds.isFinite, seconds >= 0, seconds * 1000 < Double(Int32.max) else { return nil }
        return Int((seconds * 1000).rounded())
    }

    // MARK: Audio

    static func prepareAudio(_ file: LocalAttachmentFile) async -> PreparedAttachment {
        var prepared = PreparedAttachment(original: file)
        prepared.durationMs = await durationMs(AVURLAsset(url: file.url))
        return prepared
    }
}
