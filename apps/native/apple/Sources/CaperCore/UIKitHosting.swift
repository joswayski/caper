#if os(iOS)
import SwiftUI
import UIKit

/// The SwiftUI surfaces the UIKit iPhone client hosts rather than rewrites:
/// on-demand pickers, drawers, editors and viewers, and attachment content
/// inside message rows. Scrolling lists, rows and the composer bar stay UIKit.
/// Presentations go through `present`, a UIKit presenter, because a hosted
/// leaf view has no SwiftUI presentation context of its own.
@MainActor public enum UIKitHosting {
    public typealias Present = @MainActor (UIViewController) -> Void

    /// A SwiftUI root in Caper's dark appearance; `close` dismisses it.
    static func controller<Content: View>(clear: Bool = false, _ content: (_ close: @escaping () -> Void) -> Content) -> UIViewController {
        let box = WeakController()
        let host = UIHostingController(rootView: AnyView(content({ box.value?.dismiss(animated: false) })
            .preferredColorScheme(.dark)))
        box.value = host
        host.overrideUserInterfaceStyle = .dark
        if clear { host.view.backgroundColor = .clear }
        return host
    }

    /// The composer's attach drawer: recent photos and videos, the camera,
    /// Files, and Send. Nil when the conversation can't take files.
    public static func attachDrawer(chat: ChatModel, inThread: Bool = false) -> UIViewController? {
        guard let target = chat.attachmentTarget(inThread: inThread) else { return nil }
        let draft = AttachDrawerDraft(
            files: { chat.attachmentFiles(inThread: inThread).map { AttachDrawerFile(id: $0.id, source: $0.source) } },
            add: { chat.addAttachments($0, to: target) },
            remove: { chat.removeAttachmentDraft(id: $0) },
            canSend: { chat.canPressSend(inThread: inThread) },
            send: { Task { await chat.send(inThread: inThread) } },
            valid: { chat.canAttach && chat.attachmentTarget(inThread: inThread) == target })
        let host = controller(clear: true) { _ in AttachDrawer(draft: draft) }
        host.modalPresentationStyle = .overFullScreen
        return host
    }

    /// Prepares the drawer's gallery while the composer is up.
    public static func warmAttachGallery() { AttachGalleryCache.warm() }

    /// Emojis, GIFs and stickers. An emoji goes to `insert`; a GIF or sticker sends at once.
    public static func expressionPicker(chat: ChatModel, inThread: Bool = false, insert: @escaping (String) -> Void) -> UIViewController {
        let host = controller { close in
            ComposerExpressionPicker(configuration: chat.klipy, mediaEnabled: chat.canSendGif(inThread: inThread), select: { emoji in
                insert(emoji); close()
            }, send: { gif, query in
                close(); Task { await chat.sendGif(gif, inThread: inThread, query: query) }
            })
        }
        sheet(host, background: CaperTheme.surface)
        return host
    }

    /// The full emoji picker, for a reaction.
    public static func reactionPicker(select: @escaping (String) -> Void) -> UIViewController {
        let host = controller { close in ReactionPicker { emoji in close(); select(emoji) } }
        sheet(host, background: CaperTheme.surface)
        return host
    }

    /// New direct message: an exact username, with suggestions. Opening the
    /// conversation selects it on the model and closes the sheet.
    public static func newDirectMessage(model: AppModel) -> UIViewController {
        let host = controller { close in NewDirectMessageSheet(model: model, close: close) }
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
        let box = WeakController()
        let guardian = DismissGuard()
        var pendingBlock: BlockTarget?
        let host = DisappearingHostingController(rootView: AnyView(
            MentionCardView(model: model, person: person, requestBlock: { target in
                pendingBlock = target
                if box.value?.presentingViewController != nil { box.value?.dismiss(animated: true) }
            }, close: { box.value?.dismiss(animated: true) })
            .onPreferenceChange(ProfileDismissPreference.self) { action in MainActor.assumeIsolated { guardian.profileEditor = action } }
            .preferredColorScheme(.dark)))
        box.value = host
        // Like MentionCardHost: a swipe down asks an open profile editor to
        // close itself, and never drops an unsent draft.
        let draft = model.profileMessageDraft(person: person)
        guardian.allow = { !draft.preventsImplicitDismissal }
        host.dismissGuard = guardian
        host.presentationController?.delegate = guardian
        host.overrideUserInterfaceStyle = .dark
        host.disappeared = { if let target = pendingBlock { pendingBlock = nil; requestBlock(target) } }
        sheet(host, background: CaperTheme.surface)
        host.sheetPresentationController?.detents = [.large()]
        host.view.accessibilityIdentifier = "mention-card"
        return host
    }

