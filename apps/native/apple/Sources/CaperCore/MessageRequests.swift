import Foundation

/// A DM's `status`. Older servers omit it, which means `accepted`.
public enum DirectMessageStatus: String, Codable, Sendable {
    /// A normal conversation.
    case accepted
    /// You started it and they have not accepted; they see it as a request.
    case outgoing
    /// A request to you from someone you share no space with.
    case incoming
}

/// Someone you blocked, from `GET /api/blocks` (newest first).
public struct BlockedAccount: Codable, Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String
    public let displayName: String
    public var avatarId: Int? = nil
}

public struct BlocksResponse: Codable, Sendable {
    public let blocks: [BlockedAccount]
}

/// "Who can start a DM with you".
public enum DirectMessagePrivacy: String, Codable, CaseIterable, Identifiable, Sendable {
    case anyone, spaces, nobody

    public var id: String { rawValue }
    public var title: String {
        switch self {
        case .anyone: return "Anyone"
        case .spaces: return "People in my spaces"
        case .nobody: return "No one new"
        }
    }
    public var detail: String? {
        switch self {
        case .anyone: return "People outside your spaces send a message request first."
        case .spaces: return nil
        case .nobody: return "Conversations you already have stay open."
        }
    }
}

public struct PrivacySettings: Codable, Equatable, Sendable {
    public var directMessages: DirectMessagePrivacy
}

/// The person a Block action targets: a DM peer, a request's sender or a
/// message author (whose username the message does not carry).
public struct BlockTarget: Equatable, Identifiable, Sendable {
    public let id: String
    public let username: String?
    public let displayName: String
    public var avatarId: Int? = nil

    public init(id: String, username: String?, displayName: String, avatarId: Int? = nil) {
        self.id = id; self.username = username; self.displayName = displayName; self.avatarId = avatarId
    }
    public init(peer: DirectMessagePeer) {
        self.init(id: peer.id, username: peer.username, displayName: peer.displayName, avatarId: peer.avatarId)
    }
    public init(author: ChatAuthor) {
        self.init(id: author.id, username: nil, displayName: author.name, avatarId: author.avatarId)
    }

    public var confirmationTitle: String { "Block \(displayName)?" }
    public static let confirmationMessage = "You won't see their messages unless you choose to, and they can't send you DMs or requests."
}

enum DirectMessageErrors {
    static let notAccepted = "This person isn't accepting direct messages."
    static let blocked = "You blocked this person. Unblock them to message them."

    /// The client's wording for the DM privacy and block error codes.
    static func message(code: String?) -> String? {
        switch code {
        case "dm_not_accepted": return notAccepted
        case "dm_blocked": return blocked
        default: return nil
        }
    }

    /// A DM send or start that the recipient's privacy or a block stopped:
    /// the message cannot be retried as is.
    static func isRefusal(status: Int, code: String?) -> Bool {
        status == 403 && message(code: code) != nil
    }
}

enum MessageRequests {
    /// The main "Direct messages" list: accepted and outgoing conversations.
    static func visible(_ conversations: [DirectMessageConversation]) -> [DirectMessageConversation] {
        conversations.filter { $0.status != .incoming }
    }

    /// Incoming requests, for the "Message requests" row and its list.
    static func incoming(_ conversations: [DirectMessageConversation]) -> [DirectMessageConversation] {
        conversations.filter { $0.status == .incoming }
    }

    /// Requests never count: only main-list conversations can be unread.
    static func unreadCount(_ conversations: [DirectMessageConversation]) -> Int {
        visible(conversations).filter(\.unread).count
    }

    static func prompt(for peer: DirectMessagePeer) -> (name: String, detail: String) {
        (peer.displayName, " (@\(peer.username)) wants to message you. You don't share a space.")
    }

    static func waitingNotice(for peer: DirectMessagePeer) -> String {
        "Waiting for @\(peer.username) to accept. They'll see your messages when they do."
    }

    static func blockedNotice(for peer: DirectMessagePeer) -> String {
        "You blocked @\(peer.username)."
    }

