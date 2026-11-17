import Foundation

public struct ReleaseHighlight: Codable, Equatable, Sendable {
    public let title: String
    public let body: String
}

public struct PlatformRelease: Codable, Equatable, Sendable {
    public let sequence: Int64
    public let revision: String
    public let publishedAt: String
    public let highlights: [ReleaseHighlight]
    public let version: String?

    public init(sequence: Int64, revision: String, publishedAt: String, highlights: [ReleaseHighlight], version: String? = nil) {
        self.sequence = sequence; self.revision = revision; self.publishedAt = publishedAt
        self.highlights = highlights; self.version = version
    }
}

public struct WhatsNewSnapshot: Codable, Equatable, Sendable {
    public let schema: Int
    public let platform: String
    public let revision: String
    public let sequence: Int64
    public let releases: [PlatformRelease]

    public static let maximumBytes = 64 * 1024
    static let maximumSequence: Int64 = 9_007_199_254_740_991
    public static func validRevision(_ value: String) -> Bool {
        value.utf8.count == 40 && value.range(of: "^[0-9a-f]{40}$", options: .regularExpression) != nil
    }

    public func validated(platform expectedPlatform: String, revision expectedRevision: String) throws -> Self {
        func require(_ condition: Bool) throws {
            if !condition { throw APIError(status: 502, message: "Caper returned invalid release notes.") }
        }
        try require(schema == 1 && platform == expectedPlatform && ["android", "ios", "macos"].contains(platform))
        try require(Self.validRevision(revision) && revision == expectedRevision && (1...Self.maximumSequence).contains(sequence))
        try require((1...30).contains(releases.count))
        let dates = ISO8601DateFormatter()
        let fractionalDates = ISO8601DateFormatter()
        fractionalDates.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        var previousSequence: Int64 = 0
        var previousDate: Date?
        var revisions = Set<String>()
        for release in releases {
            try require((1...Self.maximumSequence).contains(release.sequence) && release.sequence > previousSequence)
            try require(Self.validRevision(release.revision) && revisions.insert(release.revision).inserted)
            try require(release.version.map { Self.validText($0, limit: 64) } ?? true)
            try require(release.publishedAt.utf8.count <= 40 && release.publishedAt.hasSuffix("Z"))
            guard let date = dates.date(from: release.publishedAt) ?? fractionalDates.date(from: release.publishedAt) else {
                throw APIError(status: 502, message: "Caper returned invalid release dates.")
            }
            try require(previousDate.map { date >= $0 } ?? true)
            try require(release.highlights.count <= 5)
            for highlight in release.highlights {
                try require(Self.validText(highlight.title, limit: 100) && Self.validText(highlight.body, limit: 500))
            }
            previousSequence = release.sequence
            previousDate = date
        }
        try require(releases.last?.sequence == sequence && releases.last?.revision == revision)
        return self
    }

    private static func validText(_ value: String, limit: Int) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && value.unicodeScalars.count <= limit &&
            !value.unicodeScalars.contains { ($0.value < 32 || (127...159).contains($0.value)) && $0 != "\n" && $0 != "\t" }
    }
}

/// Per installation, origin and platform; logging out never resets this watermark.
public final class WhatsNewHistory {
    private let read: () -> Int64?
    private let write: (Int64) -> Void
    public private(set) var snapshot: WhatsNewSnapshot?
    public init(read: @escaping () -> Int64?, write: @escaping (Int64) -> Void) {
        self.read = read; self.write = write
    }
    public var unread: [PlatformRelease] {
        guard let ack = read() else { return [] }
        return (snapshot?.releases ?? []).filter { $0.sequence > ack && !$0.highlights.isEmpty }.reversed()
    }
    public var retained: [PlatformRelease] { (snapshot?.releases ?? []).filter { !$0.highlights.isEmpty }.reversed() }
    public func receive(_ value: WhatsNewSnapshot, platform: String, revision: String) throws {
        snapshot = try value.validated(platform: platform, revision: revision)
        if read() == nil || unread.isEmpty { acknowledge(value.sequence) }
    }
    public func acknowledge(_ sequence: Int64) {
        precondition((1...WhatsNewSnapshot.maximumSequence).contains(sequence))
        write(max(read() ?? 0, sequence))
    }
}

/// The installed version, as every client shows and copies it: "Version
/// 0.1.654" on screen, "Caper 0.1.654 for macOS (5c00469)" when copied.
/// Builds without a release revision are development builds.
public struct InstalledVersion: Equatable, Sendable {
    public let version: String?
    public let revision: String?
    public let platform: String

    public init(version: String?, revision: String?, platform: String) {
        self.version = version; self.platform = platform
        self.revision = revision.flatMap { WhatsNewSnapshot.validRevision($0) ? $0 : nil }
    }

    public static var current: InstalledVersion {
        #if os(iOS)
        let platform = "iOS"
        #else
        let platform = "macOS"
        #endif
        return InstalledVersion(version: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
                                revision: Bundle.main.object(forInfoDictionaryKey: "CaperSourceRevision") as? String,
                                platform: platform)
    }

    /// Only release builds, which carry a full source revision, can be copied.
    public var copyText: String? {
        guard let version, let revision else { return nil }
        return "Caper \(version) for \(platform) (\(revision.prefix(7)))"
    }
    public var label: String { copyText == nil ? "Development build" : "Version \(version ?? "")" }
    public var accessibilityLabel: String { copyText == nil ? label : "\(label), copy" }
    public static let copied = "Copied"
    public static let hint = "Click to copy the version"
    /// How long "Copied" replaces the label.
    public static let feedbackSeconds: TimeInterval = 2
}
