import Foundation

// Live profile and space updates (the gateway's `profiles` subscription):
// `profile.updated` and `space.updated` events, and the pure functions that
// apply them over every retained copy of a person or space.

/// One event from the `profiles` subscription.
public enum ProfileFeedEvent: Equatable, Sendable {
    /// Someone's name, username or picture changed. Absent picture fields mean
    /// they removed their picture.
    case person(Person)
    /// A space was renamed or its icon changed. Absent icon fields mean it was removed.
    case space(Space)

    /// The event, or nil for other or malformed frames (newer servers may add types).
    public static func parse(_ event: [String: Any]) -> ProfileFeedEvent? {
        switch event["type"] as? String {
        case "profile.updated":
            guard let person: Person = decode(event["person"]), !person.id.isEmpty else { return nil }
            return .person(person)
        case "space.updated":
            guard let space: Space = decode(event["space"]), !space.id.isEmpty else { return nil }
            return .space(space)
        default:
            return nil
        }
    }

    private static func decode<T: Decodable>(_ raw: Any?) -> T? {
        guard let raw, JSONSerialization.isValidJSONObject(raw), let data = try? JSONSerialization.data(withJSONObject: raw) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }
}

/// A newer picture over a retained snapshot's.
enum PictureMerge {
    /// No newer asset means the picture was removed. For the same asset the
    /// signature that expires later wins (ties and unreadable URLs go to the
    /// newer one), so long sessions never keep an expired URL. Pictures are
    /// cached by asset, so a fresher URL doesn't reload them.
    static func newer(asset: String?, url: String?, over snapshotAsset: String?, _ snapshotURL: String?) -> (asset: String?, url: String?) {
        guard let asset else { return (nil, nil) }
        guard snapshotAsset == asset else { return (asset, url) }
        return (asset, fresher(url, over: snapshotURL))
    }

    /// The signed URL that stays valid longer, preferring `url` unless both
    /// carry an expiry and `snapshotURL`'s is strictly later.
    static func fresher(_ url: String?, over snapshotURL: String?) -> String? {
        guard let url else { return snapshotURL }
        guard let snapshotURL, let later = expiry(snapshotURL), let current = expiry(url) else { return url }
        return later > current ? snapshotURL : url
    }

    /// The CDN signature's `exp` (Unix seconds), when present.
    static func expiry(_ url: String) -> Int64? {
        URLComponents(string: url)?.queryItems?.first { $0.name == "exp" }?.value.flatMap { Int64($0) }
    }
}

/// People's newest known identities by account ID, from `profile.updated`
/// events and from server lists that return current profiles (members and
/// invitations, people, DM peers, blocks). Applied over every retained copy
/// of a person (members, invitations, DM peers, suggestions, authors and
/// their nested copies, reactors, feedback reporters, blocked accounts), as
/// the signed-in account's own profile is. Message text and cursors are never
/// touched. Because each refresh records what the server has now, a missed
/// event is repaired by the next refresh; loaded message authors (snapshots
/// from load time) never feed it.
public struct ProfileDirectory: Equatable, Sendable {
    /// Where a fetch started: only events recorded after it are newer than its result.
    public struct Mark: Equatable, Sendable {
        fileprivate let epoch: UUID
        fileprivate let revision: Int
    }

    private var people: [String: Person] = [:]
    /// A directory from another session (before a logout) ignores old fetches.
    private let epoch = UUID()
    private var revision = 0
    private var eventRevisions: [String: Int] = [:]

    public init(_ people: [Person] = []) { people.forEach { record($0) } }

    public var isEmpty: Bool { people.isEmpty }
    public subscript(id: String) -> Person? { people[id] }

    /// Capture before a request whose result lists current profiles.
    public var mark: Mark { Mark(epoch: epoch, revision: revision) }

    /// A `profile.updated` event: the newest state for that person.
    public mutating func record(_ person: Person) {
        revision += 1
        people[person.id] = person
        eventRevisions[person.id] = revision
    }

