import Foundation

public enum CaperAvatar {
    public static func index(for avatarID: Int?) -> Int? { avatarID.flatMap { (0...799).contains($0) ? $0 : nil } }
}

public struct Account: Codable, Equatable, Sendable {
    public let id: String
    public var username: String?
    public var displayName: String?
    public var debugEnabled: Bool?
    public var avatarId: Int? = nil
}

public struct Inviter: Codable, Equatable, Sendable {
    public let username: String
    public let displayName: String
}

public struct Space: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let ownerId: String
    public var demo: Bool?
    public var inviter: Inviter?
}

public struct Channel: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let spaceId: String
    public let name: String
    public let `private`: Bool
    public let joined: Bool

    private enum CodingKeys: String, CodingKey { case id, spaceId, name, `private`, joined }
    public init(id: String, spaceId: String, name: String, private: Bool, joined: Bool = true) {
        self.id = id; self.spaceId = spaceId; self.name = name; self.private = `private`; self.joined = joined
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        spaceId = try values.decode(String.self, forKey: .spaceId)
        name = try values.decode(String.self, forKey: .name)
        `private` = try values.decode(Bool.self, forKey: .private)
        joined = try values.decodeIfPresent(Bool.self, forKey: .joined) ?? true
    }
}

public struct ChannelInvitation: Codable, Equatable, Identifiable, Sendable {
    public var id: String { channel.id }
    public let channel: Channel
    public let inviter: Inviter
}

public struct DirectMessagePeer: Codable, Equatable, Sendable {
    public let id: String
    public let username: String
    public let displayName: String
    public var avatarId: Int? = nil
}

public struct DirectMessageConversation: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let peer: DirectMessagePeer
    public let lastSeq: String
    public let readSeq: String
    /// Missing on older servers: `accepted`.
    public let status: DirectMessageStatus
    /// You blocked the peer (they are never told). Missing means false.
    public let blocked: Bool

    /// Incoming requests never count as unread.
    public var unread: Bool { status != .incoming && (try? Sequence.compare(lastSeq, readSeq)) == .orderedDescending }

    private enum CodingKeys: String, CodingKey { case id, peer, lastSeq, readSeq, status, blocked }
    public init(id: String, peer: DirectMessagePeer, lastSeq: String, readSeq: String,
                status: DirectMessageStatus = .accepted, blocked: Bool = false) {
        self.id = id; self.peer = peer; self.lastSeq = lastSeq; self.readSeq = readSeq
        self.status = status; self.blocked = blocked
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        peer = try values.decode(DirectMessagePeer.self, forKey: .peer)
        lastSeq = try values.decode(String.self, forKey: .lastSeq)
        readSeq = try values.decode(String.self, forKey: .readSeq)
        // An unknown future status is treated like an older server's omission.
        status = (try? values.decodeIfPresent(DirectMessageStatus.self, forKey: .status)) ?? .accepted
        blocked = (try? values.decodeIfPresent(Bool.self, forKey: .blocked)) ?? false
    }

    func with(readSeq: String) -> DirectMessageConversation {
        DirectMessageConversation(id: id, peer: peer, lastSeq: lastSeq, readSeq: readSeq, status: status, blocked: blocked)
    }
    func with(blocked: Bool) -> DirectMessageConversation {
        DirectMessageConversation(id: id, peer: peer, lastSeq: lastSeq, readSeq: readSeq, status: status, blocked: blocked)
    }
}

public struct DirectMessagesResponse: Codable, Sendable {
    public let conversations: [DirectMessageConversation]
}

/// An account that shares an active space or a DM with you, from
/// `GET /api/people` (you excluded, ordered by username, at most 500).
public struct Person: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String
    public let displayName: String
    public var avatarId: Int? = nil
}

public struct PeopleResponse: Codable, Sendable {
    public let people: [Person]
}

public struct PushConfiguration: Codable, Equatable, Sendable {
    public let platforms: [String]
}

public struct Member: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String
    public let displayName: String
    public let owner: Bool
    public var avatarId: Int? = nil
}

public struct SpaceLimits: Codable, Equatable, Sendable {
    public let ownedSpaces: Int
    public let totalSpaces: Int
    public let channelsPerSpace: Int
}

public struct SpacesResponse: Codable, Sendable {
    public let spaces: [Space]
    public let invitations: [Space]
    public let limits: SpaceLimits

