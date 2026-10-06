import Foundation

/// What the mention card shows for a person's pill, resolved locally.
struct MentionCardPerson: Equatable, Sendable {
    let id: String
    let username: String
    /// nil when nobody you know matches: the card then titles itself
    /// `@username` and drops the second line.
    let displayName: String?
    let avatarId: Int?
    /// The signed-in account: a muted "You" line instead of **Message**.
    let isSelf: Bool

    var title: String { displayName ?? "@\(username)" }
    var subtitle: String? { displayName == nil ? nil : "@\(username)" }
    /// The generic initial avatar uses the username when the name is unknown.
    var avatarName: String { displayName ?? username }
}

/// Clicking a person's `@mention` pill opens a small card with **Message**.
/// Pills are `Text` links in this private scheme, handled by the message's
/// `openURL` action; nothing outside the app ever sees them.
enum MentionCard {
    static let scheme = "caper-mention"
    /// The timeline's coordinate space, for anchoring the macOS popover.
    static let timelineSpace = "chat-timeline"

    static func url(for pill: MentionPill) -> URL? {
        var components = URLComponents()
        components.scheme = scheme
        components.path = pill.id
        return components.url
    }

    static func userID(from url: URL) -> String? {
        guard url.scheme == scheme,
              let path = URLComponents(url: url, resolvingAgainstBaseURL: false)?.path, !path.isEmpty else { return nil }
        return path
    }

    /// Card data without a network call: the loaded space members, then the
    /// `GET /api/people` list, then DM peers, all matched by id, then your own
    /// account; otherwise the unknown fallback (DM creation is by username, so
    /// **Message** still works).
    static func resolve(_ pill: MentionPill, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                        account: Account?, viewerID: String?) -> MentionCardPerson {
        let isSelf = (viewerID ?? account?.id) == pill.id
        func person(_ username: String, _ displayName: String?, _ avatarId: Int?) -> MentionCardPerson {
            let name = displayName?.trimmingCharacters(in: .whitespacesAndNewlines)
            return MentionCardPerson(id: pill.id, username: username.isEmpty ? pill.username : username,
                                     displayName: name?.isEmpty == false ? name : nil, avatarId: avatarId, isSelf: isSelf)
        }
        if let member = members.first(where: { $0.id == pill.id }) {
            return person(member.username, member.displayName, member.avatarId)
        }
        if let known = people?.first(where: { $0.id == pill.id }) {
            return person(known.username, known.displayName, known.avatarId)
        }
        if let peer = peers.first(where: { $0.id == pill.id }) {
            return person(peer.username, peer.displayName, nil)
        }
        if let account, account.id == pill.id {
            return person(account.username ?? pill.username, account.displayName, account.avatarId)
        }
        return MentionCardPerson(id: pill.id, username: pill.username, displayName: nil, avatarId: nil, isSelf: isSelf)
    }
}
