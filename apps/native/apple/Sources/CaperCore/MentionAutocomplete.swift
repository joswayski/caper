import Foundation

/// A composer `@` suggestion: a person, or the `everyone`/`here` specials
/// offered only in space channels.
struct MentionCandidate: Equatable, Identifiable, Sendable {
    enum Kind: Equatable, Sendable { case member, everyone, here }
    let kind: Kind
    /// The user's external id, or the special's name.
    let id: String
    /// Inserted as `@username `.
    let username: String
    /// A member's display name, or the special's muted description.
    let displayName: String
    var avatarId: Int? = nil

    static let everyone = MentionCandidate(kind: .everyone, id: "everyone", username: "everyone",
                                           displayName: "Everyone in this channel")
    static let here = MentionCandidate(kind: .here, id: "here", username: "here",
                                       displayName: "Everyone online in this channel")

    static func member(id: String, username: String, displayName: String, avatarId: Int? = nil) -> MentionCandidate {
        MentionCandidate(kind: .member, id: id, username: username, displayName: displayName, avatarId: avatarId)
    }
}

/// Who the composer may suggest in the open conversation.
struct MentionSource: Equatable, Sendable {
    var members: [MentionCandidate]
    var specials: Bool

    static let empty = MentionSource(members: [], specials: false)

    /// A space channel: the members from `GET /api/spaces/{space}` (empty
    /// until loaded) except the signed-in account, then `everyone`/`here`.
    static func space(members: [Member], excluding accountID: String?) -> MentionSource {
        MentionSource(members: members.filter { $0.id != accountID }.map {
            MentionCandidate.member(id: $0.id, username: $0.username, displayName: $0.displayName, avatarId: $0.avatarId)
        }, specials: true)
    }

    /// Any DM, self-notes included: everyone from `GET /api/people` once it
    /// has loaded (plus the DM peer if that list predates the conversation);
    /// until then only the other participant, so nobody in self-notes. Never
    /// the specials.
    static func direct(peer: DirectMessagePeer?, people: [Person]? = nil, accountID: String?) -> MentionSource {
        var members = (people ?? []).filter { $0.id != accountID }.map {
            MentionCandidate.member(id: $0.id, username: $0.username, displayName: $0.displayName, avatarId: $0.avatarId)
        }
        if let peer, peer.id != accountID, !members.contains(where: { $0.id == peer.id }) {
            members.append(.member(id: peer.id, username: peer.username, displayName: peer.displayName))
        }
        return members.isEmpty ? .empty : MentionSource(members: members, specials: false)
    }
}

/// The `@` token ending at the composer caret.
struct MentionQuery: Equatable {
    /// UTF-16 range from the `@` through the caret, as text views report it.
    let range: NSRange
    /// The typed name, lowercased (possibly empty).
    let query: String
}

struct MentionAutocompleteMatch: Equatable {
    let replacementRange: NSRange
    let choices: [MentionCandidate]
}

/// One `@name` token in message text.
struct MentionToken: Equatable {
    /// From the `@` through the end of the name, aligned to Unicode scalars.
    let range: Range<String.Index>
    /// Lowercased, without the `@`.
    let name: String
}

/// A run of message text and whether it renders as a mention pill.
struct MentionSegment: Equatable {
    let text: String
    let highlighted: Bool
}

/// The shared `@mention` contract: a token starts at an `@` at the start of the
/// text or after whitespace, `(`, `[` or `{` (the `:` emoji start rule), and its
/// name is the maximal following run of ASCII `[A-Za-z0-9_]`; runs over 32
/// characters are not mentions. Names compare lowercased. The text is scanned by
/// Unicode scalar, exactly as the server parses it.
enum MentionAutocomplete {
    static let maximumNameLength = 32
    static let maximumChoices = 6