    private enum CodingKeys: String, CodingKey { case spaces, invitations, limits }
    public init(spaces: [Space], invitations: [Space] = [], limits: SpaceLimits) {
        self.spaces = spaces; self.invitations = invitations; self.limits = limits
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        spaces = try values.decode([Space].self, forKey: .spaces)
        invitations = try values.decodeIfPresent([Space].self, forKey: .invitations) ?? []
        limits = try values.decode(SpaceLimits.self, forKey: .limits)
    }
}

public struct SpaceDetail: Codable, Sendable {
    public let space: Space
    public let channels: [Channel]
    public let members: [Member]
    public let channelInvitations: [ChannelInvitation]

    private enum CodingKeys: String, CodingKey { case space, channels, members, channelInvitations }
    public init(space: Space, channels: [Channel], members: [Member], channelInvitations: [ChannelInvitation] = []) {
        self.space = space; self.channels = channels; self.members = members; self.channelInvitations = channelInvitations
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        space = try values.decode(Space.self, forKey: .space)
        channels = try values.decode([Channel].self, forKey: .channels)
        members = try values.decode([Member].self, forKey: .members)
        channelInvitations = try values.decodeIfPresent([ChannelInvitation].self, forKey: .channelInvitations) ?? []
    }
}

public enum PresenceStatus: String, Codable, Sendable { case online, idle, offline, unknown }

public struct PresenceMember: Codable, Equatable, Sendable {
    public let userId: String
    public let status: PresenceStatus
}

public enum WorkspaceValidation {
    public static func normalizeUsername(_ value: String) -> String {
        String(value.lowercased().filter { $0.isASCII && ($0.isLowercase || $0.isNumber || $0 == "_") }.prefix(32))
    }

    public static func usernameError(_ value: String) -> String? {
        (3...32).contains(value.utf8.count) && value.utf8.allSatisfy({
            (97...122).contains($0) || (48...57).contains($0) || $0 == 95
        }) ? nil : "Username must be 3–32 lowercase letters, numbers, or underscores."
    }

    public static func spaceNameError(_ value: String) -> String? {
        let name = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.isEmpty { return "Enter a space name." }
        if name.unicodeScalars.count > 80 { return "Space names can be at most 80 characters." }
        if name.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) {
            return "Space names cannot contain control characters."
        }
        return nil
    }

    public static func normalizeChannelName(_ value: String) -> String {
        var normalized = value.lowercased().replacingOccurrences(of: " ", with: "-")
        normalized = String(normalized.filter { $0.isASCII && ($0.isLowercase || $0 == "-") }.prefix(80))
        while normalized.contains("--") { normalized = normalized.replacingOccurrences(of: "--", with: "-") }
        while normalized.first == "-" { normalized.removeFirst() }
        return normalized
    }

    public static func channelNameError(_ value: String) -> String? {
        guard !value.isEmpty else { return "Enter a channel name." }
        guard value.unicodeScalars.count <= 80 else { return "Channel names can be at most 80 characters." }
        let parts = value.split(separator: "-", omittingEmptySubsequences: false)
        guard !parts.contains(where: { $0.isEmpty }), parts.allSatisfy({ $0.allSatisfy { $0.isASCII && $0.isLowercase } }) else {
            return "Use lowercase letters separated by single dashes."
        }
        return nil
    }
}

public struct ChatAuthor: Codable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let isGuest: Bool
    public var avatarId: Int? = nil
}

/// One `content.mentions` entry, as resolved by the server on send.
public enum MessageMention: Codable, Equatable, Hashable, Sendable {
    case user(id: String, username: String)
    case everyone
    case here

    private enum Keys: String, CodingKey { case type, id, username }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        let type = try values.decode(String.self, forKey: .type)
        switch type {
        case "user": self = .user(id: try values.decode(String.self, forKey: .id), username: try values.decode(String.self, forKey: .username))
        case "everyone": self = .everyone
        case "here": self = .here
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: values, debugDescription: "Unknown mention type \(type)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        switch self {
        case let .user(id, username):
            try values.encode("user", forKey: .type)
            try values.encode(id, forKey: .id)
            try values.encode(username, forKey: .username)
        case .everyone: try values.encode("everyone", forKey: .type)
        case .here: try values.encode("here", forKey: .type)
        }
    }
}

/// Decodes one `mentions` element without failing the message: entries of an
/// unknown type (from a newer server) or a malformed shape become nil.
private struct LenientMessageMention: Decodable {
    let value: MessageMention?
    init(from decoder: Decoder) throws { value = try? MessageMention(from: decoder) }
}

public struct ChatContent: Codable, Equatable, Sendable {
    public let version: Int
    public let type: String
    public let text: String
    /// `content.mentions`, in first-appearance order. Optional on the wire:
    /// older messages and servers omit it, so it decodes as empty.
    public let mentions: [MessageMention]

