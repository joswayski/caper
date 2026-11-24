#if os(macOS)
import AppKit
import SwiftUI

/// A native workspace root whose AX tree follows its modal child without
/// freezing a snapshot of the changing navigation and message descendants.
@MainActor public final class AppKitWorkspaceContainer: NSView {
    fileprivate weak var dialogView: NSView?
    private var themeObserver: NSObjectProtocol?
    public override var isFlipped: Bool { true }
    public init() {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = CaperThemeStore.frameColor.cgColor
        themeObserver = NotificationCenter.default.addObserver(
            forName: CaperThemeStore.didChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.layer?.backgroundColor = CaperThemeStore.frameColor.cgColor
                self?.needsDisplay = true
            }
        }
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit {
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
    }
    public override func accessibilityChildren() -> [Any]? {
        if let dialogView { return [dialogView] }
        return super.accessibilityChildren()
    }
}

/// Workspace dialogs share validation, permissions and pending / unsaved-edit
/// guards. The presenter owns native content, focus and modal accessibility.
@MainActor public final class AppKitWorkspacePresenter {
    private let model: AppModel
    private var host: NSViewController?
    private var profile: AppKitProfilePresentation?
    private var dialogMedia: NSViewController?
    private weak var mediaResponder: NSResponder?
    private var state: AppKitWorkspaceDialogState?
    private let observation = AppKitObservation()
    private weak var previousResponder: NSResponder?
    private var previousContext: String?
    private var focusContext: String { [model.chat.editingContext, model.chat.threadRootID ?? "channel"].joined(separator: ":") }
    /// Set by the workspace: focuses its composer when nothing else should have focus.
    public var focusComposer: (() -> Void)?

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
        // A link opened from another app replaces any open dialog, as in CaperView.
        if model.deliverableInviteLink != nil {
            if host != nil { close() }
            if (parent.view as? AppKitWorkspaceContainer)?.dialogView == nil, let link = model.takeInviteLink() {
                show(.joinInvite(link), in: parent)
            }
            return
        }
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

    /// ⌘K and ⌘/: each opens over the workspace, or closes itself when it is
    /// already open. Another dialog keeps the keys.
    public func toggleQuickSwitcher(in parent: NSViewController) { toggle(.switcher, in: parent) }
    public func toggleShortcutHelp(in parent: NSViewController) { toggle(.shortcuts, in: parent) }
    /// Whether a workspace dialog is open (⌥↑/↓ then leave the conversation alone).
    public var isShowingDialog: Bool { host != nil }
    private func toggle(_ item: AppKitWorkspaceDialogItem, in parent: NSViewController) {
        if let open = state?.item {
            switch (open, item) {
            case (.switcher, .switcher), (.shortcuts, .shortcuts): close(); return
            case (.switcher, _), (.shortcuts, _): close()
            default: return
            }
        }
        show(AppKitWorkspaceDialogState(item: item), in: parent)
    }

    public func leaveChannel(_ channel: Channel, in parent: NSViewController) {
        show(AppKitWorkspaceDialogState(item: .leaveChannel(channel)), in: parent)
    }

    /// A reload changes editingContext's generation, but remains in this
    /// account/conversation. Native timeline anchors use this stable identity.
    public var conversationContext: String { "\(model.chat.channelID ?? ""):\(model.chat.viewerID ?? "")" }
    public var messageContext: String { model.chat.editingContext }
    public var isPresenting: Bool { model.whatsNew.workspaceModalOpen }
    public func currentMessage(_ message: ChatMessage) -> ChatMessage? {
        guard message.channelId == model.chat.channelID else { return nil }
        return model.chat.displayedMessages.first { $0.id == message.id }
            ?? model.chat.displayedPins.first { $0.id == message.id }
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

    /// Block starts on Cancel like the other destructive confirmations; a system
    /// alert would make Block the Return default.
    public func confirmBlock(_ target: BlockTarget, in parent: NSViewController) {
        guard target.canBlock, target.id != model.account?.id else { return }
        show(AppKitWorkspaceDialogState(item: .block(target)), in: parent)
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
        } else if let item = state?.item {
            switch item {
            case .forward, .forwardConversation: close()
            default: break
            }
        }
    }

