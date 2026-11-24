#if os(iOS)
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Native picker, drawer, editor and viewer factories for the UIKit client.
/// Presentation ownership stays with the calling conversation or workspace.
@MainActor public enum UIKitHosting {
    public typealias Present = @MainActor (UIViewController) -> Void

    /// The composer's attach drawer: recent photos and videos, the camera,
    /// Files, and Send. Nil when the conversation can't take files. With a
    /// `presenter` it docks in the keyboard's place (the composer sends, and
    /// `close` hides it); otherwise it is a sheet with its own Send.
    public static func attachDrawer(chat: ChatModel, inThread: Bool = false, presenter: (() -> UIViewController?)? = nil,
                                    dockInset: CGFloat = 0, close: (() -> Void)? = nil) -> UIViewController? {
        guard let target = chat.attachmentTarget(inThread: inThread) else { return nil }
        let draft = AttachDrawerDraft(
            files: { chat.attachmentFiles(inThread: inThread).map { AttachDrawerFile(id: $0.id, source: $0.source) } },
            add: { chat.addAttachments($0, to: target) },
            remove: { chat.removeAttachmentDraft(id: $0) },
            canSend: { chat.canPressSend(inThread: inThread) },
            send: { Task { await chat.send(inThread: inThread) } },
            valid: { chat.canAttach && chat.attachmentTarget(inThread: inThread) == target })
        let box = WeakController()
        let host = UIKitAttachDrawer(mode: .attach(draft), presenter: presenter, dockInset: dockInset) { if let close { close() } else { box.value?.dismiss(animated: false) } }
        box.value = host
        return host
    }

    /// Prepares the drawer's gallery while the composer is up.
    public static func warmAttachGallery() { AttachGalleryCache.warm() }

    /// Emojis, GIFs and stickers. An emoji goes to `insert`; a GIF or sticker sends at once.
    public static func expressionPicker(chat: ChatModel, inThread: Bool = false, insert: @escaping (String) -> Void) -> UIViewController {
        let box = WeakController(), context = chat.editingContext, root = chat.threadRootID
        let close: () -> Void = { box.value?.dismiss(animated: false) }
        let host = UIKitExpressionPicker(configuration: chat.klipy, mediaEnabled: chat.canSendGif(inThread: inThread), close: close,
            select: { emoji in
                close()
                if chat.editingContext == context, !inThread || chat.threadRootID == root { insert(emoji) }
            }, send: { gif, query in
                close()
                Task {
                    guard chat.editingContext == context, !inThread || chat.threadRootID == root, chat.canSendGif(inThread: inThread) else { return }
                    await chat.sendGif(gif, inThread: inThread, query: query)
                }
            })
        box.value = host
        sheet(host, background: CaperTheme.surface)
        return host
    }

    /// The full emoji picker, for a reaction.
    public static func reactionPicker(select: @escaping (String) -> Void) -> UIViewController {
        let box = WeakController()
        let close: () -> Void = { box.value?.dismiss(animated: false) }
        let host = UIKitReactionPicker(close: close) { emoji in close(); select(emoji) }
        box.value = host
        sheet(host, background: CaperTheme.surface)
        return host
    }

    /// New direct message: an exact username, with suggestions. Opening the
    /// conversation selects it on the model and closes the sheet.
    public static func newDirectMessage(model: AppModel) -> UIViewController {
        let box = WeakController()
        let host = UIKitNewDirectMessage(model: model) { box.value?.dismiss(animated: false) }
        box.value = host
        sheet(host, background: CaperTheme.surface)
        host.sheetPresentationController?.detents = [.large()]
        return host
    }

    /// A message's edit history, as SwiftUI's "(edited)" opens.
    public static func messageHistory(chat: ChatModel, message: ChatMessage) -> UIViewController {
        let box = WeakController()
        let host = UIKitNativeMessageHistory(chat: chat, message: message) { box.value?.dismiss(animated: false) { CaperFocus.presentationDidClose() } }
        box.value = host
        sheet(host, background: CaperTheme.surface)
        host.sheetPresentationController?.detents = [.large()]
        return host
    }

    /// The mention card for a pill in `message`; nil when the id is no person there.
    public static func profileCard(model: AppModel, userID: String, in message: ChatMessage,
                                   requestBlock: @escaping (BlockTarget) -> Void) -> UIViewController? {
        guard let pill = MentionAutocomplete.pills(in: message.content.text, mentions: message.content.mentions)
            .first(where: { $0.id == userID }) else { return nil }
        return profileCard(model: model, person: resolve(model) { MentionCard.resolve(pill, members: $0, people: $1, peers: $2, account: $3, viewerID: $4) },
                           requestBlock: requestBlock)
    }

