import Foundation

/// What the mention card shows for a person's pill, resolved locally.
struct MentionCardPerson: Equatable, Sendable {
    let id: String
    let username: String?
    /// nil when nobody you know matches: the card then titles itself
    /// `@username` and drops the second line.
    let displayName: String?
    let avatarId: Int?
    /// The signed-in account: a muted "You" line instead of **Message**.
    let isSelf: Bool

    var title: String { displayName ?? username.map { "@\($0)" } ?? "Profile" }
    var subtitle: String? { displayName == nil ? nil : username.map { "@\($0)" } }
    /// The generic initial avatar uses the username when the name is unknown.
    var avatarName: String { displayName ?? username ?? title }
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
        resolve(id: pill.id, username: pill.username, members: members, people: people, peers: peers, account: account, viewerID: viewerID)
    }

    /// Pin metadata has a real identity but no username. Never match by name or invent a DM recipient.
    static func resolve(_ author: ChatAuthor, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                        account: Account?, viewerID: String?) -> MentionCardPerson {
        let person = resolve(id: author.id, username: nil, members: members, people: people, peers: peers, account: account, viewerID: viewerID)
        return MentionCardPerson(id: author.id, username: person.username, displayName: person.displayName ?? author.name,
                                 avatarId: person.username == nil ? author.avatarId : person.avatarId, isSelf: person.isSelf)
    }

    private static func resolve(id: String, username: String?, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                                account: Account?, viewerID: String?) -> MentionCardPerson {
        let isSelf = (viewerID ?? account?.id) == id
        func person(_ knownUsername: String?, _ displayName: String?, _ avatarId: Int?) -> MentionCardPerson {
            let name = displayName?.trimmingCharacters(in: .whitespacesAndNewlines)
            return MentionCardPerson(id: id, username: knownUsername?.isEmpty == false ? knownUsername : username,
                                     displayName: name?.isEmpty == false ? name : nil, avatarId: avatarId, isSelf: isSelf)
        }
        if let member = members.first(where: { $0.id == id }) {
            return person(member.username, member.displayName, member.avatarId)
        }
        if let known = people?.first(where: { $0.id == id }) {
            return person(known.username, known.displayName, known.avatarId)
        }
        if let peer = peers.first(where: { $0.id == id }) {
            return person(peer.username, peer.displayName, nil)
        }
        if let account, account.id == id {
            return person(account.username, account.displayName, account.avatarId)
        }
        return MentionCardPerson(id: id, username: username, displayName: nil, avatarId: nil, isSelf: isSelf)
    }
}
