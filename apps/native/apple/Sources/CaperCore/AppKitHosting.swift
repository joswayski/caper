#if os(macOS)
import AppKit
import SwiftUI

/// A native workspace root whose AX tree follows its modal child without
/// freezing a snapshot of the changing navigation and message descendants.
@MainActor public final class AppKitWorkspaceContainer: NSView {
    fileprivate weak var dialogView: NSView?
    public override var isFlipped: Bool { true }
    public init() {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor(CaperTheme.blackout).cgColor
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func accessibilityChildren() -> [Any]? {
        if let dialogView { return [dialogView] }
        return super.accessibilityChildren()
    }
}

/// Workspace dialogs share the shipping validation, permissions and pending /
/// unsaved-edit guards. Only the open dialog has a SwiftUI hosting controller.
@MainActor public final class AppKitWorkspacePresenter {
    private let model: AppModel
    private var host: NSHostingController<AppKitWorkspaceDialog>?
    private weak var previousResponder: NSResponder?
    private var previousContext: String?
    private var focusContext: String { [model.chat.editingContext, model.chat.threadRootID ?? "channel"].joined(separator: ":") }

    public init(model: AppModel) { self.model = model }

    public func show(_ item: WorkspaceSheet, in parent: NSViewController) {
        show(AppKitWorkspaceDialogState(item: .workspace(item)), in: parent)
    }

    public static func activityUnread(_ model: AppModel) -> Int { model.activityUnread }

    public func activityInbox(in parent: NSViewController) {
        guard model.account != nil else { return }
        show(AppKitWorkspaceDialogState(item: .activity), in: parent)
    }

    /// Notification routes can arrive while another modal is open. Consume the
    /// destination only after it can be presented, including an inbox handoff.
    public func presentInvitations(in parent: NSViewController) {
        guard model.account != nil, host == nil,
              (parent.view as? AppKitWorkspaceContainer)?.dialogView == nil else { return }
        if let invitation = model.notificationInvitation {
            show(.invitation(invitation), in: parent)
            if host != nil { model.notificationInvitation = nil }
        } else if let invitation = model.notificationChannelInvitation {
            show(.channelInvitation(invitation), in: parent)
            if host != nil { model.notificationChannelInvitation = nil }
        }
    }

    public func leaveChannel(_ channel: Channel, in parent: NSViewController) {
        show(AppKitWorkspaceDialogState(item: .leaveChannel(channel)), in: parent)
    }

    /// A reload changes editingContext's generation, but remains in this
    /// account/conversation. Native timeline anchors use this stable identity.
    public var conversationContext: String { "\(model.chat.channelID ?? ""):\(model.chat.viewerID ?? "")" }
    public var messageContext: String { model.chat.editingContext }
    public func currentMessage(_ message: ChatMessage) -> ChatMessage? {
        guard message.channelId == model.chat.channelID else { return nil }
        return model.chat.displayedMessages.first { $0.id == message.id }
    }

    public func canEdit(_ message: ChatMessage) -> Bool { model.chat.canEdit(message) }
    public func canDelete(_ message: ChatMessage) -> Bool { model.chat.canDelete(message) }
    public func canBlock(_ author: ChatAuthor) -> Bool {
        BlockedMessages.canBlock(author, viewerID: model.chat.viewerID, blocked: model.chat.blockedAuthorIDs)
    }
    public func canUnblock(_ author: ChatAuthor) -> Bool {
        BlockedMessages.canUnblock(author, viewerID: model.chat.viewerID, blocked: model.chat.blockedAuthorIDs)
    }

    public func editMessage(_ message: ChatMessage, in parent: NSViewController) {
        guard canEdit(message) else { return }
        show(AppKitWorkspaceDialogState(item: .edit(message)), in: parent)
    }

    public func messageHistory(_ message: ChatMessage, in parent: NSViewController) {
        guard !message.isDeleted, message.forward == nil, message.channelId == model.chat.channelID else { return }
        show(AppKitWorkspaceDialogState(item: .history(message)), in: parent)
    }

    public func reactions(_ message: ChatMessage, emoji: String, in parent: NSViewController) {
        guard let current = currentMessage(message), !current.isDeleted,
              current.reactions?.contains(where: { $0.emoji == emoji }) == true else { return }
        // Listing people uses read access, even when adding a reaction is disabled.
        show(AppKitWorkspaceDialogState(item: .reactions(current, emoji)), in: parent)
    }

