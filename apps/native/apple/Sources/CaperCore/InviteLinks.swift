import Foundation

/// Who made an invite link, as the invite endpoints describe them.
public struct InvitePerson: Codable, Equatable, Sendable {
    public let username: String
    public let displayName: String
    public var avatarId: Int?
}

/// `GET /api/invites/{code}`: what anyone holding a live link may see.
public struct InvitePreview: Codable, Equatable, Sendable {
    public struct SpaceName: Codable, Equatable, Sendable { public let name: String }
    public struct ChannelName: Codable, Equatable, Sendable { public let name: String }
    public let code: String
    public let space: SpaceName
    public let channel: ChannelName?
    public let inviter: InvitePerson
    public let memberCount: Int
    public let expiresAt: String?
}

/// `POST /api/invites/{code}/join`. `joined` is false when already a member.
public struct InviteJoin: Codable, Equatable, Sendable {
    public let space: Space
    public let channelId: String?
    public let joined: Bool
}

/// An owner's live invite link (`/api/spaces/{space}/invite-links`).
public struct InviteLink: Codable, Equatable, Identifiable, Sendable {
    public struct LinkChannel: Codable, Equatable, Sendable { public let id: String; public let name: String }
    public let code: String
    public let channel: LinkChannel?
    public let createdBy: InvitePerson
    public let uses: Int
    public let maxUses: Int?
    public let expiresAt: String?
    public let createdAt: String
    public var id: String { code }
}

struct InviteLinksResponse: Decodable { let links: [InviteLink] }

/// One choice in an invite link picker.
public struct InviteOption<Value: Hashable & Sendable>: Hashable, Sendable {
    public let label: String
    public let value: Value
    public init(label: String, value: Value) { self.label = label; self.value = value }
}

/// Invite link parsing, URLs and wording, shared by the owner and join dialogs.
public enum InviteLinks {
    public static let codeLength = 10
    static let invalid = "This invite link is invalid or has expired."

    /// "Expire after": the API's `maxAge` seconds, or nil for Never.
    public static let expiryOptions: [InviteOption<Int?>] = [
        InviteOption(label: "30 minutes", value: 1_800), InviteOption(label: "1 hour", value: 3_600),
        InviteOption(label: "6 hours", value: 21_600), InviteOption(label: "12 hours", value: 43_200),
        InviteOption(label: "1 day", value: 86_400), InviteOption(label: "7 days", value: 604_800),
        InviteOption(label: "Never", value: nil),
    ]
    public static let defaultExpiry: Int? = 604_800

    /// "Max uses": the API's `maxUses`, or nil for No limit.
    public static let useOptions: [InviteOption<Int?>] = [
        InviteOption(label: "No limit", value: nil), InviteOption(label: "1 use", value: 1),
        InviteOption(label: "5 uses", value: 5), InviteOption(label: "10 uses", value: 10),
        InviteOption(label: "25 uses", value: 25), InviteOption(label: "50 uses", value: 50),
        InviteOption(label: "100 uses", value: 100),
    ]

    public static func isCode(_ value: Substring) -> Bool {
        value.count == codeLength && value.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber) }
    }

    /// The code in what a person pasted: a full link on any host
    /// (`https://caper.chat/invite/<code>`, optionally with a trailing slash,
    /// query or fragment), the same without a scheme, or the bare code.
    public static func code(from input: String) -> String? {
        var rest = Substring(input.trimmingCharacters(in: .whitespacesAndNewlines))
        if isCode(rest) { return String(rest) }
        if let end = rest.firstIndex(where: { $0 == "?" || $0 == "#" }) { rest = rest[..<end] }
        if let separator = rest.range(of: "://") {
            let scheme = rest[..<separator.lowerBound].lowercased()
            guard scheme == "https" || scheme == "http" else { return nil }
            rest = rest[separator.upperBound...]
        }
        guard let slash = rest.firstIndex(of: "/") else { return nil }
        let host = rest[..<slash]
        guard !host.isEmpty, !host.contains(where: { $0.isWhitespace }) else { return nil }
        var segments = rest[rest.index(after: slash)...].split(separator: "/", omittingEmptySubsequences: false)
        if segments.count == 3, segments[2].isEmpty { segments.removeLast() }
        guard segments.count == 2, segments[0] == "invite", isCode(segments[1]) else { return nil }
        return String(segments[1])
    }

    /// `<origin of the API base>/invite/<code>`, e.g. https://caper.chat/invite/<code>.
    public static func url(code: String, baseURL: URL) -> String {
        var components = URLComponents()
        components.scheme = baseURL.scheme
        components.host = baseURL.host
        components.port = baseURL.port
        components.path = "/invite/\(code)"
        return components.string ?? "https://caper.chat/invite/\(code)"
    }

    public static func memberCount(_ count: Int) -> String { "\(count) \(count == 1 ? "member" : "members")" }

    /// "#feedback · 3 of 10 uses · Expires in 6 days" or "2 uses · Never expires".
    public static func meta(for link: InviteLink, now: Date = Date()) -> String {
        var parts: [String] = []
        if let channel = link.channel { parts.append("#\(channel.name)") }
        if let maxUses = link.maxUses { parts.append("\(link.uses) of \(maxUses) \(maxUses == 1 ? "use" : "uses")") }
        else { parts.append("\(link.uses) \(link.uses == 1 ? "use" : "uses")") }
        parts.append(expiry(link.expiresAt, now: now))
        return parts.joined(separator: " · ")
    }

    /// "Never expires", "Expires in 45 minutes", "Expires in 6 hours", "Expires in 7 days".
    public static func expiry(_ expiresAt: String?, now: Date = Date()) -> String {
        guard let expiresAt else { return "Never expires" }
        guard let date = date(expiresAt) else { return "Expires soon" }
        let seconds = date.timeIntervalSince(now)
        guard seconds > 0 else { return "Expired" }
        let minutes = Int((seconds / 60).rounded())
        if minutes < 60 { return "Expires in \(plural(max(1, minutes), "minute"))" }
        let hours = Int((seconds / 3_600).rounded())
        if hours < 24 { return "Expires in \(plural(hours, "hour"))" }
        return "Expires in \(plural(Int((seconds / 86_400).rounded()), "day"))"
    }

    private static func plural(_ count: Int, _ unit: String) -> String { "\(count) \(unit)\(count == 1 ? "" : "s")" }

    /// RFC 3339 with or without (any number of) fractional digits.
    static func date(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value) { return date }
        guard let dot = value.firstIndex(of: ".") else { return nil }
        let tail = value[value.index(after: dot)...]
        let digits = tail.prefix { $0.isASCII && $0.isNumber }.count
        return ISO8601DateFormatter().date(from: String(value[..<dot]) + String(tail.dropFirst(digits)))
    }
}