    /// The card for a message's author (avatar or name).
    public static func profileCard(model: AppModel, author: ChatAuthor, requestBlock: @escaping (BlockTarget) -> Void) -> UIViewController {
        profileCard(model: model, person: resolve(model) { MentionCard.resolve(author, members: $0, people: $1, peers: $2, account: $3, viewerID: $4) },
                    requestBlock: requestBlock)
    }

    /// The card for someone in the member list.
    public static func profileCard(model: AppModel, member: Member, requestBlock: @escaping (BlockTarget) -> Void) -> UIViewController {
        let pill = MentionPill(id: member.id, username: member.username)
        return profileCard(model: model, person: resolve(model) { MentionCard.resolve(pill, members: $0, people: $1, peers: $2, account: $3, viewerID: $4) },
                           requestBlock: requestBlock)
    }

    private static func resolve(_ model: AppModel, _ body: ([Member], [Person]?, [DirectMessagePeer], Account?, String?) -> MentionCardPerson) -> MentionCardPerson {
        body(model.detail?.members ?? [], model.people, model.directMessages.map(\.peer), model.account,
             model.chat.currentAuthor?.id ?? model.account?.id)
    }

    /// A sheet, as SwiftUI's MentionCardHost presents on iPhone. Block closes
    /// the card first; the conversation then asks to confirm.
    private static func profileCard(model: AppModel, person: MentionCardPerson, requestBlock: @escaping (BlockTarget) -> Void) -> UIViewController {
        UIKitProfileCard(model: model, person: person, requestBlock: requestBlock)
    }

    /// Native full-screen pages for accounts with no spaces or an unfinished profile.
    public static func firstSpace(model: AppModel) -> UIViewController {
        UIKitOnboarding(model: model, page: .firstSpace)
    }

    public static func profile(model: AppModel) -> UIViewController {
        UIKitOnboarding(model: model, page: .profile)
    }

    public static func spacesUnavailable(model: AppModel) -> UIViewController {
        UIKitOnboarding(model: model, page: .unavailable)
    }

    /// A workspace dialog (space and channel settings, invites, create and
    /// join, profile, privacy, notifications, audio, leave), drawn as on
    /// SwiftUI's iPhone: a scrim, then a card fitted to its content.
    public static func workspaceDialog(model: AppModel, _ item: WorkspaceSheet) -> UIViewController {
        let native: UIViewController
        let holder = WeakController()
        let close: () -> Void = { (holder.value as? UIKitWorkspaceFrame)?.close() }
        switch item {
        case .profile: native = UIKitProfilePresentation(model: model, close: close)
        case .joinInvite(let opened): native = UIKitJoinInvite(model: model, opened: opened, close: close)
        case .createSpace, .manageSpace:
            native = UIKitSpaceEditor(model: model, managing: item.id == "manage-space", close: close) {
                (holder.value as? UIKitWorkspaceFrame)?.replace(with: UIKitJoinInvite(model: model, opened: nil, close: close))
            }
        case .invitePeople(let spaceID, let channelID):
            if model.detail?.space.id == spaceID {
                native = UIKitSpaceEditor(model: model, managing: true, inviteOnly: true, channelID: channelID, close: close, joinInvite: {})
            } else {
                let notice = UIKitMessageDialog(title: "Invite people", close: close)
                notice.body.addArrangedSubview(UIKitMessageDialog.label("Open that space to invite people.")); native = notice
            }
        case .createChannel: native = UIKitChannelEditor(model: model, channel: nil, close: close)
        case .manageChannel(let channel): native = UIKitChannelEditor(model: model, channel: channel, close: close)
        case .privacy: native = UIKitPrivacyDialog(model: model, close: close)
        case .notifications: native = UIKitNotificationsDialog(model: model, close: close)
        case .audio: native = UIKitAudioTest(voice: model.voice, debugEnabled: model.account?.debugEnabled == true, close: close)
        case .connection: native = UIKitAudioDiagnostics(voice: model.voice, kind: .connection, close: close)
        case .diagnostics:
            native = model.account?.debugEnabled == true ? UIKitAudioDiagnostics(voice: model.voice, kind: .processing, close: close)
                : UIKitMessageDialog(title: "Audio diagnostics", close: close)
        case .newDirectMessage: native = UIKitNewDirectMessage(model: model, close: close)
        case .leaveSpace:
            native = UIKitConfirmationDialog(title: "Leave \(model.detail?.space.name ?? "space")?",
                detail: "You will lose access to its channels and conversations. An owner can add you again later.",
                action: "Leave space", pendingLabel: "Leaving…", close: close) { try await model.leaveCurrentSpace() }
        case .invitation(let space):
            let inviter = space.inviter.map { "\n\($0.displayName) (@\($0.username)) invited you." } ?? ""
            native = UIKitConfirmationDialog(title: "You’re invited!", detail: "Join \(space.name)?\(inviter)", action: "Accept", pendingLabel: "Accepting…",
                invitationArt: true, close: close, decline: { try await model.declineInvitation(space) }) { try await model.acceptInvitation(space) }
        case .channelInvitation(let invitation):
            native = UIKitConfirmationDialog(title: "Private channel invitation",
                detail: "\(invitation.inviter.displayName) (@\(invitation.inviter.username)) invited you to private channel #\(invitation.channel.name).",
                action: "Accept", pendingLabel: "Accepting…", close: close, decline: { try await model.declineChannelInvitation(invitation) }) {
                    try await model.acceptChannelInvitation(invitation)
                }
        case .login:
            // Authentication belongs to the native root, not a workspace dialog.
            native = UIViewController()
        }
        let frame = UIKitWorkspaceFrame(content: native, width: item.id.contains("manage") ? 600 : 560)
        holder.value = frame
        return frame
    }

