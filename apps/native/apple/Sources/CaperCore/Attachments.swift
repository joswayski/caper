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
    /// as a HEIC original whose conversion failed and uploads unchanged.
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

/// Server-side processing state from the parked server pipeline (see
/// `docs/media.md`). Today's API never sends it; absent means `ready`.
public enum AttachmentStatus: String, Codable, Equatable, Sendable {
    case processing, ready, failed
}

/// A file on a message. Signed URLs are added per response and expire after
/// 24–48 hours; `ChatModel` refreshes them for long-open windows. A
/// `processing` file (parked server pipeline only) has no `url` yet.
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

/// Photo format the server asks for (`compression.imageFormat`; webp when absent).
public enum AttachmentImageFormat: String, Codable, Equatable, Sendable { case avif, webp }

/// Server-tunable client compression settings from `GET /api/assets/usage`
/// (`docs/media.md`, "Client compression and previews"). Missing or
/// out-of-range values fall back to the contract defaults.
public struct AttachmentCompression: Codable, Equatable, Sendable {
    /// `.avif` (only the exact value "avif") encodes photos as AVIF where this
    /// device can, else WebP/JPEG at `imageQuality`. Missing or unknown means
    /// WebP, so older servers behave as before.
    public var imageFormat: AttachmentImageFormat = .webp
    /// AVIF photo quality on libavif's `quality` scale (as `avifenc -q`).
    public var avifQuality: Int = 85
    /// WebP/JPEG photo quality and the fallback whenever AVIF is unavailable
    /// or fails; 100 disables lossy photo re-encoding in either format.
    public var imageQuality: Int = 92
    public var imageMaxEdge: Int = 4096
    public var paletteColors: Int = 256
    public var previewEdge: Int = 640
    public var videoMaxHeight: Int = 1080
    public var videoBitrateKbps: Int = 6000
    public var audioBitrateKbps: Int = 128

    public init() {}

    private enum CodingKeys: String, CodingKey {
        case imageFormat, avifQuality, imageQuality, imageMaxEdge, paletteColors, previewEdge, videoMaxHeight, videoBitrateKbps, audioBitrateKbps
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        func value(_ key: CodingKeys, _ range: ClosedRange<Int>, _ fallback: Int) -> Int {
            guard let decoded = try? values.decodeIfPresent(Int.self, forKey: key), range.contains(decoded) else { return fallback }
            return decoded
        }
        let format: String? = try? values.decodeIfPresent(String.self, forKey: .imageFormat)
        imageFormat = format == AttachmentImageFormat.avif.rawValue ? .avif : .webp
        avifQuality = value(.avifQuality, 1...100, 85)
        imageQuality = value(.imageQuality, 1...100, 92)
        imageMaxEdge = value(.imageMaxEdge, 0...65_536, 4096)
        paletteColors = value(.paletteColors, 0...256, 256)
        previewEdge = value(.previewEdge, 0...4096, 640)
        videoMaxHeight = value(.videoMaxHeight, 0...8192, 1080)
        videoBitrateKbps = value(.videoBitrateKbps, 1...1_000_000, 6000)
        audioBitrateKbps = value(.audioBitrateKbps, 1...10_000, 128)
    }
}

/// `GET /api/assets/usage`.
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

/// `POST /api/assets` body: the (compressed) file exactly as it will be
/// stored, plus its optional preview. Optional fields are omitted when nil.
public struct AssetCreateInput: Encodable, Equatable, Sendable {
    public struct Preview: Encodable, Equatable, Sendable {
        public let contentType: String
        public let byteSize: Int
        public init(contentType: String, byteSize: Int) { self.contentType = contentType; self.byteSize = byteSize }
    }
    public let channelId: String
    public let filename: String
    public let contentType: String
    public let byteSize: Int
    public var sourceByteSize: Int?
    public var width: Int?
    public var height: Int?
    public var durationMs: Int?
    public var preview: Preview?

