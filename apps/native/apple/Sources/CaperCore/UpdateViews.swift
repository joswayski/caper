import SwiftUI

#if os(macOS)
import AppKit

/// The updater's review payload, shared directly with the window presentation.
public struct DesktopUpdate: Equatable, Sendable {
    public struct ChangelogEntry: Decodable, Equatable, Sendable {
        public let version: String
        public let notes: String
        public init(version: String, notes: String) { self.version = version; self.notes = notes }
    }
    public struct Highlight: Decodable, Equatable, Sendable {
        public let title: String
        public let body: String
        public init(title: String, body: String) { self.title = title; self.body = body }
    }
    public let version: String
    public let notes: String
    public let changelog: [ChangelogEntry]
    public let historyComplete: Bool
    public let canApply: Bool
    public let lastError: String?
    /// Every release being installed, stacked newest first. `nil` when the
    /// highlights could not be read, so the plain notes explain why.
    public let highlights: [Highlight]?
    /// The newest highlighted release's date (YYYY-MM-DD).
    public let released: String?
    /// The update's source commit, linked from its version.
    public let revision: String?
    /// The update's highlights sequence, marked read when it is installed.
    public let sequence: Int64?
    /// Bytes the update downloads: a patch from this build when the release
    /// has one, else the full archive. `nil` from updaters that predate it.
    public let downloadSize: Int64?
    /// Whether `downloadSize` is a patch (only what changed).
    public let patch: Bool

    public init(version: String, notes: String, changelog: [ChangelogEntry],
                historyComplete: Bool, canApply: Bool, lastError: String?,
                highlights: [Highlight]? = nil, released: String? = nil,
                revision: String? = nil, sequence: Int64? = nil,
                downloadSize: Int64? = nil, patch: Bool = false) {
        self.version = version; self.notes = notes; self.changelog = changelog
        self.historyComplete = historyComplete; self.canApply = canApply; self.lastError = lastError
        self.highlights = highlights; self.released = released
        self.revision = revision; self.sequence = sequence
        self.downloadSize = downloadSize; self.patch = patch
    }

    /// "1.5 MB download, only what changed" or "129.8 MB download", matching
    /// the Windows and Linux review.
    public var downloadLabel: String? {
        guard let downloadSize else { return nil }
        let megabytes = Double(downloadSize) / 1_000_000
        let amount = megabytes < 0.1 ? "Under 0.1 MB" : String(format: "%.1f MB", megabytes)
        return patch ? "\(amount) download, only what changed" : "\(amount) download"
    }

    /// This version's commit on GitHub, only for a full source SHA.
    public var commitURL: URL? {
        guard let revision, revision.count == 40,
              revision.allSatisfy({ $0.isHexDigit && !$0.isUppercase }) else { return nil }
        return URL(string: "https://github.com/joswayski/caper/commit/\(revision)")
    }

    /// People know when they updated, not which build: "October 18, 2026".
    public var releasedLabel: String? {
        guard let released else { return nil }
        let parser = DateFormatter()
        parser.locale = Locale(identifier: "en_US_POSIX")
        parser.timeZone = TimeZone(identifier: "UTC")
        parser.dateFormat = "yyyy-MM-dd"
        guard let date = parser.date(from: released) else { return nil }
        let formatter = DateFormatter()
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateStyle = .long
        formatter.timeStyle = .none
        return formatter.string(from: date)
    }

    public var entries: [ChangelogEntry] {
        // The signed manifest already orders history by release build, not version text.
        changelog.isEmpty ? [ChangelogEntry(version: version, notes: notes)] : changelog
    }

    public var historyNotice: String {
        historyComplete ? "This update includes all of the following changes."
            : "Recorded changes are shown below. Earlier release notes aren’t available."
    }
}
#endif
