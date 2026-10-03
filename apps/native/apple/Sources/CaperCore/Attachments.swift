import Foundation

/// Mirrors `assets::kind` on the API: only these types render inline.
public enum AttachmentKind: String, Codable, Equatable, Sendable {
    case image, video, audio, file

    public init(contentType: String) {
        let type = contentType.split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
        if ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].contains(type) { self = .image }
        else if ["video/mp4", "video/webm", "video/quicktime"].contains(type) { self = .video }
        else if ["audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac"].contains(type) { self = .audio }
        else { self = .file }
    }
}

/// A file on a message. Signed URLs are added per response and expire after
/// 24–48 hours; `ChatModel` refreshes them for long-open windows.
public struct ChatAttachment: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let kind: AttachmentKind
    public let contentType: String
    public let name: String
    public let size: Int
    public var width: Int?
    public var height: Int?
    public var durationMs: Int?
    /// The server stored a preview image (`previewUrl` when signed).
    public var hasPreview: Bool
    public var url: String?
    public var previewUrl: String?
    /// The file was deleted; show a "File removed" placeholder.
    public var unavailable: Bool

    private enum CodingKeys: String, CodingKey { case id, kind, contentType, name, size, width, height, durationMs, preview, url, previewUrl, unavailable }
    private struct PreviewMarker: Codable {}

    public init(id: String, kind: AttachmentKind, contentType: String, name: String, size: Int, width: Int? = nil, height: Int? = nil,
                durationMs: Int? = nil, hasPreview: Bool = false, url: String? = nil, previewUrl: String? = nil, unavailable: Bool = false) {
        self.id = id; self.kind = kind; self.contentType = contentType; self.name = name; self.size = size
        self.width = width; self.height = height; self.durationMs = durationMs; self.hasPreview = hasPreview
        self.url = url; self.previewUrl = previewUrl; self.unavailable = unavailable
    }

    /// Strict for one entry (like web's `isChatAttachment`); `ChatContent`
    /// skips entries that fail instead of rejecting the message.
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        kind = try values.decode(AttachmentKind.self, forKey: .kind)
        contentType = try values.decode(String.self, forKey: .contentType)
        name = try values.decode(String.self, forKey: .name)
        size = try values.decode(Int.self, forKey: .size)
        width = try values.decodeIfPresent(Int.self, forKey: .width)
        height = try values.decodeIfPresent(Int.self, forKey: .height)
        durationMs = try values.decodeIfPresent(Int.self, forKey: .durationMs)
        hasPreview = values.contains(.preview) && (try? values.decodeNil(forKey: .preview)) == false
        url = try values.decodeIfPresent(String.self, forKey: .url)
        previewUrl = try values.decodeIfPresent(String.self, forKey: .previewUrl)
        unavailable = try values.decodeIfPresent(Bool.self, forKey: .unavailable) ?? false
        guard !id.isEmpty, size >= 0, [width, height, durationMs].allSatisfy({ ($0 ?? 0) >= 0 }),
              [url, previewUrl].allSatisfy({ $0 == nil || Self.isWebURL($0!) }) else {
            throw DecodingError.dataCorruptedError(forKey: .id, in: values, debugDescription: "Invalid attachment")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(id, forKey: .id)
        try values.encode(kind, forKey: .kind)
        try values.encode(contentType, forKey: .contentType)
        try values.encode(name, forKey: .name)
        try values.encode(size, forKey: .size)
        try values.encodeIfPresent(width, forKey: .width)
        try values.encodeIfPresent(height, forKey: .height)
        try values.encodeIfPresent(durationMs, forKey: .durationMs)
        if hasPreview { try values.encode(PreviewMarker(), forKey: .preview) }
        try values.encodeIfPresent(url, forKey: .url)
        try values.encodeIfPresent(previewUrl, forKey: .previewUrl)
        if unavailable { try values.encode(true, forKey: .unavailable) }
    }

    static func isWebURL(_ value: String) -> Bool {
        guard let scheme = URL(string: value)?.scheme?.lowercased() else { return false }
        return scheme == "https" || scheme == "http"
    }
}

/// Decodes one array element without failing the surrounding array.
struct LossyAttachment: Decodable {
    let value: ChatAttachment?
    init(from decoder: Decoder) throws { value = try? ChatAttachment(from: decoder) }
}

/// Server-tunable client compression settings from `GET /api/assets/usage`.
/// Missing or out-of-range values fall back to the contract defaults.
public struct AttachmentCompression: Codable, Equatable, Sendable {
    public var imageQuality: Int = 92
    public var imageMaxEdge: Int = 4096
    public var paletteColors: Int = 256
    public var previewEdge: Int = 640
    public var videoMaxHeight: Int = 1080
    public var videoBitrateKbps: Int = 4000
    public var audioBitrateKbps: Int = 128

