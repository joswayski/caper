import Foundation
import Observation

/// What the mention card shows for a person's pill, resolved locally.
struct MentionCardPerson: Equatable, Sendable {
    var admin: Bool = false
    var team: Bool = false
    var canBlock: Bool { !admin && !team }
    let id: String
    let username: String?
    /// nil when nobody you know matches: the card then titles itself
    /// `@username` and drops the second line.
    let displayName: String?
    let avatarId: Int?
    /// The signed-in account: a muted "You" line instead of **Message**.
    let isSelf: Bool
    /// Their custom picture over the mascot, when known.
    var picture: CaperPicture? = nil

    var title: String { displayName ?? username.map { "@\($0)" } ?? "Profile" }
    var subtitle: String? { displayName == nil ? nil : username.map { "@\($0)" } }
    /// The generic initial avatar uses the username when the name is unknown.
    var avatarName: String { displayName ?? username ?? title }
}

/// Clicking a person's `@mention` pill opens a profile with an inline composer.
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
    /// account; otherwise the unknown fallback (DM creation is by username).
    static func resolve(_ pill: MentionPill, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                        account: Account?, viewerID: String?) -> MentionCardPerson {
        resolve(id: pill.id, username: pill.username, members: members, people: people, peers: peers, account: account, viewerID: viewerID)
    }

    /// Pin metadata has a real identity but no username. Never match by name or invent a DM recipient.
    static func resolve(_ author: ChatAuthor, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                        account: Account?, viewerID: String?) -> MentionCardPerson {
        let person = resolve(id: author.id, username: nil, members: members, people: people, peers: peers, account: account, viewerID: viewerID)
        return MentionCardPerson(admin: author.isAdmin || person.admin, team: person.team, id: author.id, username: person.username, displayName: person.displayName ?? author.name,
                                 avatarId: person.username == nil ? author.avatarId : person.avatarId, isSelf: person.isSelf,
                                 picture: person.username == nil ? author.avatarPicture : person.picture)
    }

    private static func resolve(id: String, username: String?, members: [Member], people: [Person]?, peers: [DirectMessagePeer],
                                account: Account?, viewerID: String?) -> MentionCardPerson {
        let isSelf = (viewerID ?? account?.id) == id
        let admin = members.contains { $0.id == id && $0.admin == true }
            || people?.contains { $0.id == id && $0.admin == true } == true
            || peers.contains { $0.id == id && $0.admin == true }
        let team = peers.contains { $0.id == id && $0.isTeam }
        func person(_ knownUsername: String?, _ displayName: String?, _ avatarId: Int?, _ picture: CaperPicture?) -> MentionCardPerson {
            let name = displayName?.trimmingCharacters(in: .whitespacesAndNewlines)
            return MentionCardPerson(admin: admin, team: team, id: id, username: knownUsername?.isEmpty == false ? knownUsername : username,
                                     displayName: name?.isEmpty == false ? name : nil, avatarId: avatarId, isSelf: isSelf, picture: picture)
        }
        if let member = members.first(where: { $0.id == id }) {
            return person(member.username, member.displayName, member.avatarId, member.avatarPicture)
        }
        if let known = people?.first(where: { $0.id == id }) {
            return person(known.username, known.displayName, known.avatarId, known.avatarPicture)
        }
        if let peer = peers.first(where: { $0.id == id }) {
            // DM peers carry their saved mascot and picture, as on web.
            return person(peer.username, peer.displayName, peer.avatarId, peer.avatarPicture)
        }
        if let account, account.id == id {
            return person(account.username, account.displayName, account.avatarId, account.avatarPicture)
        }
        return MentionCardPerson(id: id, username: username, displayName: nil, avatarId: nil, isSelf: isSelf)
    }
}

/// Account-scoped, per-person profile drafts with eager, coalesced uploads.
@MainActor @Observable
final class ProfileMessageDraft {
    struct File: Identifiable {
        let id = UUID().uuidString
        let local: LocalAttachmentFile
        var prepared: PreparedAttachment?
        var attachment: ChatAttachment?
        var uploadedAt: Date?
        var spoiler = false
        var uploadError: String?
        var progress: Double?
        /// The editor can open it (a still or video this device can change).
        var editable = false
        /// Applied to `local` when Send prepares it.
        var edit = AttachmentEdit.original
        /// A small render of `edit` for the chip.
        var editPreview: URL?
        /// A staged voice note's length and waveform (its `prepared` is set at once).
        var voiceDurationMs: Int?
        var voiceWaveform: [UInt8]?
    }

    var text = ""
    private(set) var files: [File] = []
    var error: String?
    var sending = false
    var presentingDialog = false
    var progress: String?
    var conversation: DirectMessageConversation?
    /// Once a POST may have reached the server, freeze its entire payload,
    /// including asset ids, until a retry confirms it or it is definitively refused.
    var pending: PendingMessage?
    /// A recorded voice message sent on its own to this person (the text and
    /// files above stay). Kept after a failed attempt for Retry: its upload,
    /// and once a POST may have reached the server, the frozen send.
    struct Voice {
        let prepared: PreparedAttachment
        var attachment: ChatAttachment?
        var uploadedAt: Date?
        var command: PendingMessage?
    }
    private(set) var voice: Voice?
    private(set) var discarded = false
    var prepareUploads: (() async throws -> Void)?
    private var uploadTask: Task<Void, Error>?
    private var uploadGeneration = UUID()
    private var uploadAPI: APIClient?

