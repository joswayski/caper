import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// Custom profile pictures and space icons (docs/media.md, "Saved default
// avatars"): preparation, upload and display decoding. Views live in
// ProfilePictureViews.swift.

/// What a picture upload replaces: the account's own picture or a space's icon.
public enum PicturePurpose: String, Sendable {
    case avatar
    case spaceIcon

    /// `avatar.<ext>` / `space-icon.<ext>`.
    var filenameStem: String { self == .avatar ? "avatar" : "space-icon" }
}

/// `POST /api/assets` for a picture. No channel, preview, duration or source
/// size; `spaceId` only for a space icon (nil is omitted).
public struct PictureCreateInput: Encodable, Equatable, Sendable {
    public let purpose: String
    public let spaceId: String?
    public let filename: String
    public let contentType: String
    public let byteSize: Int
    public let width: Int
    public let height: Int

    public init(picture: PreparedPicture, purpose: PicturePurpose, spaceID: String? = nil) {
        self.purpose = purpose.rawValue
        spaceId = purpose == .spaceIcon ? spaceID : nil
        filename = picture.filename(for: purpose)
        contentType = picture.contentType
        byteSize = picture.data.count
        width = picture.width
        height = picture.height
    }
}

/// The bytes that will be uploaded: an unchanged GIF, or a re-encoded square still.
public struct PreparedPicture: Equatable, Sendable {
    public let data: Data
    public let contentType: String
    public let width: Int
    public let height: Int

    public init(data: Data, contentType: String, width: Int, height: Int) {
        self.data = data; self.contentType = contentType; self.width = width; self.height = height
    }

    public func filename(for purpose: PicturePurpose) -> String {
        "\(purpose.filenameStem).\(ProfilePicturePolicy.fileExtension(contentType))"
    }
}

/// Why a picked file can't become a picture, before anything is uploaded.
public enum PictureError: Error, Equatable, Sendable {
    /// A GIF over 5 MiB or with a logical screen over 1024 × 1024.
    case gifTooLarge
    /// Not a picture this device can decode.
    case unreadable
    /// Still over 1 MiB after re-encoding.
    case tooLarge
}

/// Pure picture decisions, kept separate from ImageIO and views for unit tests.
public enum ProfilePicturePolicy {
    /// GIFs upload unchanged up to this size (5 MiB).
    public static let gifMaxBytes = 5 * 1_024 * 1_024
    /// Every other picture after re-encoding (1 MiB).
    public static let stillMaxBytes = 1_024 * 1_024
    /// The server's largest accepted width or height.
    public static let maxDimension = 1_024
    /// Stills are scaled to at most this square edge.
    public static let stillEdge = 512
    /// libwebp lossy qualities, tried in order until the picture fits.
    static let webPQualities: [Float] = [90, 80, 70, 60]
    /// The largest edge decoded while preparing a still.
    static let maxPrepareDecode = 4_096

    /// Display decoding: at most 300 frames and 16 MiB of frame pixels per
    /// picture (fewer than inline attachments' 600 frames and 32 MiB); an
    /// animation over either shows one still. The cache keeps 96 MiB.
    static let maxAnimationFrames = 300
    static let maxAnimationBytes = 16 * 1_024 * 1_024
    static let cacheBytes = 96 * 1_024 * 1_024
    static let cacheCount = 400
    /// Downloaded files kept so an evicted or resized picture (an animation's
    /// frames are the large part) decodes again without downloading.
    static let fileCacheBytes = 24 * 1_024 * 1_024
    /// A downloaded picture is never larger than the biggest allowed GIF.
    static let maxDownloadBytes = 6 * 1_024 * 1_024
    /// A failed picture is retried with a fresh URL at once, else after this.
    static let retryAfter: TimeInterval = 120

    public static let gifLimitMessage = "GIFs can be up to 5 MB and 1024 × 1024."
    public static let unusableMessage = "That picture couldn’t be used. Try another one."
    public static let unavailableMessage = "Picture uploads aren’t available right now."

    // MARK: GIF