    /// "Leave #name?", as the channel options menu confirms on SwiftUI.
    public static func leaveChannel(model: AppModel, channel: Channel) -> UIViewController {
        let detail = channel.private && !model.isOwner
            ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call."
            : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call."
        let box = WeakController()
        let host = UIKitConfirmationDialog(title: "Leave #\(channel.name)?", detail: detail, action: "Leave channel", pendingLabel: "Leaving…",
            close: { box.value?.dismiss(animated: false) }, perform: { try await model.leaveChannel(channel) })
        box.value = host
        sheet(host, background: CaperTheme.surface)
        host.sheetPresentationController?.detents = [.medium()]
        return host
    }

    static func sheet(_ host: UIViewController, background: Color) {
        host.view.backgroundColor = UIColor(background)
        host.modalPresentationStyle = .pageSheet
        if let sheet = host.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
    }
}

private final class WeakController {
    weak var value: UIViewController?
}

/// Above the composer: the reply draft, voice notices, a failed dictation's
/// Retry, and the staged files.
@MainActor public enum UIKitComposerAccessories {
    /// Whether anything shows, so the composer collapses the space otherwise.
    public static func shows(chat: ChatModel, inThread: Bool = false) -> Bool {
        if let root = inThread ? chat.threadRootID : "channel", chat.replyTargets[root] != nil { return true }
        let owner = VoiceComposerID(chat: chat, inThread: inThread)
        let recorder = VoiceMessageRecorder.shared
        return (inThread ? chat.threadAttachmentNotice : chat.attachmentNotice) != nil
            || !chat.attachmentFiles(inThread: inThread).isEmpty
            || recorder.notice(for: owner) != nil || recorder.dictation?.target.owner == owner
            || (recorder.isRecording(owner) && recorder.holding)
    }
}

/// One composer's voice controls: the microphone and the recording bar share
/// its dictation target. `dictate` inserts words at the caret.
@MainActor public final class UIKitVoiceComposer {
    let chat: ChatModel
    let inThread: Bool
    let controller = ComposerAutocompleteController()
    private lazy var session = NativeVoiceSession { [weak self, chat, inThread] in
        self?.target ?? VoiceTarget(owner: VoiceComposerID(chat: chat, inThread: inThread), enabled: false,
            unavailableReason: "Composer is closed", identifier: inThread ? "thread-voice" : "composer-voice",
            send: { AttachmentStaging.remove($0.fileURL) }, stage: { AttachmentStaging.remove($0.fileURL) }, isCurrent: { false })
    }
    private lazy var microphone = UIKitVoiceControls(session: session, kind: .microphone)
    private lazy var recordingBar = UIKitVoiceControls(session: session, kind: .bar)

    public init(chat: ChatModel, inThread: Bool = false, dictate: @escaping (String) -> Bool) {
        self.chat = chat; self.inThread = inThread
        controller.dictateAction = dictate
    }