    /// SwiftUI's full-screen pages for an account in no spaces: the
    /// first-space page, or, when spaces fail to load, the page with Try
    /// again and Direct messages. Each fills the window like CaperRootView.
    public static func firstSpace(model: AppModel) -> UIViewController {
        page(FirstSpaceView(model: model))
    }

    public static func spacesUnavailable(model: AppModel) -> UIViewController {
        page(UIKitSpacesUnavailable(model: model))
    }

    private static func page<Content: View>(_ content: Content) -> UIViewController {
        let host = controller { _ in
            content.foregroundStyle(CaperTheme.text).tint(CaperTheme.terracottaBright)
                .buttonStyle(CaperSecondaryButton()).background(CaperTheme.blackout.ignoresSafeArea())
        }
        host.view.backgroundColor = UIColor(CaperTheme.blackout)
        return host
    }

    /// A workspace dialog (space and channel settings, invites, create and
    /// join, profile, privacy, notifications, audio, leave), drawn as on
    /// SwiftUI's iPhone: a scrim, then a card fitted to its content.
    public static func workspaceDialog(model: AppModel, _ item: WorkspaceSheet) -> UIViewController {
        let state = UIKitWorkspaceDialogState(item: item)
        let box = WeakController()
        let host = UIHostingController(rootView: UIKitWorkspaceDialog(model: model, state: state) {
            box.value?.dismiss(animated: false)
        })
        box.value = host
        host.overrideUserInterfaceStyle = .dark
        host.view.backgroundColor = .clear
        host.modalPresentationStyle = .overFullScreen
        host.view.accessibilityViewIsModal = true
        return host
    }

    /// "Leave #name?", as the channel options menu confirms on SwiftUI.
    public static func leaveChannel(model: AppModel, channel: Channel) -> UIViewController {
        let detail = channel.private && !model.isOwner
            ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call."
            : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call."
        let host = controller { close in
            ConfirmationSheet(title: "Leave #\(channel.name)?", detail: detail, action: "Leave channel", pendingLabel: "Leaving…",
                              close: close) { try await model.leaveChannel(channel) }
        }
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

    static func mediaViewer(_ present: @escaping Present) -> MediaViewerAction {
        MediaViewerAction { request in
            let host = controller { close in MediaViewer(request: request, close: close) }
            host.view.backgroundColor = .black
            host.modalPresentationStyle = .overFullScreen
            present(host)
        }
    }

    static func attachmentEditor(_ present: @escaping Present) -> AttachmentEditorAction {
        AttachmentEditorAction { request in
            let host = controller(clear: true) { close in AttachmentEditor(request: request, close: close) }
            host.modalPresentationStyle = .overFullScreen
            present(host)
        }
    }
}

@MainActor @Observable final class UIKitWorkspaceDialogState {
    var item: WorkspaceSheet
    var dismissDisabled = false
    var profileDismissAction: ProfileDismissAction?
    init(item: WorkspaceSheet) { self.item = item }
}

/// CaperView's iPhone dialog container around WorkspaceSheetView.
struct UIKitWorkspaceDialog: View {
    let model: AppModel
    @Bindable var state: UIKitWorkspaceDialogState
    let dismiss: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let top = max(16, geometry.size.height * 0.1)
            ZStack(alignment: .top) {
                Color.black.opacity(0.55).ignoresSafeArea()
                    .onTapGesture(perform: tappedOutside)
                    .accessibilityHidden(true)
                    .modifier(Entrance.scrim)
                DialogHeight(maximum: min(680, max(1, geometry.size.height - top - 16))) {
                    // Replacing the dialog (Create a space → Join with link) keeps the container.
                    WorkspaceSheetView(item: state.item, model: model, show: { state.item = $0 }) { dismiss() }
                        .id(state.item.id)
                }
                .frame(width: min(state.item.id.contains("manage") ? 600 : 560, geometry.size.width - 32))
                .background(CaperTheme.surface.onTapGesture {})
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                .shadow(color: .black.opacity(0.3), radius: 24, y: 12)
                .modifier(Entrance.dialog)
                .padding(.top, top)
                .accessibilityAddTraits(.isModal)
                .onPreferenceChange(DialogDismissDisabled.self) { value in MainActor.assumeIsolated { state.dismissDisabled = value } }
                .onPreferenceChange(ProfileDismissPreference.self) { value in MainActor.assumeIsolated { state.profileDismissAction = value } }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .accessibilityAction(.escape, tappedOutside)
        .preferredColorScheme(.dark)
    }