    /// Swaps the open dialog for another in place (Create a space → Join with
    /// invite link): the backdrop stays, nothing behind it takes focus in
    /// between, and closing the new dialog restores the original focus.
    private func replace(with item: WorkspaceSheet, in parent: NSViewController) {
        let responder = previousResponder, context = previousContext
        close(handingOff: true)
        show(AppKitWorkspaceDialogState(item: .workspace(item)), in: parent, animated: false)
        guard host != nil else { model.whatsNew.workspaceModalOpen = false; focusComposer?(); return }
        previousResponder = responder; previousContext = context
    }

    private func show(_ state: AppKitWorkspaceDialogState, in parent: NSViewController, animated: Bool = true) {
        guard host == nil, let container = parent.view as? AppKitWorkspaceContainer, container.dialogView == nil else { return }
        let dismiss: () -> Void = { [weak self] in
            guard let self, self.state === state else { return }
            self.close()
        }
        guard let content = nativeContent(state.item, close: dismiss) else { return }
        let host = NSViewController()
            if let profile { host.addChild(profile) }
            host.view = AppKitDialogFrame(content: content, width: state.item.width, animated: animated, requestClose: { [weak self] in
                if let profile = self?.profile { profile.requestClose() } else { dismiss() }
            })
            // As the SwiftUI dialog: message dialogs end with their conversation,
            // and every dialog ends with the account.
            let messageScoped = state.item.messageID != nil
            let context = model.chat.editingContext, account = model.account?.id
            observation.start(reading: { [model] in (model.chat.editingContext, model.account?.id) }) { now in
                if now.1 != account || (messageScoped && now.0 != context) { dismiss() }
            }
        self.host = host
        self.state = state
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
        if let focus = content.initialFocus, parent.view.window?.makeFirstResponder(focus) == true {} else {
            parent.view.window?.makeFirstResponder(host.view)
        }
        model.whatsNew.workspaceModalOpen = true
    }