    private enum CodingKeys: String, CodingKey { case version, type, text, mentions }
    public init(version: Int, type: String, text: String, mentions: [MessageMention] = []) {
        self.version = version; self.type = type; self.text = text; self.mentions = mentions
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        version = try values.decode(Int.self, forKey: .version)
        type = try values.decode(String.self, forKey: .type)
        text = try values.decode(String.self, forKey: .text)
        // Mentions are decoration: a malformed list must not drop the message.
        let entries = try? values.decodeIfPresent([LenientMessageMention].self, forKey: .mentions)
        mentions = entries?.compactMap { $0.value } ?? []
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(version, forKey: .version)
        try values.encode(type, forKey: .type)
        try values.encode(text, forKey: .text)
        if !mentions.isEmpty { try values.encode(mentions, forKey: .mentions) }
    }
}

public struct MessageVersion: Codable, Equatable, Identifiable, Sendable {
    public let revision: Int
    public let content: ChatContent
    public let createdAt: String
    public var id: Int { revision }
}

public struct MessageVersions: Codable, Sendable {
    public let messageId: String
    public let versions: [MessageVersion]
    public let hasMore: Bool

    func isValid(messageID: String, before: Int?) -> Bool {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return messageId == messageID && versions.count <= 50 && versions.enumerated().allSatisfy { index, version in
            version.revision > 0 && (before == nil || version.revision < before!)
                && (index == 0 || version.revision < versions[index - 1].revision)
                && version.content.version == 1 && version.content.type == "text"
                && MessageValidation.error(for: version.content.text) == nil
                && (fractional.date(from: version.createdAt) != nil || ISO8601DateFormatter().date(from: version.createdAt) != nil)
        }
    }
}

struct MessageDiffToken: Equatable {
    let text: String
    let changed: Bool
}

/// A word-level Myers diff preserving whitespace, Unicode and separate edits.
func messageDiff(before: String, after: String) -> ([MessageDiffToken], [MessageDiffToken]) {
    let pattern = try! NSRegularExpression(pattern: "\\s+|[\\p{L}\\p{N}_]+|[^\\s\\p{L}\\p{N}_]+")
    func tokens(_ text: String) -> [String] {
        pattern.matches(in: text, range: NSRange(text.startIndex..., in: text)).map { (text as NSString).substring(with: $0.range) }
    }
    let old = tokens(before), new = tokens(after)
    var removed = Set<Int>(), added = Set<Int>()
    for change in new.difference(from: old) {
        switch change {
        case .remove(let offset, _, _): removed.insert(offset)
        case .insert(let offset, _, _): added.insert(offset)
        }
    }
    return (old.enumerated().map { MessageDiffToken(text: $0.element, changed: removed.contains($0.offset)) },
            new.enumerated().map { MessageDiffToken(text: $0.element, changed: added.contains($0.offset)) })
}

public struct MessagePin: Codable, Equatable, Sendable {
    public let author: ChatAuthor
    public let createdAt: String
}

/// Local presentation only. Never seed revision caches or history with these values.
struct MessageMutations {
    struct PinIntent { let message: ChatMessage; let pin: MessagePin? }
    var pins: [String: PinIntent] = [:]
    var edits: [String: (text: String, revision: Int)] = [:]

    func project(_ message: ChatMessage) -> ChatMessage {
        var result = message
        if let intent = pins[message.id] { result.pin = intent.pin }
        if let edit = edits[message.id], (message.revision ?? 1) <= edit.revision {
            result.content = ChatContent(version: 1, type: "text", text: edit.text)
        }
        return result
    }

    func pinned(messages: [ChatMessage], confirmed: [ChatMessage]) -> [ChatMessage] {
        var rows = Dictionary(uniqueKeysWithValues: confirmed.map { ($0.id, $0) })
        for (id, intent) in pins {
            if intent.pin == nil { rows[id] = nil }
            else { rows[id] = messages.first { $0.id == id } ?? rows[id] ?? intent.message }
        }
        return rows.values.map(project).sorted {
            (try? Sequence.compare($0.pinSeq ?? "0", $1.pinSeq ?? "0")) == .orderedDescending
        }
    }
}

