import Foundation

/// Keyboard navigation shared by the quick switcher (⌘K) and stepping through
/// the sidebar's conversations (⌥↑/↓, with ⇧ only the unread ones). Every
/// client implements the same matching and stepping, checked against
/// `shared/shortcuts/switcher-cases.json`.
public enum KeyboardNavigation {
    private static func word(_ scalar: Unicode.Scalar) -> Bool {
        CharacterSet.letters.contains(scalar) || CharacterSet.decimalDigits.contains(scalar)
    }

    /// How well `query` matches `text`: higher is better, nil for no match, 0 for an empty query.
    public static func score(_ query: String, _ text: String) -> Int? {
        var trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix("#") || trimmed.hasPrefix("@") { trimmed.removeFirst() }
        let q = Array(trimmed.lowercased().unicodeScalars)
        if q.isEmpty { return 0 }
        let lower = text.lowercased()
        let joined = String(String.UnicodeScalarView(q))
        if lower == joined { return 1000 }
        if lower.hasPrefix(joined) { return 900 }
        let t = Array(lower.unicodeScalars)
        func wordStart(_ index: Int) -> Bool { index == 0 || !word(t[index - 1]) }
        if t.count >= q.count {
            for index in 0...(t.count - q.count) where wordStart(index) && Array(t[index..<index + q.count]) == q {
                return 800
            }
        }
        if lower.contains(joined) { return 700 }
        var score = 100, last = -2, from = 0
        for scalar in q {
            guard from <= t.count, let index = t[from...].firstIndex(of: scalar) else { return nil }
            if index == last + 1 { score += 10 }
            if wordStart(index) { score += 5 }
            score -= index - from
            last = index; from = index + 1
        }
        return min(699, max(1, score))
    }

    /// The indexes of the items that match, best first; unread ones lead among equals.
    public static func rank(_ items: [(texts: [String], unread: Bool)], query: String) -> [Int] {
        items.enumerated().compactMap { index, item -> (Int, Int)? in
            guard let best = item.texts.compactMap({ score(query, $0) }).max() else { return nil }
            return (best + (item.unread ? 50 : 0), index)
        }
        .sorted { $0.0 != $1.0 ? $0.0 > $1.0 : $0.1 < $1.1 }
        .map(\.1)
    }

    /// The conversation ⌥↑/↓ (or with `unreadOnly`, ⌥⇧↑/↓) moves to from
    /// `current`, wrapping; nil when there is none.
    public static func adjacent(unread: [Bool], current: Int?, forward: Bool, unreadOnly: Bool) -> Int? {
        let count = unread.count
        for step in 0..<count {
            let index: Int
            if let current {
                index = ((current + (forward ? step + 1 : -step - 1)) % count + count) % count
            } else {
                index = forward ? step : count - 1 - step
            }
            if index == current { return nil }
            if !unreadOnly || unread[index] { return index }
        }
        return nil
    }

    /// The keyboard shortcuts sheet's sections, in Apple's key names. Settings
    /// and End exist only on the Mac.
    public static let shortcuts: [(title: String, rows: [(action: String, keys: String)])] = {
        var navigation: [(action: String, keys: String)] = [
            ("Quick switcher", "⌘K"),
            ("Previous or next conversation", "⌥↑ / ⌥↓"),
            ("Previous or next unread conversation", "⌥⇧↑ / ⌥⇧↓"),
        ]
        var conversation: [(action: String, keys: String)] = [
            ("Mark as read", "Esc"),
            ("Edit your last message (in an empty message box)", "↑"),
        ]
        #if os(macOS)
        navigation.append(("Settings", "⌘,"))
        conversation.append(("Jump to latest (from the messages)", "End"))
        conversation.append(("New line", "⇧Return"))
        #endif
        navigation.append(("Keyboard shortcuts", "⌘/"))
        conversation.append(("Bold, italic, underline, code", "⌘B / ⌘I / ⌘U / ⌘E"))
        return [
            ("Navigation", navigation),
            ("Conversation", conversation),
            ("Voice", [
                ("Mute or unmute (in a call)", "⌘⇧M"),
                ("Deafen or undeafen (in a call)", "⌘⇧D"),
            ]),
        ]
    }()
}

/// A quick switcher destination.
public struct SwitcherItem: Identifiable, Sendable {
    public enum Target: Sendable {
        case channel(spaceID: String, channelID: String)
        case direct(String)
        case space(String)
        /// Someone you share a space with but have no DM with: opening starts one.
        case person(username: String)
    }
    public let id: String
    public let name: String
    public let alternates: [String]
    /// The second line: the space, a username, or "Space".
    public let detail: String
    public let unread: Bool
    public let target: Target
}

@MainActor extension AppModel {
    private func channelIsUnread(spaceID: String, channelID: String) -> Bool {
        unreadSpaces.contains { $0.id == spaceID && $0.channels.contains { $0.id == channelID } }
    }

    private func directListedUnread(_ direct: DirectMessageConversation) -> Bool {
        direct.unread && !blockedIDs.contains(direct.peer.id) && !notificationsMuted(.direct(direct.id))
    }