    /// `GIF87a` or `GIF89a` magic bytes; the file name and picker type are not trusted.
    public static func isGIF(_ data: Data) -> Bool {
        let header = [UInt8](data.prefix(6))
        return header == Array("GIF87a".utf8) || header == Array("GIF89a".utf8)
    }

    /// The logical screen size (bytes 6–9, little-endian) of a GIF.
    public static func gifSize(_ data: Data) -> (width: Int, height: Int)? {
        let header = [UInt8](data.prefix(10))
        guard header.count == 10, isGIF(data) else { return nil }
        return (Int(header[6]) | Int(header[7]) << 8, Int(header[8]) | Int(header[9]) << 8)
    }

    /// Why a GIF can't upload unchanged, or nil when it can.
    public static func gifError(_ data: Data) -> PictureError? {
        guard let size = gifSize(data), size.width > 0, size.height > 0 else { return .unreadable }
        if data.count > gifMaxBytes || size.width > maxDimension || size.height > maxDimension { return .gifTooLarge }
        return nil
    }

    // MARK: Stills

    /// The centred square of a `width` × `height` image.
    public static func squareCrop(width: Int, height: Int) -> CGRect {
        let side = max(0, min(width, height))
        return CGRect(x: (width - side) / 2, y: (height - side) / 2, width: side, height: side)
    }

    /// The uploaded square's edge: `min(512, short edge)`.
    public static func outputEdge(width: Int, height: Int) -> Int {
        max(1, min(stillEdge, min(width, height)))
    }

    /// ImageIO's thumbnail size (longest edge) for preparing a still whose
    /// oriented size is `width` × `height`: just large enough that the short
    /// edge still covers 512 px, never larger than the original or 4096.
    public static func decodeMaxPixelSize(width: Int, height: Int) -> Int {
        let long = max(1, max(width, height)), short = max(1, min(width, height))
        guard short > stillEdge else { return min(long, maxPrepareDecode) }
        let covering = Int((Double(long) * Double(stillEdge) / Double(short)).rounded(.up))
        return min(long, covering, maxPrepareDecode)
    }

    public static func fileExtension(_ contentType: String) -> String {
        switch contentType {
        case "image/gif": return "gif"
        case "image/webp": return "webp"
        case "image/png": return "png"
        default: return "jpg"
        }
    }

    // MARK: Display

    /// Custom pictures are rounded squares: corners 30% of the side (the
    /// shared `--radius-avatar`). Saved mascots keep their circle.
    public static func cornerRadius(side: CGFloat) -> CGFloat { side * 0.3 }

    /// Pixels to decode a picture shown `points` wide: the screen's pixels
    /// (at most 3×), rounded up to a 64-pixel step so nearby avatar sizes
    /// share one decoded copy, and at most 256.
    public static func displayPixelSize(points: CGFloat, scale: CGFloat) -> Int {
        let pixels = Double(points) * Double(min(max(scale, 1), 3))
        guard pixels.isFinite, pixels > 0 else { return 64 }
        return Int(min(256, max(64, (pixels / 64).rounded(.up) * 64)))
    }

    /// The longest edge to decode so a `width` × `height` picture still
    /// covers a `display`-pixel square (aspect-fill), within twice that.
    public static func coverPixelSize(display: Int, width: Int, height: Int) -> Int {
        guard width > 0, height > 0 else { return display }
        let long = max(width, height), short = min(width, height)
        let covering = Int((Double(display) * Double(long) / Double(short)).rounded(.up))
        return max(1, min(covering, long, display * 2))
    }

    /// Load a picture whose last load failed again: at once for a fresh URL
    /// (signatures change daily), otherwise only after `retryAfter`.
    public static func retries(failedURL: URL, failedAt: Date, url: URL, now: Date) -> Bool {
        url != failedURL || now.timeIntervalSince(failedAt) >= retryAfter
    }

    // MARK: Errors