    private func nativeContent(_ item: AppKitWorkspaceDialogItem, close: @escaping () -> Void) -> AppKitDialogContent? {
        switch item {
        case .activity:
            return AppKitActivityInbox(model: model, close: close)
        case .switcher:
            return AppKitQuickSwitcher(model: model, close: close)
        case .shortcuts:
            return AppKitShortcutHelp(close: close)
        case .edit(let message):
            return AppKitMessageEditor(chat: model.chat, message: message, close: close)
        case .forward(let message):
            return AppKitForwardPicker(chat: model.chat, message: message, close: close,
                showMedia: { [weak self] in self?.showDialogMedia($0) })
        case .forwardConversation(let message):
            return AppKitForwardConversation(chat: model.chat, messageID: message.id, close: close,
                showMedia: { [weak self] in self?.showDialogMedia($0) })
        case .leaveChannel(let channel):
            return AppKitConfirmationDialog(title: "Leave #\(channel.name)?",
                detail: channel.private && !model.isOwner
                    ? "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call."
                    : "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call.",
                action: "Leave channel", pendingLabel: "Leaving…", close: close) { [model] in try await model.leaveChannel(channel) }
        case .delete(let message):
            return AppKitConfirmationDialog(title: MessageDeletion.confirmTitle, detail: MessageDeletion.confirmMessage,
                action: "Delete", pendingLabel: "Deleting…", close: close) { [model] in await model.chat.deleteMessage(message) }
        case .block(let target):
            return AppKitConfirmationDialog(title: target.confirmationTitle, detail: BlockTarget.confirmationMessage,
                action: "Block", pendingLabel: "Blocking…", close: close) { [model] in await model.chat.block(target) }
        case .history(let message):
            return AppKitMessageHistory(chat: model.chat, message: message, close: close)
        case .reactions(let message, let emoji):
            return AppKitReactors(chat: model.chat, messageID: message.id, emoji: emoji, viewerID: model.chat.viewerID, close: close)
        case .workspace(.invitation(let space)):
            var lines = [AppKitKit.label("Join \(space.name)?", size: 20, weight: .bold)]
            if let inviter = space.inviter {
                lines.append(AppKitKit.label("\(inviter.displayName) (@\(inviter.username)) invited you.", size: 13, color: AppKitKit.muted))
            }
            return AppKitInvitationDialog(title: "You’re invited!", detail: nil, lines: lines, pendingLabels: true, close: close,
                decline: { [model] in try await model.declineInvitation(space) },
                accept: { [model] in try await model.acceptInvitation(space) })
        case .workspace(.channelInvitation(let invitation)):
            let line = AppKitKit.label("\(invitation.inviter.displayName) (@\(invitation.inviter.username)) invited you to private channel #\(invitation.channel.name).",
                                       size: 12, color: AppKitKit.muted)
            return AppKitInvitationDialog(title: "Private channel invitation", detail: "#\(invitation.channel.name)", lines: [line],
                pendingLabels: false, close: close,
                decline: { [model] in try await model.declineChannelInvitation(invitation) },
                accept: { [model] in try await model.acceptChannelInvitation(invitation) })
        case .workspace(.privacy):
            return AppKitPrivacyDialog(model: model, close: close)
        case .workspace(.notifications):
            return AppKitNotificationsDialog(model: model, close: close)
        case .workspace(.audio):
            return AppKitAudioTest(voice: model.voice, debugEnabled: model.account?.debugEnabled == true, close: close)
        case .workspace(.connection):
            return AppKitAudioDiagnostics(voice: model.voice, kind: .connection, close: close)
        case .workspace(.diagnostics):
            if model.account?.debugEnabled == true { return AppKitAudioDiagnostics(voice: model.voice, kind: .processing, close: close) }
            return AppKitSettingsDialog(title: "Audio diagnostics", detail: "Local processing counters", closeLabel: "Close audio diagnostics", identifier: "audio-diagnostics", close: close)
        case .workspace(.profile):
            let profile = AppKitProfilePresentation(model: model, close: close)
            self.profile = profile
            _ = profile.view
            return profile.form
        case .workspace(.joinInvite(let opened)):
            return AppKitJoinInvite(model: model, opened: opened, close: close)
        case .workspace(.createSpace), .workspace(.manageSpace):
            let managing = item.id == "manage-space"
            return AppKitSpaceEditor(model: model, managing: managing, close: close) { [weak self] in
                guard let self, let parent = self.host?.parent else { return }
                self.replace(with: .joinInvite(nil), in: parent)
            }
        case .workspace(.invitePeople(let spaceID, let channelID)):
            guard model.detail?.space.id == spaceID else { return nil }
            return AppKitSpaceEditor(model: model, managing: true, inviteOnly: true, channelID: channelID, close: close, joinInvite: {})
        case .workspace(.createChannel):
            return AppKitChannelEditor(model: model, channel: nil, close: close)
        case .workspace(.manageChannel(let channel)):
            return AppKitChannelEditor(model: model, channel: channel, close: close)
        case .workspace(.newDirectMessage):
            return AppKitNewDirectMessageDialog(model: model, close: close)
        case .workspace(.leaveSpace):
            return AppKitConfirmationDialog(title: "Leave \(model.detail?.space.name ?? "space")?",
                detail: "You will lose access to its channels and conversations. An owner can add you again later.",
                action: "Leave space", pendingLabel: "Leaving…", close: close) { [model] in try await model.leaveCurrentSpace() }
        case .workspace(.login):
            // Authentication belongs to the native root, not a workspace dialog.
            return nil
        }
    }