public struct ChatMessage: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let channelId: String
    public let seq: String
    public let author: ChatAuthor
    public var content: ChatContent
    public let createdAt: String
    public let clientMessageId: String
    public var reactions: [MessageReaction]? = nil
    public var reactionSeq: String? = nil
    public var pin: MessagePin? = nil
    public var pinSeq: String? = nil
    public var threadRootId: String? = nil
    public var broadcast: Bool? = nil
    public var thread: ThreadSummary? = nil
    public var forward: MessageForward? = nil
    public var forwardSeq: String? = nil
    public var revision: Int? = nil
    public var editedAt: String? = nil
    public var editSeq: String? = nil
    public var isChannelMessage: Bool { threadRootId == nil || broadcast == true }
}

/// Immutable reference breaks the recursive message/forward value layout.
public final class MessageForward: Codable, Equatable, Sendable {
    public let message: ChatMessage?
    public let seq: String
    public init(message: ChatMessage?, seq: String) { self.message = message; self.seq = seq }
    public static func == (lhs: MessageForward, rhs: MessageForward) -> Bool { lhs.seq == rhs.seq && lhs.message == rhs.message }
}
public struct ThreadSummary: Codable, Equatable, Sendable {
    public let replyCount: Int
    public let participants: [ChatAuthor]
    public let seq: String
}
public struct ThreadHistory: Codable, Sendable {
    public let root: ChatMessage
    public let messages: [ChatMessage]
    public let cursor: String
    public let hasMore: Bool
    public let hasNewer: Bool?
}
public struct ForwardDestination: Codable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let spaceName: String
    public let direct: Bool
}
public struct ForwardDestinations: Decodable, Sendable { public let destinations: [ForwardDestination] }
public struct ForwardConversationHistory: Decodable, Sendable {
    public let root: ChatMessage?
    public var messages: [ChatMessage]
    public let cursor: String
    public var hasMore: Bool
}
public struct MessageForwardEvent: Decodable, Sendable {
    public let type: String
    public let schemaVersion: Int
    public let channelId: String
    public let seq: String
    public let message: ChatMessage
    public var isValid: Bool {
        type == "message.forward" && schemaVersion == 1 && channelId == message.channelId
            && message.forward != nil && message.forwardSeq == seq && message.isValidForward
            && (try? Sequence.compare(seq, "0")) != nil
    }
}
extension ChatMessage {
    var isValidForward: Bool {
        guard let forward else { return true }
        guard (try? Sequence.compare(forward.seq, "0")) != nil,
              forward.message?.forward == nil else { return false }
        return forward.message.map { $0.content.version == 1 && $0.content.type == "text" && (try? Sequence.compare($0.seq, "0")) != nil } ?? true
    }
}

public struct MessageReaction: Codable, Equatable, Sendable, Identifiable {
    public let emoji: String
    public let authorIds: [String]
    public var id: String { emoji }
}

/// One person in a who-reacted list. `id` is the public user ID used in
/// snapshot `authorIds`; names may be missing.
public struct ReactorPerson: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String?
    public let displayName: String?
    public let avatarId: Int?

    public init(id: String, username: String? = nil, displayName: String? = nil, avatarId: Int? = nil) {
        self.id = id; self.username = username; self.displayName = displayName; self.avatarId = avatarId
    }

    /// Display name, then username, then "Someone".
    public var name: String {
        if let displayName, !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return displayName }
        if let username, !username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return username }
        return "Someone"
    }
}

public struct ReactorGroup: Codable, Equatable, Sendable {
    public let emoji: String
    /// In reaction order: the first person to react comes first.
    public let authors: [ReactorPerson]

    public init(emoji: String, authors: [ReactorPerson]) { self.emoji = emoji; self.authors = authors }
}

/// `GET /api/chat/channels/{channel}/messages/{message}/reactions`.
public struct ReactorList: Codable, Equatable, Sendable {
    public let messageId: String
    public let reactionSeq: String
    public let reactions: [ReactorGroup]

    public init(messageId: String, reactionSeq: String, reactions: [ReactorGroup]) {
        self.messageId = messageId; self.reactionSeq = reactionSeq; self.reactions = reactions
    }

    func isValid(messageID: String) -> Bool {
        messageId == messageID && (try? Sequence.compare(reactionSeq, "0")) != nil
            && Set(reactions.map(\.emoji)).count == reactions.count
            && reactions.allSatisfy { group in
                !group.emoji.isEmpty && Set(group.authors.map(\.id)).count == group.authors.count
                    && group.authors.allSatisfy { !$0.id.isEmpty }
            }
    }