    private func tappedOutside() {
        if state.item.id == "profile" { state.profileDismissAction?.action() }
        else if !state.dismissDisabled { dismiss() }
    }
}

/// Reports when its sheet has finished closing, so a follow-up can present.
private final class DisappearingHostingController: UIHostingController<AnyView> {
    var disappeared: (() -> Void)?
    var dismissGuard: DismissGuard?
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || presentingViewController == nil { disappeared?() }
    }
}

/// Refuses a swipe-down while a profile edit or a draft is unsaved.
@MainActor private final class DismissGuard: NSObject, UIAdaptivePresentationControllerDelegate {
    var profileEditor: ProfileDismissAction?
    var allow: () -> Bool = { true }

    func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool {
        if let profileEditor { profileEditor.action(); return false }
        return allow()
    }
}

private final class WeakController {
    weak var value: UIViewController?
}

/// A message's files, GIF, link cards, forwarded original and feedback note,
/// below its text. Tapping media opens the viewer through `present`.
public struct UIKitMessageExtras: View {
    let message: ChatMessage?
    let pending: PendingMessage?
    let chat: ChatModel
    let present: UIKitHosting.Present
    let openForward: () -> Void

    public init(message: ChatMessage, chat: ChatModel, present: @escaping UIKitHosting.Present, openForward: @escaping () -> Void) {
        self.message = message; pending = nil; self.chat = chat; self.present = present; self.openForward = openForward
    }

    /// The optimistic row: local copies of its files, its GIF and the
    /// composer's link card, as SwiftUI's PendingMessageRow.
    public init(pending: PendingMessage, chat: ChatModel, present: @escaping UIKitHosting.Present) {
        message = nil; self.pending = pending; self.chat = chat; self.present = present; openForward = {}
    }

    public static func shows(_ pending: PendingMessage) -> Bool {
        !pending.attachments.isEmpty || pending.gif != nil || !pending.embeds.isEmpty
    }

    /// Whether a message has anything here; rows without it host nothing.
    public static func shows(_ message: ChatMessage) -> Bool {
        guard !message.isDeleted else { return false }
        return !(message.content.attachments ?? []).isEmpty || message.content.gif != nil || !(message.content.embeds ?? []).isEmpty
            || message.forward != nil || MentionAutocomplete.asksFeedback(message)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            if let message {
                if let attachments = message.content.attachments, !attachments.isEmpty {
                    MessageAttachmentsView(attachments: attachments, chat: chat)
                }
                if let gif = message.content.gif { MessageGifView(gif: gif) }
                if !(message.content.embeds ?? []).isEmpty { MessageEmbedsView(embeds: message.content.embeds ?? []) }
                if MentionAutocomplete.asksFeedback(message) {
                    Text(FeedbackInbox.notified).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                        .accessibilityIdentifier("feedback-notified-\(message.id)")
                }
                ForwardCardView(message: message, chat: chat, open: openForward)
            } else if let pending {
                if !pending.attachments.isEmpty { MessageAttachmentsView(attachments: pending.attachments, chat: nil).opacity(0.7) }
                if let gif = pending.gif { MessageGifView(gif: gif).opacity(0.7) }
                MessageEmbedsView(embeds: pending.embeds)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.mediaViewer, UIKitHosting.mediaViewer(present))
        .preferredColorScheme(.dark)
    }
}