    public func deleteMessage(_ message: ChatMessage, in parent: NSViewController) {
        guard canDelete(message) else { return }
        show(AppKitWorkspaceDialogState(item: .delete(message)), in: parent)
    }

    /// Channel, thread and Pins actions share the model's single forward target.
    /// The workspace owns presentation, never a transient pin-card popover.
    public func presentForwarding(in parent: NSViewController) {
        if let message = model.chat.forwardTarget {
            guard model.chat.canForward, !message.isDeleted else { model.chat.forwardTarget = nil; return }
            show(AppKitWorkspaceDialogState(item: .forward(message)), in: parent)
        } else if let message = model.chat.forwardConversationTarget {
            show(AppKitWorkspaceDialogState(item: .forwardConversation(message)), in: parent)
        } else if let item = host?.rootView.state.item {
            switch item {
            case .forward, .forwardConversation: close()
            default: break
            }
        }
    }

    private func show(_ state: AppKitWorkspaceDialogState, in parent: NSViewController) {
        guard host == nil, let container = parent.view as? AppKitWorkspaceContainer, container.dialogView == nil else { return }
        let host = NSHostingController(rootView: AppKitWorkspaceDialog(model: model, state: state) { [weak self] in
            guard let self, self.host?.rootView.state === state else { return }
            self.close()
        })
        self.host = host
        previousResponder = parent.view.window?.firstResponder
        previousContext = focusContext
        if case .activity = state.item { model.activityShown = true }
        parent.addChild(host)
        host.view.frame = parent.view.bounds
        host.view.autoresizingMask = [.width, .height]
        parent.view.addSubview(host.view)
        // The parent is an AX group. Restrict its children instead of hiding
        // ignored layout views, whose controls AppKit flattens into the tree.
        container.dialogView = host.view
        parent.view.window?.makeFirstResponder(host.view)
        model.whatsNew.workspaceModalOpen = true
    }

    private func close() {
        guard let host else { return }
        let item = host.rootView.state.item
        let window = host.view.window
        let parent = host.parent
        host.rootView.state.closed = true
        (host.parent?.view as? AppKitWorkspaceContainer)?.dialogView = nil
        host.view.removeFromSuperview()
        host.removeFromParent()
        self.host = nil
        switch item {
        case .activity: model.activityShown = false
        case .forward(let message):
            if model.chat.forwardTarget?.id == message.id { model.chat.forwardTarget = nil }
        case .forwardConversation(let message):
            if model.chat.forwardConversationTarget?.id == message.id { model.chat.forwardConversationTarget = nil }
        default: break
        }
        // Opening an inbox destination can replace the old conversation/root.
        // Never put focus back into a detached or different conversation editor.
        if previousContext == focusContext,
           let responder = previousResponder as? NSView, responder.window === window, !responder.isHiddenOrHasHiddenAncestor {
            window?.makeFirstResponder(responder)
        }
        previousResponder = nil
        previousContext = nil
        model.whatsNew.workspaceModalOpen = false
        if let parent { presentInvitations(in: parent) }
    }
}

private enum AppKitWorkspaceDialogItem {
    case workspace(WorkspaceSheet), leaveChannel(Channel)
    case activity
    case edit(ChatMessage), history(ChatMessage), delete(ChatMessage)
    case reactions(ChatMessage, String)
    case forward(ChatMessage), forwardConversation(ChatMessage)
    var id: String {
        switch self {
        case .workspace(let item): item.id
        case .leaveChannel(let channel): "leave-channel-\(channel.id)"
        case .activity: "activity-inbox"
        case .edit(let message): "edit-\(message.id)"
        case .history(let message): "history-\(message.id)"
        case .delete(let message): "delete-\(message.id)"
        case .reactions(let message, let emoji): "reactions-\(message.id)-\(emoji)"
        case .forward(let message): "forward-\(message.id)"
        case .forwardConversation(let message): "forward-conversation-\(message.id)"
        }
    }
    var messageID: String? {
        switch self {
        case .edit(let message), .history(let message), .delete(let message), .reactions(let message, _),
             .forward(let message), .forwardConversation(let message): message.id
        default: nil
        }
    }
    var width: CGFloat {
        switch self {
        case .activity: 680
        case .edit: 520
        case .history: 820
        case .forward: 460
        case .forwardConversation: 520
        default: id.contains("manage") ? 600 : 560
        }
    }
}

@MainActor @Observable private final class AppKitWorkspaceDialogState {
    var item: AppKitWorkspaceDialogItem
    var closed = false
    var dismissDisabled = false
    var profileDismissAction: ProfileDismissAction?
    init(item: AppKitWorkspaceDialogItem) { self.item = item }
}

private struct AppKitWorkspaceDialog: View {
    let model: AppModel
    @Bindable var state: AppKitWorkspaceDialogState
    let dismiss: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let top = max(16, geometry.size.height * 0.1)
            let item = state.item
            ZStack(alignment: .top) {
                Color.black.opacity(0.55).ignoresSafeArea()
                    .onTapGesture(perform: requestClose)
                    .accessibilityHidden(true).modifier(Entrance.scrim)
                DialogHeight(maximum: min(680, max(1, geometry.size.height - top - 16))) {
                    content(item).id(item.id)
                }
                .frame(width: min(item.width, geometry.size.width - 32))
                .background(CaperTheme.surface.onTapGesture {})
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                .shadow(color: .black.opacity(0.3), radius: 24, y: 12)
                .modifier(Entrance.dialog).padding(.top, top)
                .accessibilityAddTraits(.isModal)
                .onPreferenceChange(DialogDismissDisabled.self) { state.dismissDisabled = $0 }
                .onPreferenceChange(ProfileDismissPreference.self) { state.profileDismissAction = $0 }
                .onExitCommand(perform: requestClose)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onChange(of: state.item.id) { _, _ in state.dismissDisabled = false; state.profileDismissAction = nil }
        .onChange(of: model.chat.editingContext) { _, _ in if state.item.messageID != nil { dismiss() } }
        .onChange(of: model.account?.id) { _, _ in dismiss() }
        .onDisappear {
            // Explicit close already cleaned up; its delayed disappearance must
            // not clear the modal flag of an invitation replacing this inbox.
            guard !state.closed else { return }
            model.whatsNew.workspaceModalOpen = false
            if case .activity = state.item { model.activityShown = false }
        }
        .preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
    }