    /// After blocking or unblocking `peerID`: marks their conversations, and a
    /// block drops their pending request (the server declines it).
    static func applying(blocked: Bool, peerID: String, to conversations: [DirectMessageConversation]) -> [DirectMessageConversation] {
        conversations.compactMap { conversation in
            guard conversation.peer.id == peerID else { return conversation }
            if blocked && conversation.status == .incoming { return nil }
            return conversation.with(blocked: blocked)
        }
    }
}

/// Consecutive messages by blocked authors, collapsed into one row.
struct BlockedRun: Equatable, Identifiable {
    let messages: [ChatMessage]
    /// Shown in place (in memory only); the row then offers Hide.
    let revealed: Bool

    var id: String { "blocked-\(messages.first?.id ?? "")" }
    var count: Int { messages.count }
    var label: String { "\(count) blocked \(count == 1 ? "message" : "messages")" }
    var messageIDs: Set<String> { Set(messages.map(\.id)) }
}

/// One timeline row: a message, or a run of blocked authors' messages.
enum TimelineEntry: Equatable, Identifiable {
    case message(ChatMessage)
    case blocked(BlockedRun)

    /// The newest message the entry contains: both the row's list identity
    /// and its scroll `.id`. When blocks load and messages fold into a run,
    /// the run takes over that message's row instead of clashing with it.
    var id: String {
        switch self {
        case let .message(message): return message.id
        case let .blocked(run): return run.messages.last?.id ?? run.id
        }
    }
    var messages: [ChatMessage] {
        switch self {
        case let .message(message): return [message]
        case let .blocked(run): return run.messages
        }
    }
    /// The scroll target: the last message the entry contains.
    var scrollID: String { id }
    var firstCreatedAt: String { messages.first?.createdAt ?? "" }
    var lastCreatedAt: String { messages.last?.createdAt ?? "" }
}

enum BlockedMessages {
    /// Guests and your own messages never collapse.
    static func isHidden(_ message: ChatMessage, blocked: Set<String>, viewerID: String?) -> Bool {
        !message.author.isGuest && message.author.id != viewerID && blocked.contains(message.author.id)
    }

    /// Whether message actions offer Block for this author.
    static func canBlock(_ author: ChatAuthor, viewerID: String?, blocked: Set<String>) -> Bool {
        guard let viewerID, !viewerID.isEmpty else { return false }
        return !author.isGuest && author.id != viewerID && !blocked.contains(author.id)
    }

    /// Whether message actions offer Unblock: an author you already blocked,
    /// on a message you chose to show.
    static func canUnblock(_ author: ChatAuthor, viewerID: String?, blocked: Set<String>) -> Bool {
        guard let viewerID, !viewerID.isEmpty else { return false }
        return !author.isGuest && author.id != viewerID && blocked.contains(author.id)
    }

    /// Groups each run of consecutive hidden messages. A run is revealed when
    /// any of its messages is in `revealed`, so a run that grows stays open.
    static func entries(_ messages: [ChatMessage], blocked: Set<String>, viewerID: String?,
                        revealed: Set<String> = []) -> [TimelineEntry] {
        guard !blocked.isEmpty else { return messages.map(TimelineEntry.message) }
        var entries: [TimelineEntry] = []
        var run: [ChatMessage] = []
        func flush() {
            guard !run.isEmpty else { return }
            entries.append(.blocked(BlockedRun(messages: run, revealed: run.contains { revealed.contains($0.id) })))
            run = []
        }
        for message in messages {
            if isHidden(message, blocked: blocked, viewerID: viewerID) { run.append(message) }
            else { flush(); entries.append(.message(message)) }
        }
        flush()
        return entries
    }

    /// Show or Hide a run: returns the new revealed set.
    static func toggling(_ run: BlockedRun, in revealed: Set<String>) -> Set<String> {
        run.revealed ? revealed.subtracting(run.messageIDs) : revealed.union(run.messageIDs)
    }
}