    public init() {}

    private enum CodingKeys: String, CodingKey { case imageQuality, imageMaxEdge, paletteColors, previewEdge, videoMaxHeight, videoBitrateKbps, audioBitrateKbps }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func value(_ key: CodingKeys, _ range: ClosedRange<Int>, _ fallback: Int) -> Int {
            guard let decoded = try? values.decodeIfPresent(Int.self, forKey: key), range.contains(decoded) else { return fallback }
            return decoded
        }
        imageQuality = value(.imageQuality, 1...100, 92)
        imageMaxEdge = value(.imageMaxEdge, 0...65_536, 4096)
        paletteColors = value(.paletteColors, 0...256, 256)
        previewEdge = value(.previewEdge, 0...4096, 640)
        videoMaxHeight = value(.videoMaxHeight, 0...8192, 1080)
        videoBitrateKbps = value(.videoBitrateKbps, 1...1_000_000, 4000)
        audioBitrateKbps = value(.audioBitrateKbps, 1...10_000, 128)
    }
}

public struct AssetUsage: Decodable, Equatable, Sendable {
    public let used: Int
    public let limit: Int
    public let compression: AttachmentCompression

    private enum CodingKeys: String, CodingKey { case used, limit, compression }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        used = try values.decode(Int.self, forKey: .used)
        limit = try values.decode(Int.self, forKey: .limit)
        compression = (try? values.decodeIfPresent(AttachmentCompression.self, forKey: .compression)) ?? AttachmentCompression()
    }
}

/// `POST /api/assets` body. Optional fields are omitted when nil.
public struct AssetCreateInput: Encodable, Equatable, Sendable {
    public struct Preview: Encodable, Equatable, Sendable { public let contentType: String; public let byteSize: Int }
    public let channelId: String
    public let filename: String
    public let contentType: String
    public let byteSize: Int
    public var sourceByteSize: Int?
    public var width: Int?
    public var height: Int?
    public var durationMs: Int?
    public var preview: Preview?
}

public struct PresignedUpload: Decodable, Equatable, Sendable {
    public let method: String
    public let url: String
    public let headers: [String: String]
}

public struct AssetReservation: Decodable, Equatable, Sendable {
    public let id: String
    public let upload: PresignedUpload
    public let previewUpload: PresignedUpload?
}

public struct AttachmentURLs: Decodable, Equatable, Sendable {
    public let url: String
    public let previewUrl: String?
}

struct AttachmentURLsResponse: Decodable { let urls: [String: AttachmentURLs] }

/// When a signed delivery URL must be replaced. URLs carry `exp` in Unix
/// seconds; refresh when it is past or within an hour, or after a 403/404.
public enum AttachmentURLPolicy {
    public static let refreshMargin: TimeInterval = 60 * 60

    public static func expiry(of url: String?) -> Date? {
        guard let url, let components = URLComponents(string: url),
              let raw = components.queryItems?.first(where: { $0.name == "exp" })?.value,
              let seconds = Int64(raw), seconds > 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(seconds))
    }

    public static func needsRefresh(_ url: String?, now: Date = Date()) -> Bool {
        guard let expiry = expiry(of: url) else { return false }
        return expiry.timeIntervalSince(now) <= refreshMargin
    }

    /// Load failures that a fresh signature can fix.
    public static func refreshesAfterFailure(status: Int) -> Bool { status == 403 || status == 404 }
}

/// Pure decisions from the attachment compression contract, kept separate
/// from ImageIO/AVFoundation work so they can be unit tested.
public enum AttachmentPolicy {
    public static let maxAttachments = 10
    public static let previewMaxBytes = 512 * 1024
    public static let previewQuality = 0.8
    /// Decoding enormous images can exhaust memory on phones; upload as-is.
    public static let maxCompressPixels = 50_000_000

    public enum StillEncoding: Equatable, Sendable { case indexedPNG, lossy(quality: Double), none }

    /// Re-encode stills except animated or vector formats.
    public static func compressible(_ contentType: String) -> Bool {
        let type = contentType.lowercased()
        return type.hasPrefix("image/") && !["image/gif", "image/svg+xml", "image/avif"].contains(type)
    }

    /// `colorCount` is nil when the image has more colours than the palette limit.
    public static func stillEncoding(colorCount: Int?, settings: AttachmentCompression) -> StillEncoding {
        if settings.paletteColors > 0, let colorCount, colorCount <= settings.paletteColors { return .indexedPNG }
        if settings.imageQuality < 100 { return .lossy(quality: Double(settings.imageQuality) / 100) }
        return .none
    }

    /// Keep a re-encoded file only when it is at least 10% smaller, or when the
    /// original type cannot render inline at all (e.g. HEIC).
    public static func keepReencoded(originalType: String, originalSize: Int, encodedSize: Int) -> Bool {
        AttachmentKind(contentType: originalType) != .image || Double(encodedSize) <= Double(originalSize) * 0.9
    }