    @ViewBuilder private func content(_ item: AppKitWorkspaceDialogItem) -> some View {
        switch item {
        case .workspace(let sheet):
            WorkspaceSheetView(item: sheet, model: model, show: { state.item = .workspace($0) }) {
                // An old request must not close the dialog replacing it.
                if state.item.id == item.id { dismiss() }
            }
        case .activity:
            ActivityInboxView(model: model, close: dismiss)
        case .leaveChannel(let channel):
            ConfirmationSheet(title: "Leave #\(channel.name)?",
                detail: channel.private && !model.isOwner
                    ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call."
                    : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call.",
                action: "Leave channel", pendingLabel: "Leaving…", close: dismiss) { try await model.leaveChannel(channel) }
        case .edit(let message):
            MessageEditorView(chat: model.chat, message: message, close: dismiss)
        case .history(let message):
            MessageHistoryView(chat: model.chat, message: message, close: dismiss)
        case .delete(let message):
            ConfirmationSheet(title: MessageDeletion.confirmTitle, detail: MessageDeletion.confirmMessage,
                action: "Delete", pendingLabel: "Deleting…", close: dismiss) { await model.chat.deleteMessage(message) }
        case .reactions(let message, let emoji):
            ReactorsSheet(chat: model.chat, messageID: message.id, emoji: emoji, viewerID: model.chat.viewerID, close: dismiss)
        case .forward(let message):
            ForwardPickerView(chat: model.chat, message: message, close: dismiss)
        case .forwardConversation(let message):
            ForwardConversationView(chat: model.chat, messageID: message.id, close: dismiss)
        }
    }

    private func requestClose() {
        if state.item.id == "profile" { state.profileDismissAction?.action() }
        else if !state.dismissDisabled { dismiss() }
    }
}

/// Pins use the shipping, bounded card list in a native header-anchored popover.
/// Reading pins does not require an editable composer (channel previews too).
@MainActor public final class AppKitPinsPresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private let media: AppKitMediaPresenter
    private var popover: NSPopover?
    private var restoreFocus: (() -> Void)?

    public init(model: AppModel, media: AppKitMediaPresenter) {
        self.model = model; self.media = media; super.init()
    }

    public var isAvailable: Bool { model.chat.channelID != nil }