    /// The inline message for a failed change or removal.
    public static func message(for error: Error, purpose: PicturePurpose) -> String {
        if let error = error as? PictureError {
            switch error {
            case .gifTooLarge: return gifLimitMessage
            case .unreadable: return unusableMessage
            case .tooLarge: return "That picture is too large. Try another one."
            }
        }
        if let error = error as? APIError {
            if error.code == "picture_invalid" || error.status == 422 { return unusableMessage }
            if error.code == "picture_too_large" || error.status == 413 {
                return "That picture is too large. GIFs can be up to 5 MB and other pictures up to 1 MB."
            }
            if error.code == "picture_type" { return "Use a PNG, JPEG, WebP or GIF picture." }
            if error.code == "picture_rate_limited" || error.status == 429 {
                return "You’ve changed pictures too many times today. Try again tomorrow."
            }
            switch error.status {
            case 400: return "Pictures can be up to 1024 × 1024."
            case 404:
                return purpose == .spaceIcon ? "Only the space owner can change its icon." : "Your picture couldn’t be saved. Try again."
            case 409: return "The upload didn’t finish. Try again."
            case 503: return unavailableMessage
            default: return FriendlyError.message(for: error)
            }
        }
        if error is StorageUploadError { return "Storage refused the upload. Try again." }
        return FriendlyError.message(for: error)
    }

    /// Uploads aren't configured on this server: hide the change controls.
    public static func uploadsUnavailable(_ error: Error) -> Bool { (error as? APIError)?.status == 503 }
}

// MARK: Preparation

/// Turns picked bytes into an uploadable picture (contract "Client
/// preparation"). Synchronous ImageIO work: call it off the main actor.
public enum ProfilePicturePreparer {
    /// GIFs (by magic bytes) upload unchanged within their limits so the
    /// animation survives. Anything else ImageIO decodes (PNG, JPEG, HEIC,
    /// WebP, AVIF, BMP…) becomes an oriented, centre-cropped square of at
    /// most 512 px, as lossy WebP (PNG/JPEG if WebP fails), at most 1 MiB.
    /// Re-encoding drops EXIF/GPS metadata; HEIC and AVIF are never uploaded.
    public static func prepare(_ data: Data) throws -> PreparedPicture {
        if ProfilePicturePolicy.isGIF(data) { return try prepareGIF(data) }
        return try prepareStill(data)
    }

    static func prepareGIF(_ data: Data) throws -> PreparedPicture {
        if let error = ProfilePicturePolicy.gifError(data) { throw error }
        guard let size = ProfilePicturePolicy.gifSize(data),
              let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) > 0 else { throw PictureError.unreadable }
        return PreparedPicture(data: data, contentType: "image/gif", width: size.width, height: size.height)
    }

    static func prepareStill(_ data: Data) throws -> PreparedPicture {
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) > 0 else { throw PictureError.unreadable }
        let index = CGImageSourceGetPrimaryImageIndex(source)
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
              let pixelWidth = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue,
              let pixelHeight = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue,
              pixelWidth > 0, pixelHeight > 0 else { throw PictureError.unreadable }
        let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
        let rotated = (5...8).contains(orientation)
        let width = rotated ? pixelHeight : pixelWidth, height = rotated ? pixelWidth : pixelHeight
        // The thumbnail applies the EXIF orientation.
        let maxPixelSize = ProfilePicturePolicy.decodeMaxPixelSize(width: width, height: height)
        guard let decoded = AttachmentPreparer.thumbnail(source, index: index, maxPixelSize: maxPixelSize),
              let square = decoded.cropping(to: ProfilePicturePolicy.squareCrop(width: decoded.width, height: decoded.height)),
              let image = render(square, edge: ProfilePicturePolicy.outputEdge(width: decoded.width, height: decoded.height)) else {
            throw PictureError.unreadable
        }
        guard let encoded = encode(image) else { throw PictureError.tooLarge }
        return PreparedPicture(data: encoded.data, contentType: encoded.contentType, width: image.width, height: image.height)
    }

    /// `image` scaled into an `edge`-pixel sRGB square.
    static func render(_ image: CGImage, edge: Int) -> CGImage? {
        guard edge > 0, let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil, width: edge, height: edge, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return nil }
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: edge, height: edge))
        return context.makeImage()
    }

    /// Lossy WebP at the first quality that fits 1 MiB; if libwebp fails,
    /// PNG for transparent pictures, else JPEG on white. Only pixels are
    /// written, never metadata.
    static func encode(_ image: CGImage) -> (data: Data, contentType: String)? {
        let limit = ProfilePicturePolicy.stillMaxBytes
        let pixels = AttachmentPreparer.rgbaPixels(image)
        if let pixels {
            for quality in ProfilePicturePolicy.webPQualities {
                guard let data = AttachmentPreparer.encodeLossyWebP(pixels, quality: quality) else { break }
                if data.count <= limit { return (data, "image/webp") }
            }
        }
        let transparent = pixels.map { pixels in stride(from: 3, to: pixels.straight.count, by: 4).contains { pixels.straight[$0] != 255 } } ?? true
        if transparent, let png = AttachmentPreparer.encode(image, type: UTType.png.identifier, quality: 1), png.count <= limit {
            return (png, "image/png")
        }
        guard let flat = AttachmentPreparer.flattened(image) else { return nil }
        for quality in [0.9, 0.8, 0.7] {
            if let jpeg = AttachmentPreparer.encode(flat, type: UTType.jpeg.identifier, quality: quality), jpeg.count <= limit {
                return (jpeg, "image/jpeg")
            }
        }
        return nil
    }
}