/// The message a row replies to, above it; tapping jumps to it.
public struct UIKitReplyContext: View {
    let reference: MessageReply
    let motionKey: String
    let chat: ChatModel
    public init(reference: MessageReply, motionKey: String, chat: ChatModel) {
        self.reference = reference; self.motionKey = motionKey; self.chat = chat
    }
    public var body: some View {
        NativeReplyContext(reference: reference, motionKey: motionKey, chat: chat)
            .frame(maxWidth: .infinity, alignment: .leading)
            .preferredColorScheme(.dark)
    }
}

/// Above the composer: voice notices, a failed dictation's Retry, and the
/// staged files (tap to edit, remove, spoiler).
public struct UIKitComposerAccessories: View {
    @Bindable var chat: ChatModel
    let inThread: Bool
    let present: UIKitHosting.Present

    public init(chat: ChatModel, inThread: Bool = false, present: @escaping UIKitHosting.Present) {
        self.chat = chat; self.inThread = inThread; self.present = present
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let root = replyRoot, let reference = chat.replyTargets[root] {
                NativeReplyDraft(reference: reference, chat: chat, root: root)
            }
            if let notice = inThread ? chat.threadAttachmentNotice : chat.attachmentNotice {
                Text(notice).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VoiceComposerNotice(chat: chat, inThread: inThread)
            AttachmentDraftsView(chat: chat, inThread: inThread)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.attachmentEditor, UIKitHosting.attachmentEditor(present))
        .preferredColorScheme(.dark)
    }

    private var replyRoot: String? { inThread ? chat.threadRootID : "channel" }

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

    public init(chat: ChatModel, inThread: Bool = false, dictate: @escaping (String) -> Bool) {
        self.chat = chat; self.inThread = inThread
        controller.dictateAction = dictate
    }

    var target: VoiceTarget { .chat(chat, inThread: inThread, controller: controller) }

    /// While recording, the bar covers the field.
    public var isRecording: Bool { VoiceMessageRecorder.shared.isRecording(target.owner) }

    /// Tap to dictate, hold for a voice message, slide up to lock.
    public var button: some View {
        VoiceRecordButton(target: target, height: 42)
            // The lock hint draws above the microphone, outside the composer box.
            .modifier(VoiceLockLayer())
            .preferredColorScheme(.dark)
    }

    /// Discard, the timer, live bars and "Voice note".
    public var bar: some View {
        VoiceRecordingBar(target: target, height: 42).preferredColorScheme(.dark)
    }
}
/// What a UIKit conversation asks SwiftUI to present: message actions (the
/// long-press sheet), who reacted, a block confirmation. `chat.forwardTarget`
/// presents the forward picker as it does in SwiftUI.
@MainActor @Observable public final class UIKitPresentations {
    public var actions: ChatMessage?
    public var reactors: ReactorsRequest?
    public var block: BlockTarget?
    var showingEmojiPicker = false
    public init() {}

    public struct ReactorsRequest: Identifiable, Equatable {
        public let messageID: String
        public let emoji: String
        public var id: String { "\(messageID)|\(emoji)" }
        public init(messageID: String, emoji: String) { self.messageID = messageID; self.emoji = emoji }
    }

    /// Whether any of these is showing, so the conversation leaves its gestures alone.
    public var presenting: Bool { actions != nil || reactors != nil || block != nil }
}

