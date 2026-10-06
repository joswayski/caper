import Foundation
import UniformTypeIdentifiers

/// Mirrors `assets::kind` on the API: only these types render inline.
public enum AttachmentKind: String, Codable, Equatable, Sendable {
    case image, video, audio, file

    public init(contentType: String) {
        let type = Self.baseType(contentType)
        if ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].contains(type) { self = .image }
        else if ["video/mp4", "video/webm", "video/quicktime"].contains(type) { self = .video }
        else if ["audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac"].contains(type) { self = .audio }
        else { self = .file }
    }

    /// What this device can show for a local copy (draft chips and the
    /// pending row). ImageIO and AVFoundation decode more than browsers, such
    /// as HEIC photos, which upload unchanged and are converted by the server.
    public static func local(contentType: String) -> AttachmentKind {
        guard let type = UTType(mimeType: baseType(contentType)) else { return AttachmentKind(contentType: contentType) }
        if type.conforms(to: .movie) { return .video }
        if type.conforms(to: .audio) { return .audio }
        if type.conforms(to: .image), !type.conforms(to: .svg) { return .image }
        return AttachmentKind(contentType: contentType)
    }

    private static func baseType(_ contentType: String) -> String {
        contentType.split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
    }
}

/// Server-side processing state. Payloads from before server-side
/// compression omit it, which means `ready`.
public enum AttachmentStatus: String, Codable, Equatable, Sendable {
    case processing, ready, failed
}

/// A file on a message. Signed URLs are added per response and expire after
/// 24–48 hours; `ChatModel` refreshes them for long-open windows. `url` is
/// present only once the media worker has finished (`status == .ready`).
public struct ChatAttachment: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public var kind: AttachmentKind
    public let contentType: String
    public let name: String
    public let size: Int
    public var width: Int?
    public var height: Int?
    public var durationMs: Int?
    /// The server stored a preview image (`previewUrl` when signed).
    public var hasPreview: Bool
    public var status: AttachmentStatus
    /// A GIF or animated image stored as a silent looping MP4: play muted,
    /// looping and without controls.
    public var animated: Bool
    public var url: String?
    public var previewUrl: String?
    /// The file was deleted; show a "File removed" placeholder.
    public var unavailable: Bool

    private enum CodingKeys: String, CodingKey {
        case id, kind, contentType, name, size, width, height, durationMs, preview, status, animated, url, previewUrl, unavailable
    }
    private struct PreviewMarker: Codable {}

    public init(id: String, kind: AttachmentKind, contentType: String, name: String, size: Int, width: Int? = nil, height: Int? = nil,
                durationMs: Int? = nil, hasPreview: Bool = false, status: AttachmentStatus = .ready, animated: Bool = false,
                url: String? = nil, previewUrl: String? = nil, unavailable: Bool = false) {
        self.id = id; self.kind = kind; self.contentType = contentType; self.name = name; self.size = size
        self.width = width; self.height = height; self.durationMs = durationMs; self.hasPreview = hasPreview
        self.status = status; self.animated = animated
        self.url = url; self.previewUrl = previewUrl; self.unavailable = unavailable
    }

    /// Strict for one entry (like web's `isChatAttachment`); `ChatContent`
    /// skips entries that fail instead of rejecting the message. The newer
    /// optional fields (`status`, `animated`) never fail an entry.
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
        animated = (try? values.decodeIfPresent(Bool.self, forKey: .animated)) ?? false
        if let raw = try? values.decodeIfPresent(String.self, forKey: .status) {
            // An unknown future state renders like processing until it has a URL.
            status = AttachmentStatus(rawValue: raw) ?? (url == nil ? .processing : .ready)
        } else {
            status = .ready
        }
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
        if status != .ready { try values.encode(status, forKey: .status) }
        if animated { try values.encode(true, forKey: .animated) }
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