    public static func needsPreview(kind: AttachmentKind, width: Int?, height: Int?, byteSize: Int, settings: AttachmentCompression) -> Bool {
        guard settings.previewEdge > 0 else { return false }
        if kind == .video { return width != nil && height != nil }
        guard kind == .image else { return false }
        return max(width ?? 0, height ?? 0) > settings.previewEdge || byteSize > previewMaxBytes
    }

    /// Longest edge scaled to `edge`, never enlarged.
    public static func fitWithin(width: Int, height: Int, edge: Int) -> (width: Int, height: Int) {
        guard width > 0, height > 0, edge > 0 else { return (max(1, width), max(1, height)) }
        let scale = min(1, Double(edge) / Double(max(width, height)))
        return (max(1, Int((Double(width) * scale).rounded())), max(1, Int((Double(height) * scale).rounded())))
    }

    /// Message display frame that reserves layout space before loading.
    public static func displaySize(width: Int?, height: Int?, maxWidth: Double = 360, maxHeight: Double = 300) -> (width: Double, height: Double)? {
        guard let width, let height, width > 0, height > 0 else { return nil }
        let scale = min(1, maxWidth / Double(width), maxHeight / Double(height))
        return ((Double(width) * scale).rounded(), (Double(height) * scale).rounded())
    }

    public static func renamed(_ name: String, contentType: String) -> String {
        let extensions = ["image/webp": "webp", "image/jpeg": "jpg", "image/png": "png", "video/mp4": "mp4"]
        guard let ext = extensions[contentType] else { return name }
        let base = (name as NSString).deletingPathExtension
        return "\(base.isEmpty ? name : base).\(ext)"
    }

    /// AVAssetExportSession size presets as (long edge, short edge).
    public static let videoPresets: [(name: String, long: Int, short: Int)] = [
        ("AVAssetExportPreset640x480", 640, 480),
        ("AVAssetExportPreset960x540", 960, 540),
        ("AVAssetExportPreset1280x720", 1280, 720),
        ("AVAssetExportPreset1920x1080", 1920, 1080),
        ("AVAssetExportPreset3840x2160", 3840, 2160),
    ]

    /// The largest size preset whose output height (display orientation) stays
    /// within `maxHeight`. Presets fit the video inside long×short without
    /// enlarging it. Nil disables transcoding.
    public static func videoPreset(width: Int, height: Int, maxHeight: Int) -> String? {
        guard maxHeight > 0, width > 0, height > 0 else { return nil }
        let candidates = videoPresets.filter { preset in
            let scale = min(1, Double(preset.long) / Double(max(width, height)), Double(preset.short) / Double(min(width, height)))
            return Int((Double(height) * scale).rounded()) <= maxHeight
        }
        return candidates.last?.name
    }

    public static func formatBytes(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        let units = ["KB", "MB", "GB"]
        var value = Double(bytes) / 1024
        var unit = 0
        while value >= 1024 && unit < units.count - 1 { value /= 1024; unit += 1 }
        let number = value >= 10 ? String(Int(value.rounded())) : String(format: "%.1f", value)
        return "\(number) \(units[unit])"
    }

    public static func uploadErrorMessage(_ error: Error) -> String {
        if let error = error as? APIError {
            if error.code == "storage_full" { return "You’ve used all of your file storage." }
            switch error.status {
            case 413: return "This file is too large to upload."
            case 429: return "Uploading too quickly. Try again shortly."
            case 404: return "You can’t upload files here."
            case 409: return "The upload didn’t finish. Remove it and try again."
            case 422: return "The uploaded file didn’t match. Remove it and try again."
            default: return error.message
            }
        }
        if error is StorageUploadError { return "Storage refused the upload." }
        if error is URLError { return "The upload was interrupted." }
        return "Upload failed."
    }
}

/// A presigned storage `PUT` did not succeed.
public struct StorageUploadError: Error, Equatable { public let status: Int }

/// A draft file in the composer, from selection until it is sent.
public struct AttachmentDraft: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public var kind: AttachmentKind
    /// Local file shown as the chip thumbnail and the pending-message preview.
    public var localURL: URL
    public var sourceSize: Int
    public var storedSize: Int?
    public var progress: Double = 0
    public var error: String?
    public var attachment: ChatAttachment?

    public var statusLabel: String {
        if let error { return error }
        guard attachment != nil else { return "Uploading… \(Int((progress * 100).rounded()))%" }
        if let storedSize, storedSize < sourceSize {
            return "\(AttachmentPolicy.formatBytes(sourceSize)) → \(AttachmentPolicy.formatBytes(storedSize))"
        }
        return AttachmentPolicy.formatBytes(storedSize ?? sourceSize)
    }
}
