import Foundation

/// Unsent composer text kept on this device across launches and updates, as
/// web keeps it in `localStorage`: per account, conversation drafts by
/// channel or DM ID and thread replies by root message ID. Logout removes the
/// account's, so another account on the device never sees them. Text only;
/// staged files and reply targets stay in memory. The text also syncs with
/// the account's other devices (`DraftSync`); this copy stays the instant,
/// offline source.
struct SavedDrafts: Codable, Equatable {
    var conversations: [String: String] = [:]
    var threads: [String: String] = [:]
    /// Each thread draft's conversation, so a draft typed before sync (or
    /// offline) can be saved to the server. Optional: older builds never wrote it.
    var threadChannels: [String: String]?
    /// `DraftKey.id`s edited here and not yet saved to the server, so a relaunch
    /// keeps them over an older server copy and saves them on the next load.
    var dirty: [String]?

    static func key(accountID: String) -> String { "caper.drafts.\(accountID)" }

    /// Missing or corrupt storage means no drafts.
    static func load(accountID: String, defaults: UserDefaults) -> SavedDrafts {
        guard !accountID.isEmpty, let data = defaults.data(forKey: key(accountID: accountID)),
              let saved = try? JSONDecoder().decode(SavedDrafts.self, from: data) else { return SavedDrafts() }
        return saved
    }

    /// Blank drafts are dropped (a dirty one keeps only its mark, so the clear
    /// still reaches the server); with nothing left the entry goes.
    static func save(_ drafts: SavedDrafts, accountID: String, defaults: UserDefaults) {
        guard !accountID.isEmpty else { return }
        let threads = drafts.threads.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        let dirty = drafts.dirty ?? []
        let dirtyRoots = Set(dirty.compactMap { DraftKey(id: $0).threadRootID })
        let channels = (drafts.threadChannels ?? [:]).filter { threads[$0.key] != nil || dirtyRoots.contains($0.key) }
        let kept = SavedDrafts(conversations: drafts.conversations.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty },
                               threads: threads, threadChannels: channels.isEmpty ? nil : channels,
                               dirty: dirty.isEmpty ? nil : dirty.sorted())
        if kept.conversations.isEmpty && kept.threads.isEmpty && kept.dirty == nil {
            defaults.removeObject(forKey: key(accountID: accountID))
        } else if let data = try? JSONEncoder().encode(kept) {
            defaults.set(data, forKey: key(accountID: accountID))
        }
    }

    static func remove(accountID: String, defaults: UserDefaults) {
        guard !accountID.isEmpty else { return }
        defaults.removeObject(forKey: key(accountID: accountID))
    }
}

/// One synced draft: a conversation's composer (`threadRootID` nil) or a
/// thread's reply composer.
struct DraftKey: Hashable, Sendable {
    let channelID: String
    let threadRootID: String?

    init(channelID: String, threadRootID: String?) {
        self.channelID = channelID
        self.threadRootID = threadRootID
    }

    /// `channel` or `channel root`, as web stores it.
    var id: String { threadRootID.map { "\(channelID) \($0)" } ?? channelID }

    init(id: String) {
        let parts = id.split(separator: " ", maxSplits: 1).map(String.init)
        channelID = parts.first ?? ""
        threadRootID = parts.count > 1 ? parts[1] : nil
    }
}

/// A draft as the server keeps it (`GET`/`PUT /api/drafts`, `draft.updated`).
/// `updatedAt` is fixed-width UTC with microseconds, so two compare as strings.
public struct ServerDraft: Codable, Equatable, Sendable {
    public let channelId: String
    public let threadRootId: String?
    public let text: String
    public let updatedAt: String

    var key: DraftKey { DraftKey(channelID: channelId, threadRootID: threadRootId) }
}

struct ServerDrafts: Decodable { let drafts: [ServerDraft] }

/// The cross-device rules for drafts (docs/media.md, "Draft sync"), without
/// timers or storage: `ChatModel` debounces saves, writes the device copy and
/// updates the composer on screen.
struct DraftSync: Equatable {
    /// Edited here and not saved yet (rule 1). Server drafts never replace these.
    var dirty: Set<DraftKey> = []
    /// The newest server `updatedAt` applied (or saved) per draft (rule 2).
    var applied: [DraftKey: String] = [:]
    /// One save per draft at a time; `again` asks for another once it finishes.
    var saving: Set<DraftKey> = []
    var again: Set<DraftKey> = []

    /// Rule 2: whether a server draft replaces this device's copy. Accepting it
    /// records its `updatedAt`, so an older or repeated one changes nothing.
    mutating func accept(_ draft: ServerDraft) -> Bool {
        let key = draft.key
        if dirty.contains(key) { return false }
        if let last = applied[key], draft.updatedAt <= last { return false }
        applied[key] = draft.updatedAt
        return true
    }

    /// A save's response counts as applied, so its own `draft.updated` echo
    /// changes nothing. `unchanged`: no edit arrived while it was in flight.
    mutating func saved(_ draft: ServerDraft, unchanged: Bool) {
        let key = draft.key
        if applied[key].map({ draft.updatedAt > $0 }) ?? true { applied[key] = draft.updatedAt }
        if unchanged { dirty.remove(key) }
    }

    /// Rule 3: non-empty device drafts the server has no entry for, to save
    /// after `GET`.
    static func deviceOnly(_ device: [DraftKey: String], server: [ServerDraft]) -> Set<DraftKey> {
        let known = Set(server.map(\.key))
        return Set(device.filter { !$0.value.isEmpty && !known.contains($0.key) }.keys)
    }
}