    /// The people behind `reaction` as the snapshot shows it now, in this
    /// list's reaction order. The list only supplies names: people who have
    /// since removed their reaction are left out, and the viewer's own newer
    /// reaction is added. Returns nil when anyone else is missing, so callers
    /// show the count-only summary and fetch again.
    public func people(for reaction: MessageReaction, viewer: ReactorPerson?) -> [ReactorPerson]? {
        let current = Set(reaction.authorIds)
        var people = (reactions.first { $0.emoji == reaction.emoji }?.authors ?? []).filter { current.contains($0.id) }
        let named = Set(people.map(\.id))
        for id in reaction.authorIds where !named.contains(id) {
            guard let viewer, id == viewer.id else { return nil }
            people.append(reactions.flatMap(\.authors).first(where: { $0.id == id }) ?? viewer)
        }
        return people
    }
}

/// Who reacted, in the wording every Caper client uses.
public enum ReactionSummary {
    /// "You, Alice A, Bob B and 2 others reacted with :thumbs-up:". The
    /// viewer moves to the front as "You"; others keep reaction order.
    public static func text(authors: [ReactorPerson], selfID: String?, emojiName: String?, emoji: String) -> String {
        guard !authors.isEmpty else { return fallback(authorIDs: [], selfID: selfID, emojiName: emojiName, emoji: emoji) }
        var names = authors.map(\.name)
        if let selfID, let index = authors.firstIndex(where: { $0.id == selfID }) {
            names.remove(at: index)
            names.insert("You", at: 0)
        }
        return list(names) + reacted(emojiName: emojiName, emoji: emoji)
    }

    /// Before names load, or when loading fails: only the snapshot's count.
    public static func fallback(authorIDs: [String], selfID: String?, emojiName: String?, emoji: String) -> String {
        let count = authorIDs.count
        let subject: String
        if let selfID, authorIDs == [selfID] { subject = "You" }
        else { subject = "\(count) \(count == 1 ? "person" : "people")" }
        return subject + reacted(emojiName: emojiName, emoji: emoji)
    }

    /// ":thumbs-up:" for a catalog emoji, otherwise the emoji itself.
    public static func emojiLabel(emojiName: String?, emoji: String) -> String {
        guard let emojiName, !emojiName.isEmpty else { return emoji }
        return ":\(emojiName):"
    }

    static func list(_ names: [String]) -> String {
        switch names.count {
        case 0: return ""
        case 1: return names[0]
        case 2: return "\(names[0]) and \(names[1])"
        case 3: return "\(names[0]), \(names[1]) and \(names[2])"
        default:
            let others = names.count - 3
            return "\(names[0]), \(names[1]), \(names[2]) and \(others) \(others == 1 ? "other" : "others")"
        }
    }

    private static func reacted(emojiName: String?, emoji: String) -> String {
        " reacted with \(emojiLabel(emojiName: emojiName, emoji: emoji))"
    }
}

public struct MessageReactionsEvent: Codable, Equatable, Sendable {
    public let type: String
    public let schemaVersion: Int
    public let channelId: String
    public let seq: String
    public let messageId: String
    public let reactions: [MessageReaction]

    public var isValid: Bool {
        type == "message.reactions" && schemaVersion == 1 && !channelId.isEmpty && !messageId.isEmpty
            && (try? Sequence.compare(seq, "0")) != nil
            && Set(reactions.map(\.emoji)).count == reactions.count
            && reactions.allSatisfy {
                !$0.emoji.isEmpty && !$0.authorIds.isEmpty
                    && Set($0.authorIds).count == $0.authorIds.count
                    && $0.authorIds.allSatisfy { !$0.isEmpty }
            }
    }
}

public struct MessagePinEvent: Codable, Equatable, Sendable {
    public let type: String
    public let schemaVersion: Int
    public let channelId: String
    public let seq: String
    public let message: ChatMessage

    public var isValid: Bool {
        type == "message.pin" && schemaVersion == 1 && !channelId.isEmpty
            && message.channelId == channelId && message.pinSeq == seq
            && (try? Sequence.compare(seq, "0")) != nil
            && (try? Sequence.compare(message.seq, "0")) != nil
    }
}

public struct ChatHistory: Codable, Sendable {
    public let space: HistoryIdentity?
    public let channel: HistoryIdentity?
    public let messages: [ChatMessage]
    public let pinnedMessages: [ChatMessage]
    public let cursor: String
    public let hasMore: Bool
    public let hasNewer: Bool