    func startUploads() {
        guard !discarded, pending == nil, uploadTask == nil, !files.isEmpty,
              let prepareUploads else { return }
        let generation = uploadGeneration
        uploadTask = Task { [weak self] in
            defer {
                if self?.uploadGeneration == generation {
                    self?.uploadTask = nil
                    self?.progress = nil
                }
            }
            do { try await prepareUploads() }
            catch {
                if let self, self.uploadGeneration == generation, !self.discarded, !(error is CancellationError) {
                    self.error = FriendlyError.message(for: error)
                }
                throw error
            }
        }
    }

    func awaitUploads() async throws {
        if let uploadTask { try await uploadTask.value }
    }

    private func invalidateUploads() {
        uploadGeneration = UUID()
        uploadTask?.cancel()
        uploadTask = nil
    }

    private func cancelAsset(_ file: File) {
        if let id = file.attachment?.id, let api = uploadAPI { Task { await api.cancelAsset(id: id) } }
    }
    var editable: Bool { !discarded && !sending && pending == nil }
    var preventsImplicitDismissal: Bool { sending || presentingDialog }
    var canSend: Bool {
        !discarded && !sending && (pending != nil || MessageValidation.error(for: text, attachmentCount: files.count) == nil)
    }
    /// The microphone starts only while nothing else is in flight or awaiting Retry.
    var canRecordVoice: Bool { !discarded && !sending && pending == nil && voice == nil }

    /// Staging (the default): one voice note per message, within the file limit.
    var canStageVoice: Bool {
        editable && VoiceMessage.canStage(staged: files.count, voiceStaged: files.contains { $0.voiceDurationMs != nil },
                                          limit: AttachmentPolicy.maxAttachments)
    }

    /// Stages a stopped recording among the card's files, to go with the next
    /// Send; it uploads now, eagerly like the other files. False (and the
    /// recording removed) when it can't be staged.
    @discardableResult
    func stageVoice(_ recording: VoiceRecording) -> Bool {
        guard canStageVoice, let prepared = PreparedAttachment.voice(recording) else {
            AttachmentStaging.remove(recording.fileURL)
            return false
        }
        var file = File(local: LocalAttachmentFile(url: recording.fileURL, name: VoiceMessage.label,
                                                   contentType: VoiceMessage.uploadContentType, size: prepared.byteSize))
        file.prepared = prepared
        file.voiceDurationMs = recording.durationMs
        file.voiceWaveform = recording.waveform
        files.append(file)
        error = nil
        startUploads()
        return true
    }

    /// Holds a finished recording for `AppModel.sendProfileVoice`; false
    /// (and the recording removed) when another voice message is still here.
    func stageVoice(_ prepared: PreparedAttachment) -> Bool {
        guard !discarded, voice == nil else { AttachmentStaging.remove(prepared.fileURL); return false }
        voice = Voice(prepared: prepared)
        return true
    }

    func updateVoice(_ change: (inout Voice) -> Void, api: APIClient) {
        guard var value = voice else { return }
        uploadAPI = api
        change(&value)
        voice = value
    }

    /// Drops the voice message: its upload is cancelled unless a send may
    /// have used it, and the recording is removed.
    func discardVoice() {
        guard let voice else { return }
        if voice.command == nil, let id = voice.attachment?.id, let api = uploadAPI { Task { await api.cancelAsset(id: id) } }
        AttachmentStaging.remove(voice.prepared.fileURL)
        self.voice = nil
    }

    /// After the server confirmed it: only the local recording goes.
    func finishVoice() {
        if let voice { AttachmentStaging.remove(voice.prepared.fileURL) }
        voice = nil
    }

    func add(_ incoming: [LocalAttachmentFile]) {
        guard editable else { incoming.forEach { AttachmentStaging.remove($0.url) }; return }
        let room = max(0, AttachmentPolicy.maxAttachments - files.count)
        let added = incoming.prefix(room).map { File(local: $0) }
        files.append(contentsOf: added)
        incoming.dropFirst(room).forEach { AttachmentStaging.remove($0.url) }
        error = incoming.count > room ? "You can attach up to \(AttachmentPolicy.maxAttachments) files." : nil
        for file in added {
            Task { [weak self] in
                let editable = await AttachmentPreparer.canEdit(file.local)
                guard let self, let index = self.files.firstIndex(where: { $0.id == file.id }) else { return }
                self.files[index].editable = editable
            }
        }
        startUploads()
    }

    func remove(id: String) {
        guard editable, let file = files.first(where: { $0.id == id }) else { return }
        invalidateUploads()
        cancelAsset(file)
        Self.removeFiles(of: file)
        files.removeAll { $0.id == id }
        error = nil
        startUploads()
    }