    /// Current profiles from a server list fetched since `mark`: the newest
    /// state for those IDs (absent picture fields mean none), except anyone
    /// whose event arrived while the request was in flight.
    public mutating func recordFetched(_ fetched: [Person], since mark: Mark) {
        guard mark.epoch == epoch else { return }
        for person in fetched where !person.id.isEmpty && (eventRevisions[person.id] ?? 0) <= mark.revision {
            people[person.id] = person
        }
    }

    public func apply(_ author: ChatAuthor) -> ChatAuthor { people[author.id].map { author.applying($0) } ?? author }
    public func apply(_ member: Member) -> Member { people[member.id].map { member.applying($0) } ?? member }
    public func apply(_ person: Person) -> Person { people[person.id].map { person.applying($0) } ?? person }
    public func apply(_ reactor: ReactorPerson) -> ReactorPerson { people[reactor.id].map { reactor.applying($0) } ?? reactor }
    public func apply(_ account: BlockedAccount) -> BlockedAccount { people[account.id].map { account.applying($0) } ?? account }
    public func apply(_ message: ChatMessage) -> ChatMessage { message.applying(self) }

    public func apply(_ conversation: DirectMessageConversation) -> DirectMessageConversation {
        guard let person = people[conversation.peer.id] else { return conversation }
        return DirectMessageConversation(id: conversation.id, peer: conversation.peer.applying(person), lastSeq: conversation.lastSeq,
                                         readSeq: conversation.readSeq, status: conversation.status, blocked: conversation.blocked)
    }

    public func apply(_ detail: SpaceDetail) -> SpaceDetail {
        guard !isEmpty else { return detail }
        return detail.with(members: detail.members.map { apply($0) })
    }

    public func apply(_ report: FeedbackReport) -> FeedbackReport {
        guard let person = people[report.reporter.id] else { return report }
        return FeedbackReport(id: report.id, messageId: report.messageId, threadRootId: report.threadRootId, createdAt: report.createdAt,
                              text: report.text, attachments: report.attachments, reporter: report.reporter.applying(person),
                              channel: report.channel, space: report.space, replyCount: report.replyCount,
                              answered: report.answered, archived: report.archived)
    }
}

// MARK: Current profiles in server lists

extension Member {
    /// This member's current profile, as a list endpoint returned it.
    var profile: Person {
        Person(admin: admin, id: id, username: username, displayName: displayName, avatarId: avatarId, avatarAsset: avatarAsset, avatarUrl: avatarUrl)
    }
}

extension DirectMessagePeer {
    var profile: Person {
        Person(admin: admin, id: id, username: username, displayName: displayName, avatarId: avatarId, avatarAsset: avatarAsset, avatarUrl: avatarUrl)
    }
}

extension BlockedAccount {
    var profile: Person {
        Person(id: id, username: username, displayName: displayName, avatarId: avatarId, avatarAsset: avatarAsset, avatarUrl: avatarUrl)
    }
}

extension SpaceDetail {
    /// The members `GET /api/spaces/{id}` returned, as current profiles.
    var profiles: [Person] { members.map(\.profile) }
}

// MARK: Applying one person

extension Account {
    /// The signed-in account after a `profile.updated` for it (from another
    /// device). The saved mascot is kept when the event omits it.
    func applying(_ person: Person) -> Account {
        guard id == person.id else { return self }
        var result = self
        result.username = person.username
        result.displayName = person.displayName
        if let avatarId = person.avatarId { result.avatarId = avatarId }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        result.avatarAsset = picture.asset
        result.avatarUrl = picture.url
        return result
    }
}