/// Message requests on iPhone: the page Browse pushes in place of its lists.
/// Opening a request reads it and never accepts it.
public struct UIKitMessageRequests: View {
    @Bindable var model: AppModel
    public init(model: AppModel) { self.model = model }
    public var body: some View {
        ScrollView { MessageRequestsList(model: model, pushed: true) }
            .background(CaperTheme.sidebar).preferredColorScheme(.dark)
    }
}

/// The feedback team's inbox on iPhone: the page Browse pushes in place of
/// its lists, pulled to refresh. A row visits its conversation.
public struct UIKitFeedbackInbox: View {
    @Bindable var model: AppModel
    public init(model: AppModel) { self.model = model }
    public var body: some View {
        ScrollView { FeedbackInboxList(model: model, pushed: true) }
            .refreshable { await model.refreshFeedback() }
            .background(CaperTheme.sidebar).preferredColorScheme(.dark)
    }
}

/// The account's notifications inbox ("Notifications" atop Direct messages):
/// SwiftUI's sheet, polling while it is up as SwiftUI's binding has it.
@MainActor public enum UIKitActivity {
    public static func unread(_ model: AppModel) -> Int { model.activityUnread }

    public static func inbox(model: AppModel) -> UIViewController {
        model.activityShown = true
        let host = UIKitHosting.controller { _ in
            ActivityInboxView(model: model).onDisappear { model.activityShown = false }
        }
        UIKitHosting.sheet(host, background: CaperTheme.surface)
        host.sheetPresentationController?.detents = [.large()]
        return host
    }
}

/// What replaces a DM's composer: Accept, Decline or Block in an incoming
/// request, Unblock in a conversation with someone you blocked.
public struct UIKitDirectBar: View {
    @Bindable var model: AppModel
    let block: (BlockTarget) -> Void

    public init(model: AppModel, block: @escaping (BlockTarget) -> Void) { self.model = model; self.block = block }

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

    @State private var joiningChannelID: String?
    @State private var joinError: String?
    private var joining: Bool { joiningChannelID != nil && joiningChannelID == model.selectedChannelID }

    public var body: some View {
        Group {
            if model.previewingChannel {
                // A channel you haven't joined reads as a preview, with Join in place of the composer.
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        VStack(alignment: .leading, spacing: 5) {
                            Text("Preview").font(CaperTheme.font(12, weight: .bold))
                            (Text("Join ") + Text("#\(model.chat.channelName)").font(CaperTheme.font(12, weight: .bold)) + Text(" to interact with people here"))
                                .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        }
                        Spacer()
                        Button(joining ? "Joining…" : "Join channel") {
                            guard !joining, let channel = model.selectedChannel else { return }
                            joiningChannelID = channel.id; joinError = nil
                            Task {
                                do { try await model.joinChannel(channel) }
                                catch { if model.selectedChannelID == channel.id { joinError = FriendlyError.message(for: error) } }
                                if joiningChannelID == channel.id { joiningChannelID = nil }
                            }
                        }.buttonStyle(CaperPrimaryButton()).fixedSize().disabled(joining)
                            .accessibilityIdentifier("join-channel")
                    }
                    if let joinError { Text(joinError).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error) }
                }
            } else if let request = model.selectedDirectMessage, request.status == .incoming {
                MessageRequestBar(model: model, conversation: request) { block(BlockTarget(peer: request.peer)) }
            } else if let conversation = Self.peer(model), conversation.blocked {
                BlockedConversationBar(conversation: conversation) { Self.unblock(model: model, accountID: conversation.peer.id) }
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .preferredColorScheme(.dark)
    }
}

/// Phones: your call, or the open channel's voice with Join, under the header.
public struct UIKitVoiceStrip: View {
    @Bindable var model: AppModel
    let browse: () -> Void
    public init(model: AppModel, browse: @escaping () -> Void) { self.model = model; self.browse = browse }

    /// The open channel once joined, which has voice; DMs and previews have none.
    static func channel(_ model: AppModel) -> Channel? {
        if model.visit != nil { return model.visitChannel.flatMap { model.isVisitVoice($0) ? $0 : nil } }
        guard model.selectedDirectMessageID == nil, let channel = model.selectedChannel, channel.joined else { return nil }
        return channel
    }

