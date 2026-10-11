import Foundation

/// One `@feedback` (or `@caper`) report from `GET /api/feedback`, for the
/// feedback team's inbox. The message stays where it was written; opening a
/// report visits that conversation at `messageId`, in the thread `threadRootId`.
public struct FeedbackReport: Decodable, Equatable, Identifiable, Sendable {
    public struct Reporter: Decodable, Equatable, Sendable {
        public let id: String
        public let username: String
        public let displayName: String
        public var avatarId: Int? = nil

        public init(id: String, username: String, displayName: String, avatarId: Int? = nil) {
            self.id = id; self.username = username; self.displayName = displayName; self.avatarId = avatarId
        }
    }

    /// Where it was written: a space channel, a DM ("Alice & Bob") or personal
    /// notes ("Alice"); `direct` for the last two.
    public struct Conversation: Decodable, Equatable, Sendable {
        public let id: String
        public let name: String
        public let direct: Bool

        public init(id: String, name: String, direct: Bool) { self.id = id; self.name = name; self.direct = direct }
    }

    public struct SpaceReference: Decodable, Equatable, Sendable {
        public let id: String
        public let name: String

        public init(id: String, name: String) { self.id = id; self.name = name }
    }

    /// The report's stable key, the same as `messageId`.
    public let id: String
    /// The message that said `@feedback`.
    public let messageId: String
    /// Where replies go: the message itself when it was top-level, else its thread root.
    public let threadRootId: String
    public let createdAt: String
    /// At most 280 characters; may be empty.
    public let text: String
    /// Files on the message.
    public let attachments: Int
    public let reporter: Reporter
    public let channel: Conversation
    /// nil for DMs and personal notes.
    public let space: SpaceReference?
    public let replyCount: Int
    /// Someone on the team replied in the thread after the report.
    public let answered: Bool

    public init(id: String, messageId: String, threadRootId: String, createdAt: String, text: String, attachments: Int = 0,
                reporter: Reporter, channel: Conversation, space: SpaceReference?, replyCount: Int = 0, answered: Bool = false) {
        self.id = id; self.messageId = messageId; self.threadRootId = threadRootId; self.createdAt = createdAt
        self.text = text; self.attachments = attachments; self.reporter = reporter; self.channel = channel
        self.space = space; self.replyCount = replyCount; self.answered = answered
    }

    private enum CodingKeys: String, CodingKey {
        case id, messageId, threadRootId, createdAt, text, attachments, reporter, channel, space, replyCount, answered
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        messageId = try values.decode(String.self, forKey: .messageId)
        id = (try? values.decodeIfPresent(String.self, forKey: .id)) ?? messageId
        threadRootId = (try? values.decodeIfPresent(String.self, forKey: .threadRootId)) ?? messageId
        createdAt = (try? values.decodeIfPresent(String.self, forKey: .createdAt)) ?? ""
        text = (try? values.decodeIfPresent(String.self, forKey: .text)) ?? ""
        attachments = max(0, (try? values.decodeIfPresent(Int.self, forKey: .attachments)) ?? 0)
        reporter = try values.decode(Reporter.self, forKey: .reporter)
        channel = try values.decode(Conversation.self, forKey: .channel)
        // An empty or malformed space is the same as none (a DM).
        let decodedSpace = try? values.decodeIfPresent(SpaceReference.self, forKey: .space)
        space = decodedSpace.flatMap { $0.id.isEmpty ? nil : $0 }
        replyCount = max(0, (try? values.decodeIfPresent(Int.self, forKey: .replyCount)) ?? 0)
        answered = (try? values.decodeIfPresent(Bool.self, forKey: .answered)) ?? false
    }

    /// What opening this report visits.
    public var target: FeedbackTarget {
        FeedbackTarget(channelID: channel.id, spaceID: space?.id, messageID: messageId, threadRootID: threadRootId, reporter: reporter)
    }
}

/// `GET /api/feedback`: newest first, 50 at a time. Pass `cursor` as
/// `?before=` for the next page; nil at the end.
public struct FeedbackReportsPage: Decodable, Sendable {
    public let reports: [FeedbackReport]
    public let cursor: String?

    private enum CodingKeys: String, CodingKey { case reports, cursor }

    public init(reports: [FeedbackReport], cursor: String?) { self.reports = reports; self.cursor = cursor }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        // A malformed report is skipped, never failing the page.
        let entries = try values.decode([LossyReport].self, forKey: .reports)
        reports = entries.compactMap(\.value)
        let next = try? values.decodeIfPresent(String.self, forKey: .cursor)
        cursor = next.flatMap { $0.isEmpty ? nil : $0 }
    }

    private struct LossyReport: Decodable {
        let value: FeedbackReport?
        init(from decoder: Decoder) throws { value = try? FeedbackReport(from: decoder) }
    }
}

/// A conversation to visit as the feedback team: from an inbox row or a
/// `feedback` notification.
public struct FeedbackTarget: Equatable, Sendable {
    public let channelID: String
    /// The space for a space channel; nil (or empty) for a DM.
    public let spaceID: String?
    /// The reported message.
    public let messageID: String
    /// Its thread root; nil when the message is top-level (it is its own root).
    public let threadRootID: String?
    /// The person who asked, when known (inbox rows), for `@` suggestions.
    public var reporter: FeedbackReport.Reporter? = nil

