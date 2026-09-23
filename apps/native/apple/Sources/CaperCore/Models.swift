import Foundation

public struct Account: Codable, Equatable, Sendable {
    public let id: String
    public var username: String?
    public var displayName: String?
}

public struct Space: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public let ownerId: String
    public var demo: Bool?
}

public struct Channel: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let spaceId: String
    public let name: String
    public let `private`: Bool
}

public struct Member: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String
    public let displayName: String
    public let owner: Bool
}

public struct SpaceLimits: Codable, Equatable, Sendable {
    public let ownedSpaces: Int
    public let totalSpaces: Int
    public let channelsPerSpace: Int
}

public struct SpacesResponse: Codable, Sendable {
    public let spaces: [Space]
    public let limits: SpaceLimits
}

public struct SpaceDetail: Codable, Sendable {
    public let space: Space
    public let channels: [Channel]
    public let members: [Member]
}

public enum PresenceStatus: String, Codable, Sendable { case online, idle, offline, unknown }

public struct PresenceMember: Codable, Equatable, Sendable {
    public let userId: String
    public let status: PresenceStatus
}

public enum WorkspaceValidation {
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
}

/// Owns the send/replay invariants shared by HTTP confirmation and gateway
/// delivery. HTTP can confirm a write but only ordered gateway events advance
/// the durable replay cursor.
public struct ChatDeliveryState: Sendable {
    public private(set) var cursor: String
    public private(set) var pending: PendingMessage?

    public init(cursor: String = "0") { self.cursor = cursor }

    public mutating func begin(text: String, makeID: () -> String = { UUID().uuidString }) -> PendingMessage {
        if let pending { return pending }
        let command = PendingMessage(id: makeID(), text: text)
        pending = command
        return command
    }

    public mutating func confirmHTTP(id: String) {
        if pending?.id == id { pending = nil }
    }

    public mutating func reject(id: String) {
        if pending?.id == id { pending = nil }
    }

    @discardableResult
    public mutating func confirmGateway(clientMessageID: String, authorID: String, ownAuthorID: String?) -> Bool {
        guard authorID == ownAuthorID, pending?.id == clientMessageID else { return false }
        pending = nil
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
        if !preservingPending { pending = nil }
    }
}