    /// The header subtitle's "N in voice" for the open (or visited) channel.
    public static func inVoice(_ model: AppModel) -> Int {
        channel(model).map { voiceOccupants(model, channelID: $0.id).count } ?? 0
    }

    public var body: some View {
        let channel = Self.channel(model)
        PhoneVoiceStrip(model: model, channel: channel, browse: browse)
            // Checked as a channel opens: the strip's Join needs it.
            .task(id: channel?.id) { if let channel { await model.refreshVoiceAvailability(channel: channel) } }
            .preferredColorScheme(.dark)
    }
}

/// Above the composer: a failed send or action, and voice permission or call errors.
public struct UIKitComposerNotices: View {
    @Bindable var model: AppModel
    @Bindable var chat: ChatModel
    @Bindable var voice: VoiceClient
    /// Back to latest: reopens at the live end; the host scrolls there.
    let backToLatest: () -> Void
    public init(model: AppModel, backToLatest: @escaping () -> Void) {
        self.model = model; chat = model.chat; voice = model.voice; self.backToLatest = backToLatest
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // After a jump to an older message, as SwiftUI: page forward, or
            // reopen at the live end.
            if chat.hasNewer {
                HStack {
                    Button(chat.loadingNewer ? "Loading…" : "Load newer messages") { Task { await chat.loadNewer() } }
                        .disabled(chat.loadingNewer)
                    Button("Back to latest", action: backToLatest)
                        .accessibilityIdentifier("back-to-latest")
                }.font(CaperTheme.font(11)).buttonStyle(CaperSecondaryButton()).padding(10)
            }
            if let sessionError = chat.sessionError {
                HStack(spacing: 8) {
                    Text(sessionError).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                    Button("Retry session") { Task { await chat.retrySession() } }.buttonStyle(.plain)
                        .font(CaperTheme.font(11, weight: .bold)).foregroundStyle(CaperTheme.text)
                    Spacer()
                }.padding(.horizontal, 18)
            }
            if let error = chat.error, chat.pendingMessage == nil, !chat.loadFailed, !chat.displayedMessages.isEmpty {
                Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18)
                    .accessibilityIdentifier("chat-error")
            }
            if let error = voice.error {
                HStack(alignment: .top, spacing: 8) {
                    Text(error).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    Button { voice.error = nil } label: { CaperIcon(name: "x", size: 14) }
                        .buttonStyle(SidebarIconButton()).accessibilityLabel("Dismiss voice error")
                }.padding(9).background(CaperTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                    .padding(.horizontal, 18).padding(.top, 8)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("voice-error-inline")
            }
            // A 1:1 request you sent, until they accept.
            if let conversation = model.selectedDirectMessage, conversation.peer.id != model.account?.id,
               conversation.status == .outgoing {
                Text(MessageRequests.waitingNotice(for: conversation.peer))
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18)
                    .accessibilityIdentifier("message-request-waiting")
            }
            // A thread reply that failed while its thread is closed.
            if let root = chat.pendingMessage?.threadRootId, !chat.sending,
               chat.error != nil || chat.sendRejected, chat.threadRootID != root {
                HStack(spacing: 8) {
                    Text(chat.sendRejected ? "A thread reply wasn’t sent." : "A thread reply couldn’t be confirmed.")
                        .foregroundStyle(CaperTheme.error)
                    Button("Review reply") { Task { await chat.openThread(root) } }.buttonStyle(CaperSecondaryButton())
                }.font(CaperTheme.font(11)).padding(.horizontal, 18)
            }
        }
        .preferredColorScheme(.dark)
    }
}

/// A space's rail tile, exactly as SwiftUI draws it on iPhone: icon, the
/// unread dot, mention count, muted bell, voice badge and selection glow.
public struct UIKitSpaceRailTile: View {
    @Bindable var model: AppModel
    let space: Space
    public init(model: AppModel, space: Space) { self.model = model; self.space = space }
    public var body: some View {
        SpaceRailButton(model: model, space: space, narrow: true)
            .accessibilityIdentifier("space-\(space.id)")
            .preferredColorScheme(.dark)
    }
}