    public init(channelID: String, spaceID: String?, messageID: String, threadRootID: String?, reporter: FeedbackReport.Reporter? = nil) {
        self.channelID = channelID
        self.spaceID = spaceID.flatMap { $0.isEmpty ? nil : $0 }
        self.messageID = messageID
        self.threadRootID = threadRootID.flatMap { $0.isEmpty ? nil : $0 }
        self.reporter = reporter
    }

    /// The thread the visit opens, so replies land there by default.
    public var rootID: String { threadRootID ?? messageID }
    /// The reported message is a reply inside `rootID`'s thread.
    public var isReply: Bool { rootID != messageID }
}

/// An open visit: a conversation the feedback team reads and replies in
/// without joining. It is in neither the space nor the DM lists.
public struct FeedbackVisit: Equatable, Sendable {
    public let channelID: String
    /// Empty for a DM.
    public let spaceID: String
    public let spaceName: String
    public let channelName: String
    public let direct: Bool
    /// The report it was opened for (its message ID), to mark the inbox row.
    public let reportID: String
    public let reporter: FeedbackReport.Reporter?

    public init(channelID: String, spaceID: String, spaceName: String, channelName: String, direct: Bool,
                reportID: String, reporter: FeedbackReport.Reporter? = nil) {
        self.channelID = channelID; self.spaceID = spaceID; self.spaceName = spaceName
        self.channelName = channelName; self.direct = direct; self.reportID = reportID; self.reporter = reporter
    }

    /// The header: "#plans · Studio" for a channel, "Alice & Bob" for a DM.
    public var title: String { FeedbackInbox.title(channelName: channelName, spaceName: spaceName, direct: direct) }
}

/// Feedback inbox and visit copy and presentation shared by iOS and macOS.
public enum FeedbackInbox {
    public static let title = "Feedback"
    public static let subtitle = "Conversations where someone asked the Caper team. Replies post there, for everyone in it."
    public static let empty = "No feedback yet."
    public static let loadFailed = "Couldn’t load feedback."
    public static let banner = "Visiting as the Caper team — everyone here sees your replies."
    /// Under a message that said `@feedback` or `@caper`.
    public static let notified = "Caper team notified"
    public static let answered = "Answered"
    public static let waiting = "Waiting"

    public static func title(channelName: String, spaceName: String, direct: Bool) -> String {
        direct ? channelName : "#\(channelName.lowercased()) · \(spaceName)"
    }

    /// "#general · Studio" or "DM · Alice & Bob".
    public static func location(_ report: FeedbackReport) -> String {
        report.channel.direct || report.space == nil
            ? "DM · \(report.channel.name)"
            : "#\(report.channel.name.lowercased()) · \(report.space?.name ?? "")"
    }

    /// The text on one line, or "Sent N files" when it has none.
    public static func preview(_ report: FeedbackReport) -> String {
        let text = report.text.split(whereSeparator: \.isNewline).joined(separator: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.isEmpty { return text }
        guard report.attachments > 0 else { return "" }
        return "Sent \(report.attachments) \(report.attachments == 1 ? "file" : "files")"
    }

    public static func status(_ report: FeedbackReport) -> String { report.answered ? answered : waiting }

    /// Waiting reports among those loaded, for the navigation row's count.
    public static func waitingCount(_ reports: [FeedbackReport]) -> Int { reports.filter { !$0.answered }.count }

    /// "now", "5m", "3h", "2d", then a short date.
    public static func relativeTime(_ createdAt: String, now: Date = Date(), calendar: Calendar = .current,
                                    locale: Locale = .current) -> String {
        guard let date = parseDate(createdAt) else { return "" }
        let seconds = max(0, now.timeIntervalSince(date))
        if seconds < 60 { return "now" }
        if seconds < 3_600 { return "\(Int(seconds / 60))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3_600))h" }
        if seconds < 7 * 86_400 { return "\(Int(seconds / 86_400))d" }
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = calendar.timeZone
        let sameYear = calendar.component(.year, from: date) == calendar.component(.year, from: now)
        formatter.setLocalizedDateFormatFromTemplate(sameYear ? "MMMd" : "yMMMd")
        return formatter.string(from: date)
    }

    /// RFC 3339, with or without fractional seconds (Postgres sends microseconds).
    static func parseDate(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value) { return date }
        // Drop the fraction and keep whole seconds.
        guard let dot = value.firstIndex(of: "."), let end = value[dot...].dropFirst().firstIndex(where: { !$0.isNumber }) else { return nil }
        return ISO8601DateFormatter().date(from: String(value[..<dot]) + String(value[end...]))
    }

    /// The row's VoiceOver label.
    public static func accessibilityLabel(_ report: FeedbackReport, now: Date = Date()) -> String {
        [report.reporter.displayName, location(report), preview(report), relativeTime(report.createdAt, now: now), status(report)]
            .filter { !$0.isEmpty }.joined(separator: ", ")
    }

    /// Appends a later page, skipping reports already listed.
    public static func merging(_ existing: [FeedbackReport], _ page: [FeedbackReport]) -> [FeedbackReport] {
        var seen = Set(existing.map(\.id))
        return existing + page.filter { seen.insert($0.id).inserted }
    }
}
