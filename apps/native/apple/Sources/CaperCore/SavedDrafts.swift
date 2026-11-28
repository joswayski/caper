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
public struct ServerDraft: Decodable, Equatable, Sendable {
    public let channelId: String
    public let threadRootId: String?
    public let text: String
    /// Its staged files, in order, in the message attachment shape with signed
    /// URLs and spoiler marks. Nil from a server without file sync: files stay.
    public var attachments: [ChatAttachment]? = nil
    public let updatedAt: String

    var key: DraftKey { DraftKey(channelID: channelId, threadRootID: threadRootId) }

    private enum CodingKeys: String, CodingKey { case channelId, threadRootId, text, attachments, updatedAt }
}

extension ServerDraft {
    /// A malformed file is skipped, never the draft.
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        channelId = try values.decode(String.self, forKey: .channelId)
        threadRootId = try values.decodeIfPresent(String.self, forKey: .threadRootId)
        text = try values.decode(String.self, forKey: .text)
        attachments = values.decodeLossyAttachments(forKey: .attachments)
        updatedAt = try values.decode(String.self, forKey: .updatedAt)
    }
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
    /// Drafts whose files this device has (a server draft listed them, or
    /// they changed here). Saves of other drafts leave `attachmentIds` out, so
    /// a relaunch (staged files live in memory) never drops another device's.
    var filesKnown: Set<DraftKey> = []

    /// Rule 2: whether a server draft replaces this device's copy. Accepting it
    /// records its `updatedAt`, so an older or repeated one changes nothing.
    mutating func accept(_ draft: ServerDraft) -> Bool {
        let key = draft.key
        if dirty.contains(key) { return false }
        if let last = applied[key], draft.updatedAt <= last { return false }
        applied[key] = draft.updatedAt
        if draft.attachments != nil { filesKnown.insert(key) }
        return true
    }

    /// A save's response counts as applied, so its own `draft.updated` echo
    /// changes nothing. `unchanged`: no edit arrived while it was in flight.
    /// True when the response lists files this device didn't know of yet
    /// (the save left them out), so it should show them.
    @discardableResult
    mutating func saved(_ draft: ServerDraft, unchanged: Bool) -> Bool {
        let key = draft.key
        if applied[key].map({ draft.updatedAt > $0 }) ?? true { applied[key] = draft.updatedAt }
        if unchanged { dirty.remove(key) }
        guard draft.attachments != nil else { return false }
        return filesKnown.insert(key).inserted
    }

    /// Rule 3: non-empty device drafts the server has no entry for, to save
    /// after `GET`.
    static func deviceOnly(_ device: [DraftKey: String], server: [ServerDraft]) -> Set<DraftKey> {
        let known = Set(server.map(\.key))
        return Set(device.filter { !$0.value.isEmpty && !known.contains($0.key) }.keys)
    }

    /// Rule 4: what a save sends for a composer's files, the staged files
    /// whose upload completed in tile order, and those marked as spoilers.
    static func fileIDs(_ files: [AttachmentDraft]) -> (ids: [String], spoilers: [String]) {
        let ready = files.filter { $0.attachment != nil }
        return (ready.compactMap { $0.attachment?.id }, ready.filter(\.spoiler).compactMap { $0.attachment?.id })
    }

    /// Rule 4: a composer's tiles after applying a server draft's files. Listed
    /// files keep their tile (a local one, or a synced one shown before; its
    /// spoiler mark from the server), others arrive as synced tiles; this
    /// device's files still preparing, uploading or failed stay after them.
    /// `dropped` are ready tiles the server no longer lists, to release as
    /// Remove does.
    static func mergeFiles(local: [AttachmentDraft], server: [ChatAttachment],
                           newID: () -> String = { UUID().uuidString }) -> (files: [AttachmentDraft], dropped: [AttachmentDraft]) {
        var files: [AttachmentDraft] = []
        var kept = Set<String>()
        for attachment in server where !kept.contains(attachment.id) {
            if var tile = local.first(where: { $0.attachment?.id == attachment.id }) {
                tile.spoiler = attachment.spoiler
                // A synced tile takes the newer signed URLs (a downloaded one keeps its local copy).
                if tile.synced { tile.refreshSynced(attachment) }
                files.append(tile); kept.insert(attachment.id)
            } else if let tile = AttachmentDraft(synced: attachment, id: newID()) {
                files.append(tile); kept.insert(attachment.id)
            }
        }
        files += local.filter { $0.attachment == nil }
        let dropped = local.filter { tile in tile.attachment.map { !kept.contains($0.id) } ?? false }
        return (files, dropped)
    }
}

extension AttachmentDraft {
    /// A file a server draft lists that this device didn't pick (or no longer
    /// has): a ready tile from its signed URLs, with no upload and no local
    /// original. It sends by its ID; Edit downloads it first. Nil without a URL.
    init?(synced attachment: ChatAttachment, id: String) {
        guard !attachment.unavailable, let url = attachment.url.flatMap({ URL(string: $0) }) else { return nil }
        self.init(id: id, name: attachment.name, kind: attachment.kind, localURL: url, sourceSize: attachment.size)
        synced = true
        preparing = false
        progress = 1
        storedSize = attachment.size
        spoiler = attachment.spoiler
        editable = !attachment.voice && !attachment.animated && (attachment.kind == .image || attachment.kind == .video)
        if attachment.voice {
            voiceDurationMs = attachment.durationMs ?? 0
            voiceWaveform = attachment.waveformBytes
        }
        refreshSynced(attachment)
    }

    /// The server's copy of a synced tile: its attachment and, until it is
    /// downloaded for editing, the URLs it shows from.
    mutating func refreshSynced(_ server: ChatAttachment) {
        var copy = server
        copy.spoiler = spoiler
        attachment = copy
        guard original == nil else { return }
        if let url = server.url.flatMap({ URL(string: $0) }) { localURL = url }
        // The server's still preview is the thumbnail, for videos too.
        editPreviewURL = server.hasPreview && server.kind != .audio ? server.previewUrl.flatMap({ URL(string: $0) }) : nil
    }
}

/// What one save sends: the text and, when this device knows the draft's
/// files (`DraftSync.filesKnown`), its ready files in order and their spoiler
/// marks. A save whose content still matches when it returns leaves the draft clean.
struct DraftContent: Equatable, Sendable {
    let text: String
    let attachmentIDs: [String]?
    let spoilerAttachmentIDs: [String]?
}