/// Inside the channel composer's box: the draft's first link card, loaded
/// after a pause in typing, with × to send without it.
public struct UIKitComposerLinkPreview: View {
    let chat: ChatModel
    public init(chat: ChatModel) { self.chat = chat }
    public var body: some View {
        VStack(spacing: 0) {
            Color.clear.frame(height: 0)
            ComposerEmbedPreview(chat: chat)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task(id: chat.composerPreviewLink) { await chat.loadComposerLinkPreview() }
        .preferredColorScheme(.dark)
    }
}

/// The counter's colour for a draft of `count` Unicode scalars.
public enum UIKitComposerCounter {
    public static func color(_ count: Int) -> UIColor { UIColor(ComposerCounter.tone(count)) }
}

/// A joined channel's Browse row, exactly as SwiftUI draws it on iPhone: name,
/// unread and mute, quick join and options; a live one grows into its voice
/// card in place. `show` presents the dialogs its menu opens.
public struct UIKitChannelRow: View {
    @Bindable var model: AppModel
    let channel: Channel
    let show: (WorkspaceSheet) -> Void
    public init(model: AppModel, channel: Channel, show: @escaping (WorkspaceSheet) -> Void) {
        self.model = model; self.channel = channel; self.show = show
    }
    public var body: some View {
        ChannelSidebarItem(model: model, sheet: Binding(get: { nil }, set: { if let item = $0 { show(item) } }), channel: channel, narrow: true)
            .preferredColorScheme(.dark)
    }
    /// Live rows stay listed while Channels is collapsed, so voice never hides.
    public static func live(_ model: AppModel, _ channel: Channel) -> Bool { channelIsLive(model, channel) }
}

/// Voice with no row in the list: your call in another space, a visited room.
public struct UIKitLiveNow: View {
    @Bindable var model: AppModel
    public init(model: AppModel) { self.model = model }
    public var body: some View { LiveNowSection(model: model, narrow: true).preferredColorScheme(.dark) }

    /// Whether LiveNowSection draws anything: your call outside this list, or a visited live room.
    public static func shows(_ model: AppModel) -> Bool {
        if inVoiceCall(model.voice), let context = model.voice.context,
           !LiveNow.hasRow(context.channelID, rows: model.detail?.channels.filter(\.joined).map(\.id) ?? []) { return true }
        guard let channel = model.visitChannel, model.isVisitVoice(channel), !model.voice.isActive(channelID: channel.id) else { return false }
        return !model.voiceRoster(for: channel.id).isEmpty
    }
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

/// SwiftUI's What's new: the release-notes sheet, opened from the account
/// menu or shown once for a new build's unread notes. Its zero-size host
/// sits in the root; the automatic check waits, as on SwiftUI, until nothing
/// else is presented and no call is starting.
@MainActor public enum UIKitWhatsNew {
    public static func presenter(model: AppModel) -> UIViewController {
        let host = UIHostingController(rootView: Color.clear.modifier(WhatsNewPresentation(model: model)).preferredColorScheme(.dark))
        host.overrideUserInterfaceStyle = .dark
        host.view.backgroundColor = .clear
        host.view.isUserInteractionEnabled = false
        host.view.accessibilityElementsHidden = true
        return host
    }