    private enum CodingKeys: String, CodingKey { case space, channel, messages, pinnedMessages, cursor, hasMore, hasNewer }
    public init(space: HistoryIdentity?, channel: HistoryIdentity?, messages: [ChatMessage], pinnedMessages: [ChatMessage] = [], cursor: String, hasMore: Bool, hasNewer: Bool = false) {
        self.space = space; self.channel = channel; self.messages = messages; self.pinnedMessages = pinnedMessages
        self.cursor = cursor; self.hasMore = hasMore; self.hasNewer = hasNewer
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        space = try values.decodeIfPresent(HistoryIdentity.self, forKey: .space)
        channel = try values.decodeIfPresent(HistoryIdentity.self, forKey: .channel)
        messages = try values.decode([ChatMessage].self, forKey: .messages)
        pinnedMessages = try values.decodeIfPresent([ChatMessage].self, forKey: .pinnedMessages) ?? []
        cursor = try values.decode(String.self, forKey: .cursor)
        hasMore = try values.decode(Bool.self, forKey: .hasMore)
        hasNewer = try values.decodeIfPresent(Bool.self, forKey: .hasNewer) ?? false
    }
}

public struct HistoryIdentity: Codable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let direct: Bool?

    public init(id: String, name: String, direct: Bool? = nil) {
        self.id = id; self.name = name; self.direct = direct
    }
}

public struct ChatSession: Codable, Sendable {
    public let token: String
    public let author: ChatAuthor
}

public enum Sequence: Error, Equatable {
    case invalid

    public static func compare(_ lhs: String, _ rhs: String) throws -> ComparisonResult {
        guard valid(lhs), valid(rhs) else { throw Sequence.invalid }
        let a = lhs.drop(while: { $0 == "0" })
        let b = rhs.drop(while: { $0 == "0" })
        if a.count != b.count { return a.count < b.count ? .orderedAscending : .orderedDescending }
        if a == b { return .orderedSame }
        return a.lexicographicallyPrecedes(b) ? .orderedAscending : .orderedDescending
    }

    public static func isSuccessor(_ value: String, of previous: String) -> Bool {
        guard valid(value), valid(previous), let p = UInt64(previous), p < UInt64.max else { return false }
        return value == String(p + 1)
    }

    private static func valid(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.allSatisfy({ $0 >= 48 && $0 <= 57 }) && (value == "0" || value.first != "0")
    }
}

public enum ProfileValidation {
    public static func error(username: String, displayName: String) -> String? {
        let username = username.trimmingCharacters(in: .whitespacesAndNewlines)
        if !(3...32).contains(username.utf8.count) || !username.utf8.allSatisfy({
            (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 95
        }) { return "Username must be 3–32 letters, numbers, or underscores." }
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !(1...64).contains(name.unicodeScalars.count)
            || name.unicodeScalars.contains(where: { $0.properties.generalCategory == .control }) {
            return "Display name must be 1–64 characters without control characters."
        }
        return nil
    }
}

public enum MessageValidation {
    public static func error(for text: String) -> String? {
        if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "Write a message first." }
        if text.unicodeScalars.count > 4_000 { return "Messages can be at most 4,000 characters." }
        if text.unicodeScalars.contains(where: { $0.properties.generalCategory == .control && $0 != "\n" && $0 != "\t" }) {
            return "Messages cannot contain control characters."
        }
        return nil
    }

    public static func acceptsResponse(_ message: ChatMessage, channelID: String, command: PendingMessage, authorID: String) -> Bool {
        message.channelId == channelID
            && message.clientMessageId == command.id
            && message.author.id == authorID
            && message.content.version == 1
            && message.content.type == "text"
            && message.content.text == command.text
            && message.threadRootId == command.threadRootId
            && (message.broadcast ?? false) == command.broadcast
            && (try? Sequence.compare(message.seq, "0")) != nil
    }
}

public struct PendingMessage: Equatable, Sendable {
    public let id: String
    public let text: String
    public let createdAt: String
    public let threadRootId: String?
    public let broadcast: Bool

    public init(id: String, text: String, createdAt: String = ISO8601DateFormatter().string(from: Date()), threadRootId: String? = nil, broadcast: Bool = false) {
        self.id = id
        self.text = text
        self.createdAt = createdAt
        self.threadRootId = threadRootId
        self.broadcast = broadcast
    }
}

/// Owns the send/replay invariants shared by HTTP confirmation and gateway
/// delivery. HTTP can confirm a write but only ordered gateway events advance
/// the durable replay cursor.
public struct ChatDeliveryState: Sendable {
    public private(set) var cursor: String
    public private(set) var pending: PendingMessage?
    public private(set) var rejected = false

    public init(cursor: String = "0") { self.cursor = cursor }

