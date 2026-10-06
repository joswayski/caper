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
}

public struct DirectMessageConversation: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let peer: DirectMessagePeer
    public let lastSeq: String
    public let readSeq: String

    public var unread: Bool { (try? Sequence.compare(lastSeq, readSeq)) == .orderedDescending }
}

public struct DirectMessagesResponse: Codable, Sendable {
    public let conversations: [DirectMessageConversation]
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

public struct ChatContent: Codable, Equatable, Sendable {
    public let version: Int
    public let type: String
    public let text: String
}

public struct ChatMessage: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let channelId: String
    public let seq: String
    public let author: ChatAuthor
    public let content: ChatContent
    public let createdAt: String
    public let clientMessageId: String
    public var reactions: [MessageReaction]? = nil
    public var reactionSeq: String? = nil
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

public struct ChatHistory: Codable, Sendable {
    public let space: HistoryIdentity?
    public let channel: HistoryIdentity?
    public let messages: [ChatMessage]
    public let cursor: String
    public let hasMore: Bool
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
            && (try? Sequence.compare(message.seq, "0")) != nil
    }
}

public struct PendingMessage: Equatable, Sendable {
    public let id: String
    public let text: String
    public let createdAt: String

    public init(id: String, text: String, createdAt: String = ISO8601DateFormatter().string(from: Date())) {
        self.id = id
        self.text = text
        self.createdAt = createdAt
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

    public mutating func begin(text: String, makeID: () -> String = { UUID().uuidString }) -> PendingMessage {
        if let pending { return pending }
        // Rust's UUID serialization returns lowercase in both HTTP and replay.
        let command = PendingMessage(id: makeID().lowercased(), text: text)
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