    private func close(handingOff: Bool = false) {
        guard let host, let state else { return }
        closeDialogMedia()
        profile?.stop(); profile?.removeFromParent(); profile = nil
        let item = state.item
        let window = host.view.window
        let parent = host.parent
        state.closed = true
        observation.stop()
        self.state = nil
        let container = host.parent?.view as? AppKitWorkspaceContainer
        container?.dialogView = nil
        host.view.removeFromSuperview()
        host.removeFromParent()
        self.host = nil
        if let container { window?.invalidateCursorRects(for: container) }
        switch item {
        case .activity: model.activityShown = false
        case .forward(let message):
            if model.chat.forwardTarget?.id == message.id { model.chat.forwardTarget = nil }
        case .forwardConversation(let message):
            if model.chat.forwardConversationTarget?.id == message.id { model.chat.forwardConversationTarget = nil }
        default: break
        }
        if handingOff { return }
        // Opening an inbox destination can replace the old conversation/root.
        // Never put focus back into a detached or different conversation editor.
        var restoreComposer = false
        if previousContext == focusContext,
           let responder = previousResponder as? NSView, responder.window === window, !responder.isHiddenOrHasHiddenAncestor,
           (responder as? NSTextView)?.isEditable != false {
            window?.makeFirstResponder(responder)
        } else { restoreComposer = true }
        previousResponder = nil
        previousContext = nil
        model.whatsNew.workspaceModalOpen = false
        // Nothing useful to restore (message text, the window, another
        // conversation): focus returns to the composer, as on web.
        if restoreComposer { focusComposer?() }
        if let parent { presentInvitations(in: parent) }
    }

    /// A native forwarding dialog's media covers the full workspace, not the
    /// small preview leaf. It temporarily owns AX and focus above the dialog.
    private func showDialogMedia(_ request: MediaViewerRequest) {
        guard dialogMedia == nil, let host,
              let container = host.parent?.view as? AppKitWorkspaceContainer else { return }
        let media = AppKitMediaViewer(request: request) { [weak self] in self?.closeDialogMedia() }
        dialogMedia = media; mediaResponder = host.view.window?.firstResponder
        host.addChild(media)
        media.view.frame = host.view.bounds; media.view.autoresizingMask = [.width, .height]
        host.view.addSubview(media.view)
        container.dialogView = media.view
        host.view.window?.makeFirstResponder(media.view)
    }

    private func closeDialogMedia() {
        guard let media = dialogMedia else { return }
        let window = media.view.window
        media.view.removeFromSuperview(); media.removeFromParent(); dialogMedia = nil
        (host?.parent?.view as? AppKitWorkspaceContainer)?.dialogView = host?.view
        if let responder = mediaResponder as? NSView, responder.window === window { window?.makeFirstResponder(responder) }
        mediaResponder = nil
    }
}