    public static func open(model: AppModel) { model.whatsNew.open() }
}

/// The account menu's audio settings, stored where SwiftUI keeps them.
@MainActor public enum UIKitAudioSettings {
    public static var soundEffects: Bool {
        get { CaperEffects.shared.soundsEnabled }
        set { CaperEffects.shared.soundsEnabled = newValue }
    }
    public static var sendVoiceOnStop: Bool {
        get { VoiceMessageSettings.shared.sendOnStop }
        set { VoiceMessageSettings.shared.sendOnStop = newValue }
    }
}

/// The quick reactions on every client: 👍 ❤️ 😂.
public enum UIKitReactions {
    public static let quick = MessageQuickReactions.emoji
    /// Caper's emoji artwork, where the catalog has it.
    public static func artwork(_ emoji: String) -> CGImage? {
        EmojiArtwork.entry(for: emoji).flatMap(EmojiArtwork.image(for:))
    }
}

/// An invisible SwiftUI root, a child of the UIKit conversation, carrying
/// exactly the presentations CaperView's conversation uses on iPhone.
public struct UIKitPresentationHost: View {
    @Bindable var state: UIKitPresentations
    @Bindable var chat: ChatModel
    let model: AppModel
    /// The thread's host, or the channel's: only the frontmost one presents forwards.
    let inThread: Bool

    public init(state: UIKitPresentations, model: AppModel, inThread: Bool = false) {
        self.state = state; self.model = model; chat = model.chat; self.inThread = inThread
    }

    private var forwards: Binding<ChatMessage?> {
        (chat.threadRootID != nil) == inThread ? $chat.forwardTarget : .constant(nil)
    }

    private var viewerID: String? { chat.currentAuthor?.id ?? model.account?.id }

    public var body: some View {
        Color.clear
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .sheet(item: $state.reactors) { target in
                ReactorsSheet(chat: chat, messageID: target.messageID, emoji: target.emoji, viewerID: viewerID)
            }
            .sheet(item: $state.actions) { message in actionsSheet(message) }
            .sheet(item: forwards) { message in ForwardPickerView(chat: chat, message: message) }
            .modifier(BlockConfirmation(target: $state.block) { target in
                do { try await model.block(target) } catch { chat.error = FriendlyError.message(for: error) }
            })
            .onChange(of: chat.isPreview) { _, preview in if preview { state.actions = nil; state.showingEmojiPicker = false } }
            .onChange(of: chat.currentAuthor?.id) { _, _ in state.actions = nil; state.showingEmojiPicker = false }
            .onChange(of: state.actions?.id) { _, _ in state.showingEmojiPicker = false }
            .preferredColorScheme(.dark)
    }

    private func actionsSheet(_ message: ChatMessage) -> some View {
        MessageActionsSheet(message: message, chat: chat, showingEmojiPicker: $state.showingEmojiPicker,
                            canReact: chat.canInteract,
                            canPin: chat.canInteract && !chat.pendingPins.contains(message.id),
                            togglePin: {
                                state.actions = nil
                                Task { await chat.setPin(messageID: message.id, active: message.pin == nil) }
                            },
                            // Inside a thread, Reply already replies there.
                            reply: inThread ? nil : { state.actions = nil; chat.replyInThread(to: message) },
                            inThread: inThread,
                            canForward: chat.canForward,
                            forward: { state.actions = nil; chat.forwardTarget = message },
                            quickReaction: { emoji in
                                guard chat.canInteract, let author = chat.currentAuthor,
                                      let current = chat.messages.first(where: { $0.id == message.id }) else { return }
                                let own = current.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(author.id) == true
                                state.actions = nil
                                Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: !own) }
                            }, selectReaction: { emoji in
                                guard chat.canInteract else { return }
                                state.actions = nil
                                Task { await chat.setReaction(messageID: message.id, emoji: emoji, active: true) }
                            },
                            block: BlockedMessages.canBlock(message.author, viewerID: viewerID, blocked: chat.blockedAuthorIDs)
                                ? { state.block = BlockTarget(author: message.author) } : nil,
                            unblock: BlockedMessages.canUnblock(message.author, viewerID: viewerID, blocked: chat.blockedAuthorIDs)
                                ? { chat.unblock(message.author) } : nil,
                            viewReactions: { emoji in state.reactors = .init(messageID: message.id, emoji: emoji) })
    }
}

/// Reads the current error, so a failed retry shows its new message.
private struct UIKitSpacesUnavailable: View {
    @Bindable var model: AppModel
    var body: some View { SpacesUnavailableView(model: model, error: model.spacesError ?? "") }
}
#endif