    public func show(from anchor: NSView, composer: ComposerTextView) {
        guard let window = anchor.window, isAvailable,
              window.attachedSheet == nil, !model.whatsNew.workspaceModalOpen else { return }
        if popover != nil { close(); return }
        let context = model.chat.editingContext, root = model.chat.threadRootID
        restoreFocus = { [weak self, weak composer, weak window] in
            guard let self, let composer, let window, composer.window === window, composer.isEditable,
                  self.model.chat.editingContext == context, self.model.chat.threadRootID == root,
                  window.attachedSheet == nil, !self.model.whatsNew.workspaceModalOpen else { return }
            window.makeFirstResponder(composer)
        }
        let popover = NSPopover()
        self.popover = popover
        popover.behavior = .transient
        popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        popover.delegate = self
        let host = NSHostingController(rootView: PinnedMessagesView(chat: model.chat, model: model) { [weak self] in self?.close() }
            .frame(width: 470, height: 360)
            .environment(\.mediaViewer, MediaViewerAction { [weak self] request in
                guard let self else { return }
                self.close()
                self.media.showViewer(request)
            })
            .preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
            .onChange(of: model.whatsNew.workspaceModalOpen) { [weak self] _, open in if open { self?.close(restoringFocus: false) } }
            .onExitCommand { [weak self] in self?.close() })
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
    }

    public func close(restoringFocus: Bool = true) {
        if !restoringFocus { restoreFocus = nil }
        popover?.animates = false
        popover?.close()
    }
    public func popoverShouldClose(_ popover: NSPopover) -> Bool { popover.animates = false; return true }
    public func popoverDidClose(_ notification: Notification) {
        guard let closed = notification.object as? NSPopover else { return }
        closed.contentViewController = nil
        guard popover === closed else { return }
        popover = nil
        let restore = restoreFocus; restoreFocus = nil
        restore?()
    }
}

/// On-demand profile surfaces shared with the shipping Mac app. The workspace,
/// timeline and main composer remain AppKit; no hosting view exists until open.
@MainActor public final class AppKitProfilePresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
    private var personID: String?
    private var draft: ProfileMessageDraft?
    private var dismissAction: ProfileDismissAction?

    public init(model: AppModel) { self.model = model }

    public func show(author: ChatAuthor, relativeTo rect: NSRect, of view: NSView) {
        guard !author.isGuest else { return }
        show(MentionCard.resolve(author, members: model.detail?.members ?? [], people: model.people,
            peers: model.directMessages.map(\.peer), account: model.account, viewerID: model.chat.viewerID), rect: rect, view: view)
    }

    public func show(member: Member, relativeTo rect: NSRect, of view: NSView) {
        show(mention: .user(id: member.id, username: member.username), relativeTo: rect, of: view)
    }

    public func show(mention: MessageMention, relativeTo rect: NSRect, of view: NSView) {
        guard case let .user(id, username) = mention else { return }
        show(MentionCard.resolve(MentionPill(id: id, username: username), members: model.detail?.members ?? [],
            people: model.people, peers: model.directMessages.map(\.peer), account: model.account,
            viewerID: model.chat.viewerID), rect: rect, view: view)
    }

    private func show(_ person: MentionCardPerson, rect: NSRect, view: NSView) {
        guard let window = view.window else { return }
        if let popover, popover.isShown {
            // Hover and click on the same pinner must not recreate their draft.
            guard personID != person.id else { return }
            popover.animates = false
            popover.performClose(nil)
            guard !popover.isShown else { return }
        }
        let popover = NSPopover()
        self.popover = popover
        personID = person.id
        draft = model.profileMessageDraft(person: person)
        dismissAction = nil
        popover.behavior = .transient
        popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        popover.delegate = self
        let host = NSHostingController(rootView: MentionCardView(model: model, person: person,
            requestBlock: { [weak self, weak window] target in
                guard let self, let window else { return }
                self.confirmBlock(target, window: window)
            }, close: { [weak popover] in
                // Entrances animate, closing is instant on every motion setting.
                popover?.animates = false
                popover?.close()
            })
            .onPreferenceChange(ProfileDismissPreference.self) { [weak self] action in self?.dismissAction = action }
            .preferredColorScheme(.dark)
            .tint(CaperTheme.terracottaBright))
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        popover.show(relativeTo: rect, of: view, preferredEdge: .maxY)
    }

    public func popoverShouldClose(_ popover: NSPopover) -> Bool {
        if let dismissAction { dismissAction.action(); return false }
        guard draft?.preventsImplicitDismissal != true else { return false }
        popover.animates = false
        return true
    }

    public func popoverDidClose(_ notification: Notification) {
        // Releasing the host runs the card's onDisappear (including a requested
        // block) and leaves its account-scoped draft in the model for reopening.
        guard let closed = notification.object as? NSPopover else { return }
        closed.contentViewController = nil
        // A delayed old notification must never tear down its replacement.
        guard popover === closed else { return }
        popover = nil; personID = nil; draft = nil; dismissAction = nil
    }

    public func confirmBlock(_ target: BlockTarget, window: NSWindow) {
        guard target.canBlock, target.id != model.account?.id else { return }
        let alert = NSAlert()
        alert.messageText = target.confirmationTitle
        alert.informativeText = BlockTarget.confirmationMessage
        alert.addButton(withTitle: "Block"); alert.addButton(withTitle: "Cancel")
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self else { return }
            Task { await self.model.chat.block(target) }
        }
    }
}