    static func isNameScalar(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 48...57, 65...90, 97...122, 95: return true
        default: return false
        }
    }

    /// Whether an `@` after `previous` (nil at the start of the text) can begin a token.
    static func startsToken(after previous: Unicode.Scalar?) -> Bool {
        guard let previous else { return true }
        return previous.properties.isWhitespace || previous == "(" || previous == "[" || previous == "{"
    }

    // MARK: Composer

    /// The active token: a collapsed selection outside IME composition, right
    /// after `@` plus 0–32 name characters, where that `@` satisfies the start
    /// rule and the character at the caret (if any) is neither a name character
    /// nor `@`.
    static func activeQuery(text: String, selection: NSRange, markedText: Bool) -> MentionQuery? {
        guard !markedText, selection.length == 0,
              let caret = Range(NSRange(location: selection.location, length: 0), in: text)?.lowerBound else { return nil }
        let scalars = text.unicodeScalars
        if caret < scalars.endIndex, isNameScalar(scalars[caret]) || scalars[caret] == "@" { return nil }
        var start = caret
        var name: [Unicode.Scalar] = []
        while start > scalars.startIndex {
            let previous = scalars.index(before: start)
            guard isNameScalar(scalars[previous]) else { break }
            name.insert(scalars[previous], at: 0)
            guard name.count <= maximumNameLength else { return nil }
            start = previous
        }
        guard start > scalars.startIndex else { return nil }
        let at = scalars.index(before: start)
        guard scalars[at] == "@" else { return nil }
        guard startsToken(after: at > scalars.startIndex ? scalars[scalars.index(before: at)] : nil) else { return nil }
        return MentionQuery(range: NSRange(at..<caret, in: text), query: string(name).lowercased())
    }

    /// Members ranked 0 (exact username), 1 (username prefix), 2 (display name
    /// or one of its space-separated words starts with the query), 3 (username
    /// contains it), ties by username; an empty query puts every member at 1.
    /// Matching specials (`everyone`, then `here`) follow the members and keep
    /// their slots within the cap of six.
    static func rank(query: String, source: MentionSource) -> [MentionCandidate] {
        let query = query.lowercased()
        let specials = source.specials ? [MentionCandidate.everyone, .here].filter { $0.username.hasPrefix(query) } : []
        let members = source.members.compactMap { candidate -> (rank: Int, username: String, candidate: MentionCandidate)? in
            let username = candidate.username.lowercased()
            let displayName = candidate.displayName.lowercased()
            let rank: Int
            if query.isEmpty { rank = 1 }
            else if username == query { rank = 0 }
            else if username.hasPrefix(query) { rank = 1 }
            else if displayName.hasPrefix(query) || displayName.split(separator: " ").contains(where: { $0.hasPrefix(query) }) { rank = 2 }
            else if username.contains(query) { rank = 3 }
            else { return nil }
            return (rank, username, candidate)
        }.sorted { ($0.rank, $0.username) < ($1.rank, $1.username) }.map { $0.candidate }
        return Array(members.prefix(max(0, maximumChoices - specials.count))) + specials
    }

    static func match(text: String, selection: NSRange, markedText: Bool, source: MentionSource) -> MentionAutocompleteMatch? {
        guard let token = activeQuery(text: text, selection: selection, markedText: markedText) else { return nil }
        let choices = rank(query: token.query, source: source)
        guard !choices.isEmpty else { return nil }
        return MentionAutocompleteMatch(replacementRange: token.range, choices: choices)
    }

    /// Replaces `@` through the caret with `@username ` and leaves the caret
    /// after the space; nil when the draft would exceed `scalarLimit`.
    static func inserting(_ candidate: MentionCandidate, in text: String, match: MentionAutocompleteMatch,
                          scalarLimit: Int = 4_000) -> (text: String, selection: NSRange)? {
        guard let range = Range(match.replacementRange, in: text) else { return nil }
        let insertion = "@\(candidate.username) "
        let result = text.replacingCharacters(in: range, with: insertion)
        guard result.unicodeScalars.count <= scalarLimit else { return nil }
        return (result, NSRange(location: match.replacementRange.location + insertion.utf16.count, length: 0))
    }

    // MARK: Messages

    /// Every grammar token in `text`, in order (names of 1–32 characters).
    static func tokens(in text: String) -> [MentionToken] {
        let scalars = text.unicodeScalars
        var tokens: [MentionToken] = []
        var previous: Unicode.Scalar?
        var index = scalars.startIndex
        while index < scalars.endIndex {
            let scalar = scalars[index]
            let starts = scalar == "@" && startsToken(after: previous)
            previous = scalar
            var end = scalars.index(after: index)
            guard starts else { index = end; continue }
            var name: [Unicode.Scalar] = []
            while end < scalars.endIndex, isNameScalar(scalars[end]) {
                name.append(scalars[end]); previous = scalars[end]
                end = scalars.index(after: end)
            }
            if (1...maximumNameLength).contains(name.count) {
                tokens.append(MentionToken(range: index..<end, name: string(name).lowercased()))
            }
            index = end
        }
        return tokens
    }

    /// A token is highlighted only when `content.mentions` resolves it:
    /// `everyone`/`here` need their entry, other names a `user` entry with that
    /// username. Unresolved names stay plain text.
    static func isHighlighted(name: String, mentions: [MessageMention]) -> Bool {
        let name = name.lowercased()
        return mentions.contains { mention in
            switch mention {
            case .everyone: return name == "everyone"
            case .here: return name == "here"
            case let .user(_, username): return username.lowercased() == name
            }
        }
    }

    /// `text` split into plain and highlighted runs; concatenating them
    /// reproduces `text` exactly.
    static func segments(in text: String, mentions: [MessageMention]) -> [MentionSegment] {
        guard !mentions.isEmpty else { return text.isEmpty ? [] : [MentionSegment(text: text, highlighted: false)] }
        let scalars = text.unicodeScalars
        var segments: [MentionSegment] = []
        var cursor = scalars.startIndex
        for token in tokens(in: text) where isHighlighted(name: token.name, mentions: mentions) {
            if cursor < token.range.lowerBound {
                segments.append(MentionSegment(text: string(scalars[cursor..<token.range.lowerBound]), highlighted: false))
            }
            segments.append(MentionSegment(text: string(scalars[token.range]), highlighted: true))
            cursor = token.range.upperBound
        }
        if cursor < scalars.endIndex {
            segments.append(MentionSegment(text: string(scalars[cursor..<scalars.endIndex]), highlighted: false))
        }
        return segments
    }

    /// A message mentions the signed-in account when a `user` entry carries its
    /// id, or when it has an `everyone`/`here` entry and someone else wrote it.
    static func mentionsCurrentUser(_ message: ChatMessage, currentUserID: String?) -> Bool {
        guard let currentUserID, !currentUserID.isEmpty else { return false }
        return message.content.mentions.contains { mention in
            switch mention {
            case let .user(id, _): return id == currentUserID
            case .everyone, .here: return message.author.id != currentUserID
            }
        }
    }

    // `Swift.` because Models.swift declares a `Sequence` enum for message seqs.
    private static func string<S: Swift.Sequence>(_ scalars: S) -> String where S.Element == Unicode.Scalar {
        var result = ""
        result.unicodeScalars.append(contentsOf: scalars)
        return result
    }
}