// MARK: Upload

/// Reserve, PUT straight to storage with exactly the presigned headers (no
/// app credentials), then complete, which applies the picture at once. Any
/// failure before completion cancels the reservation, as for attachments.
public enum ProfilePictureUploader {
    public static func uploadAvatar(_ picture: PreparedPicture, api: APIClient,
                                    completeRetryDelays: [Duration] = AttachmentUploader.completeRetryDelays) async throws -> Account {
        try await upload(picture, input: PictureCreateInput(picture: picture, purpose: .avatar), api: api,
                         completeRetryDelays: completeRetryDelays) { id in try await api.completeAvatar(id: id) }
    }

    public static func uploadSpaceIcon(_ picture: PreparedPicture, spaceID: String, api: APIClient,
                                       completeRetryDelays: [Duration] = AttachmentUploader.completeRetryDelays) async throws -> Space {
        try await upload(picture, input: PictureCreateInput(picture: picture, purpose: .spaceIcon, spaceID: spaceID), api: api,
                         completeRetryDelays: completeRetryDelays) { id in try await api.completeSpaceIcon(id: id) }
    }

    private static func upload<Applied: Sendable>(_ picture: PreparedPicture, input: PictureCreateInput, api: APIClient,
                                                  completeRetryDelays: [Duration],
                                                  complete: (String) async throws -> Applied) async throws -> Applied {
        let reservation = try await api.createPicture(input)
        do {
            try Task.checkCancellation()
            try await api.putToStorage(reservation.upload, body: .data(picture.data))
            try Task.checkCancellation()
            var delays = completeRetryDelays[...]
            while true {
                do {
                    // Applied once this returns: never cancel afterwards.
                    return try await complete(reservation.id)
                } catch let error as APIError where error.status == 409 {
                    guard let delay = delays.popFirst() else { throw error }
                    try await Task.sleep(for: delay)
                }
            }
        } catch {
            await api.cancelAsset(id: reservation.id)
            throw error
        }
    }
}

// MARK: Display decoding

/// A decoded picture: every frame of an animated GIF within the display
/// budget, or one still. Immutable, so the cache and views share it.
final class ProfilePictureFrames: @unchecked Sendable {
    let images: [CGImage]
    /// Seconds per frame; empty for a still.
    let durations: [Double]

    init(images: [CGImage], durations: [Double]) { self.images = images; self.durations = durations }

    var animated: Bool { images.count > 1 }
    /// Retained pixel bytes, the cache cost.
    var cost: Int { images.reduce(0) { $0 + $1.bytesPerRow * $1.height } }
}