/// The shipping 400ms hover bubble, outside the scroll clip and without a
/// key window, mouse interception, per-row requests or a second name cache.
@MainActor public final class AppKitReactionTooltipPresenter {
    private let model: AppModel
    private weak var anchor: NSView?
    private var task: Task<Void, Never>?
    private var overlay: AppKitPointerPassthroughView?
    private var position: (() -> Void)?

    public init(model: AppModel) { self.model = model }

    public func hover(_ message: ChatMessage, emoji: String, entered: Bool, from anchor: NSView, in container: NSView) {
        if !entered { if self.anchor === anchor { close() }; return }
        close()
        self.anchor = anchor
        let context = model.chat.editingContext
        task = Task { [weak self, weak anchor, weak container] in
            do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
            guard let self, let anchor, let container, self.anchor === anchor,
                  anchor.window?.isKeyWindow == true, anchor.window?.attachedSheet == nil,
                  !anchor.isHiddenOrHasHiddenAncestor, !anchor.visibleRect.isEmpty,
                  self.model.chat.editingContext == context, !self.model.whatsNew.workspaceModalOpen else { return }
            self.show(messageID: message.id, emoji: emoji, anchor: anchor, container: container)
        }
    }

    public func close() {
        task?.cancel(); task = nil
        overlay?.removeFromSuperview(); overlay = nil
        position = nil; anchor = nil
    }

    private func show(messageID: String, emoji: String, anchor: NSView, container: NSView) {
        let overlay = AppKitPointerPassthroughView()
        let surface = AppKitAttachmentSurface {
            AppKitReactionTooltipContent(model: model, messageID: messageID, emoji: emoji) { [weak self] in self?.close() }
                .modifier(Entrance.anchored)
        }
        self.overlay = overlay
        overlay.addSubview(surface)
        container.addSubview(overlay)
        position = { [weak self, weak anchor, weak container, weak overlay, weak surface] in
            guard let self, let anchor, let container, let overlay, let surface else { return }
            guard anchor.window != nil, !anchor.visibleRect.isEmpty, !anchor.isHiddenOrHasHiddenAncestor else { self.close(); return }
            let chip = anchor.convert(anchor.bounds, to: container)
            let width = min(280, max(1, container.bounds.width - 16))
            let height = surface.height(for: width)
            let x = min(max(8, chip.minX), max(8, container.bounds.width - width - 8))
            let top = chip.minY - height - 6
            let y = min(max(8, top >= 0 ? top : chip.maxY + 6), max(8, container.bounds.height - height - 8))
            overlay.frame = NSRect(x: x, y: y, width: width, height: height)
            surface.frame = overlay.bounds
        }
        surface.heightChanged = { [weak self] in self?.position?() }
        position?()
    }
}

private final class AppKitPointerPassthroughView: NSView {
    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

private struct AppKitReactionTooltipContent: View {
    let model: AppModel
    let messageID: String
    let emoji: String
    let close: () -> Void
    private var chat: ChatModel { model.chat }
    private var message: ChatMessage? { chat.displayedMessages.first { $0.id == messageID && !$0.isDeleted } }
    private var available: Bool { message?.reactions?.contains { $0.emoji == emoji } == true }

    var body: some View {
        Group {
            if let message, available {
                ReactionTooltipBubble(emoji: emoji, text: NativePresentation.reactionSummary(message, emoji: emoji, chat: chat, viewerID: chat.viewerID))
            }
        }
        .task(id: "\(chat.editingContext):\(message?.reactionSeq ?? "0")") {
            guard available else { close(); return }
            _ = chat.requestReactors(messageID: messageID)
        }
        .onChange(of: available) { _, value in if !value { close() } }
        .onChange(of: chat.editingContext) { _, _ in close() }
        .onChange(of: model.whatsNew.workspaceModalOpen) { _, open in if open { close() } }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didResignKeyNotification)) { _ in close() }
    }
}

