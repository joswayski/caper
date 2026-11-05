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

    /// Whether anything shows, so the composer collapses the space otherwise.
    public static func shows(chat: ChatModel, inThread: Bool = false) -> Bool {
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
#endif