    /// The editor's change to a staged file starts a replacement upload.
    /// Anything prepared or uploaded before is dropped, as Remove drops it.
    func setEdit(id: String, edit: AttachmentEdit, preview: URL?) {
        guard editable, let index = files.firstIndex(where: { $0.id == id }), files[index].editable, files[index].edit != edit else {
            if let preview { AttachmentStaging.remove(preview) }
            return
        }
        invalidateUploads()
        cancelAsset(files[index])
        if let prepared = files[index].prepared, prepared.fileURL != files[index].local.url { AttachmentStaging.remove(prepared.fileURL) }
        if let old = files[index].editPreview { AttachmentStaging.remove(old) }
        files[index].prepared = nil
        files[index].attachment = nil
        files[index].uploadError = nil
        files[index].progress = nil
        files[index].edit = edit
        files[index].editPreview = edit.isEdited ? preview : nil
        if !edit.isEdited, let preview { AttachmentStaging.remove(preview) }
        error = nil
        startUploads()
    }

    private static func removeFiles(of file: File) {
        AttachmentStaging.remove(file.local.url)
        if let prepared = file.prepared { AttachmentStaging.remove(prepared.fileURL) }
        if let preview = file.editPreview { AttachmentStaging.remove(preview) }
    }

    func checkActive() throws {
        try Task.checkCancellation()
        if discarded { throw CancellationError() }
    }

    func toggleSpoiler(id: String) {
        guard editable, let index = files.firstIndex(where: { $0.id == id }) else { return }
        files[index].spoiler.toggle()
    }

    private var attachmentsToSend: [ChatAttachment] {
        files.compactMap { file in
            guard var attachment = file.attachment else { return nil }
            attachment.spoiler = file.spoiler
            return attachment
        }
    }

    func upload(channelID: String, api: APIClient) async throws -> [ChatAttachment] {
        uploadAPI = api
        let generation = uploadGeneration
        func current(_ id: String) throws -> Int {
            try checkActive()
            guard uploadGeneration == generation, let index = files.firstIndex(where: { $0.id == id }) else { throw CancellationError() }
            return index
        }
        guard !files.isEmpty else { return [] }
        // Renew before the unsent-asset cleanup, but never mutate an uncertain POST.
        if pending == nil {
            for index in files.indices where files[index].uploadedAt.map({ Date().timeIntervalSince($0) >= 23 * 60 * 60 }) == true {
                cancelAsset(files[index])
                files[index].attachment = nil
                files[index].uploadedAt = nil
            }
        }
        if files.allSatisfy({ $0.attachment != nil }) { return attachmentsToSend }
        let usage = try await api.assetUsage()
        try checkActive()
        while let id = files.first(where: { $0.attachment == nil })?.id {
            var index = try current(id)
            if files[index].attachment != nil { continue }
            let file = files[index].local
            if files[index].prepared == nil {
                progress = "Preparing \(file.name)…"
                // An edit that can't be applied stops the send; the original never goes instead.
                guard let prepared = try? await AttachmentPreparer.prepare(file, edit: files[index].edit, settings: usage.compression) else {
                    index = try current(id)
                    files[index].uploadError = AttachmentEdit.failureMessage
                    throw UserFacingError(message: AttachmentEdit.failureMessage)
                }
                do { index = try current(id) } catch {
                    if prepared.fileURL != file.url { AttachmentStaging.remove(prepared.fileURL) }
                    throw error
                }
                files[index].prepared = prepared
            }
            let remaining = files.filter { $0.attachment == nil }.count
            progress = "Uploading \(remaining) \(remaining == 1 ? "file" : "files")… Your message won’t send until every file is ready."
            files[index].uploadError = nil
            files[index].progress = 0
            let attachment: ChatAttachment
            do {
                attachment = try await AttachmentUploader.upload(files[index].prepared!, channelID: channelID, api: api) { fraction in
                    Task { @MainActor [weak self] in
                        guard let self, !self.discarded, self.uploadGeneration == generation,
                              let current = self.files.firstIndex(where: { $0.id == id }), self.files[current].progress != nil else { return }
                        self.files[current].progress = fraction
                    }
                }
            } catch {
                if !discarded, uploadGeneration == generation, let current = files.firstIndex(where: { $0.id == id }) {
                    files[current].progress = nil
                    if !(error is CancellationError) { files[current].uploadError = "Failed to upload." }
                }
                throw error
            }
            do { index = try current(id) } catch {
                await api.cancelAsset(id: attachment.id)
                throw error
            }
            files[index].attachment = attachment
            files[index].uploadedAt = Date()
            files[index].progress = nil
        }
        return attachmentsToSend
    }

    func clear() {
        invalidateUploads()
        if pending == nil { files.forEach(cancelAsset) }
        files.forEach(Self.removeFiles)
        text = ""; files = []; pending = nil; conversation = nil; error = nil; progress = nil
    }

    func discard() {
        discarded = true
        presentingDialog = false
        discardVoice()
        clear()
    }
}