private enum AppKitWorkspaceDialogItem {
    case workspace(WorkspaceSheet), leaveChannel(Channel)
    case activity, switcher, shortcuts
    case edit(ChatMessage), history(ChatMessage), delete(ChatMessage)
    case reactions(ChatMessage, String)
    case forward(ChatMessage), forwardConversation(ChatMessage)
    case block(BlockTarget)
    var id: String {
        switch self {
        case .workspace(let item): item.id
        case .leaveChannel(let channel): "leave-channel-\(channel.id)"
        case .activity: "activity-inbox"
        case .switcher: "quick-switcher"
        case .shortcuts: "keyboard-shortcuts"
        case .edit(let message): "edit-\(message.id)"
        case .history(let message): "history-\(message.id)"
        case .delete(let message): "delete-\(message.id)"
        case .reactions(let message, let emoji): "reactions-\(message.id)-\(emoji)"
        case .forward(let message): "forward-\(message.id)"
        case .forwardConversation(let message): "forward-conversation-\(message.id)"
        case .block(let target): "block-\(target.id)"
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

/// The workspace supplies native pin cards in a header-anchored popover.
/// Reading pins does not require an editable composer (channel previews too).
@MainActor public final class AppKitPinsPresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
    private var restoreFocus: (() -> Void)?

    public init(model: AppModel) {
        self.model = model; super.init()
    }

    public var isAvailable: Bool { model.chat.channelID != nil }

    public func show(from anchor: NSView, composer: ComposerTextView,
                     makeContent: (@escaping () -> Void) -> NSViewController) {
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
        // Keep the whole card inside the window: at narrow widths a popover
        // centred on the header's pin button would hang past the window edge,
        // where its Go to message buttons cannot be reached.
        guard let content = window.contentView else { return }
        let margin: CGFloat = 12
        let width = min(470, max(240, content.bounds.width - 2 * margin - 26))
        let pin = anchor.convert(anchor.bounds, to: content)
        let half = (width + 26) / 2
        let x = min(max(pin.midX, half + margin), content.bounds.width - half - margin)
        let rect = NSRect(x: x - 1, y: pin.minY, width: 2, height: pin.height)
        let popover = NSPopover()
        self.popover = popover
        popover.behavior = .transient
        popover.animates = !CaperAccessibility.reduceMotion
        popover.delegate = self
        let host = makeContent { [weak self] in self?.close() }
        host.preferredContentSize = NSSize(width: width, height: 360)
        host.view.setFrameSize(host.preferredContentSize)
        popover.contentViewController = host
        popover.show(relativeTo: rect, of: content, preferredEdge: content.isFlipped ? .maxY : .minY)
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

/// Native profile cards retain the rich composer and media/editor leaves.
@MainActor public final class AppKitProfilePresenter: NSObject, NSPopoverDelegate {
    private let model: AppModel
    private var popover: NSPopover?
    private var personID: String?
    private var draft: ProfileMessageDraft?
    private var dismissAction: ProfileDismissAction?
    private weak var anchor: NSView?
    /// Set by the workspace: where focus goes when the card closes and its opener can't take it.
    public var focusComposer: (() -> Void)?
    /// Set by the workspace: shows the Block confirmation dialog.
    public var requestBlock: ((BlockTarget) -> Void)?

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
        guard view.window != nil else { return }
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
        popover.animates = !CaperAccessibility.reduceMotion
        popover.delegate = self
        let host = AppKitProfileCard(model: model, person: person,
            requestBlock: { [weak self] target in self?.requestBlock?(target) }, close: { [weak popover] in
                // Entrances animate, closing is instant on every motion setting.
                popover?.animates = false
                popover?.close()
            }, dismissChanged: { [weak self] action in self?.dismissAction = action })
        popover.contentViewController = host
        anchor = view
        popover.show(relativeTo: rect, of: view, preferredEdge: .maxY)
    }

    public func popoverShouldClose(_ popover: NSPopover) -> Bool {
        // A hover-opened card stays open when its own anchor is then clicked;
        // that click would otherwise close it as an outside click.
        // The close request can arrive with the press or its release.
        if let anchor, let window = anchor.window,
           [.leftMouseDown, .leftMouseUp].contains(NSApp.currentEvent?.type),
           anchor.bounds.contains(anchor.convert(window.convertPoint(fromScreen: NSEvent.mouseLocation), from: nil)) { return false }
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
        // As on web, closing the card returns focus to its opener, or to the
        // composer when the opener can't take focus. A click that closed the
        // card into a field keeps that field, and a dialog the card opened
        // (Block's confirmation starts on Cancel) keeps its button.
        // Only a keyboard close (Escape) returns focus to a button opener; a
        // click shouldn't leave a focus ring on the button the pointer used.
        let byKeyboard = NSApp.currentEvent?.type == .keyDown
        DispatchQueue.main.async { [weak self, weak anchor] in
            guard let self, self.popover == nil, let anchor, let window = anchor.window,
                  (window.firstResponder as? NSTextView)?.isEditable != true, !(window.firstResponder is AppKitKitButton) else { return }
            if byKeyboard, anchor.acceptsFirstResponder, !anchor.isHiddenOrHasHiddenAncestor { window.makeFirstResponder(anchor) }
            else { self.focusComposer?() }
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
        // Ask for names now, so anyone this client can't name yet is
        // usually named by the time the bubble opens.
        _ = model.chat.requestReactors(messageID: message.id)
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
        let bubble = AppKitReactionTooltip(model: model, messageID: messageID, emoji: emoji) { [weak self] in self?.close() }
        // Already gone (the reaction was removed while the delay ran).
        guard self.anchor === anchor else { return }
        self.overlay = overlay
        overlay.addSubview(bubble)
        container.addSubview(overlay)
        position = { [weak self, weak anchor, weak container, weak overlay, weak bubble] in
            guard let self, let anchor, let container, let overlay, let bubble else { return }
            guard anchor.window != nil, !anchor.visibleRect.isEmpty, !anchor.isHiddenOrHasHiddenAncestor else { self.close(); return }
            let chip = anchor.convert(anchor.bounds, to: container)
            let size = bubble.size(maxWidth: min(280, max(1, container.bounds.width - 16)))
            let x = min(max(8, chip.minX), max(8, container.bounds.width - size.width - 8))
            let top = chip.minY - size.height - 6
            let y = min(max(8, top >= 0 ? top : chip.maxY + 6), max(8, container.bounds.height - size.height - 8))
            overlay.frame = NSRect(x: x, y: y, width: size.width, height: size.height)
            bubble.frame = overlay.bounds
        }
        bubble.changed = { [weak self] in self?.position?() }
        position?()
    }
}

private final class AppKitPointerPassthroughView: NSView {
    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// "You, Maya and Alex reacted with :tada:" beside the emoji's artwork. It
/// closes when the reaction goes away, the conversation changes, a dialog
/// opens or the window resigns key, and asks for names as reactions change.
@MainActor private final class AppKitReactionTooltip: NSView {
    private let model: AppModel
    private let messageID: String
    private let emoji: String
    private let close: () -> Void
    private let text = AppKitKit.label("", size: 12, weight: .bold)
    private let observation = AppKitObservation()
    private var requested: String?
    private var resign: NSObjectProtocol?
    var changed: (() -> Void)?

    init(model: AppModel, messageID: String, emoji: String, close: @escaping () -> Void) {
        self.model = model; self.messageID = messageID; self.emoji = emoji; self.close = close
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = CaperThemeStore.baseColor.cgColor
        layer?.cornerRadius = 8; layer?.borderWidth = 1; layer?.borderColor = AppKitKit.border.cgColor
        layer?.shadowColor = NSColor.black.cgColor; layer?.shadowOpacity = 0.35; layer?.shadowRadius = 8; layer?.shadowOffset = CGSize(width: 0, height: -3)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityIdentifier("reaction-tooltip")
        let art: NSView
        if let entry = EmojiArtwork.entry(for: emoji), let image = EmojiArtwork.image(for: entry) {
            // The artwork is 128 pixels, so 32 points stays crisp on Retina displays.
            let view = NSImageView(image: NSImage(cgImage: image, size: NSSize(width: 32, height: 32)))
            view.imageScaling = .scaleProportionallyDown
            view.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([view.widthAnchor.constraint(equalToConstant: 32), view.heightAnchor.constraint(equalToConstant: 32)])
            art = view
        } else {
            let glyph = NSTextField(labelWithString: emoji); glyph.font = .systemFont(ofSize: 26)
            art = glyph
        }
        let row = NSStackView(views: [art, text]); row.orientation = .horizontal; row.spacing = 10; row.alignment = .centerY
        row.edgeInsets = NSEdgeInsets(top: 8, left: 10, bottom: 8, right: 12)
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor), row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor), row.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        let context = model.chat.editingContext
        observation.start(reading: { [model, messageID, emoji] () -> (String?, String, Bool) in
            let chat = model.chat
            let message = (chat.displayedMessages.first { $0.id == messageID }
                ?? chat.displayedPins.first { $0.id == messageID }).flatMap { $0.isDeleted ? nil : $0 }
            let summary = message.flatMap { message in
                message.reactions?.contains { $0.emoji == emoji } == true
                    ? NativePresentation.reactionSummary(message, emoji: emoji, chat: chat, viewerID: chat.viewerID) : nil
            }
            return (summary, "\(chat.editingContext):\(message?.reactionSeq ?? "0")", model.whatsNew.workspaceModalOpen)
        }) { [weak self] summary, key, modal in
            guard let self else { return }
            guard let summary, !modal, key.hasPrefix(context + ":") else { self.close(); return }
            if self.requested != key { self.requested = key; _ = self.model.chat.requestReactors(messageID: self.messageID) }
            if self.text.stringValue != summary {
                self.text.stringValue = summary
                self.setAccessibilityLabel(summary)
                self.changed?()
            }
        }
        resign = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { resign.map(NotificationCenter.default.removeObserver) }

    override var isFlipped: Bool { true }

    /// The bubble hugs its text up to `maxWidth`.
    func size(maxWidth: CGFloat) -> NSSize {
        text.preferredMaxLayoutWidth = max(1, maxWidth - 64)
        let fitting = fittingSize
        return NSSize(width: min(maxWidth, ceil(fitting.width)), height: ceil(fitting.height))
    }
}

/// A measured media leaf, created only for staged files or realized messages.
/// Its parent supplies width; native content reports asynchronous height changes.
@MainActor public final class AppKitAttachmentSurface: NSView {
    public var heightChanged: (() -> Void)?
    private let native: NSView
    private var nativeWidth: NSLayoutConstraint?
    public override var isFlipped: Bool { true }

    init(native: NSView) {
        self.native = native; super.init(frame: .zero)
        native.translatesAutoresizingMaskIntoConstraints = false; addSubview(native)
        let width = native.widthAnchor.constraint(equalToConstant: 1); nativeWidth = width
        NSLayoutConstraint.activate([width, native.leadingAnchor.constraint(equalTo: leadingAnchor), native.topAnchor.constraint(equalTo: topAnchor)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public func height(for width: CGFloat) -> CGFloat {
        let width = max(1, width)
        nativeWidth?.constant = width
        native.setFrameSize(NSSize(width: width, height: native.frame.height))
        native.layoutSubtreeIfNeeded()
        return ceil(native.fittingSize.height)
    }
}

/// Owns full-window media presentation and captures picker targets before
/// asynchronous selection. Uploads, edits and send recovery remain ChatModel's.
@MainActor public final class AppKitMediaPresenter {
    private let model: AppModel
    private weak var parent: NSViewController?
    private var host: NSViewController?
    private weak var previousResponder: NSResponder?

    public init(model: AppModel, parent: NSViewController) { self.model = model; self.parent = parent }

    public func composerAttachments(inThread: Bool = false) -> AppKitAttachmentSurface {
        let chat = model.chat
        let content = AppKitComposerAccessories(chat: chat, inThread: inThread) { [weak self] file in
            self?.showController { close in
                if let request = NativeStagedFile.editRequest(file, chat: chat, inThread: inThread) {
                    return AppKitAttachmentEditor(request: request, close: close)
                }
                return AppKitFilePreview(url: file.localURL, close: close)
            }
        }
        let surface = AppKitAttachmentSurface(native: content)
        // Never re-enter AppKit layout from inside a child's layout pass.
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }
        return surface
    }

    public func messageAttachments(_ message: ChatMessage, beforeViewing: (() -> Void)? = nil) -> AppKitAttachmentSurface? {
        let files = message.content.attachments ?? []
        guard !message.isDeleted, !files.isEmpty || message.content.gif != nil || message.forward != nil
            || !(message.content.embeds ?? []).isEmpty || MentionAutocomplete.asksFeedback(message) else { return nil }
        let content = AppKitMessageExtras(message, chat: model.chat) { [weak self] request in
            beforeViewing?(); self?.showViewer(request)
        }
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }; return surface
    }

    fileprivate func showViewer(_ request: MediaViewerRequest) {
        showController { close in AppKitMediaViewer(request: request, close: close) }
    }

    public func pendingMessage(_ pending: PendingMessage) -> AppKitAttachmentSurface {
        let content = AppKitPendingRow(pending, chat: model.chat) { [weak self] in self?.showViewer($0) }
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }; return surface
    }

    public func pendingAttachments(_ pending: PendingMessage) -> AppKitAttachmentSurface? {
        guard !pending.attachments.isEmpty || pending.gif != nil || !pending.embeds.isEmpty else { return nil }
        let content = AppKitPendingExtras(pending) { [weak self] in self?.showViewer($0) }
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }; return surface
    }

    public func pendingReply(_ pending: PendingMessage) -> AppKitAttachmentSurface? {
        guard let reference = pending.replyTo else { return nil }
        let content = AppKitReplyContext(reference: reference, motionKey: pending.id, chat: model.chat)
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }; return surface
    }

    /// Keep the shipping quote's thumbnails, blocked/deleted states, expansion
    /// and reduced-motion behavior without hosting either conversation list.
    public func replyContext(_ message: ChatMessage) -> AppKitAttachmentSurface? {
        guard !message.isDeleted, let reference = message.replyTo else { return nil }
        let content = AppKitReplyContext(reference: reference, motionKey: message.id, chat: model.chat)
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }; return surface
    }

    public func replyDraft(inThread: Bool = false) -> AppKitAttachmentSurface {
        let content = AppKitComposerAccessories(chat: model.chat, inThread: inThread, onlyReply: true, preview: { _ in })
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }
        return surface
    }