    var target: VoiceTarget {
        let base = VoiceTarget.chat(chat, inThread: inThread, controller: controller)
        let context = chat.editingContext, owner = base.owner
        let current = { [weak self] in
            guard let self else { return false }
            return self.chat.editingContext == context && VoiceComposerID(chat: self.chat, inThread: self.inThread) == owner && self.chat.canInteract
        }
        return VoiceTarget(owner: owner, enabled: base.enabled, unavailableReason: base.unavailableReason, identifier: base.identifier,
            send: { if current() { await base.send($0) } else { AttachmentStaging.remove($0.fileURL) } },
            stage: { if current() { base.stage($0) } else { AttachmentStaging.remove($0.fileURL) } },
            transcribe: { recording in guard current(), let transcribe = base.transcribe else { throw CancellationError() }; return try await transcribe(recording) },
            insert: { current() && (base.insert?($0) ?? false) }, isCurrent: current)
    }

    /// While recording, the bar covers the field.
    public var isRecording: Bool { VoiceMessageRecorder.shared.isRecording(target.owner) }

    /// Tap to dictate, hold for a voice message, slide up to lock.
    public var button: UIView { microphone }

    /// Discard, the timer, live bars and "Voice note".
    public var bar: UIView { recordingBar }
}
/// What a UIKit conversation asks SwiftUI to present: message actions (the
/// long-press sheet), who reacted, a block confirmation. `chat.forwardTarget`
/// presents the forward picker as it does in SwiftUI.
@MainActor @Observable public final class UIKitPresentations {
    public var actions: ChatMessage?
    public var reactors: ReactorsRequest?
    public var block: BlockTarget?
    /// Edit message without the actions sheet: Up in an empty composer.
    public var edit: ChatMessage?
    var showingEmojiPicker = false
    public init() {}

    public struct ReactorsRequest: Identifiable, Equatable {
        public let messageID: String
        public let emoji: String
        public var id: String { "\(messageID)|\(emoji)" }
        public init(messageID: String, emoji: String) { self.messageID = messageID; self.emoji = emoji }
    }

    /// Whether any of these is showing, so the conversation leaves its gestures alone.
    public var presenting: Bool { actions != nil || reactors != nil || block != nil || edit != nil }
}

/// The account's notifications inbox ("Notifications" in the account panel):
/// native controls, polling only while this sheet is open.
@MainActor public enum UIKitActivity {
    public static func unread(_ model: AppModel) -> Int { model.activityUnread }

    public static func inbox(model: AppModel) -> UIViewController {
        let host = UIKitActivityInbox(model: model)
        UIKitHosting.sheet(host, background: CaperTheme.surface)
        // A bottom sheet over the conversation, as web's phone layout: three
        // quarters of the screen, dragged up for full height or down to close.
        let sheet = UISheetPresentationController.Detent.custom(identifier: .init("caper.inbox")) { $0.maximumDetentValue * 0.76 }
        host.sheetPresentationController?.detents = [sheet, .large()]
        host.sheetPresentationController?.prefersGrabberVisible = true
        return host
    }
}

/// What replaces a DM's composer: Accept, Decline or Block in an incoming
/// request, Unblock in a conversation with someone you blocked.
@MainActor public enum UIKitDirectBar {
    /// The open DM's peer, other than yourself.
    static func peer(_ model: AppModel) -> DirectMessageConversation? {
        model.selectedDirectMessage.flatMap { $0.peer.id == model.account?.id ? nil : $0 }
    }

    /// Unblock, reporting a failure in the conversation.
    public static func unblock(model: AppModel, accountID: String) {
        Task {
            do { try await model.unblock(accountID: accountID) }
            catch { model.chat.error = FriendlyError.message(for: error) }
        }
    }

    public static func shows(model: AppModel) -> Bool {
        model.previewingChannel || model.selectedDirectMessage?.status == .incoming || peer(model)?.blocked == true
    }
}

/// Native file drop onto a conversation or thread: the drop
/// overlay while files hover, then the files staged for the composer that was
/// targeted when they arrived.
@MainActor public final class UIKitFileDrop: NSObject, UIDropInteractionDelegate {
    private let model: AppModel
    private let inThread: Bool
    private var target: ChatModel.AttachmentTarget?
    private var overlay: UIView?
    public private(set) lazy var interaction = UIDropInteraction(delegate: self)

