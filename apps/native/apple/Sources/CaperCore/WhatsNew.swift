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