/// Fetching and decoding, off the main actor. Reuses the inline GIF frame
/// loader (`AttachmentImageLoader.decodeAnimation`) at display size.
enum ProfilePictureDecoder {
    static func fetch(_ url: URL, session: URLSession) async throws -> Data {
        if url.isFileURL { return try Data(contentsOf: url) }
        let (data, response) = try await session.data(from: url)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) { throw AttachmentLoadError.status(http.statusCode) }
        guard data.count <= ProfilePicturePolicy.maxDownloadBytes else { throw AttachmentLoadError.undecodable }
        return data
    }

    /// Frames at most about `pixelSize` (covering a square), within the
    /// frame and memory budget: over the memory budget frames get smaller;
    /// too many frames, or nothing fitting, falls back to one still.
    static func decode(_ data: Data, pixelSize: Int) throws -> ProfilePictureFrames {
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) > 0 else { throw AttachmentLoadError.undecodable }
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = (properties?[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue ?? 0
        let height = (properties?[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue ?? 0
        let maxPixelSize = ProfilePicturePolicy.coverPixelSize(display: pixelSize, width: width, height: height)
        if let animation = try AttachmentImageLoader.decodeAnimation(source, maxPixelSize: maxPixelSize,
                                                                     maxFrames: ProfilePicturePolicy.maxAnimationFrames,
                                                                     byteLimit: ProfilePicturePolicy.maxAnimationBytes) {
            return ProfilePictureFrames(images: animation.images, durations: animation.durations)
        }
        guard let still = AttachmentImageLoader.decodeStill(source, maxPixelSize: maxPixelSize) else { throw AttachmentLoadError.undecodable }
        return ProfilePictureFrames(images: [still], durations: [])
    }
}

/// Decoded pictures cached by asset id and decode size, never by URL:
/// signed URLs change daily while the asset stays the same. `NSCache`
/// bounds memory for long member lists; concurrent requests for one picture
/// share a single download and decode. The downloaded files are cached by
/// asset too (the session is ephemeral), within a smaller budget.
@MainActor
final class ProfilePictureLoader {
    static let shared = ProfilePictureLoader()

    private let cache = NSCache<NSString, ProfilePictureFrames>()
    private let files = NSCache<NSString, NSData>()
    private let session: URLSession
    private var loading: [String: Task<(frames: ProfilePictureFrames, data: Data)?, Never>] = [:]
    /// The URL whose load last failed for a key, and when.
    private var failures: [String: (url: URL, at: Date)] = [:]

    init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieAcceptPolicy = .never
        configuration.httpShouldSetCookies = false
        session = URLSession(configuration: configuration)
        cache.totalCostLimit = ProfilePicturePolicy.cacheBytes
        cache.countLimit = ProfilePicturePolicy.cacheCount
        files.totalCostLimit = ProfilePicturePolicy.fileCacheBytes
    }

    nonisolated static func key(asset: String, pixelSize: Int) -> String { "\(asset):\(pixelSize)" }

    func cached(_ key: String) -> ProfilePictureFrames? { cache.object(forKey: key as NSString) }

    /// The decoded picture, or nil when it can't load (the caller keeps
    /// showing the mascot or initial).
    func load(asset: String, url: URL, pixelSize: Int) async -> ProfilePictureFrames? {
        let key = Self.key(asset: asset, pixelSize: pixelSize)
        if let hit = cached(key) { return hit }
        if let failure = failures[key],
           !ProfilePicturePolicy.retries(failedURL: failure.url, failedAt: failure.at, url: url, now: Date()) { return nil }
        if let running = loading[key] { return await running.value?.frames }
        let session = self.session
        let downloaded = files.object(forKey: asset as NSString).map { $0 as Data }
        let task = Task.detached(priority: .utility) { () -> (frames: ProfilePictureFrames, data: Data)? in
            // `??` can't await on its right-hand side, so the download is its own step.
            var bytes = downloaded
            if bytes == nil { bytes = try? await ProfilePictureDecoder.fetch(url, session: session) }
            guard let data = bytes, let frames = try? ProfilePictureDecoder.decode(data, pixelSize: pixelSize) else { return nil }
            return (frames, data)
        }
        loading[key] = task
        let result = await task.value
        loading[key] = nil
        if let result {
            cache.setObject(result.frames, forKey: key as NSString, cost: result.frames.cost)
            files.setObject(result.data as NSData, forKey: asset as NSString, cost: result.data.count)
            failures[key] = nil
        } else {
            if failures.count >= 512 { failures.removeAll() }
            failures[key] = (url, Date())
        }
        return result?.frames
    }
}