    public mutating func begin(text: String, threadRootId: String? = nil, broadcast: Bool = false, makeID: () -> String = { UUID().uuidString }) -> PendingMessage {
        if let pending { return pending }
        // Rust's UUID serialization returns lowercase in both HTTP and replay.
        let command = PendingMessage(id: makeID().lowercased(), text: text, threadRootId: threadRootId, broadcast: broadcast)
        pending = command
        return command
    }

    public mutating func confirmHTTP(id: String) {
        if pending?.id == id { pending = nil; rejected = false }
    }

    public mutating func reject(id: String) {
        if pending?.id == id { rejected = true }
    }

    public mutating func discardRejected() -> String? {
        guard rejected else { return nil }
        let text = pending?.text
        pending = nil; rejected = false
        return text
    }

    @discardableResult
    public mutating func confirmGateway(clientMessageID: String, authorID: String, ownAuthorID: String?) -> Bool {
        guard authorID == ownAuthorID, pending?.id == clientMessageID else { return false }
        pending = nil; rejected = false
        return true
    }

    public mutating func receive(seq: String) -> Bool {
        if (try? Sequence.compare(seq, cursor)) == .orderedSame { return true }
        guard Sequence.isSuccessor(seq, of: cursor) else { return false }
        cursor = seq
        return true
    }

    public mutating func reset(cursor: String = "0", preservingPending: Bool = false) {
        self.cursor = cursor
        if !preservingPending { pending = nil; rejected = false }
    }
}

enum ReactionEvent {
    static func sequence(_ event: [String: Any], channelID: String) -> String? {
        guard event["schemaVersion"] as? Int == 1,
              event["channelId"] as? String == channelID,
              let messageID = event["messageId"] as? String, !messageID.isEmpty,
              let reactions = event["reactions"] as? [[String: Any]],
              reactions.allSatisfy({ reaction in
                  guard let emoji = reaction["emoji"] as? String, !emoji.isEmpty,
                        let authorIDs = reaction["authorIds"] as? [String], !authorIDs.isEmpty else { return false }
                  return Set(authorIDs).count == authorIDs.count && authorIDs.allSatisfy { !$0.isEmpty }
              }), Set(reactions.compactMap { $0["emoji"] as? String }).count == reactions.count,
              let seq = event["seq"] as? String, (try? Sequence.compare(seq, "0")) != nil else { return nil }
        return seq
    }
}

enum EditEvent {
    static func message(_ event: [String: Any], channelID: String) -> ChatMessage? {
        let timestamp = ISO8601DateFormatter()
        timestamp.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard event["type"] as? String == "message.edited", event["schemaVersion"] as? Int == 1,
              event["channelId"] as? String == channelID,
              let raw = event["message"], let data = try? JSONSerialization.data(withJSONObject: raw),
              let message = try? JSONDecoder().decode(ChatMessage.self, from: data),
              message.channelId == channelID, (message.revision ?? 1) > 1,
              message.content.version == 1, message.content.type == "text",
              let editedAt = message.editedAt,
              timestamp.date(from: editedAt) != nil || ISO8601DateFormatter().date(from: editedAt) != nil,
              let seq = event["seq"] as? String, message.editSeq == seq,
              (try? Sequence.compare(seq, message.seq)) == .orderedDescending else { return nil }
        return message
    }
}

/// Overlays only content. Never inserts an unloaded message into a timeline.
struct EditSnapshots: Sendable {
    private var values: [String: ChatMessage] = [:]
    private var knownMessageIDs: Set<String> = []
    private(set) var unseenOverflowed = false

    mutating func apply(_ message: ChatMessage) {
        guard (message.revision ?? 1) > 1 else { return }
        if let current = values[message.id], (current.revision ?? 1) >= (message.revision ?? 1) { return }
        guard knownMessageIDs.contains(message.id) || values[message.id] != nil || values.keys.filter({ !knownMessageIDs.contains($0) }).count < 256 else {
            unseenOverflowed = true; return
        }
        values[message.id] = message
    }

    mutating func seed(_ messages: [ChatMessage]) {
        knownMessageIDs.formUnion(messages.map(\.id))
        messages.forEach { apply($0) }
    }

    func overlay(_ message: ChatMessage) -> ChatMessage {
        guard let snapshot = values[message.id], snapshot.channelId == message.channelId,
              (snapshot.revision ?? 1) > (message.revision ?? 1) else { return message }
        var result = message
        result.content = snapshot.content; result.revision = snapshot.revision
        result.editedAt = snapshot.editedAt; result.editSeq = snapshot.editSeq
        return result
    }

    mutating func reset() { values = [:]; knownMessageIDs = []; unseenOverflowed = false }
}