extension ChatAuthor {
    func applying(_ person: Person) -> ChatAuthor {
        guard id == person.id, !isGuest else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return ChatAuthor(id: id, name: person.displayName, isGuest: false, avatarId: person.avatarId ?? avatarId, admin: person.admin ?? admin,
                          avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension ChatMessage {
    /// Authors here and in nested copies: reply reference, pin, thread
    /// participants and the forwarded original.
    func applying(_ profiles: ProfileDirectory) -> ChatMessage {
        guard !profiles.isEmpty else { return self }
        var result = self
        result.author = profiles.apply(author)
        if var reply = replyTo, profiles[reply.author.id] != nil {
            reply.author = profiles.apply(reply.author)
            result.replyTo = reply
        }
        if let pin, profiles[pin.author.id] != nil { result.pin = MessagePin(author: profiles.apply(pin.author), createdAt: pin.createdAt) }
        if let thread, thread.participants.contains(where: { profiles[$0.id] != nil }) {
            result.thread = ThreadSummary(replyCount: thread.replyCount, participants: thread.participants.map { profiles.apply($0) }, seq: thread.seq)
        }
        if let forward, let original = forward.message { result.forward = MessageForward(message: original.applying(profiles), seq: forward.seq) }
        return result
    }
}

extension Member {
    func applying(_ person: Person) -> Member {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return Member(admin: person.admin ?? admin, id: id, username: person.username, displayName: person.displayName, owner: owner, avatarId: person.avatarId ?? avatarId,
                      avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension Person {
    func applying(_ person: Person) -> Person {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return Person(admin: person.admin ?? admin, id: id, username: person.username, displayName: person.displayName, avatarId: person.avatarId ?? avatarId,
                      avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension DirectMessagePeer {
    /// The Caper team flag is the server's and stays.
    func applying(_ person: Person) -> DirectMessagePeer {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return DirectMessagePeer(admin: person.admin ?? admin, id: id, username: person.username, displayName: person.displayName, avatarId: person.avatarId ?? avatarId,
                                 team: team, avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension ReactorPerson {
    func applying(_ person: Person) -> ReactorPerson {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return ReactorPerson(id: id, username: person.username, displayName: person.displayName, avatarId: person.avatarId ?? avatarId,
                             avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension BlockedAccount {
    func applying(_ person: Person) -> BlockedAccount {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return BlockedAccount(id: id, username: person.username, displayName: person.displayName, avatarId: person.avatarId ?? avatarId,
                              avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

extension FeedbackReport.Reporter {
    func applying(_ person: Person) -> FeedbackReport.Reporter {
        guard id == person.id else { return self }
        let picture = PictureMerge.newer(asset: person.avatarAsset, url: person.avatarUrl, over: avatarAsset, avatarUrl)
        return FeedbackReport.Reporter(id: id, username: person.username, displayName: person.displayName, avatarId: person.avatarId ?? avatarId,
                                       avatarAsset: picture.asset, avatarUrl: picture.url)
    }
}

// MARK: Applying a space

extension Space {
    /// The space after a `space.updated` for it: its name and icon. Other
    /// spaces are returned unchanged, so an update for a space you don't
    /// have changes nothing.
    func applying(_ update: Space) -> Space {
        guard id == update.id else { return self }
        let icon = PictureMerge.newer(asset: update.iconAsset, url: update.iconUrl, over: iconAsset, iconUrl)
        return Space(id: id, name: update.name, ownerId: ownerId, demo: demo, inviter: inviter, iconAsset: icon.asset, iconUrl: icon.url,
                     canEditAppearance: update.canEditAppearance ?? canEditAppearance)
    }
}

extension SpaceDetail {
    func applying(_ update: Space) -> SpaceDetail {
        guard space.id == update.id else { return self }
        return with(space: space.applying(update))
    }
}

extension FeedbackVisit {
    /// A visit's title follows its space's name.
    func applying(_ update: Space) -> FeedbackVisit {
        guard !direct, spaceID == update.id else { return self }
        return FeedbackVisit(channelID: channelID, spaceID: spaceID, spaceName: update.name, channelName: channelName, direct: direct,
                             reportID: reportID, reporter: reporter)
    }
}

extension FeedbackReport {
    /// An inbox row's space name follows the space.
    func applying(_ update: Space) -> FeedbackReport {
        guard let space, space.id == update.id else { return self }
        return FeedbackReport(id: id, messageId: messageId, threadRootId: threadRootId, createdAt: createdAt, text: text,
                              attachments: attachments, reporter: reporter, channel: channel,
                              space: SpaceReference(id: space.id, name: update.name), replyCount: replyCount,
                              answered: answered, archived: archived)
    }
}
