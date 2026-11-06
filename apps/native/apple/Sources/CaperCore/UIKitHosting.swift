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

private final class WeakController {
    weak var value: UIViewController?
}

/// A message's files, GIF, link cards, forwarded original and feedback note,
/// below its text. Tapping media opens the viewer through `present`.
public struct UIKitMessageExtras: View {
    let message: ChatMessage?
    let pending: [ChatAttachment]
    let chat: ChatModel
    let present: UIKitHosting.Present
    let openForward: () -> Void

    public init(message: ChatMessage, chat: ChatModel, present: @escaping UIKitHosting.Present, openForward: @escaping () -> Void) {
        self.message = message; pending = []; self.chat = chat; self.present = present; self.openForward = openForward
    }

    /// The optimistic row's local copies.
    public init(pending: [ChatAttachment], chat: ChatModel, present: @escaping UIKitHosting.Present) {
        message = nil; self.pending = pending; self.chat = chat; self.present = present; openForward = {}
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
            } else if !pending.isEmpty {
                MessageAttachmentsView(attachments: pending, chat: nil).opacity(0.7)
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
#endif