enum PinEvent {
    static func sequence(_ event: [String: Any], channelID: String) -> String? {
        guard event["schemaVersion"] as? Int == 1, event["channelId"] as? String == channelID,
              let raw = event["message"], let data = try? JSONSerialization.data(withJSONObject: raw),
              let message = try? JSONDecoder().decode(ChatMessage.self, from: data),
              let seq = event["seq"] as? String,
              MessagePinEvent(type: event["type"] as? String ?? "", schemaVersion: 1,
                              channelId: channelID, seq: seq, message: message).isValid else { return nil }
        return seq
    }
}

/// Keeps each message's reaction snapshot monotonic independently of the
/// channel delivery cursor. This lets delayed HTTP acknowledgements and older
/// history pages fill missing messages without reverting a newer replay.
struct ReactionSnapshots: Sendable {
    private var values: [String: (seq: String?, reactions: [MessageReaction])] = [:]
    private var knownMessageIDs: Set<String> = []
    private(set) var unseenOverflowed = false
    static let maximumUnseen = 256

    mutating func apply(messageID: String, seq: String?, reactions: [MessageReaction]) -> Bool {
        guard let seq, (try? Sequence.compare(seq, "0")) != nil else { return false }
        if let current = values[messageID], let currentSeq = current.seq,
           (try? Sequence.compare(seq, currentSeq)) != .orderedDescending { return false }
        let unseenCount = values.keys.filter { !knownMessageIDs.contains($0) }.count
        guard knownMessageIDs.contains(messageID) || values[messageID] != nil || unseenCount < Self.maximumUnseen else {
            unseenOverflowed = true
            return false
        }
        values[messageID] = (seq, reactions)
        return true
    }

    mutating func seed(_ messages: [ChatMessage]) {
        knownMessageIDs.formUnion(messages.map(\.id))
        for message in messages {
            if message.reactionSeq == nil, values[message.id] == nil {
                values[message.id] = (nil, message.reactions ?? [])
            } else {
                _ = apply(messageID: message.id, seq: message.reactionSeq, reactions: message.reactions ?? [])
            }
        }
    }

    func overlay(_ message: ChatMessage) -> ChatMessage {
        guard let snapshot = values[message.id] else { return message }
        var result = message
        result.reactions = snapshot.reactions
        result.reactionSeq = snapshot.seq
        return result
    }

    mutating func reset() {
        values.removeAll(keepingCapacity: false)
        knownMessageIDs.removeAll(keepingCapacity: false)
        unseenOverflowed = false
    }
}

struct PinSnapshots: Sendable {
    private var values: [String: (seq: String, pin: MessagePin?)] = [:]
    private var snapshotCursor: String?

    mutating func apply(_ message: ChatMessage) -> Bool {
        guard let seq = message.pinSeq, (try? Sequence.compare(seq, "0")) != nil else { return false }
        if let snapshotCursor, (try? Sequence.compare(seq, snapshotCursor)) != .orderedDescending { return false }
        if let current = values[message.id], (try? Sequence.compare(seq, current.seq)) != .orderedDescending { return false }
        values[message.id] = (seq, message.pin)
        return true
    }

    mutating func replace(_ messages: [ChatMessage], cursor: String) {
        let newer = values.filter { (try? Sequence.compare($0.value.seq, cursor)) == .orderedDescending }
        values.removeAll(keepingCapacity: true)
        snapshotCursor = nil
        seed(messages)
        values.merge(newer) { _, next in next }
        snapshotCursor = cursor
    }

    mutating func seed(_ messages: [ChatMessage]) { messages.forEach { _ = apply($0) } }
    func overlay(_ message: ChatMessage) -> ChatMessage {
        var result = message
        if let value = values[message.id] {
            result.pin = value.pin; result.pinSeq = value.seq
        } else if let snapshotCursor, (try? Sequence.compare(message.pinSeq ?? "0", snapshotCursor)) != .orderedDescending {
            result.pin = nil; result.pinSeq = snapshotCursor
        }
        return result
    }
    mutating func reset() { values.removeAll(keepingCapacity: false); snapshotCursor = nil }
}

/// Accepts only snapshots that cannot move an already-versioned view
/// backwards. Unversioned snapshots remain usable until a revision is seen.
struct MonotonicRevision: Sendable {
    private(set) var latest: Int?

    mutating func accept(_ revision: Int?) -> Bool {
        if let latest {
            guard let revision, revision > latest else { return false }
            self.latest = revision
        } else if let revision {
            latest = revision
        }
        return true
    }
}
