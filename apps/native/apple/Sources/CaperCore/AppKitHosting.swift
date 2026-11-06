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

    public init(model: AppModel) { self.model = model }

    public func show(_ item: WorkspaceSheet, in parent: NSViewController) {
        show(AppKitWorkspaceDialogState(item: .workspace(item)), in: parent)
    }

    public func leaveChannel(_ channel: Channel, in parent: NSViewController) {
        show(AppKitWorkspaceDialogState(item: .leaveChannel(channel)), in: parent)
    }

    private func show(_ state: AppKitWorkspaceDialogState, in parent: NSViewController) {
        guard host == nil, let container = parent.view as? AppKitWorkspaceContainer, container.dialogView == nil else { return }
        let host = NSHostingController(rootView: AppKitWorkspaceDialog(model: model, state: state) { [weak self] in self?.close() })
        self.host = host
        previousResponder = parent.view.window?.firstResponder
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
        let window = host.view.window
        (host.parent?.view as? AppKitWorkspaceContainer)?.dialogView = nil
        host.view.removeFromSuperview()
        host.removeFromParent()
        self.host = nil
        window?.makeFirstResponder(previousResponder)
        previousResponder = nil
        model.whatsNew.workspaceModalOpen = false
    }
}

private enum AppKitWorkspaceDialogItem {
    case workspace(WorkspaceSheet), leaveChannel(Channel)
    var id: String {
        switch self {
        case .workspace(let item): item.id
        case .leaveChannel(let channel): "leave-channel-\(channel.id)"
        }
    }
}

@MainActor @Observable private final class AppKitWorkspaceDialogState {
    var item: AppKitWorkspaceDialogItem
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
                .frame(width: min(item.id.contains("manage") ? 600 : 560, geometry.size.width - 32))
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
        .onDisappear { model.whatsNew.workspaceModalOpen = false }
        .preferredColorScheme(.dark).tint(CaperTheme.terracottaBright)
    }

    @ViewBuilder private func content(_ item: AppKitWorkspaceDialogItem) -> some View {
        switch item {
        case .workspace(let sheet):
            WorkspaceSheetView(item: sheet, model: model, show: { state.item = .workspace($0) }) {
                // An old request must not close the dialog replacing it.
                if state.item.id == item.id { dismiss() }
            }
        case .leaveChannel(let channel):
            ConfirmationSheet(title: "Leave #\(channel.name)?",
                detail: channel.private && !model.isOwner
                    ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call."
                    : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call.",
                action: "Leave channel", pendingLabel: "Leaving…", close: dismiss) { try await model.leaveChannel(channel) }
        }
    }

    private func requestClose() {
        if state.item.id == "profile" { state.profileDismissAction?.action() }
        else if !state.dismissDisabled { dismiss() }
    }
}

/// On-demand profile surfaces shared with the shipping Mac app. The workspace,
/// timeline and main composer remain AppKit; no hosting view exists until open.
@MainActor public final class AppKitProfilePresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
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
            popover.animates = false
            popover.performClose(nil)
            guard !popover.isShown else { return }
        }
        let popover = NSPopover()
        self.popover = popover
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
        popover = nil; draft = nil; dismissAction = nil
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
        guard !message.isDeleted, let files = message.content.attachments, !files.isEmpty else { return nil }
        let chat = model.chat
        return AppKitAttachmentSurface {
            MessageAttachmentsView(attachments: files, chat: chat, isAvailable: {
                chat.channelID == message.channelId && chat.messages.contains { $0.id == message.id && !$0.isDeleted }
            })
            .environment(\.mediaViewer, MediaViewerAction { [weak self] request in
                self?.show { close in MediaViewer(request: request, close: close) }
            })
        }
    }

    public func pendingMessage(_ pending: PendingMessage) -> AppKitAttachmentSurface {
        AppKitAttachmentSurface { AppKitPendingMessage(pending: pending, chat: model.chat) }
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
            rejected: chat.sendRejected, canEdit: chat.draft.isEmpty && chat.attachmentDrafts.isEmpty,
            chat: chat, uploadFailed: chat.sendUploadFailed,
            retry: { Task { await chat.send(retryPending: true) } },
            edit: { _ = chat.discardRejected(edit: true) },
            dismiss: { _ = chat.discardRejected() })
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
#endif