    public init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread
    }

    private var chat: ChatModel { model.chat }

    public func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
        session.hasItemsConforming(toTypeIdentifiers: [UTType.item.identifier])
    }

    public func dropInteraction(_ interaction: UIDropInteraction, sessionDidEnter session: UIDropSession) {
        target = chat.attachmentTarget(inThread: inThread)
        guard chat.canAttach, overlay == nil, let view = interaction.view else { return }
        let direct = model.selectedDirectMessageID != nil || model.visit?.direct == true
        let text = AttachmentPolicy.dropText(channel: chat.channelName, direct: direct,
                                             attached: inThread ? chat.threadAttachmentDrafts.count : chat.attachmentDrafts.count)
        let overlay = NativeFileDropOverlay(title: inThread ? "Add files to thread" : text.title, detail: text.detail)
        overlay.frame = view.bounds
        overlay.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(overlay)
        self.overlay = overlay
    }

    public func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal {
        UIDropProposal(operation: chat.canAttach ? .copy : .forbidden)
    }

    public func dropInteraction(_ interaction: UIDropInteraction, sessionDidExit session: UIDropSession) { hide() }
    public func dropInteraction(_ interaction: UIDropInteraction, sessionDidEnd session: UIDropSession) { hide() }

    public func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        hide()
        guard let target = target ?? chat.attachmentTarget(inThread: inThread) else { return }
        self.target = nil
        let providers = session.items.map(\.itemProvider)
        let chat = chat
        Task { @MainActor in
            var files: [LocalAttachmentFile] = []
            for provider in providers {
                let dropped: DroppedAttachment? = await withCheckedContinuation { done in
                    _ = provider.loadTransferable(type: DroppedAttachment.self) { done.resume(returning: try? $0.get()) }
                }
                if let dropped { files.append(dropped.file) }
            }
            chat.addAttachments(files, to: target)
        }
    }

    private func hide() {
        overlay?.removeFromSuperview()
        overlay = nil
    }
}

/// Browse's Channels section, as SwiftUI lists it: collapsed, it keeps only
/// live rows (your call's card and rooms with people in voice).
@MainActor public enum UIKitBrowse {
    public static func listedChannels(_ model: AppModel, expanded: Bool) -> [Channel] {
        LiveNow.listed(model.detail?.channels.filter(\.joined) ?? [], expanded: expanded) { channelIsLive(model, $0) }
    }
}

/// The counter's colour for a draft of `count` Unicode scalars.
public enum UIKitComposerCounter {
    public static func color(_ count: Int) -> UIColor { UIColor(ComposerCounter.tone(count)) }
}

/// One timeline entry: a message, or a run of blocked authors' messages that
/// collapses into "N blocked messages — Show".
public struct UIKitTimelineEntry {
    public let messages: [ChatMessage]
    public let blocked: Bool
    public let revealed: Bool
    public let label: String

    /// Groups each run of consecutive messages by blocked authors.
    public static func entries(_ messages: [ChatMessage], blocked: Set<String>, viewerID: String?,
                               revealed: Set<String>) -> [UIKitTimelineEntry] {
        BlockedMessages.entries(messages, blocked: blocked, viewerID: viewerID, revealed: revealed).map {
            switch $0 {
            case let .message(message): UIKitTimelineEntry(messages: [message], blocked: false, revealed: false, label: "")
            case let .blocked(run): UIKitTimelineEntry(messages: run.messages, blocked: true, revealed: run.revealed, label: run.label)
            }
        }
    }

    /// Show or Hide a run: the new revealed set.
    public func toggling(_ revealed: Set<String>) -> Set<String> {
        let ids = Set(messages.map(\.id))
        return self.revealed ? revealed.subtracting(ids) : revealed.union(ids)
    }
}

/// Release notes and call effects live with the native root. Automatic notes
/// wait until no other presentation or starting call owns the screen.
@MainActor public enum UIKitWhatsNew {
    public static func presenter(model: AppModel) -> UIViewController {
        UIKitReleaseNotesPresenter(model: model)
    }

    public static func open(model: AppModel) { model.whatsNew.open() }
}

/// The account menu's audio settings, stored where SwiftUI keeps them.
@MainActor public enum UIKitAudioSettings {
    public static var sendVoiceOnStop: Bool {
        get { VoiceMessageSettings.shared.sendOnStop }
        set { VoiceMessageSettings.shared.sendOnStop = newValue }
    }
}

/// The account's quick reactions, as on every client.
public enum UIKitReactions {
    @MainActor public static var quick: [String] { MessageQuickReactions.emoji }
    /// Caper's emoji artwork, where the catalog has it.
    public static func artwork(_ emoji: String) -> CGImage? {
        EmojiArtwork.entry(for: emoji).flatMap(EmojiArtwork.image(for:))
    }
}

#endif