/// A measured media leaf, created only for staged files or realized messages.
/// The native parent owns its width and scrolling; SwiftUI owns the existing
/// gallery/editor controls and reports content-height changes (including loads).
@MainActor public final class AppKitAttachmentSurface: NSView {
    public var heightChanged: (() -> Void)?
    private let content: AnyView
    private let host = NSHostingView(rootView: AnyView(EmptyView()))
    private var measuredWidth: CGFloat = 0
    private var reportedHeight: CGFloat = 0
    public override var isFlipped: Bool { true }

    fileprivate init<Content: View>(@ViewBuilder content: () -> Content) {
        self.content = AnyView(content().preferredColorScheme(.dark).tint(CaperTheme.terracottaBright))
        super.init(frame: .zero)
        host.sizingOptions = [.intrinsicContentSize]
        addSubview(host)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public func height(for width: CGFloat) -> CGFloat {
        let width = max(1, width)
        if measuredWidth != width {
            measuredWidth = width
            host.rootView = AnyView(content.frame(width: width, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .background(GeometryReader { geometry in
                    Color.clear.onChange(of: geometry.size.height, initial: true) { [weak self] _, value in
                        // Never re-enter native layout from a SwiftUI layout pass.
                        DispatchQueue.main.async { [weak self] in
                            guard let self, self.measuredWidth == width, abs(self.reportedHeight - value) > 0.5 else { return }
                            self.reportedHeight = value
                            self.heightChanged?()
                        }
                    }
                }.allowsHitTesting(false)))
        }
        return ceil(host.fittingSize.height)
    }

    public override func layout() {
        super.layout()
        host.frame = bounds
    }
}

/// Owns full-window media presentation and captures picker targets before
/// asynchronous selection. Uploads, edits and send recovery remain ChatModel's.
@MainActor public final class AppKitMediaPresenter {
    private let model: AppModel
    private weak var parent: NSViewController?
    private var host: NSHostingController<AnyView>?
    private weak var previousResponder: NSResponder?

    public init(model: AppModel, parent: NSViewController) { self.model = model; self.parent = parent }

    public func composerAttachments(inThread: Bool = false) -> AppKitAttachmentSurface {
        AppKitAttachmentSurface {
            AppKitComposerAttachments(chat: model.chat, inThread: inThread)
            .environment(\.attachmentEditor, AttachmentEditorAction { [weak self] request in
                self?.show { close in AttachmentEditor(request: request, close: close) }
            })
        }
    }

    public func messageAttachments(_ message: ChatMessage) -> AppKitAttachmentSurface? {
        let files = message.content.attachments ?? []
        guard !message.isDeleted, !files.isEmpty || message.content.gif != nil || message.forward != nil else { return nil }
        let chat = model.chat
        return AppKitAttachmentSurface {
            VStack(alignment: .leading, spacing: 8) {
                if let gif = message.content.gif { MessageGifView(gif: gif) }
                if !files.isEmpty {
                    MessageAttachmentsView(attachments: files, chat: chat, isAvailable: {
                        chat.channelID == message.channelId && chat.displayedMessages.contains { $0.id == message.id && !$0.isDeleted }
                    })
                }
                ForwardCardView(message: message, chat: chat) {
                    Task {
                        guard chat.channelID == message.channelId,
                              let current = chat.displayedMessages.first(where: { $0.id == message.id }),
                              !current.isDeleted, current.forward?.message != nil else { return }
                        _ = await chat.onOpenForward?(current)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .environment(\.mediaViewer, MediaViewerAction { [weak self] request in
                self?.showViewer(request)
            })
        }
    }

    fileprivate func showViewer(_ request: MediaViewerRequest) {
        show { close in MediaViewer(request: request, close: close) }
    }

    public func pendingMessage(_ pending: PendingMessage) -> AppKitAttachmentSurface {
        AppKitAttachmentSurface { AppKitPendingMessage(pending: pending, chat: model.chat) }
    }

    /// Keep the shipping quote's thumbnails, blocked/deleted states, expansion
    /// and reduced-motion behavior without hosting either conversation list.
    public func replyContext(_ message: ChatMessage) -> AppKitAttachmentSurface? {
        guard !message.isDeleted, let reference = message.replyTo else { return nil }
        return AppKitAttachmentSurface {
            NativeReplyContext(reference: reference, motionKey: message.id, chat: model.chat)
        }
    }

    public func replyDraft(inThread: Bool = false) -> AppKitAttachmentSurface {
        AppKitAttachmentSurface { AppKitReplyDraft(chat: model.chat, inThread: inThread) }
    }

    public func pickFiles(inThread: Bool = false) {
        guard let window = parent?.view.window, let target = model.chat.attachmentTarget(inThread: inThread) else { return }
        let chat = model.chat
        let panel = NSOpenPanel()
        panel.canChooseFiles = true; panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Add files"
        panel.beginSheetModal(for: window) { result in
            guard result == .OK else { return }
            let files = panel.urls.prefix(AttachmentPolicy.maxAttachments).compactMap { try? AttachmentStaging.stage(copying: $0) }
            chat.addAttachments(files, to: target)
            if files.count < min(panel.urls.count, AttachmentPolicy.maxAttachments), chat.attachmentNotice(target: target) == nil {
                chat.setAttachmentNotice("Some files couldn’t be added.", target: target)
            } else if panel.urls.count > AttachmentPolicy.maxAttachments, chat.attachmentNotice(target: target) == nil {
                chat.setAttachmentNotice("You can attach up to \(AttachmentPolicy.maxAttachments) files.", target: target)
            }
        }
    }

    private func show<Content: View>(@ViewBuilder content: (@escaping () -> Void) -> Content) {
        guard host == nil, let parent, let container = parent.view as? AppKitWorkspaceContainer,
              container.dialogView == nil else { return }
        let host = NSHostingController(rootView: AnyView(content { [weak self] in self?.close() }
            .preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)))
        self.host = host
        previousResponder = parent.view.window?.firstResponder
        parent.addChild(host)
        host.view.frame = parent.view.bounds
        host.view.autoresizingMask = [.width, .height]
        parent.view.addSubview(host.view)
        container.dialogView = host.view
        parent.view.window?.makeFirstResponder(host.view)
        model.whatsNew.workspaceModalOpen = true
    }

    private func close() {
        guard let host else { return }
        let window = host.view.window
        (host.parent?.view as? AppKitWorkspaceContainer)?.dialogView = nil
        host.view.removeFromSuperview(); host.removeFromParent()
        self.host = nil
        window?.makeFirstResponder(previousResponder)
        previousResponder = nil
        model.whatsNew.workspaceModalOpen = false
    }
}

private struct AppKitPendingMessage: View {
    let pending: PendingMessage
    let chat: ChatModel

    var body: some View {
        PendingMessageRow(pending: pending, author: chat.currentAuthor, error: chat.error,
            rejected: chat.sendRejected,
            canEdit: pending.threadRootId == nil ? chat.draft.isEmpty && chat.attachmentDrafts.isEmpty
                : pending.threadRootId == chat.threadRootID && chat.threadDraft.isEmpty && chat.threadAttachmentDrafts.isEmpty,
            chat: chat, uploadFailed: chat.sendUploadFailed,
            retry: { Task { await chat.send(inThread: pending.threadRootId != nil, retryPending: true) } },
            edit: { _ = chat.discardRejected(edit: true) },
            dismiss: { _ = chat.discardRejected() })
    }
}

private struct AppKitReplyDraft: View {
    let chat: ChatModel
    let inThread: Bool

    var body: some View {
        let root = inThread ? chat.threadRootID : "channel"
        if let root, let reference = chat.replyTargets[root] {
            NativeReplyDraft(reference: reference, chat: chat, root: root)
        }
    }
}

private struct AppKitComposerAttachments: View {
    let chat: ChatModel
    let inThread: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let notice = inThread ? chat.threadAttachmentNotice : chat.attachmentNotice {
                Text(notice).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
            AttachmentDraftsView(chat: chat, inThread: inThread)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

public enum AppKitComposerCounter {
    public static func color(_ count: Int) -> NSColor { NSColor(ComposerCounter.tone(count)) }
}

/// The shipping pickers mounted only while open. AppKit owns the anchor,
/// dismissal and saved editor selection; ChatModel still owns GIF delivery.
@MainActor public final class AppKitComposerPicker: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private let inThread: Bool
    private var popover: NSPopover?
    private var restoreFocus: (() -> Void)?

    public init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread; super.init()
    }

    public func reaction(to message: ChatMessage, from anchor: NSView, composer: ComposerTextView) {
        let chat = model.chat, context = model.chat.editingContext
        guard chat.canInteract, !message.isDeleted, chat.channelID == message.channelId else { return }
        show(from: anchor, composer: composer) { close in
            ReactionPicker(close: close) { emoji in
                close()
                Task {
                    guard chat.editingContext == context, chat.canInteract,
                          chat.displayedMessages.contains(where: { $0.id == message.id && !$0.isDeleted }) else { return }
                    await chat.setReaction(messageID: message.id, emoji: emoji, active: true)
                }
            }
        }
    }

    public func emoji(from anchor: NSView, composer: ComposerTextView) {
        guard composer.isEditable, !composer.hasMarkedText(), model.chat.canInteract else { return }
        let text = composer.string, selection = composer.selectedRange()
        let context = model.chat.editingContext, root = model.chat.threadRootID
        show(from: anchor, composer: composer) { close in
            ReactionPicker(title: "Insert emoji", close: close) { [weak self, weak composer] emoji in
                close()
                guard let self, let composer, composer.isEditable, !composer.hasMarkedText(),
                      self.model.chat.editingContext == context, !self.inThread || self.model.chat.threadRootID == root,
                      self.model.chat.canInteract, composer.string == text,
                      let result = EmojiAutocomplete.inserting(emoji, in: text, selection: selection) else { return }
                // Preserve native undo and the formatter's UTF-16/limit rules.
                composer.window?.makeFirstResponder(composer)
                composer.breakUndoCoalescing()
                composer.insertText(result.text, replacementRange: NSRange(location: 0, length: text.utf16.count))
                composer.setSelectedRange(result.selection)
                composer.breakUndoCoalescing()
            }
        }
    }

    public func gifs(from anchor: NSView, composer: ComposerTextView) {
        let chat = model.chat, inThread = self.inThread
        guard composer.isEditable, !composer.hasMarkedText(), chat.canSendGif(inThread: inThread), let configuration = chat.klipy else { return }
        let context = chat.editingContext, root = chat.threadRootID
        show(from: anchor, composer: composer) { close in
            KlipyPicker(configuration: configuration, close: close) { gif, query in
                close()
                Task {
                    // Navigation/account changes must not retarget a queued selection.
                    guard chat.editingContext == context, !inThread || chat.threadRootID == root,
                          chat.canSendGif(inThread: inThread) else { return }
                    await chat.sendGif(gif, inThread: inThread, query: query)
                }
            }
        }
    }

    private func show<Content: View>(from anchor: NSView, composer: ComposerTextView,
                                    @ViewBuilder content: (@escaping () -> Void) -> Content) {
        guard let window = anchor.window, window.attachedSheet == nil, !model.whatsNew.workspaceModalOpen else { return }
        close(restoringFocus: false)
        let context = model.chat.editingContext, root = model.chat.threadRootID
        restoreFocus = { [weak self, weak composer, weak window] in
            guard let self, let composer, let window, composer.window === window, composer.isEditable,
                  self.model.chat.editingContext == context, !self.inThread || self.model.chat.threadRootID == root,
                  !self.model.whatsNew.workspaceModalOpen else { return }
            window.makeFirstResponder(composer)
        }
        let popover = NSPopover()
        self.popover = popover
        popover.behavior = .transient
        popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        popover.delegate = self
        let host = NSHostingController(rootView: content { [weak self] in self?.close() }
            .preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
            .onExitCommand { [weak self] in self?.close() })
        host.sizingOptions = [.preferredContentSize]
        popover.contentViewController = host
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
    }

    public func close(restoringFocus: Bool = true) {
        if !restoringFocus { restoreFocus = nil }
        popover?.animates = false
        popover?.close()
    }
    public func closeIfUnavailable(composer: ComposerTextView) {
        if !composer.isEditable || model.whatsNew.workspaceModalOpen { close(restoringFocus: false) }
    }
    public func popoverShouldClose(_ popover: NSPopover) -> Bool { popover.animates = false; return true }
    public func popoverDidClose(_ notification: Notification) {
        guard let closed = notification.object as? NSPopover else { return }
        closed.contentViewController = nil
        guard popover === closed else { return }
        popover = nil
        let restore = restoreFocus; restoreFocus = nil
        restore?()
    }
}
#endif