    /// The draft's first link as a card above the composer, with × to send without it.
    public func composerLinkPreview(inThread: Bool = false) -> AppKitAttachmentSurface {
        let content = AppKitComposerPreview(chat: model.chat, inThread: inThread)
        let surface = AppKitAttachmentSurface(native: content)
        content.heightChanged = { [weak surface] in DispatchQueue.main.async { surface?.heightChanged?() } }
        return surface
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

    private func showController(_ content: (@escaping () -> Void) -> NSViewController) {
        guard host == nil, let parent, let container = parent.view as? AppKitWorkspaceContainer,
              container.dialogView == nil else { return }
        let host = content { [weak self] in self?.close() }
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
    /// The message whose reaction picker is open, so its row can stay highlighted.
    public private(set) var activeReactionMessageID: String?
    public var onReactionTargetChanged: ((String?) -> Void)?

    public init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread; super.init()
    }

    private func setActiveReactionMessageID(_ id: String?) {
        guard activeReactionMessageID != id else { return }
        activeReactionMessageID = id
        onReactionTargetChanged?(id)
    }

    public func reaction(to message: ChatMessage, from anchor: NSView, composer: ComposerTextView) {
        let chat = model.chat, context = model.chat.editingContext
        guard chat.canInteract, !message.isDeleted, chat.channelID == message.channelId else { return }
        showController(from: anchor, composer: composer, reactionMessageID: message.id) { close in
            AppKitReactionPicker(close: close) { emoji in
                close()
                Task {
                    guard chat.editingContext == context, chat.canInteract,
                          (chat.displayedMessages.first(where: { $0.id == message.id })
                            ?? chat.displayedPins.first(where: { $0.id == message.id }))?.isDeleted == false else { return }
                    await chat.setReaction(messageID: message.id, emoji: emoji, active: true)
                }
            }
        }
    }

    public func emoji(from anchor: NSView, composer: ComposerTextView) {
        guard composer.isEditable, !composer.hasMarkedText(), model.chat.canInteract else { return }
        let text = composer.string, selection = composer.selectedRange()
        let context = model.chat.editingContext, root = model.chat.threadRootID
        showController(from: anchor, composer: composer) { close in
            AppKitReactionPicker(title: "Insert emoji", close: close) { [weak self, weak composer] emoji in
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
        showController(from: anchor, composer: composer) { close in
            AppKitKlipyPicker(configuration: configuration, close: close) { gif, query in
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

    private func showController(from anchor: NSView, composer: ComposerTextView,
                                reactionMessageID: String? = nil,
                                content: (@escaping () -> Void) -> NSViewController) {
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
        popover.animates = !CaperAccessibility.reduceMotion
        popover.delegate = self
        popover.contentViewController = content { [weak self] in self?.close() }
        setActiveReactionMessageID(reactionMessageID)
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
    }

    public func close(restoringFocus: Bool = true) {
        if !restoringFocus { restoreFocus = nil }
        let hadPopover = popover != nil
        popover?.animates = false
        popover?.close()
        if !hadPopover { setActiveReactionMessageID(nil) }
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
        setActiveReactionMessageID(nil)
        let restore = restoreFocus; restoreFocus = nil
        restore?()
    }
}
#endif