extension KeyedDecodingContainer {
    /// Valid entries of an attachments array, skipping malformed ones; nil
    /// when the field is absent or not an array.
    func decodeLossyAttachments(forKey key: Key) -> [ChatAttachment]? {
        guard var list = try? nestedUnkeyedContainer(forKey: key) else { return nil }
        var decoded: [ChatAttachment] = []
        while !list.isAtEnd {
            let index = list.currentIndex
            if (try? list.decodeNil()) == true { continue }
            if let entry = try? list.decode(LossyAttachment.self), let value = entry.value { decoded.append(value) }
            if list.currentIndex == index { break }
        }
        return decoded
    }
}

/// `GET /api/assets/usage`.
public struct AssetUsage: Decodable, Equatable, Sendable {
    public let used: Int
    public let limit: Int
    /// Largest original the API accepts; nil when the response omits it.
    public let maxUploadBytes: Int?

    private enum CodingKeys: String, CodingKey { case used, limit, maxUploadBytes }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        used = try values.decode(Int.self, forKey: .used)
        limit = try values.decode(Int.self, forKey: .limit)
        let declared = try? values.decodeIfPresent(Int.self, forKey: .maxUploadBytes)
        maxUploadBytes = declared.flatMap { $0 > 0 ? $0 : nil }
    }
}

/// `POST /api/assets` body: the original file, exactly as it will be uploaded.
public struct AssetCreateInput: Encodable, Equatable, Sendable {
    public let channelId: String
    public let filename: String
    public let contentType: String
    public let byteSize: Int

    public init(channelId: String, filename: String, contentType: String, byteSize: Int) {
        self.channelId = channelId; self.filename = filename; self.contentType = contentType; self.byteSize = byteSize
    }
}

public struct PresignedUpload: Decodable, Equatable, Sendable {
    public let method: String
    public let url: String
    public let headers: [String: String]
}

public struct AssetReservation: Decodable, Equatable, Sendable {
    public let id: String
    public let upload: PresignedUpload
}

/// Fresh signatures for one attachment; `url` is absent until it is ready.
public struct AttachmentURLs: Decodable, Equatable, Sendable {
    public let url: String?
    public let previewUrl: String?

    public init(url: String?, previewUrl: String?) { self.url = url; self.previewUrl = previewUrl }
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

/// Pure attachment decisions, kept separate from the views for unit tests.
public enum AttachmentPolicy {
    public static let maxAttachments = 10

    /// Message display frame that reserves layout space before loading.
    public static func displaySize(width: Int?, height: Int?, maxWidth: Double = 360, maxHeight: Double = 300) -> (width: Double, height: Double)? {
        guard let width, let height, width > 0, height > 0 else { return nil }
        let scale = min(1, maxWidth / Double(width), maxHeight / Double(height))
        return ((Double(width) * scale).rounded(), (Double(height) * scale).rounded())
    }

    /// Checked before reserving, so an oversized original never starts uploading.
    public static func tooLargeMessage(size: Int, maxUploadBytes: Int?) -> String? {
        guard let maxUploadBytes, size > maxUploadBytes else { return nil }
        return "This file is too large to upload (max \(formatBytes(maxUploadBytes)))."
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
    /// What this device can preview locally (see `AttachmentKind.local`).
    public var kind: AttachmentKind
    /// The staged original: uploaded unchanged, and shown as the chip
    /// thumbnail and the pending-message preview.
    public var localURL: URL
    public var size: Int
    public var progress: Double = 0
    public var error: String?
    /// The confirmed upload (`status: processing` until the server finishes).
    public var attachment: ChatAttachment?

    public init(id: String, name: String, kind: AttachmentKind, localURL: URL, size: Int) {
        self.id = id; self.name = name; self.kind = kind; self.localURL = localURL; self.size = size
    }

    public var statusLabel: String {
        if let error { return error }
        guard attachment != nil else { return "Uploading… \(Int((progress * 100).rounded()))%" }
        return AttachmentPolicy.formatBytes(size)
    }
}