    /// Every quick switcher destination: the open space's joined channels,
    /// then other spaces' (`switcherSpaces`), DMs (requests count as unread),
    /// spaces, then people without a DM.
    public func switcherItems() -> [SwitcherItem] {
        var items: [SwitcherItem] = []
        let open = detail.flatMap { $0.space.id.isEmpty ? nil : $0 }
        var seen = Set<String>()
        for space in (open.map { [$0] } ?? []) + switcherSpaces.filter({ $0.space.id != open?.space.id }) {
            for channel in space.channels where channel.joined && seen.insert(channel.id).inserted {
                items.append(SwitcherItem(id: "channel:\(channel.id)", name: channel.name, alternates: [], detail: space.space.name,
                    unread: channelIsUnread(spaceID: space.space.id, channelID: channel.id),
                    target: .channel(spaceID: space.space.id, channelID: channel.id)))
            }
        }
        for direct in directMessages {
            let own = direct.peer.id == account?.id
            items.append(SwitcherItem(id: "dm:\(direct.id)", name: own ? "\(direct.peer.displayName) (you)" : direct.peer.displayName,
                alternates: [direct.peer.username], detail: "@\(direct.peer.username)",
                unread: direct.status == .incoming || directListedUnread(direct), target: .direct(direct.id)))
        }
        for space in spaces where space.demo != true {
            items.append(SwitcherItem(id: "space:\(space.id)", name: space.name, alternates: [], detail: "Space",
                unread: unreadSpaces.contains { $0.id == space.id }, target: .space(space.id)))
        }
        let peers = Set(directMessages.map(\.peer.id))
        for person in people ?? [] where person.id != account?.id && !peers.contains(person.id) && !blockedIDs.contains(person.id) {
            items.append(SwitcherItem(id: "person:\(person.id)", name: person.displayName, alternates: [person.username],
                detail: "@\(person.username)", unread: false, target: .person(username: person.username)))
        }
        return items
    }

    /// Other spaces' channels for the switcher that just opened, as on web:
    /// one request per space, each listed as it arrives; failures are skipped.
    public func loadSwitcherSpaces() async {
        switcherSpaces = []
        let attempt = switcherGeneration
        let open = detail?.space.id
        let ids = spaces.filter { $0.demo != true && $0.id != open }.map(\.id)
        await withTaskGroup(of: SpaceDetail?.self) { group in
            for id in ids { group.addTask { [api] in try? await api.space(id) } }
            for await loaded in group {
                guard let loaded, switcherGeneration == attempt else { continue }
                switcherSpaces.removeAll { $0.space.id == loaded.space.id }
                switcherSpaces.append(loaded)
            }
        }
    }

    public func rankedSwitcherItems(_ items: [SwitcherItem], query: String) -> [SwitcherItem] {
        KeyboardNavigation.rank(items.map { ([$0.name] + $0.alternates, $0.unread) }, query: query).prefix(50).map { items[$0] }
    }

    public func open(_ item: SwitcherItem) async {
        switch item.target {
        case .channel(let spaceID, let channelID):
            if spaceID == detail?.space.id, let channel = detail?.channels.first(where: { $0.id == channelID }) {
                await select(channel: channel)
            } else {
                await openChannel(spaceID: spaceID, channelID: channelID)
            }
        case .direct(let id):
            if let direct = directMessages.first(where: { $0.id == id }) { await select(directMessage: direct) }
        case .space(let id):
            if let space = spaces.first(where: { $0.id == id }) { await select(space: space) }
        case .person(let username):
            do { _ = try await createDirectMessage(username: username) }
            catch { self.error = FriendlyError.message(for: error) }
        }
    }

    /// ⌥↑/↓: the previous or next conversation in the sidebar's order (the
    /// open space's joined channels, then your notes and DMs); with
    /// `unreadOnly`, the previous or next unread one.
    public func stepConversation(forward: Bool, unreadOnly: Bool) async {
        var order: [(item: SwitcherItem, unread: Bool)] = []
        if let detail, !detail.space.id.isEmpty {
            for channel in detail.channels where channel.joined {
                let unread = channelIsUnread(spaceID: detail.space.id, channelID: channel.id)
                order.append((SwitcherItem(id: channel.id, name: channel.name, alternates: [], detail: "", unread: unread,
                    target: .channel(spaceID: detail.space.id, channelID: channel.id)), unread))
            }
        }
        let own = directMessages.filter { $0.peer.id == account?.id }
        let others = visibleDirectMessages.filter { $0.peer.id != account?.id }
        for direct in own + others where direct.status != .incoming {
            let unread = directListedUnread(direct)
            order.append((SwitcherItem(id: direct.id, name: "", alternates: [], detail: "", unread: unread, target: .direct(direct.id)), unread))
        }
        let current = order.firstIndex { entry in
            switch entry.item.target {
            case .channel: selectedDirectMessageID == nil && selectedChannelID == entry.item.id
            case .direct(let id): selectedDirectMessageID == id
            default: false
            }
        }
        guard let next = KeyboardNavigation.adjacent(unread: order.map(\.unread), current: current, forward: forward, unreadOnly: unreadOnly)
        else { return }
        await open(order[next].item)
    }

    /// In a call (⌘⇧M / ⌘⇧D apply).
    public var inVoiceCall: Bool { voice.phase == .connected }
}