    public init(channelId: String, filename: String, contentType: String, byteSize: Int, sourceByteSize: Int? = nil,
                width: Int? = nil, height: Int? = nil, durationMs: Int? = nil, preview: Preview? = nil) {
        self.channelId = channelId; self.filename = filename; self.contentType = contentType; self.byteSize = byteSize
        self.sourceByteSize = sourceByteSize; self.width = width; self.height = height; self.durationMs = durationMs; self.preview = preview
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
    /// Present when the reservation declared a preview.
    public let previewUpload: PresignedUpload?
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

    // MARK: Compression decisions (docs/media.md, "Client compression and previews")

    public static let previewMaxBytes = 512 * 1024
    public static let previewQuality = 0.8
    /// Decoding enormous images can exhaust memory on phones; upload as-is.
    public static let maxCompressPixels = 50_000_000
    /// Counting colours needs a full RGBA copy; larger stills skip the
    /// palette check (photos still re-encode, lossless files stay as they are).
    public static let maxPalettePixels = 25_000_000
    /// A size-only re-encode or transcode must save at least this fraction.
    public static let minimumSaving = 0.10

    /// How a still is treated, from its type (and WebP bitstream).
    public enum StillSource: Equatable, Sendable {
        /// JPEG, HEIC/HEIF and lossy WebP: may be re-encoded lossily.
        case photo
        /// PNG, BMP, TIFF and lossless WebP: never encoded lossily.
        case lossless
        /// GIF, SVG, AVIF, animated images and anything else.
        case unchanged
    }

    /// One way to re-encode a still. Lossless candidates are all tried and
    /// the smallest verified one wins; lossy candidates are used only when no
    /// lossless candidate succeeded, in order: AVIF (libavif `quality`, 1...100)
    /// and, when the AVIF encoder fails, WebP/JPEG (`lossy`, 0...1).
    public enum StillCandidate: Equatable, Sendable { case indexedPNG, losslessWebP, avif(quality: Int), lossy(quality: Double) }

    public enum WebPFormat: Equatable, Sendable { case lossy, lossless, animated }

    static func baseType(_ contentType: String) -> String {
        contentType.split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
    }

    /// `webP` is the sniffed bitstream for `image/webp` (nil when unknown,
    /// which is treated as lossless so it is never encoded lossily).
    public static func stillSource(contentType: String, webP: WebPFormat? = nil) -> StillSource {
        switch baseType(contentType) {
        case "image/jpeg", "image/jpg", "image/pjpeg", "image/heic", "image/heif": return .photo
        case "image/png", "image/bmp", "image/x-bmp", "image/x-ms-bmp", "image/tiff": return .lossless
        case "image/webp":
            switch webP {
            case .lossy: return .photo
            case .animated: return .unchanged
            case .lossless, .none: return .lossless
            }
        default: return .unchanged
        }
    }

    /// Reads the RIFF chunks of a WebP file's first bytes: `VP8 ` is lossy,
    /// `VP8L` lossless, and an animation flag or `ANIM` chunk is animated.
    public static func webPFormat(_ bytes: Data) -> WebPFormat? {
        let b = [UInt8](bytes.prefix(65_536))
        func tag(_ offset: Int, _ value: String) -> Bool {
            let expected = Array(value.utf8)
            return offset + expected.count <= b.count && Array(b[offset..<offset + expected.count]) == expected
        }
        guard tag(0, "RIFF"), tag(8, "WEBP") else { return nil }
        var offset = 12
        while offset + 8 <= b.count {
            let size = Int(b[offset + 4]) | Int(b[offset + 5]) << 8 | Int(b[offset + 6]) << 16 | Int(b[offset + 7]) << 24
            if tag(offset, "VP8 ") { return .lossy }
            if tag(offset, "VP8L") { return .lossless }
            if tag(offset, "ANIM") || tag(offset, "ANMF") { return .animated }
            if tag(offset, "VP8X") {
                guard offset + 8 < b.count else { return nil }
                if b[offset + 8] & 0x02 != 0 { return .animated }
            }
            offset += 8 + size + (size & 1)
        }
        return nil
    }

    /// What to try for a still, in order. `colorCount` is the exact number
    /// of distinct colours, or nil when there are more than the palette limit
    /// (or they were not counted). Lossless sources only ever get lossless
    /// candidates: indexed PNG when the palette fits, and lossless WebP.
    /// HEIC/HEIF must always convert because browsers cannot show it.
    /// Photos try AVIF first when the server asks for it and this device can
    /// encode it (`avifEncodable`). `imageQuality` 100 disables lossy encoding
    /// in either format; HEIC then converts through WebP/JPEG at full quality.
    public static func stillCandidates(contentType: String, source: StillSource, colorCount: Int?, settings: AttachmentCompression,
                                       avifEncodable: Bool = false) -> [StillCandidate] {
        let paletteFits = settings.paletteColors > 0 && colorCount.map { $0 <= settings.paletteColors } == true
        switch source {
        case .unchanged:
            return []
        case .lossless:
            return paletteFits ? [.indexedPNG, .losslessWebP] : [.losslessWebP]
        case .photo:
            var candidates: [StillCandidate] = paletteFits ? [.indexedPNG] : []
            if settings.imageQuality < 100 {
                if settings.imageFormat == .avif, avifEncodable { candidates.append(.avif(quality: settings.avifQuality)) }
                candidates.append(.lossy(quality: Double(settings.imageQuality) / 100))
            } else if AttachmentKind(contentType: contentType) != .image {
                candidates.append(.lossy(quality: 1))
            }
            return candidates
        }
    }

    /// Keep a re-encoded file when the original type cannot render inline at
    /// all (e.g. HEIC, BMP, TIFF); otherwise a lossless result must be
    /// smaller and a lossy one at least 10% smaller.
    public static func keepReencoded(originalType: String, originalSize: Int, encodedSize: Int, lossless: Bool) -> Bool {
        guard encodedSize > 0 else { return false }
        if AttachmentKind(contentType: originalType) != .image { return true }
        return lossless ? encodedSize < originalSize : Double(encodedSize) <= Double(originalSize) * (1 - minimumSaving)
    }

    /// WebP's maximum width and height.
    public static let webPMaxDimension = 16_383

    /// Wraps a simple-format WebP (`RIFF/WEBP/VP8L` or `VP8 `) in the
    /// extended format with an `ICCP` chunk, so colours keep their meaning.
    public static func webPAddingICCProfile(_ webP: Data, profile: Data, width: Int, height: Int, hasAlpha: Bool) -> Data? {
        let bytes = [UInt8](webP)
        guard bytes.count >= 20, (1...webPMaxDimension).contains(width), (1...webPMaxDimension).contains(height), !profile.isEmpty,
              Array(bytes[0..<4]) == Array("RIFF".utf8), Array(bytes[8..<12]) == Array("WEBP".utf8),
              [Array("VP8L".utf8), Array("VP8 ".utf8)].contains(Array(bytes[12..<16])) else { return nil }
        func littleEndian(_ value: Int, _ count: Int) -> [UInt8] { (0..<count).map { UInt8(truncatingIfNeeded: value >> (8 * $0)) } }
        var output: [UInt8] = Array("RIFF".utf8)
        output += [0, 0, 0, 0] as [UInt8] // RIFF size, filled in below.
        output += Array("WEBP".utf8)
        output += Array("VP8X".utf8)
        output += littleEndian(10, 4)
        output += [UInt8(0x20 | (hasAlpha ? 0x10 : 0)), 0, 0, 0] // ICC (and alpha) flags.
        output += littleEndian(width - 1, 3)
        output += littleEndian(height - 1, 3)
        output += Array("ICCP".utf8)
        output += littleEndian(profile.count, 4)
        output += [UInt8](profile)
        if profile.count % 2 == 1 { output.append(0) }
        output += bytes[12...]
        let riffSize = littleEndian(output.count - 8, 4)
        output.replaceSubrange(4..<8, with: riffSize)
        return Data(output)
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

    public static func renamed(_ name: String, contentType: String) -> String {
        let extensions = ["image/avif": "avif", "image/webp": "webp", "image/jpeg": "jpg", "image/png": "png", "video/mp4": "mp4"]
        guard let ext = extensions[contentType] else { return name }
        let base = (name as NSString).deletingPathExtension
        return "\(base.isEmpty ? name : base).\(ext)"
    }

    // MARK: Video

    /// What AVFoundation reports about a source video (display orientation).
    public struct VideoFacts: Equatable, Sendable {
        public var width: Int
        public var height: Int
        public var isH264: Bool
        /// HLG or PQ transfer (iPhone HDR/Dolby Vision, HDR10).
        public var isHDR: Bool
        /// Video track data rate; nil when unknown.
        public var bitrateKbps: Double?
        /// The container is one the API serves inline (MP4, QuickTime, WebM).
        public var playableContainer: Bool

        public init(width: Int, height: Int, isH264: Bool, isHDR: Bool, bitrateKbps: Double?, playableContainer: Bool) {
            self.width = width; self.height = height; self.isH264 = isH264; self.isHDR = isHDR
            self.bitrateKbps = bitrateKbps; self.playableContainer = playableContainer
        }
    }

    public enum VideoPlan: Equatable, Sendable {
        /// Upload the original unchanged.
        case keep
        /// Copy the H.264 samples into MP4 unchanged (only the container is unplayable).
        case remux
        /// Re-encode to H.264/AAC MP4 with this export preset. `required`
        /// transcodes (codec, HDR, container) are kept at any size; size-only
        /// ones only when at least 10% smaller.
        case transcode(preset: String, required: Bool)
    }

    /// `videoBitrateKbps` is the 1080p target; smaller frames scale it by
    /// pixel count, with a floor so small videos are not starved.
    public static let videoBitrateFloorKbps = 1500.0

    public static func videoTargetKbps(width: Int, height: Int, settings: AttachmentCompression) -> Double {
        let pixels = Double(max(0, width) * max(0, height))
        return max(videoBitrateFloorKbps, Double(settings.videoBitrateKbps) * pixels / (1920 * 1080))
    }

    public static func videoPlan(_ facts: VideoFacts, settings: AttachmentCompression) -> VideoPlan {
        guard settings.videoMaxHeight > 0, facts.width > 0, facts.height > 0 else { return .keep }
        let oversized = min(facts.width, facts.height) > settings.videoMaxHeight
        let heavy = facts.bitrateKbps.map { $0 > 1.25 * videoTargetKbps(width: facts.width, height: facts.height, settings: settings) } ?? false
        let required = !facts.isH264 || facts.isHDR || !facts.playableContainer
        guard oversized || heavy || !facts.isH264 || facts.isHDR else { return facts.playableContainer ? .keep : .remux }
        guard let preset = videoPreset(width: facts.width, height: facts.height, maxHeight: settings.videoMaxHeight) else { return .keep }
        return .transcode(preset: preset, required: required)
    }

    /// Whether an export may replace the original: it must be H.264 (for a
    /// transcode), SDR, keep the audio, and be smaller unless it was required.
    public static func keepTranscoded(required: Bool, originalSize: Int, outputSize: Int, outputIsH264: Bool, outputIsHDR: Bool,
                                      sourceHasAudio: Bool, outputHasAudio: Bool) -> Bool {
        guard outputSize > 0, outputIsH264, !outputIsHDR, !sourceHasAudio || outputHasAudio else { return false }
        return required || Double(outputSize) <= Double(originalSize) * (1 - minimumSaving)
    }

    /// AVAssetExportSession H.264/AAC size presets as (long edge, short edge).
    public static let videoPresets: [(name: String, long: Int, short: Int)] = [
        ("AVAssetExportPreset640x480", 640, 480),
        ("AVAssetExportPreset960x540", 960, 540),
        ("AVAssetExportPreset1280x720", 1280, 720),
        ("AVAssetExportPreset1920x1080", 1920, 1080),
        ("AVAssetExportPreset3840x2160", 3840, 2160),
    ]

    /// The largest size preset whose output short edge stays within
    /// `maxHeight`. Presets fit the video inside long×short without
    /// enlarging it. Nil disables transcoding.
    public static func videoPreset(width: Int, height: Int, maxHeight: Int) -> String? {
        guard maxHeight > 0, width > 0, height > 0 else { return nil }
        let candidates = videoPresets.filter { preset in
            let scale = min(1, Double(preset.long) / Double(max(width, height)), Double(preset.short) / Double(min(width, height)))
            // "1080p" bounds the short edge, so portrait phone video keeps full detail.
            return Int((Double(min(width, height)) * scale).rounded()) <= maxHeight
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
    /// What this device can preview locally (see `AttachmentKind.local`).
    public var kind: AttachmentKind
    /// The staged file: the original until compression finishes, then the
    /// file that is uploaded. Shown as the chip thumbnail and the
    /// pending-message preview.
    public var localURL: URL
    public var sourceSize: Int
    /// Bytes stored after compression (nil while preparing).
    public var storedSize: Int?
    /// Compression is still running.
    public var preparing = true
    public var progress: Double = 0
    public var error: String?
    /// The confirmed upload.
    public var attachment: ChatAttachment?

    public init(id: String, name: String, kind: AttachmentKind, localURL: URL, sourceSize: Int) {
        self.id = id; self.name = name; self.kind = kind; self.localURL = localURL; self.sourceSize = sourceSize
    }

    public var statusLabel: String {
        if let error { return error }
        guard attachment != nil else {
            return preparing ? "Compressing…" : "Uploading… \(Int((progress * 100).rounded()))%"
        }
        if let storedSize, storedSize < sourceSize {
            return "\(AttachmentPolicy.formatBytes(sourceSize)) → \(AttachmentPolicy.formatBytes(storedSize))"
        }
        return AttachmentPolicy.formatBytes(storedSize ?? sourceSize)
    }
}
