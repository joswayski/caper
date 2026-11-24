import AppKit
import CaperCore

@MainActor final class WorkspaceController: NSViewController, NSTextViewDelegate {
    private let model: AppModel
    private let inThread: Bool
    var showSettings: (() -> Void)?
    var showWhatsNew: (() -> Void)?
    private var presentationParent: NSViewController { parent as? WorkspaceController ?? self }
    private var threadPane: WorkspaceController?
    /// Closing the thread with its button or Escape focuses the composer.
    private var focusAfterThreadClose = false
    private let threadObservation = NativeObservation()
    private var preferredThreadWidth = CGFloat(UserDefaults.standard.object(forKey: "caper.threadPanelWidth") as? Double ?? 340)
    private var threadDragWidth: CGFloat?
    private lazy var threadDivider = NativePanelResizeHandle("Thread width", identifier: "thread-panel-resize", defaultWidth: 340, minimum: 300, widensToLeft: true) { [weak self] width, commit in
        guard let self else { return }
        if commit {
            self.preferredThreadWidth = max(300, width); self.threadDragWidth = nil
            UserDefaults.standard.set(Double(self.preferredThreadWidth), forKey: "caper.threadPanelWidth")
        } else { self.threadDragWidth = width }
        self.view.needsLayout = true
        self.view.layoutSubtreeIfNeeded()
    }
    private var focusedRevision = -1
    private var focusedConversation: String?
    private var visibleReplyIDs: Set<String> = []
    private var composerIdentity: String?
    private let navigationObservation = NativeObservation()
    private let timelineObservation = NativeObservation()
    private let composerObservation = NativeObservation()
    private let statusObservation = NativeObservation()
    private let attachmentObservation = NativeObservation()
    private let forwardingObservation = NativeObservation()
    private let invitationObservation = NativeObservation()
    private let updateObservation = NativeObservation()
    /// The shipping app's updater; nil in the development host and thread panes.
    var updater: AppUpdater?
    private lazy var navigation = NavigationSidebar(model: model)
    private var preferredSidebarWidth = CGFloat(UserDefaults.standard.object(forKey: "caper.channelSidebarWidth") as? Double ?? 280)
    private var sidebarDragWidth: CGFloat?
    private var sidebarMaximum: CGFloat { max(220, min(440, view.bounds.width - NavigationSidebar.railWidth - 320)) }
    private var navigationWidth: CGFloat { NavigationSidebar.railWidth + min(sidebarMaximum, max(220, sidebarDragWidth ?? preferredSidebarWidth)) }
    private lazy var sidebarResize = NativePanelResizeHandle("Channel sidebar width", identifier: "channel-sidebar-resize", defaultWidth: 280, widensToLeft: false) { [weak self] width, commit in
        guard let self else { return }
        if commit {
            self.preferredSidebarWidth = width; self.sidebarDragWidth = nil
            UserDefaults.standard.set(Double(width), forKey: "caper.channelSidebarWidth")
        } else { self.sidebarDragWidth = width }
        self.view.needsLayout = true
        self.view.layoutSubtreeIfNeeded()
    }
    private lazy var dialogs = AppKitWorkspacePresenter(model: model)
    private lazy var media = AppKitMediaPresenter(model: model, parent: presentationParent)
    private lazy var pins = AppKitPinsPresenter(model: model)
    private lazy var picker = AppKitComposerPicker(model: model, inThread: inThread)
    private lazy var voiceComposer = AppKitVoiceComposer(chat: model.chat, inThread: inThread, editor: composer)
    private var wasRecording = false
    private lazy var members = MemberPanel(model: model)
    private lazy var profiles = AppKitProfilePresenter(model: model)
    private lazy var reactionTooltip = AppKitReactionTooltipPresenter(model: model)
    private var membersPreference: Bool?
    private var preferredMembersWidth = CGFloat(UserDefaults.standard.object(forKey: "caper.membersWidth") as? Double ?? 220)
    private var membersDragWidth: CGFloat?
    private lazy var membersResize = NativePanelResizeHandle("Member list width", identifier: "members-panel-resize", defaultWidth: 220, widensToLeft: true) { [weak self] width, commit in
        guard let self else { return }
        if commit {
            self.preferredMembersWidth = width; self.membersDragWidth = nil
            UserDefaults.standard.set(Double(width), forKey: "caper.membersWidth")
        } else { self.membersDragWidth = width }
        self.view.needsLayout = true
        self.view.layoutSubtreeIfNeeded()
    }
    private lazy var pinsButton: NativeButton = NativeButton.icon("Pins", symbol: "pin") { [weak self] in
        guard let self else { return }
        self.picker.close(restoringFocus: false)
        self.pins.show(from: self.pinsButton, composer: self.composer) { close in
            PinsViewController(model: self.model, configure: { [weak self] in self?.configurePin($0) },
                react: { [weak self] id, emoji in
                    guard let self, let message = self.model.chat.displayedPins.first(where: { $0.id == id }) else { return }
                    self.messageAction(message, action: .reaction(emoji), anchor: self.pinsButton)
                }, close: close)
        }
    }
    private lazy var membersToggle = NativeButton.icon("Show members", symbol: "person.2") { [weak self] in
        guard let self else { return }
        self.membersPreference = self.members.isHidden
        self.view.needsLayout = true
    }
    /// A feedback visit's Close: back to the conversation shown before it.
    private lazy var closeVisit = NativeButton.icon("Close conversation", symbol: "xmark") { [weak self] in
        guard let self else { return }; Task { await self.model.closeVisit() }
    }
    private lazy var stage = AppKitFileDropView(model: model, inThread: inThread)
    private let timeline = MessageTimeline(frame: .zero)
    private let emptyState = NativeView(.clear)
    private let emptyCaper = NSImageView()
    private let emptyTitle = NativeStyle.label("", size: 16, bold: true)
    private let emptyNote = NativeStyle.label("", size: 12)
    private var shownEmptyCaperIndex: Int?
    private let lip = AppKitComposerLip()
    private let header = NativeStyle.label("", size: 16, bold: true)
    private let status = NativeStyle.label("", size: 12)
    private var announcedTyping = ""
    private let composer = ComposerTextView()
    private let composerScroll = NSScrollView()
    /// Outer recessed bezel creating an Antigravity-style darker trough around the composer card.
    private let composerShell = AppKitComposerShell()
    /// Web's message field: one top-lit bordered box holding attach, the editor, and
    /// the GIF, emoji, microphone and send tools.
    private let composerField = AppKitComposerField()
    private var composerFocused = false
    private var laidOutEditorHeight: CGFloat = 0
    private let attachmentScroll = NSScrollView()
    private var attachmentTray: AppKitAttachmentSurface?
    private var replyDraft: AppKitAttachmentSurface?
    private var linkPreview: AppKitAttachmentSurface?
    private lazy var back = NativeButton("Back to channel") { [weak self] in self?.closeThread() }
    private lazy var broadcast: NativeButton = NativeButton("") { [weak self] in
        guard let self else { return }; self.model.chat.threadBroadcast = self.broadcast.state == .on
    }
    /// At the end of a detached window's history, clear of the toast over it.
    private lazy var newer = NativeButton("Load newer messages") { [weak self] in
        guard let self else { return }
        Task {
            if self.inThread { await self.model.chat.loadThread(newer: true) }
            else { await self.model.chat.loadNewer() }
        }
    }
    /// A detached window's Jump to latest: reloads the latest page and lands at its end.
    private func returnToLatest() {
        let context = dialogs.messageContext, root = model.chat.threadRootID
        let conversation = dialogs.conversationContext
        Task {
            if self.inThread { await self.model.chat.loadThread() }
            else { await self.model.chat.retryLoad() }
            guard self.inThread ? self.dialogs.messageContext == context && self.model.chat.threadRootID == root
                : self.dialogs.conversationContext == conversation else { return }
            guard self.inThread ? self.model.chat.threadError == nil : !self.model.chat.loadFailed else { return }
            self.model.chat.focusedMessageID = nil
            self.timeline.showLatest()
        }
    }
    /// "You’re viewing older messages", centered over the history just above
    /// the composer while the reader is up in a live history (counting other
    /// people's new messages), or always in a detached window.
    private lazy var jumpToLatest = JumpToLatestToast { [weak self] in
        guard let self else { return }
        if self.timeline.live { self.timeline.jumpToLatest() } else { self.returnToLatest() }
        self.focusComposer()
    }
    private var uploadConversationID: String?
    private var voicePreviewChecked = false
    private lazy var browse = NativeButton.icon("Browse channels", symbol: "sidebar.left") { [weak self] in
        self?.model.navigationOpen = true
    }
    private lazy var attach = NativeButton.icon("Attach files", symbol: "plus") { [weak self] in self?.showAttachMenu() }
    private lazy var emoji: NativeButton = NativeButton.icon("Insert emoji", symbol: "face.smiling") { [weak self] in
        guard let self else { return }; self.picker.emoji(from: self.emoji, composer: self.composer)
    }
    private lazy var gif: NativeButton = NativeButton("GIF") { [weak self] in
        guard let self else { return }; self.picker.gifs(from: self.gif, composer: self.composer)
    }
    private let counter = NativeStyle.label("", size: 10)
    private lazy var directBar = DirectConversationBar(model: model)
    private let suggestions = ComposerSuggestions()
    private var suggestionsDismissed = false
    private let formatToolbar = AppKitFormatToolbar()
    private var formatToolbarDismissed = false
    private var formattingDraft = false
    /// The style web carries between typed keys (shared/messages/composer.rs).
    private var typing = ComposerTyping()
    private lazy var send = NativeButton("Send") { [weak self] in self?.submit() }
    private var joiningChannelID: String?
    private lazy var join = NativeButton("Join channel", primary: true) { [weak self] in
        guard let self, self.joiningChannelID == nil, let channel = self.model.selectedChannel else { return }
        self.joiningChannelID = channel.id; self.updateComposer()
        Task {
            do { try await self.model.joinChannel(channel) }
            catch {
                if self.model.selectedChannelID == channel.id { self.model.navigationError = NativePresentation.errorMessage(error) }
            }
            self.joiningChannelID = nil; self.updateComposer()
        }
    }
    private lazy var retry = NativeButton("Retry") { [weak self] in
        guard let self else { return }
        Task {
            if self.inThread, self.model.chat.threadError != nil { await self.model.chat.loadThread() }
            else if self.model.chat.loadFailed { await self.model.chat.retryLoad() }
            else if self.model.chat.olderError != nil { await self.model.chat.loadOlder() }
            else if self.model.chat.sessionError != nil { await self.model.chat.retrySession() }
            else { await self.model.chat.send(inThread: self.inThread, retryPending: true) }
        }
    }
    private lazy var editRejected = NativeButton("Edit rejected message") { [weak self] in
        _ = self?.model.chat.discardRejected(edit: true)
    }

    init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread; super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() { view = AppKitWorkspaceContainer() }
    override func viewDidLoad() {
        super.viewDidLoad()
        if inThread { view.addSubview(stage) }
        else { [navigation, stage, members, threadDivider, membersResize, sidebarResize].forEach { view.addSubview($0) } }
        if inThread { view.setAccessibilityIdentifier("thread-pane") }
        [timeline, emptyState, lip, jumpToLatest, header, browse, pinsButton, membersToggle, closeVisit, back, broadcast, status, retry, editRejected, attachmentScroll, composerShell, composerField, attach, composerScroll, emoji, gif, voiceComposer.button, voiceComposer.bar, send, counter, join, directBar, suggestions, formatToolbar].forEach { stage.addSubview($0) }
        emptyState.isHidden = true
        emptyState.setAccessibilityElement(false)
        emptyCaper.imageScaling = .scaleProportionallyUpOrDown
        emptyCaper.setAccessibilityElement(false)
        emptyTitle.alignment = .center
        emptyTitle.lineBreakMode = .byWordWrapping
        emptyTitle.maximumNumberOfLines = 0
        emptyNote.alignment = .center
        emptyNote.textColor = NativeStyle.muted
        emptyNote.lineBreakMode = .byWordWrapping
        emptyNote.maximumNumberOfLines = 0
        [emptyCaper, emptyTitle, emptyNote].forEach { emptyState.addSubview($0) }
        browse.setAccessibilityIdentifier("open-navigation")
        pinsButton.setAccessibilityIdentifier("channel-pins")
        pinsButton.isHidden = inThread
        timeline.inThread = inThread
        if inThread { timeline.setAccessibilityIdentifier("thread-timeline"); timeline.setAccessibilityLabel("Thread replies") }
        back.isHidden = !inThread; back.font = NativeStyle.font(11, bold: true)
        back.setAccessibilityIdentifier("close-thread")
        broadcast.isHidden = !inThread; broadcast.setButtonType(.switch)
        broadcast.font = NativeStyle.font(11); broadcast.layer?.backgroundColor = NSColor.clear.cgColor
        broadcast.setAccessibilityIdentifier("thread-broadcast")
        newer.font = NativeStyle.font(11)
        jumpToLatest.setAccessibilityIdentifier(inThread ? "thread-jump-toast" : "jump-toast")
        jumpToLatest.button.setAccessibilityIdentifier(inThread ? "thread-jump-to-latest" : "jump-to-latest")
        timeline.jumpChanged = { [weak self] in self?.updateJumpToLatest() }
        newer.setAccessibilityIdentifier(inThread ? "load-newer-replies" : "load-newer-messages")
        attachmentScroll.drawsBackground = false
        attachmentScroll.hasVerticalScroller = true
        attachmentScroll.autohidesScrollers = true
        attachmentScroll.isHidden = true
        attach.setAccessibilityIdentifier("attach-files-button")
        emoji.setAccessibilityIdentifier("composer-emoji")
        gif.setAccessibilityIdentifier("composer-gif")
        gif.setAccessibilityLabel("Send a GIF"); gif.toolTip = "GIFs and stickers"
        gif.title = ""
        gif.image = Self.gifBadgeImage()
        gif.imagePosition = .imageOnly
        gif.mutesAtRest = true
        gif.contentTintColor = NativeStyle.muted
        gif.layer?.backgroundColor = NSColor.clear.cgColor
        counter.setAccessibilityIdentifier("composer-counter")
        counter.alignment = .center
        join.setAccessibilityIdentifier("join-channel")
        if !inThread {
            navigation.closeNavigation = { [weak self] in self?.model.navigationOpen = false }
            navigation.show = { [weak self] item in
                guard let self else { return }; self.dialogs.show(item, in: self)
            }
            navigation.showSettings = { [weak self] in self?.showSettings?() }
            navigation.showWhatsNew = { [weak self] in self?.showWhatsNew?() }
            navigation.showActivity = { [weak self] in
                guard let self else { return }
                self.picker.close(restoringFocus: false)
                self.threadPane?.picker.close(restoringFocus: false)
                self.pins.close(restoringFocus: false)
                self.threadPane?.pins.close(restoringFocus: false)
                self.reactionTooltip.close()
                self.threadPane?.reactionTooltip.close()
                self.dialogs.activityInbox(in: self)
            }
            navigation.leaveChannel = { [weak self] channel in
                guard let self else { return }; self.dialogs.leaveChannel(channel, in: self)
            }
            navigation.block = { [weak self] target in self?.confirmBlock(target) }
            directBar.block = { [weak self] target in self?.confirmBlock(target) }
            directBar.changed = { [weak self] in self?.view.needsLayout = true }
            members.close = { [weak self] in
                self?.membersPreference = false
                self?.view.needsLayout = true
                self?.focusComposer()
            }
            members.focusComposer = { [weak self] in self?.focusComposer() }
            members.openProfile = { [weak self] member, view in
                self?.profiles.show(member: member, relativeTo: view.bounds, of: view)
            }
        }
        dialogs.focusComposer = { [weak self] in self?.focusComposer() }
        profiles.focusComposer = { [weak self] in self?.focusComposer() }
        timeline.typeToCompose = { [weak self] event in
            guard let self, self.focusComposer() else { return false }
            self.composer.keyDown(with: event); return true
        }
        profiles.requestBlock = { [weak self] target in self?.confirmBlock(target) }
        timeline.openAuthor = { [weak self] author, view in
            self?.profiles.show(author: author, relativeTo: view.bounds, of: view)
        }
        timeline.openMention = { [weak self] mention, view, rect in
            self?.profiles.show(mention: mention, relativeTo: rect, of: view)
        }
        membersToggle.setAccessibilityIdentifier("toggle-members")
        membersToggle.mutesAtRest = false
        closeVisit.setAccessibilityIdentifier("feedback-visit-close"); closeVisit.isHidden = true
        status.textColor = NativeStyle.muted
        composer.textContainer?.replaceLayoutManager(DraftLinkLayoutManager())
        composer.layoutManager?.delegate = composer.composerGlyphs
        composer.font = NativeStyle.font(14)
        composer.textColor = NativeStyle.text
        composer.insertionPointColor = NativeStyle.text
        composer.drawsBackground = false
        composer.backgroundColor = .clear
        composer.isRichText = false
        composer.allowsUndo = true
        composer.attachmentDropDestination = stage
        stage.acceptedDrop = { [weak self] in
            guard let self, self.composer.isEditable else { return }
            self.view.window?.makeFirstResponder(self.composer)
        }
        composer.updateDragTypeRegistration()
        // One line sits centred in the field's 40-point inner row.
        composer.textContainerInset = NSSize(width: 0, height: max(4, floor((40 - draftLineHeight) / 2)))
        composer.composerGlyphs.fixedLine = draftLine
        composer.isHorizontallyResizable = false
        composer.isVerticallyResizable = true
        composer.autoresizingMask = [.width]
        composer.textContainer?.widthTracksTextView = true
        composer.setAccessibilityLabel(inThread ? "Reply to thread" : "Message")
        composer.setAccessibilityIdentifier(inThread ? "thread-composer" : "message-composer")
        composer.delegate = self
        composer.handleKey = { [weak self] in self?.handleComposerKey($0) == true }
        composer.formatSelection = { [weak self] in self?.formatDraft($0) }
        composer.pasteAttachments = { [weak self] in
            guard let self, self.composer.isEditable else { return false }
            return self.model.chat.pasteAttachments(inThread: self.inThread)
        }
        composer.focusChanged = { [weak self] focused in
            guard let self else { return }
            self.composerFocused = focused
            self.applyComposerChrome()
            if focused { self.refreshSuggestions() } else { self.suggestions.update(nil); self.formatToolbar.isHidden = true }
        }
        composer.setAccessibilityHelp("Return sends. Shift-Return adds a new line. Use arrows and Return or Tab to choose a suggestion; Escape dismisses.")
        suggestions.accept = { [weak self] in self?.acceptSuggestion($0) }
        formatToolbar.formatAction = { [weak self] in self?.formatDraft($0) }
        composerScroll.documentView = composer
        composerScroll.hasVerticalScroller = true
        composerScroll.autohidesScrollers = true
        composerScroll.drawsBackground = false
        applyComposerChrome()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleThemeChange),
            name: CaperThemeStore.didChangeNotification,
            object: nil
        )
        send.setAccessibilityIdentifier(inThread ? "thread-send" : "send-message")
        // The paper plane's weight sits up and right of its box: nudge it to look centred.
        if let plane = NSImage(systemSymbolName: "paperplane.fill", accessibilityDescription: nil) {
            let nudged = NSImage(size: plane.size, flipped: false) { rect in
                plane.draw(in: rect.offsetBy(dx: -1, dy: -1)); return true
            }
            nudged.isTemplate = true
            send.image = nudged
        }
        send.imagePosition = .imageOnly
        send.layer?.cornerRadius = 16; send.layer?.masksToBounds = false
        retry.setAccessibilityIdentifier(inThread ? "thread-retry" : "conversation-retry")
        timeline.loadOlder = { [weak self] in
            guard let self else { return }
            Task {
                if self.inThread { await self.model.chat.loadThread(older: true) }
                else { await self.model.chat.loadOlder() }
            }
        }
        timeline.makeAttachments = { [weak self] message in self?.media.messageAttachments(message) }
        timeline.makeReply = { [weak self] message in self?.media.replyContext(message) }
        timeline.makePendingAttachments = { [weak self] pending in self?.media.pendingAttachments(pending) }
        timeline.makePendingReply = { [weak self] pending in self?.media.pendingReply(pending) }
        timeline.pendingAction = { [weak self] pending, action in
            guard let self, self.model.chat.pendingMessage?.id == pending.id else { return }
            switch action {
            case .retry:
                Task { await self.model.chat.send(inThread: pending.threadRootId != nil, retryPending: true) }
            case .edit:
                _ = self.model.chat.discardRejected(edit: true)
            case .dismiss:
                _ = self.model.chat.discardRejected()
            }
        }
        timeline.visibleMessagesChanged = { [weak self] ids in
            guard let self, self.inThread, self.visibleReplyIDs != ids else { return }
            self.visibleReplyIDs = ids
            // Model observations never run inside the timeline's layout pass.
            DispatchQueue.main.async { [weak self] in self?.acknowledgeVisibleThread() }
        }
        timeline.messageMenu = { [weak self] message, anchor in self?.messageMenu(message, anchor: anchor) }
        timeline.messageAction = { [weak self] message, action, anchor in self?.messageAction(message, action: action, anchor: anchor) }
        timeline.hoverReaction = { [weak self] message, emoji, entered, anchor in
            guard let self else { return }
            self.reactionTooltip.hover(message, emoji: emoji, entered: entered, from: anchor, in: self.stage)
        }
        timeline.dismissHover = { [weak self] in self?.reactionTooltip.close() }
        picker.onReactionTargetChanged = { [weak self] id in
            self?.timeline.setActiveMessageID(id)
        }
        timeline.react = { [weak self] id, emoji in
            guard let self,
                  let message = self.model.chat.displayedMessages.first(where: { $0.id == id }) else { return }
            self.messageAction(message, action: .reaction(emoji), anchor: self.timeline)
        }
        navigationObservation.start { [weak self] in
            guard let self else { return }
            self.browse.setAccessibilityValue(self.model.navigationOpen ? "Open" : "Closed")
            let chat = self.model.chat
            // A visit reads as "#plans · Studio" or "Alice & Bob", as on iPhone and web.
            self.header.stringValue = self.inThread ? "Thread"
                : self.model.visit.map { FeedbackInbox.title(channelName: chat.channelName, spaceName: chat.spaceName, direct: $0.direct) }
                ?? self.model.selectedDirectMessage?.peer.displayName ?? "# \(chat.channelName)"
            self.header.setAccessibilityLabel(self.model.visit != nil && !self.inThread ? "Visiting \(self.header.stringValue)" : self.header.stringValue)
            self.back.title = self.model.selectedDirectMessageID == nil ? "Back to channel" : "Back to conversation"
            self.back.setAccessibilityLabel(self.back.title)
            if !self.inThread, !self.voicePreviewChecked, self.model.detail != nil {
                self.voicePreviewChecked = true
                // The runtime guard permits explicitly labelled loopback fixtures only.
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }; CaperRuntime.showVoiceRosterPreview(self.model)
                }
            }
            let conversationID = self.model.selectedDirectMessageID ?? self.model.selectedChannelID
            if self.uploadConversationID != conversationID {
                self.picker.close(restoringFocus: false)
                self.uploadConversationID = conversationID
                if !self.inThread { Task {
                    await self.model.chat.checkUploadAvailability()
                    await self.model.chat.checkKlipyAvailability()
                } }
            }
            self.view.needsLayout = true
        }
        timelineObservation.start { [weak self] in
            guard let self else { return }
            let chat = self.model.chat
            guard !self.inThread || chat.threadRootID != nil else { return }
            self.timeline.canLoadOlder = self.inThread ? chat.threadHasMore && !chat.threadLoading && chat.threadError == nil
                : chat.hasMore && !chat.loading && !chat.loadingOlder && chat.olderError == nil
            let root = self.inThread ? chat.threadRootID : nil
            let ids = Set((self.inThread ? chat.threadMessages : chat.channelMessages).map(\.id))
            let projected = chat.displayedMessages
            let visible = (root.flatMap { id in projected.first { $0.id == id }.map { [$0] } } ?? [])
                + projected.filter { ids.contains($0.id) }
            let identity = [self.dialogs.conversationContext, root ?? "channel"].joined(separator: ":")
            self.timeline.newMessagesID = self.inThread ? nil : self.model.channelNewMessagesDividerID()
            // A window detached from the live end (after a jump) always shows
            // the toast, whose Jump to latest reloads the latest page.
            self.timeline.live = !(self.inThread ? chat.threadHasNewer : chat.hasNewer)
            let pending = chat.pendingMessage.flatMap { $0.threadRootId == root ? $0 : nil }
            let pendingError = pending != nil ? chat.error : nil
            let sendRejected = pending != nil && chat.sendRejected
            let sendUploadFailed = pending != nil && chat.sendUploadFailed
            // Read the draft only for a rejected send, so typing does not rebuild rows.
            let canEditPending = !(sendRejected && pendingError != nil) || (self.inThread
                ? chat.threadDraft.isEmpty && chat.threadAttachmentDrafts.isEmpty
                : chat.draft.isEmpty && chat.attachmentDrafts.isEmpty)
            self.timeline.update(visible, conversationID: identity,
                viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs,
                pending: pending, pendingAuthor: pending != nil ? chat.currentAuthor : nil,
                pendingError: pendingError, sendRejected: sendRejected,
                sendUploadFailed: sendUploadFailed, canEditPending: canEditPending,
                canInteract: chat.canInteract, reactionErrors: chat.reactionErrors, pinErrors: chat.pinErrors, pendingPins: chat.pendingPins)
            self.updateEmptyState(visible: visible, pending: pending)
            if let id = chat.focusedMessageID, self.focusedRevision != chat.focusRevision || self.focusedConversation != identity,
               visible.contains(where: { $0.id == id }) {
                self.focusedRevision = chat.focusRevision; self.focusedConversation = identity
                self.timeline.focusMessage(id)
            }
            DispatchQueue.main.async { [weak self] in self?.acknowledgeVisibleThread() }
        }
        composerObservation.start { [weak self] in self?.updateComposer() }
        statusObservation.start { [weak self] in self?.updateStatus() }
        attachmentObservation.start(reading: { [weak self] in
            guard let self else { return false }
            let chat = self.model.chat
            return !self.model.previewingChannel && (self.model.selectedDirectMessage.map { $0.status != .incoming && !$0.blocked } ?? true)
                && !self.voiceComposer.isRecording
                && (self.voiceComposer.hasNotice || (self.inThread ? !chat.threadAttachmentDrafts.isEmpty || chat.threadAttachmentNotice != nil
                    : !chat.attachmentDrafts.isEmpty || chat.attachmentNotice != nil))
        }, update: { [weak self] visible in
            guard let self else { return }
            if visible, self.attachmentTray == nil {
                let tray = self.media.composerAttachments(inThread: self.inThread)
                tray.heightChanged = { [weak self] in self?.view.needsLayout = true }
                self.attachmentTray = tray
                self.attachmentScroll.documentView = tray
            } else if !visible {
                self.attachmentScroll.documentView = nil
                self.attachmentTray = nil
            }
            self.attachmentScroll.isHidden = !visible
            self.view.needsLayout = true
        })
        if !inThread {
            if let updater {
                updateObservation.start(reading: { updater.controls }) { [weak self] in self?.navigation.updates = $0 }
            }
            invitationObservation.start(reading: { [weak self] in
                (self?.model.notificationInvitation?.id, self?.model.notificationChannelInvitation?.id,
                 self?.model.deliverableInviteLink?.id)
            }, update: { [weak self] _ in
                guard let self else { return }; self.dialogs.presentInvitations(in: self)
            })
            threadObservation.start(reading: { [weak self] in self?.model.chat.threadRootID }, update: { [weak self] root in
                self?.updateThreadPane(root)
            })
            forwardingObservation.start(reading: { [weak self] in
                (self?.model.chat.forwardTarget?.id, self?.model.chat.forwardConversationTarget?.id)
            }, update: { [weak self] targets in
                guard let self else { return }
                if targets.0 != nil || targets.1 != nil {
                    self.picker.close(restoringFocus: false)
                    self.threadPane?.picker.close(restoringFocus: false)
                    self.pins.close()
                    self.threadPane?.pins.close()
                }
                self.dialogs.presentForwarding(in: self)
            })
        }
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        voiceComposer.synchronize(visible: false)
        picker.close(restoringFocus: false)
        pins.close(restoringFocus: false)
        reactionTooltip.close()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        updateComposer()
    }

    private func updateThreadPane(_ root: String?) {
        pins.close(restoringFocus: false)
        let requested = focusAfterThreadClose; focusAfterThreadClose = false
        if root != nil, threadPane == nil {
            let pane = WorkspaceController(model: model, inThread: true)
            addChild(pane)
            view.addSubview(pane.view, positioned: .below, relativeTo: members)
            threadPane = pane
            if !CaperAccessibility.reduceMotion {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0; fade.toValue = 1; fade.duration = 0.14
                fade.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                pane.view.layer?.add(fade, forKey: "entrance")
            }
        } else if root == nil, let pane = threadPane {
            let returnFocus = (view.window?.firstResponder as? NSView).map { $0.isDescendant(of: pane.view) } == true
            pane.voiceComposer.synchronize(visible: false)
            pane.picker.close(restoringFocus: false)
            pane.view.removeFromSuperview(); pane.removeFromParent()
            threadPane = nil
            stage.isHidden = false
            updateComposer()
            model.threadVisibilityChanged(rootID: nil, seq: nil)
            if returnFocus || requested { focusComposer() }
        }
        view.needsLayout = true
    }

    private func closeThread() {
        picker.close(restoringFocus: false)
        // The channel's controller removes the pane; it focuses its composer.
        (inThread ? parent as? WorkspaceController : self)?.focusAfterThreadClose = true
        model.chat.closeThread()
    }

    /// Returns keyboard focus to this pane's composer when something over the
    /// conversation closes, as on web. A narrow layout that shows only the
    /// thread hands off to the thread's composer.
    @discardableResult private func focusComposer() -> Bool {
        if stage.isHidden, let threadPane { return threadPane.focusComposer() }
        guard composer.isEditable, !stage.isHiddenOrHasHiddenAncestor, !dialogs.isPresenting,
              let window = view.window, window.isKeyWindow, window.attachedSheet == nil else { return false }
        return window.makeFirstResponder(composer)
    }

    private func acknowledgeVisibleThread() {
        guard inThread else { return }
        let chat = model.chat
        let reply = !chat.threadHasNewer && !chat.threadLoading && view.window != nil && !view.isHiddenOrHasHiddenAncestor
            ? chat.threadMessages.last(where: { visibleReplyIDs.contains($0.id) }) : nil
        if chat.visibleThreadReply?.rootID != reply?.threadRootId || chat.visibleThreadReply?.seq != reply?.seq {
            model.threadVisibilityChanged(rootID: reply?.threadRootId, seq: reply?.seq)
        }
    }

    private func focusComposerIfRequested() {
        let chat = model.chat
        guard composer.isEditable, !stage.isHiddenOrHasHiddenAncestor,
              let request = chat.composerFocusRequest(inThread: inThread), let window = view.window else { return }
        if window.makeFirstResponder(composer) { chat.consumeComposerFocus(request) }
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        let previousStage = stage.frame
        let wasHidden = stage.isHidden
        let width = view.bounds.width, height = view.bounds.height
        let narrow = !inThread && width <= 760
        if !inThread, model.narrowLayout != narrow {
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.model.narrowLayout = self.view.bounds.width <= 760
            }
        }
        let browsing = narrow && model.navigationOpen
        let navigationWidth = narrow ? 0 : self.navigationWidth
        let docked = !narrow && width - navigationWidth >= 540
        let hasMembers = !inThread && model.selectedDirectMessageID == nil && model.visit == nil
        let visible = hasMembers && !browsing && !(narrow && model.previewingChannel) && (membersPreference ?? docked)
        if !visible { membersDragWidth = nil }
        let maximum = min(440, max(220, width - navigationWidth - (docked ? 320 : 64)))
        // Clamping a small window never overwrites the user's saved width.
        let memberWidth = narrow ? min(280, width - 64) : min(maximum, max(220, membersDragWidth ?? preferredMembersWidth))
        if inThread { stage.frame = view.bounds }
        else {
            navigation.narrow = narrow
            let navigationWasHidden = navigation.isHidden
            navigation.isHidden = narrow && !browsing
            navigation.frame = NSRect(x: 0, y: 0, width: narrow ? width : navigationWidth, height: height)
            if browsing && navigationWasHidden && !CaperAccessibility.reduceMotion {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0; fade.toValue = 1; fade.duration = 0.14
                fade.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                navigation.layer?.add(fade, forKey: "entrance")
            }
            sidebarResize.isHidden = narrow
            sidebarResize.frame = NSRect(x: navigationWidth - 8, y: 0, width: 8, height: height)
            sidebarResize.update(width: navigationWidth - NavigationSidebar.railWidth, maximum: sidebarMaximum)
            // Like shipping SwiftUI, only the viewport breakpoint folds the
            // channel. The same controllers survive every width and retain drafts.
            let available = max(0, width - navigationWidth - (visible && docked ? memberWidth : 0))
            let split = !narrow
            if threadPane == nil || !split { threadDragWidth = nil }
            let maximumThreadWidth = min(600, available - min(300, available / 2))
            let minimumThreadWidth = min(300, maximumThreadWidth)
            let threadWidth: CGFloat = threadPane == nil ? 0 : split ? min(maximumThreadWidth, max(minimumThreadWidth, threadDragWidth ?? preferredThreadWidth)) : available
            stage.isHidden = browsing || (threadPane != nil && !split)
            stage.frame = NSRect(x: navigationWidth, y: 0, width: max(0, available - (split ? threadWidth : 0)), height: height)
            if let pane = threadPane {
                let wasCovered = pane.view.isHidden
                pane.view.isHidden = browsing
                pane.view.frame = NSRect(x: navigationWidth + available - threadWidth, y: 0, width: threadWidth, height: height)
                if wasCovered != browsing {
                    DispatchQueue.main.async { [weak pane] in pane?.updateComposer(); pane?.acknowledgeVisibleThread() }
                }
            }
            threadDivider.isHidden = threadPane == nil || !split
            threadDivider.frame = NSRect(x: navigationWidth + available - threadWidth, y: 0, width: 8, height: height)
            threadDivider.update(width: threadWidth, maximum: maximumThreadWidth, minimum: minimumThreadWidth)
            members.frame = NSRect(x: width - memberWidth, y: docked ? 0 : 50, width: memberWidth, height: height - (docked ? 0 : 50))
            members.setVisible(visible)
            membersResize.isHidden = !visible || narrow
            membersResize.frame = NSRect(x: members.frame.minX, y: members.frame.minY, width: 8, height: members.frame.height)
            membersResize.update(width: memberWidth, maximum: maximum)
        }
        if stage.isHidden != wasHidden {
            // Ending a recording publishes model state, never during layout.
            DispatchQueue.main.async { [weak self] in self?.updateComposer() }
        }
        if let focused = view.window?.firstResponder as? NSView, focused.isDescendant(of: view), focused.isHiddenOrHasHiddenAncestor {
            view.window?.makeFirstResponder(nil)
        }
        if stage.frame != previousStage || stage.isHidden { reactionTooltip.close() }
        let center = stage.bounds.width
        let compactBack = center < 300
        back.title = compactBack ? "" : (model.selectedDirectMessageID == nil ? "Back to channel" : "Back to conversation")
        back.image = compactBack ? NSImage(systemSymbolName: "chevron.left", accessibilityDescription: nil) : nil
        back.imagePosition = compactBack ? .imageOnly : .noImage
        back.frame = NSRect(x: center - (compactBack ? 48 : 170), y: 10, width: compactBack ? 28 : 150, height: 28)
        browse.isHidden = !narrow || inThread
        browse.frame = NSRect(x: 12, y: 8, width: 32, height: 32)
        membersToggle.frame = NSRect(x: center - 48, y: 8, width: 32, height: 32)
        membersToggle.isHidden = !hasMembers
        // A visit has no member list; its Close takes that place.
        closeVisit.isHidden = inThread || model.visit == nil
        closeVisit.frame = membersToggle.frame
        pinsButton.frame = NSRect(x: center - (membersToggle.isHidden && closeVisit.isHidden ? 48 : 88), y: 8, width: 32, height: 32)
        let headerLeft: CGFloat = browse.isHidden ? 20 : 52
        let headerRight: CGFloat = inThread ? (compactBack ? 56 : 175) : membersToggle.isHidden && closeVisit.isHidden ? 60 : 100
        header.frame = NSRect(x: headerLeft, y: 14, width: max(0, center - headerLeft - headerRight), height: 26)
        membersToggle.setAccessibilityLabel(visible ? "Hide members" : "Show members")
        membersToggle.toolTip = visible ? "Hide members" : "Show members"
        membersToggle.contentTintColor = visible ? NativeStyle.accent : NativeStyle.muted
        let base = height - (broadcast.isHidden ? 0 : 28)
        let stackedActions = center < 250
        // Web's field: 16 above the panel's bottom, 42 tall for one line, the
        // tools inside its bottom edge. A very narrow split wraps them into two
        // rows under the editor instead.
        let fieldBottom = base - (counter.isHidden ? 16 : 28)
        let toolSide: CGFloat = 32, toolGap: CGFloat = 6, toolStep = toolSide + toolGap
        let fieldX: CGFloat = 20, fieldWidth = max(0, center - 40), fieldRight = fieldX + fieldWidth
        let toolRows: CGFloat = stackedActions ? 74 : 0
        var editorX = fieldX + 43, editorRight = fieldRight - 1
        // Only a thread has Send reply; the channel's microphone takes its place.
        let rightEdge = fieldRight - 6 - toolSide
        let voiceX = inThread ? rightEdge - toolStep : rightEdge
        if stackedActions {
            attach.frame.origin.x = fieldX + 6
            gif.frame.origin.x = fieldX + 6 + toolStep
            emoji.frame.origin.x = gif.isHidden ? fieldX + 6 + toolStep : fieldX + 6 + toolStep * 2
            send.frame.origin.x = rightEdge
            voiceComposer.button.frame.origin.x = voiceX
            // The draft starts in line with the attach glyph below it.
            editorX = fieldX + 10; editorRight = fieldRight - 6
        } else {
            attach.frame.origin.x = fieldX + 5
            send.frame.origin.x = rightEdge
            voiceComposer.button.frame.origin.x = voiceX
            emoji.frame.origin.x = voiceComposer.button.frame.minX - toolStep
            gif.frame.origin.x = emoji.frame.minX - toolStep
            editorRight = (gif.isHidden ? emoji.frame.minX : gif.frame.minX) - 8
        }
        composerScroll.frame = NSRect(x: editorX, y: composerScroll.frame.minY, width: max(0, editorRight - editorX), height: max(1, composerScroll.frame.height))
        composer.frame.size.width = composerScroll.contentSize.width
        laidOutEditorHeight = editorContentHeight()
        let editorHeight = min(laidOutEditorHeight, max(40, min(318, height * 0.4) - toolRows))
        let fieldHeight = max(42, editorHeight + 2 + toolRows)
        composerField.frame = NSRect(x: fieldX, y: fieldBottom - fieldHeight, width: fieldWidth, height: fieldHeight)
        composerShell.frame = composerField.frame.insetBy(dx: -4, dy: -4)
        composerScroll.frame = NSRect(x: editorX, y: composerField.frame.minY + 1, width: max(0, editorRight - editorX), height: fieldHeight - 2 - toolRows)
        composer.frame.size.width = composerScroll.contentSize.width
        composer.minSize = NSSize(width: 0, height: composerScroll.contentSize.height)
        // Centered in a one-line field; bottom-aligned once the draft wraps, so
        // they stay on its last line.
        let oneLine = editorHeight < draftLineHeight * 2 + composer.textContainerInset.height * 2
        let toolY = stackedActions || !oneLine ? fieldBottom - 37 : (fieldBottom - fieldHeight / 2 - toolSide / 2).rounded()
        let toolSize = NSSize(width: toolSide, height: toolSide)
        send.frame.origin.y = toolY; send.frame.size = toolSize
        voiceComposer.button.frame.origin.y = toolY; voiceComposer.button.frame.size = toolSize
        if stackedActions {
            for tool in [attach, gif, emoji] { tool.frame.origin.y = toolY - 36; tool.frame.size = toolSize }
        } else {
            for tool in [attach, gif, emoji] { tool.frame.origin.y = toolY; tool.frame.size = toolSize }
        }
        let recordingHeight: CGFloat = center - 40 < 300 ? 126 : 42
        voiceComposer.bar.frame = NSRect(x: fieldX, y: fieldBottom - recordingHeight, width: fieldWidth, height: recordingHeight)
        let composerTop = composerField.isHidden && !voiceComposer.bar.isHidden ? voiceComposer.bar.frame.minY : composerShell.frame.minY
        let attachmentHeight = attachmentTray?.height(for: max(1, center - 40)) ?? 0
        let trayHeight = min(attachmentHeight, max(80, height * 0.3))
        let quoteHeight = replyDraft?.height(for: max(1, center - 40)) ?? 0
        // The link card sits on the composer and ends with its own 8pt gap; the tray stacks above it.
        let linkHeight = linkPreview?.height(for: max(1, center - 40)) ?? 0
        linkPreview?.frame = NSRect(x: 20, y: composerTop - linkHeight, width: center - 40, height: linkHeight)
        let aboveLink = composerTop - linkHeight
        let trayGap: CGFloat = trayHeight > 0 ? 8 : 0
        attachmentScroll.frame = NSRect(x: 20, y: aboveLink - trayGap - trayHeight, width: center - 40, height: trayHeight)
        attachmentTray?.frame = NSRect(x: 0, y: 0, width: center - 40, height: attachmentHeight)
        let quoteBottom = trayHeight > 0 ? attachmentScroll.frame.minY - 8 : aboveLink
        replyDraft?.frame = NSRect(x: 20, y: quoteBottom - quoteHeight, width: center - 40, height: quoteHeight)
        let recoveryHeight: CGFloat = editRejected.isHidden ? 0 : 34
        // A channel's typing row sits snugly over the composer (16pt + 2pt breathing room);
        // a thread, or a row holding Retry, keeps a 22pt row.
        let roomyStatus = inThread || !retry.isHidden
        let statusHeight: CGFloat = roomyStatus ? 22 : 16
        // Directly over the field, the row overlaps the shell's 4-point ring, as
        // web's typing row sits 2 above the field: the field then has as much
        // room above it as below.
        let stackTop = quoteBottom - quoteHeight - recoveryHeight
        let ringOverlap: CGFloat = stackTop == composerShell.frame.minY && !composerShell.isHidden ? 4 : 0
        let statusY = directBar.isHidden ? stackTop + ringOverlap - (roomyStatus ? 24 : 18) : height - max(138, directBar.preferredHeight(width: center - 40) + 32) + 6
        let top: CGFloat = 48
        timeline.frame = NSRect(x: 0, y: top, width: center, height: max(0, statusY - top))
        layoutEmptyState()
        lip.frame = NSRect(x: 0, y: max(top, timeline.frame.maxY - 8), width: center, height: min(8, timeline.frame.height))
        status.frame = NSRect(x: 20, y: statusY, width: center - (retry.isHidden ? 40 : 116), height: statusHeight)
        directBar.frame = NSRect(x: 20, y: status.frame.maxY, width: center - 40, height: max(0, height - status.frame.maxY - 8))
        counter.frame = NSRect(x: 20, y: base - 24, width: center - 40, height: 16)
        // A checkbox toggles only on its box and title, so its frame (and the
        // accessibility frame pointer clients aim at) must not run past them.
        broadcast.frame = NSRect(x: 20, y: height - 26, width: min(center - 40, broadcast.fittingSize.width), height: 22)
        join.frame = NSRect(x: 20, y: height - 80, width: min(140, center - 40), height: 38)
        layoutSuggestions()
        layoutFormatToolbar()
        retry.frame = NSRect(x: center - 84, y: statusY, width: 64, height: 24)
        layoutJumpToLatest()
        editRejected.frame = NSRect(x: 20, y: quoteBottom - quoteHeight - recoveryHeight, width: min(200, center - 40), height: 30)
        focusComposerIfRequested()
    }

    private func updateEmptyState(visible: [ChatMessage], pending: PendingMessage?) {
        let chat = model.chat
        guard !inThread, visible.isEmpty, pending == nil, !chat.loading, !chat.loadFailed, chat.error == nil,
              !chat.hasMore, !chat.hasNewer, let channelID = chat.channelID else {
            emptyState.isHidden = true
            return
        }
        let caperIndex = CaperDailyIcon.conversationIndex(for: channelID)
        if shownEmptyCaperIndex != caperIndex {
            shownEmptyCaperIndex = caperIndex
            emptyCaper.image = CaperDailyIcon.brandingImage(for: caperIndex)
        }
        emptyCaper.isHidden = emptyCaper.image == nil
        if let conversation = model.selectedDirectMessage, conversation.peer.id == model.account?.id {
            emptyTitle.stringValue = ""
            emptyTitle.isHidden = true
            emptyNote.stringValue = "You can message yourself here to keep notes, reminders, and ideas."
            emptyNote.isHidden = false
        } else {
            emptyTitle.stringValue = "No messages yet."
            emptyTitle.textColor = NativeStyle.text
            emptyTitle.isHidden = false
            emptyNote.stringValue = model.selectedDirectMessageID == nil
                ? "Start the conversation in #\(chat.channelName.lowercased())."
                : FeedbackInbox.emptyDirectNote(peer: model.selectedDirectMessage?.peer, name: chat.channelName)
            emptyNote.textColor = NativeStyle.muted
            emptyNote.isHidden = false
        }
        emptyState.isHidden = false
        layoutEmptyState()
    }

    private func layoutEmptyState() {
        guard !emptyState.isHidden else { return }
        let area = timeline.frame
        let maxTextWidth = max(120, min(360, area.width - 48))
        let caperSide: CGFloat = emptyCaper.isHidden ? 0 : 88
        let titleHeight: CGFloat = emptyTitle.isHidden ? 0 : ceil(emptyTitle.sizeThatFits(NSSize(width: maxTextWidth, height: 200)).height)
        let noteHeight: CGFloat = emptyNote.isHidden ? 0 : ceil(emptyNote.sizeThatFits(NSSize(width: maxTextWidth, height: 200)).height)
        let caperGap: CGFloat = caperSide > 0 ? 12 : 0
        let titleGap: CGFloat = !emptyTitle.isHidden && !emptyNote.isHidden ? 6 : 0
        let totalHeight = caperSide + caperGap + titleHeight + titleGap + noteHeight
        let boxWidth = max(caperSide, maxTextWidth)
        let boxX = area.minX + floor((area.width - boxWidth) / 2)
        let boxY = area.minY + floor(max(0, (area.height - totalHeight) / 2))
        emptyState.frame = NSRect(x: boxX, y: boxY, width: boxWidth, height: totalHeight)
        var y: CGFloat = 0
        if !emptyCaper.isHidden {
            emptyCaper.frame = NSRect(x: floor((boxWidth - caperSide) / 2), y: y, width: caperSide, height: caperSide)
            y += caperSide + caperGap
        }
        if !emptyTitle.isHidden {
            emptyTitle.frame = NSRect(x: 0, y: y, width: boxWidth, height: titleHeight)
            y += titleHeight + titleGap
        }
        if !emptyNote.isHidden {
            emptyNote.frame = NSRect(x: 0, y: y, width: boxWidth, height: noteHeight)
        }
    }

    private func updateComposer() {
        let chat = model.chat
        if inThread { directBar.isHidden = true } else { directBar.update() }
        let root = inThread ? chat.threadRootID : nil
        let identity = [dialogs.messageContext, root ?? "channel"].joined(separator: ":")
        let sameComposer = composerIdentity == identity
        if composerIdentity != identity {
            picker.close(restoringFocus: false)
            pins.close(restoringFocus: false)
            composerIdentity = identity
            typing.end()
            composer.unmarkText()
            suggestionsDismissed = false
        }
        let draft = inThread ? chat.threadDraft : chat.draft
        if composer.string != draft, !composer.hasMarkedText() {
            typing.end()
            composer.string = draft
            composer.setSelectedRange(NSRange(location: composer.string.utf16.count, length: 0))
            composer.needsDisplay = true
            relayoutIfDraftResized()
        }
        composer.placeholder = inThread ? "Reply to thread…"
            : model.visit?.direct == true ? "Message \(chat.channelName)"
            : "Message \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        // VoiceOver reads the drawn placeholder too, naming where a message goes.
        composer.setAccessibilityPlaceholderValue(composer.placeholder)
        let ready = !inThread || (root != nil && !chat.threadLoading)
        let hidden = model.previewingChannel || !directBar.isHidden
        voiceComposer.synchronize(visible: ready && !hidden && chat.canInteract && view.window != nil && !stage.isHiddenOrHasHiddenAncestor)
        let recording = voiceComposer.isRecording
        let restoreFocus = wasRecording && !recording && sameComposer
            && (view.window?.firstResponder as? NSView).map { $0 === voiceComposer.bar || $0.isDescendant(of: voiceComposer.bar) } == true
        let startedRecording = recording && !wasRecording
        wasRecording = recording
        pinsButton.isEnabled = pins.isAvailable
        // Typing goes on while a message is in flight; Return queues the next one behind it.
        composer.isEditable = !recording && ready && directBar.isHidden && chat.canInteract && !chat.sendRejected
            && (chat.sending || chat.pendingMessage == nil)
        send.isEnabled = !recording && ready && directBar.isHidden && chat.canInteract && chat.canPressSend(inThread: inThread)
        let sendLabel = chat.sending ? "Sending…" : inThread ? "Send reply" : "Send"
        send.title = ""
        send.setAccessibilityLabel(sendLabel); send.toolTip = sendLabel
        // As web: a soft chalk glow when there is something to send, not a solid fill.
        let glow = NativeStyle.text
        send.layer?.backgroundColor = (send.isEnabled ? glow.withAlphaComponent(0.09) : .clear).cgColor
        send.layer?.borderWidth = send.isEnabled ? 1 : 0
        send.layer?.borderColor = glow.withAlphaComponent(0.24).cgColor
        send.layer?.shadowColor = glow.cgColor; send.layer?.shadowOffset = .zero; send.layer?.shadowRadius = 6
        send.layer?.shadowOpacity = send.isEnabled ? 0.3 : 0
        send.contentTintColor = send.isEnabled ? NativeStyle.text : NativeStyle.muted
        composerScroll.isHidden = hidden || recording
        if composerField.isHidden != composerScroll.isHidden {
            composerField.isHidden = composerScroll.isHidden
            composerShell.isHidden = composerScroll.isHidden
            view.needsLayout = true
        } else {
            composerShell.isHidden = composerField.isHidden
        }
        voiceComposer.button.isHidden = hidden
        voiceComposer.bar.isHidden = !recording
        // Return sends in the channel's composer, as on web, Windows and Linux,
        // so it has no Send button; a thread keeps Send reply.
        send.isHidden = composerScroll.isHidden || !inThread
        attach.isHidden = composerScroll.isHidden
        attach.isEnabled = chat.canAttach && composer.isEditable
        attach.toolTip = chat.canAttach ? "Attach files" : chat.attachUnavailableReason
        attach.setAccessibilityValue(chat.canAttach ? "" : chat.attachUnavailableReason)
        emoji.isHidden = composerScroll.isHidden
        emoji.isEnabled = composer.isEditable
        let hideGif = composerScroll.isHidden || chat.klipy == nil
        if gif.isHidden != hideGif { gif.isHidden = hideGif; view.needsLayout = true }
        gif.isEnabled = composer.isEditable && chat.canSendGif(inThread: inThread)
        picker.closeIfUnavailable(composer: composer)
        let count = draft.unicodeScalars.count
        // The counter sits under the field, which moves up to make room.
        let hideCounter = composerScroll.isHidden || count < 3000
        if counter.isHidden != hideCounter { counter.isHidden = hideCounter; view.needsLayout = true }
        counter.stringValue = "\(count.formatted()) / 4,000"
        counter.textColor = AppKitComposerCounter.color(count)
        join.isHidden = inThread || !model.previewingChannel
        join.isEnabled = joiningChannelID == nil
        join.title = joiningChannelID == model.selectedChannelID ? "Joining…" : "Join channel"
        broadcast.isHidden = !inThread || hidden
        let broadcastTitle = "Also send to \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        if broadcast.title != broadcastTitle { broadcast.title = broadcastTitle; view.needsLayout = true }
        broadcast.setAccessibilityLabel(broadcast.title)
        broadcast.state = chat.threadBroadcast ? .on : .off
        broadcast.isEnabled = !recording && ready && chat.canInteract && chat.pendingMessage == nil
        let hasQuote = !composerScroll.isHidden && chat.replyTargets[root ?? "channel"] != nil
        if hasQuote, replyDraft == nil {
            let banner = media.replyDraft(inThread: inThread)
            banner.heightChanged = { [weak self] in self?.view.needsLayout = true }
            stage.addSubview(banner, positioned: .below, relativeTo: suggestions)
            replyDraft = banner
        } else if !hasQuote { replyDraft?.removeFromSuperview(); replyDraft = nil }
        let hasLink = !composerScroll.isHidden && chat.draftPreviewLink(inThread: inThread) != nil
        if hasLink, linkPreview == nil {
            let card = media.composerLinkPreview(inThread: inThread)
            card.heightChanged = { [weak self] in self?.view.needsLayout = true }
            stage.addSubview(card, positioned: .below, relativeTo: suggestions)
            linkPreview = card
        } else if !hasLink { linkPreview?.removeFromSuperview(); linkPreview = nil }
        let detached = inThread ? chat.threadHasNewer : chat.hasNewer
        let loadingNewer = inThread ? chat.threadLoading : chat.loadingNewer
        newer.title = loadingNewer ? "Loading…" : inThread ? "Load newer replies" : "Load newer messages"
        newer.setAccessibilityLabel(newer.title)
        newer.isEnabled = !loadingNewer
        timeline.footer = detached ? newer : nil
        // A detached window's Jump to latest reloads; not while a load is under way.
        jumpToLatest.button.isEnabled = !detached || (inThread ? !chat.threadLoading : !chat.loading && !chat.jumpingToMessage)
        // Keystrokes change only the composer, not timeline geometry.
        let pending = chat.pendingMessage.flatMap { $0.threadRootId == root ? $0 : nil }
        let historyError = inThread ? chat.threadError : chat.olderError
        retry.isHidden = !(chat.loadFailed || historyError != nil || chat.sessionError != nil || (pending != nil && !chat.sending && !chat.sendRejected))
        retry.isEnabled = inThread ? !chat.threadLoading : !chat.loadingOlder
        editRejected.isHidden = !chat.sendRejected || pending == nil
        // Layout consumes focus only once the requested pane is on screen.
        _ = chat.composerFocusRequest(inThread: inThread)
        view.needsLayout = true
        refreshSuggestions()
        if startedRecording {
            typing.end()
            picker.close(restoringFocus: false)
            if !dialogs.isPresenting, let window = view.window, window.isKeyWindow, window.attachedSheet == nil {
                window.makeFirstResponder(voiceComposer.bar)
            }
        }
        if restoreFocus, composer.isEditable, !stage.isHiddenOrHasHiddenAncestor, !dialogs.isPresenting,
           let window = view.window, window.isKeyWindow, window.attachedSheet == nil {
            window.makeFirstResponder(composer)
        }
    }

    /// Shows, hides or recounts the toast.
    private func updateJumpToLatest() {
        let state = timeline.jump
        jumpToLatest.update(shows: state.shows, count: state.count)
        layoutJumpToLatest()
    }

    /// Centered 12 points above the history's bottom edge, just above the
    /// composer, and at most the history's width less 24. Below the history
    /// it would overhang the status row, which is stacked above it and would
    /// take its clicks.
    private func layoutJumpToLatest() {
        guard !jumpToLatest.isHidden else { return }
        let maxWidth = max(0, timeline.frame.width - 24)
        let size = jumpToLatest.size(maxWidth: maxWidth)
        let width = min(size.width, maxWidth)
        jumpToLatest.frame = NSRect(x: timeline.frame.midX - width / 2, y: timeline.frame.maxY - 12 - size.height,
                                    width: width, height: size.height)
    }

    /// The status row: an error, loading, or who is typing. Its own
    /// observation, so a typing event never reruns the composer or layout.
    private func updateStatus() {
        let chat = model.chat
        let root = inThread ? chat.threadRootID : nil
        let pending = chat.pendingMessage.flatMap { $0.threadRootId == root ? $0 : nil }
        let historyError = inThread ? chat.threadError : chat.olderError
        let error = (chat.pendingMessage == nil || pending != nil ? chat.error : nil) ?? historyError ?? chat.jumpError ?? chat.sessionError ?? model.navigationError
        status.setAccessibilityIdentifier(inThread ? (error == nil ? "thread-status" : "thread-error") : (error == nil ? "conversation-status" : "conversation-error"))
        status.textColor = error == nil ? NativeStyle.muted : NSColor(CaperTheme.error)
        status.stringValue = error
            ?? (inThread ? (chat.threadLoading ? "Loading replies…" : chat.threadMessages.isEmpty ? "No replies yet. Start the thread." : "")
                : model.previewingChannel ? "Preview · Join #\(chat.channelName) to interact with people here" : chat.loading ? "Loading messages…" : chat.loadingOlder ? "Loading older messages…" : chat.typingNames.isEmpty ? (model.selectedDirectMessage?.status == .outgoing ? model.selectedDirectMessage.flatMap(NativePresentation.directMessageNotice) ?? "" : "") : "\(chat.typingNames.joined(separator: ", ")) typing…")
        // Who is typing is announced politely when it starts or changes, like
        // web's role="status", without moving VoiceOver's focus.
        let typingShown = !inThread && error == nil && !chat.typingNames.isEmpty && status.stringValue.hasSuffix(" typing…") ? status.stringValue : ""
        if !typingShown.isEmpty, typingShown != announcedTyping {
            NSAccessibility.post(element: status, notification: .announcementRequested, userInfo: [
                .announcement: typingShown, .priority: NSAccessibilityPriorityLevel.medium.rawValue,
            ])
        }
        announcedTyping = typingShown
    }

    private func showAttachMenu() {
        let menu = NSMenu()
        let count = (inThread ? model.chat.threadAttachmentDrafts : model.chat.attachmentDrafts).count
        if count >= AttachmentPolicy.maxAttachments {
            menu.addItem(withTitle: "You can attach up to \(AttachmentPolicy.maxAttachments) files.", action: nil, keyEquivalent: "")
        }
        let item = menu.addItem(withTitle: "Upload a file", action: #selector(pickFiles), keyEquivalent: "")
        item.target = self; item.isEnabled = count < AttachmentPolicy.maxAttachments
        menu.autoenablesItems = false
        CaperMenu.show(menu, below: attach)
    }

    @objc private func pickFiles() { media.pickFiles(inThread: inThread) }

    private func configurePin(_ row: MessageCell) {
        row.openAuthor = { [weak self] author, anchor in
            self?.profiles.show(author: author, relativeTo: anchor.bounds, of: anchor)
        }
        row.body.openMention = { [weak self] mention, anchor, rect in
            self?.profiles.show(mention: mention, relativeTo: rect, of: anchor)
        }
        row.makeAttachments = { [weak self] message in
            self?.media.messageAttachments(message, beforeViewing: { [weak self] in self?.pins.close() })
        }
        row.makeReply = { [weak self] message in self?.media.replyContext(message) }
        row.messageMenu = { [weak self] message, anchor in self?.messageMenu(message, anchor: anchor) }
        row.messageAction = { [weak self] message, action, anchor in self?.messageAction(message, action: action, anchor: anchor) }
        row.hoverReaction = { [weak self] message, emoji, entered, anchor in
            guard let self, let container = anchor.window?.contentView else { return }
            self.reactionTooltip.hover(message, emoji: emoji, entered: entered, from: anchor, in: container)
        }
    }

    private func messageAction(_ message: ChatMessage, action: MessageAction, anchor: NSView) {
        let chat = model.chat
        guard let current = dialogs.currentMessage(message) else { return }
        if current.isDeleted { guard case .openThread = action else { return } }
        let context = dialogs.messageContext
        switch action {
        case .reply: chat.reply(to: current, inThread: inThread)
        case .replyInThread: chat.replyInThread(to: current)
        case .openThread:
            Task {
                guard dialogs.messageContext == context else { return }
                await chat.openThread(current.threadRootId ?? current.id)
            }
        case .history: dialogs.messageHistory(current, in: presentationParent)
        case .reactors(let emoji): dialogs.reactions(current, emoji: emoji, in: presentationParent)
        case .addReaction: picker.reaction(to: current, from: anchor, composer: composer)
        case .dismissReaction: chat.reactionErrors[current.id] = nil
        case .dismissPin: chat.pinErrors[current.id] = nil
        case .pin, .retryPin:
            Task {
                guard dialogs.messageContext == context, chat.canInteract,
                      let latest = dialogs.currentMessage(current), !latest.isDeleted,
                      !chat.pendingPins.contains(latest.id) else { return }
                if case .retryPin = action { await chat.retryPin(messageID: latest.id) }
                else { await chat.setPin(messageID: latest.id, active: latest.pin == nil) }
            }
        case .retryReaction:
            Task {
                guard dialogs.messageContext == context, chat.canInteract,
                      let latest = dialogs.currentMessage(current), !latest.isDeleted else { return }
                await chat.retryReaction(messageID: latest.id)
            }
        case .reaction(let emoji):
            Task {
                guard dialogs.messageContext == context, chat.canInteract,
                      let latest = dialogs.currentMessage(current), !latest.isDeleted else { return }
                let own = latest.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(chat.viewerID ?? "") == true
                await chat.setReaction(messageID: latest.id, emoji: emoji, active: !own)
            }
        }
    }

    private func messageMenu(_ message: ChatMessage, anchor: NSView) -> NSMenu? {
        reactionTooltip.close()
        let chat = model.chat, context = dialogs.messageContext
        let root = chat.threadRootID
        guard let current = dialogs.currentMessage(message) else { return nil }
        // A menu can outlive navigation or an incoming edit. Resolve again
        // on activation instead of applying the snapshot that opened it.
        func perform(_ action: @escaping (ChatMessage) -> Void) -> () -> Void {
            { [weak self] in
                guard let self, self.dialogs.messageContext == context,
                      !self.inThread || self.model.chat.threadRootID == root,
                      let latest = self.dialogs.currentMessage(current) else { return }
                action(latest)
            }
        }
        func copy(_ text: String) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
        }
        let copyID = NavigationMenuItem("Copy message ID", action: perform { copy($0.id) })
        if current.isDeleted {
            var items: [NSMenuItem] = []
            if !inThread, current.threadRootId != nil || (current.thread?.replyCount ?? 0) > 0 {
                items.append(NavigationMenuItem("View thread", action: perform { [weak self, weak anchor] in
                    guard let self, let anchor else { return }; self.messageAction($0, action: .openThread, anchor: anchor)
                }))
            }
            return NavigationMenus.menu(items + [copyID])
        }
        var items: [NSMenuItem] = [NavigationMenuItem("Add reaction…", enabled: chat.canInteract, action: perform { [weak self, weak anchor] in
            guard let self, let anchor else { return }; self.messageAction($0, action: .addReaction, anchor: anchor)
        }), .separator()]
        if let reaction = current.reactions?.first {
            items.insert(NavigationMenuItem("View reactions", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.reactions($0, emoji: reaction.emoji, in: self.presentationParent)
            }), at: 1)
        }
        for (title, action) in [("Reply", MessageAction.reply), ("Reply in thread", MessageAction.replyInThread)] {
            items.append(NavigationMenuItem(title, enabled: chat.canInteract && chat.pendingMessage == nil, action: perform { [weak self, weak anchor] in
                guard let self, let anchor else { return }; self.messageAction($0, action: action, anchor: anchor)
            }))
        }
        if chat.canForward {
            items.append(NavigationMenuItem("Forward message", action: perform {
                guard chat.canForward, !$0.isDeleted else { return }; chat.forwardTarget = $0
            }))
        }
        items.append(.separator())
        if dialogs.canEdit(current) {
            items.append(NavigationMenuItem("Edit message", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.editMessage($0, in: self.presentationParent)
            }))
        }
        if current.forward == nil, (current.revision ?? 1) > 1 {
            items.append(NavigationMenuItem("Message history", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.messageHistory($0, in: self.presentationParent)
            }))
        }
        items.append(NavigationMenuItem(current.pin == nil ? "Pin message" : "Unpin message",
            enabled: chat.canInteract && !chat.pendingPins.contains(current.id), action: perform { [weak self, weak anchor] in
                guard let self, let anchor else { return }; self.messageAction($0, action: .pin, anchor: anchor)
            }))
        if !current.content.text.isEmpty {
            items.append(NavigationMenuItem("Copy text", action: perform { if !$0.isDeleted { copy($0.content.text) } }))
        }
        let links = NativePresentation.links(in: current.content.text)
        for link in links {
            func linkAction(_ action: @escaping () -> Void) -> () -> Void {
                perform { latest in
                    guard !latest.isDeleted, NativePresentation.links(in: latest.content.text).contains(where: { $0.url == link.url }) else { return }
                    action()
                }
            }
            let actions = [NavigationMenuItem("Open link", action: linkAction { NSWorkspace.shared.open(link.url) }),
                NavigationMenuItem("Copy link", action: linkAction { copy(link.url.absoluteString) })]
            if links.count == 1 { items += actions }
            else { items.append(NavigationMenus.submenu(link.text, actions)) }
        }
        items.append(copyID)
        if dialogs.canDelete(current) {
            items += [.separator(), NavigationMenuItem("Delete message", role: .destructive, action: perform { [weak self] in
                guard let self else { return }; self.dialogs.deleteMessage($0, in: self.presentationParent)
            })]
        }
        if dialogs.canBlock(current.author) {
            items += [.separator(), NavigationMenuItem("Block \(current.author.name)…", role: .destructive, action: perform { [weak self] in
                self?.confirmBlock(BlockTarget(author: $0.author))
            })]
        } else if dialogs.canUnblock(current.author) {
            items += [.separator(), NavigationMenuItem("Unblock \(current.author.name)", action: perform { chat.unblock($0.author) })]
        }
        return NavigationMenus.menu(items)
    }

    private func confirmBlock(_ target: BlockTarget) {
        dialogs.confirmBlock(target, in: presentationParent)
    }

    func textView(_ view: NSTextView, shouldChangeTextIn affectedCharRange: NSRange, replacementString: String?) -> Bool {
        guard !formattingDraft, !view.hasMarkedText(), let replacement = replacementString,
              view.undoManager?.isUndoing != true, view.undoManager?.isRedoing != true else { return true }
        let proposed = (view.string as NSString).replacingCharacters(in: affectedCharRange, with: replacement)
        let result = typing.repairEdit(view.string, text: proposed,
            selection: NSRange(location: affectedCharRange.location + replacement.utf16.count, length: 0), replacing: affectedCharRange)
        guard result.text != proposed else { return true }
        formattingDraft = true
        view.insertText(result.text, replacementRange: NSRange(location: 0, length: view.string.utf16.count))
        view.setSelectedRange(result.selection)
        formattingDraft = false
        draftChanged()
        return false
    }

    func textDidChange(_ notification: Notification) {
        guard !formattingDraft else { return }
        draftChanged()
    }
    /// The draft's height at the editor's current width, insets included.
    /// One line of the draft as the editor lays it out, which can be a point
    /// taller than the font's nominal line height. The empty editor is sized
    /// with it, so the field (and the history resting on it) does not move
    /// when the first character arrives.
    private var draftLineHeight: CGFloat { draftLine.height }
    /// Its height and baseline; emoji lines keep both (ComposerGlyphs.fixedLine).
    private lazy var draftLine: (height: CGFloat, baseline: CGFloat) = {
        let font = NativeStyle.font(14)
        let storage = NSTextStorage(string: "M", attributes: [.font: font])
        let manager = DraftLinkLayoutManager()
        let container = NSTextContainer(size: NSSize(width: 1000, height: CGFloat.greatestFiniteMagnitude))
        manager.addTextContainer(container); storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        let height = max(manager.defaultLineHeight(for: font), manager.usedRect(for: container).height)
        return (height, manager.location(forGlyphAt: 0).y)
    }()
    private func editorContentHeight() -> CGFloat {
        guard let manager = composer.layoutManager, let container = composer.textContainer else { return 40 }
        manager.ensureLayout(for: container)
        return ceil(max(draftLineHeight, manager.usedRect(for: container).height)) + composer.textContainerInset.height * 2
    }
    /// The field grows with its draft; keystrokes that keep its height lay nothing out.
    private func relayoutIfDraftResized() {
        if editorContentHeight() != laidOutEditorHeight { view.needsLayout = true }
    }
    private func draftChanged() {
        composer.needsDisplay = true
        relayoutIfDraftResized()
        if inThread { model.chat.threadDraft = composer.string }
        else { model.chat.draft = composer.string; model.chat.draftChanged(composer.string) }
        suggestionsDismissed = false
        refreshSuggestions()
    }
    func textDidEndEditing(_ notification: Notification) {
        if notification.object as? NSTextView === composer {
            typing.end()
            formatToolbar.isHidden = true
        }
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        guard !formattingDraft else { return }
        typing.observe(composer.string, selection: composer.selectedRange())
        suggestionsDismissed = false
        formatToolbarDismissed = false
        refreshSuggestions()
    }
    private func refreshSuggestions() {
        guard !formattingDraft else { return }
        if !composer.hasMarkedText(), let storage = composer.textStorage {
            formattingDraft = true
            let selection = MessageFormatting.normalizedSelection(composer.string, selection: composer.selectedRange())
            if selection != composer.selectedRange() {
                composer.setSelectedRange(selection, affinity: composer.selectionAffinity, stillSelecting: false)
            }
            highlightDraft(storage, source: model.composerMentions)
            for key in [NSAttributedString.Key.composerDelimiter, .obliqueness, .backgroundColor, .toolTip, .draftLink, .kern] + composerFormattingAttributes {
                composer.typingAttributes.removeValue(forKey: key)
            }
            composer.typingAttributes[.font] = NativeStyle.font(14)
            composer.typingAttributes[.foregroundColor] = NativeStyle.text
            formattingDraft = false
        }
        let focused = composer.isEditable && composer.window?.firstResponder === composer
        let active = focused && !suggestionsDismissed
        suggestions.update(active ? NativePresentation.autocomplete(text: composer.string, selection: composer.selectedRange(),
            markedText: composer.hasMarkedText(), model: model) : nil)
        layoutSuggestions()
        let range = composer.selectedRange()
        let showFormat = focused && !composerField.isHidden && !composer.hasMarkedText() && range.length > 0 && !formatToolbarDismissed
        if showFormat {
            formatToolbar.update(text: composer.string, selection: range)
            formatToolbar.isHidden = false
            layoutFormatToolbar()
        } else {
            formatToolbar.isHidden = true
        }
    }
    private func layoutSuggestions() {
        let width = suggestions.match?.isMention == true ? composerField.frame.width : min(260, composerField.frame.width)
        suggestions.frame = NSRect(x: composerField.frame.minX, y: composerField.frame.minY - suggestions.height - 8,
            width: width, height: suggestions.height)
    }
    private func layoutFormatToolbar() {
        guard !formatToolbar.isHidden else { return }
        let barSize = formatToolbar.preferredSize
        let range = composer.selectedRange()
        var targetX = composerField.frame.minX
        if range.length > 0, let layout = composer.layoutManager, let container = composer.textContainer {
            let glyphs = layout.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            if glyphs.length > 0 {
                let rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
                let inComposer = NSRect(
                    x: rect.origin.x + composer.textContainerOrigin.x,
                    y: rect.origin.y + composer.textContainerOrigin.y,
                    width: rect.width,
                    height: rect.height
                )
                let inStage = stage.convert(inComposer, from: composer)
                targetX = inStage.midX - barSize.width / 2
            }
        }
        let minX = composerShell.frame.minX
        let maxX = max(minX, stage.bounds.width - barSize.width - 16)
        let x = min(max(targetX, minX), maxX)
        let y = max(52, composerShell.frame.minY - barSize.height - 8)
        formatToolbar.frame = NSRect(x: floor(x), y: floor(y), width: barSize.width, height: barSize.height)
    }
    private func formatDraft(_ style: MessageStyle) {
        guard composer.isEditable, !composer.hasMarkedText(),
              let result = MessageFormatting.toggle(composer.string, selection: composer.selectedRange(), style: style),
              result.text.unicodeScalars.count <= 4000, result.text != composer.string else { return }
        formattingDraft = true
        composer.breakUndoCoalescing()
        composer.insertText(result.text, replacementRange: NSRange(location: 0, length: composer.string.utf16.count))
        composer.setSelectedRange(result.selection)
        composer.breakUndoCoalescing()
        formattingDraft = false
        draftChanged()
    }
    private func handleComposerKey(_ event: NSEvent) -> Bool {
        guard !composer.hasMarkedText() else { return false }
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
           let key = event.charactersIgnoringModifiers, let item = MessageFormattingAction.shortcut(key) {
            formatDraft(item.style); return true
        }
        if handleSuggestionKey(event) { return true }
        if editLastMessage(event) { return true }
        if event.keyCode == 53, !formatToolbar.isHidden {
            formatToolbarDismissed = true
            formatToolbar.isHidden = true
            return true
        }
        if event.keyCode == 53, model.chat.threadRootID != nil { closeThread(); return true }
        if event.keyCode == 36, !event.modifierFlags.contains(.shift) { submit(); return true }
        return false
    }
    /// A bare Up in a composer with no text and no files edits your newest
    /// message here, as in Discord and Slack (web `editLastKey`). Open
    /// suggestions took the arrows first.
    private func editLastMessage(_ event: NSEvent) -> Bool {
        let chat = model.chat
        guard event.keyCode == 126, event.modifierFlags.intersection([.shift, .option, .control, .command]).isEmpty,
              !composer.hasMarkedText(), suggestions.match == nil, composer.string.isEmpty,
              (inThread ? chat.threadAttachmentDrafts : chat.attachmentDrafts).isEmpty,
              let message = chat.lastEditableMessage(inThread: inThread) else { return false }
        dialogs.editMessage(message, in: presentationParent)
        return true
    }
    private func handleSuggestionKey(_ event: NSEvent) -> Bool {
        guard !composer.hasMarkedText(), suggestions.match != nil else { return false }
        switch event.keyCode {
        case 125, 126:
            suggestions.move(down: event.keyCode == 125); return true
        case 36, 48:
            guard !event.modifierFlags.contains(.shift) else { return false }
            acceptSuggestion(suggestions.selected); return true
        case 53:
            suggestionsDismissed = true; suggestions.update(nil); return true
        default: return false
        }
    }
    private func acceptSuggestion(_ index: Int) {
        guard composer.isEditable, !composer.hasMarkedText(), let match = suggestions.match,
              match == NativePresentation.autocomplete(text: composer.string, selection: composer.selectedRange(), markedText: false, model: model),
              let result = NativePresentation.inserting(choice: index, in: composer.string, match: match) else { return }
        composer.window?.makeFirstResponder(composer)
        // insertText, rather than assigning string, preserves native undo.
        // Separate completion from the typed query and subsequent typing.
        composer.breakUndoCoalescing()
        composer.insertText(result.text, replacementRange: NSRange(location: 0, length: composer.string.utf16.count))
        composer.breakUndoCoalescing()
        composer.setSelectedRange(result.selection)
        suggestionsDismissed = true
        suggestions.update(nil)
    }
    private func applyComposerChrome() {
        composerShell.applyTheme()
        composerField.applyTheme(focused: composerFocused)
        formatToolbar.applyTheme()
        composer.drawsBackground = false
        composer.backgroundColor = .clear
        composer.textColor = NativeStyle.text
        composer.insertionPointColor = NativeStyle.text
    }

    @objc private func handleThemeChange() {
        applyComposerChrome()
        emptyTitle.textColor = NativeStyle.text
        emptyNote.textColor = NativeStyle.muted
        updateComposer()
    }

    private func submit() {
        guard send.isEnabled, !composer.hasMarkedText() else { return }
        timeline.showLatest()
        let context = dialogs.messageContext, root = model.chat.threadRootID
        Task {
            guard dialogs.messageContext == context, !inThread || model.chat.threadRootID == root else { return }
            await model.chat.send(inThread: inThread)
        }
    }

    private static func gifBadgeImage() -> NSImage {
        let size = NSSize(width: 24, height: 17)
        let label = NSAttributedString(string: "GIF", attributes: [
            .font: NativeStyle.font(9.5, bold: true),
            .foregroundColor: NSColor.black,
        ])
        let textSize = label.size()
        let image = NSImage(size: size, flipped: false) { rect in
            let stroke = NSBezierPath(roundedRect: rect.insetBy(dx: 0.75, dy: 0.75), xRadius: 4, yRadius: 4)
            stroke.lineWidth = 1.5
            NSColor.black.setStroke()
            stroke.stroke()
            label.draw(at: NSPoint(x: (rect.width - textSize.width) / 2, y: (rect.height - textSize.height) / 2))
            return true
        }
        image.isTemplate = true
        return image
    }
}

/// Recessed dark outer trough around the composer card, slightly deeper toward
/// the bottom edge to ground the elevated input field inside it.
private final class AppKitComposerShell: NSView {
    private let depth = CAGradientLayer()

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 24
        layer?.cornerCurve = .continuous
        layer?.borderWidth = 1
        layer?.masksToBounds = true
        // Unflipped: (0.5, 1) is the top edge and (0.5, 0) is the bottom edge.
        depth.startPoint = CGPoint(x: 0.5, y: 1)
        depth.endPoint = CGPoint(x: 0.5, y: 0)
        layer?.addSublayer(depth)
        setAccessibilityElement(false)
        applyTheme()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        depth.frame = bounds
        CATransaction.commit()
    }

    func applyTheme() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        depth.colors = [NativeStyle.composerOuterBg.cgColor, NativeStyle.composerOuterDepth.cgColor]
        layer?.borderColor = NativeStyle.composerOuterBorder.cgColor
        CATransaction.commit()
    }
}

/// Elevated inner composer surface with a delicate overhead top specular rim,
/// soft top sheen, and subtle bottom inner shade matching web's composer depth.
private final class AppKitComposerField: NSView {
    private let sheen = CAGradientLayer()
    private let topRim = CAGradientLayer()
    private let topRimMask = CAShapeLayer()
    private let bottomShade = CAGradientLayer()
    private let bottomShadeMask = CAShapeLayer()

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 20
        layer?.cornerCurve = .continuous
        layer?.borderWidth = 1
        layer?.masksToBounds = true
        // Unflipped: (0.5, 1) is top and (0.5, 0) is bottom.
        sheen.startPoint = CGPoint(x: 0.5, y: 1)
        sheen.endPoint = CGPoint(x: 0.5, y: 0)
        sheen.locations = [0, 0.28]
        layer?.addSublayer(sheen)

        topRim.startPoint = CGPoint(x: 0.5, y: 1)
        topRim.endPoint = CGPoint(x: 0.5, y: 0)
        topRim.locations = [0, 0.26]
        topRimMask.fillColor = nil
        topRimMask.strokeColor = NSColor.black.cgColor
        topRimMask.lineWidth = 1
        topRim.mask = topRimMask
        layer?.addSublayer(topRim)

        bottomShade.startPoint = CGPoint(x: 0.5, y: 0)
        bottomShade.endPoint = CGPoint(x: 0.5, y: 1)
        bottomShade.locations = [0, 0.2]
        bottomShadeMask.fillColor = nil
        bottomShadeMask.strokeColor = NSColor.black.cgColor
        bottomShadeMask.lineWidth = 1
        bottomShade.mask = bottomShadeMask
        layer?.addSublayer(bottomShade)

        setAccessibilityElement(false)
        applyTheme(focused: false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        sheen.frame = bounds
        topRim.frame = bounds
        topRimMask.frame = bounds
        bottomShade.frame = bounds
        bottomShadeMask.frame = bounds
        let inset = bounds.insetBy(dx: 1.5, dy: 1.5)
        if inset.width > 0, inset.height > 0 {
            let radius = min(18.5, inset.width / 2, inset.height / 2)
            let path = CGPath(roundedRect: inset, cornerWidth: radius, cornerHeight: radius, transform: nil)
            topRimMask.path = path
            bottomShadeMask.path = path
        } else {
            topRimMask.path = nil
            bottomShadeMask.path = nil
        }
        CATransaction.commit()
    }

    func applyTheme(focused: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer?.backgroundColor = NativeStyle.composer.cgColor
        layer?.borderColor = (focused ? NativeStyle.composerFocusBorder : NativeStyle.composerBorder).cgColor
        let topSheen = NativeStyle.composerTopSheen
        sheen.colors = [topSheen.cgColor, topSheen.withAlphaComponent(0).cgColor]
        let topShine = NativeStyle.composerTopShine
        topRim.colors = [topShine.cgColor, topShine.withAlphaComponent(0).cgColor]
        let shade = NativeStyle.composerBottomShade
        bottomShade.colors = [shade.cgColor, shade.withAlphaComponent(0).cgColor]
        CATransaction.commit()
    }
}

/// The composer's lip, as SwiftUI's ComposerLip: an 8pt shade over the bottom
/// of the timeline, black at 14% along the bottom edge fading out upward. It
/// takes no clicks.
private final class AppKitComposerLip: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        let gradient = CAGradientLayer()
        // Unflipped: the gradient runs from the bottom edge up.
        gradient.colors = [NSColor.black.withAlphaComponent(0.14).cgColor, NSColor.black.withAlphaComponent(0).cgColor]
        layer = gradient
        wantsLayer = true
        setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// Floating selection formatting bar anchored over the macOS AppKit composer,
/// matching web's `.chat-format-toolbar` without stealing composer focus.
@MainActor private final class AppKitFormatToolbar: NSView {
    override var isFlipped: Bool { true }
    var formatAction: ((MessageStyle) -> Void)?
    private var buttons: [(button: AppKitFormatButton, item: MessageFormattingAction)] = []
    private let buttonSize: CGFloat = 36
    private let dividerGap: CGFloat = 12

    var preferredSize: NSSize {
        let count = CGFloat(MessageFormattingAction.all.count)
        return NSSize(width: count * buttonSize + dividerGap, height: buttonSize)
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.masksToBounds = false
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.32)
        shadow.shadowBlurRadius = 16
        shadow.shadowOffset = NSSize(width: 0, height: -4)
        self.shadow = shadow
        setAccessibilityElement(false)
        setAccessibilityIdentifier("format-toolbar")
        setAccessibilityLabel("Message formatting")
        isHidden = true

        var x: CGFloat = 0
        for item in MessageFormattingAction.all {
            if item.style == .quote { x += dividerGap }
            let button = AppKitFormatButton(item: item) { [weak self] in
                self?.formatAction?(item.style)
            }
            button.frame = NSRect(x: x, y: 0, width: buttonSize, height: buttonSize)
            addSubview(button)
            buttons.append((button, item))
            x += buttonSize
        }
        applyTheme()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(text: String, selection: NSRange) {
        let pressed = MessageFormatting.selectionStyles(text, selection: selection)
        for (button, item) in buttons {
            let allowed = MessageFormatting.toggle(text, selection: selection, style: item.style) != nil
            button.isEnabled = allowed
            button.isActive = pressed.contains(item.style)
            button.toolTip = allowed
                ? (item.key.map { "\(item.title) (⌘\($0.uppercased()))" } ?? item.title)
                : "This selection cannot be formatted without changing its text or link"
            button.applyTheme()
        }
    }

    func applyTheme() {
        layer?.backgroundColor = NativeStyle.surface.cgColor
        layer?.borderColor = NativeStyle.border.cgColor
        for (button, _) in buttons {
            button.applyTheme()
        }
    }
}

@MainActor private final class AppKitFormatButton: NSButton {
    private let handler: () -> Void
    private var hovered = false
    var isActive = false
    override var acceptsFirstResponder: Bool { false }

    init(item: MessageFormattingAction, action: @escaping () -> Void) {
        self.handler = action
        super.init(frame: .zero)
        isBordered = false
        title = ""
        imagePosition = .imageOnly
        focusRingType = .none
        wantsLayer = true
        layer?.cornerRadius = 6
        if item.style == .strikethrough {
            image = Self.strikethroughImage()
        } else {
            image = NSImage(systemSymbolName: item.symbol, accessibilityDescription: item.title)?
                .withSymbolConfiguration(.init(pointSize: 13.5, weight: .medium))
        }
        setAccessibilityLabel(item.title)
        setAccessibilityIdentifier("format-\(item.title.lowercased().replacingOccurrences(of: " ", with: "-"))")
        target = self
        self.action = #selector(clicked)
        let tracking = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(tracking)
        applyTheme()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func clicked() {
        guard isEnabled else { return }
        handler()
    }

    override func mouseEntered(with event: NSEvent) {
        hovered = true
        applyTheme()
    }

    override func mouseExited(with event: NSEvent) {
        hovered = false
        applyTheme()
    }

    override func resetCursorRects() {
        if isEnabled { addCursorRect(bounds, cursor: .pointingHand) }
    }

    func applyTheme() {
        alphaValue = isEnabled ? 1 : 0.35
        contentTintColor = (isActive || (hovered && isEnabled)) ? NativeStyle.text : NativeStyle.muted
        if isActive {
            layer?.backgroundColor = NativeStyle.accent.withAlphaComponent(hovered && isEnabled ? 0.38 : 0.28).cgColor
        } else if hovered && isEnabled {
            layer?.backgroundColor = NativeStyle.hoverWash.cgColor
        } else {
            layer?.backgroundColor = NSColor.clear.cgColor
        }
        setAccessibilityValue(isActive ? 1 : 0)
    }

    /// Clear S with a full-width horizontal strike line through its middle.
    private static func strikethroughImage() -> NSImage {
        let size = NSSize(width: 17, height: 17)
        let image = NSImage(size: size, flipped: true) { rect in
            let glyph = NSAttributedString(string: "S", attributes: [
                .font: NativeStyle.font(13.5, bold: false),
                .foregroundColor: NSColor.black,
            ])
            let glyphSize = glyph.size()
            glyph.draw(at: NSPoint(x: (rect.width - glyphSize.width) / 2, y: (rect.height - glyphSize.height) / 2 - 0.5))
            let bar = NSBezierPath()
            bar.move(to: NSPoint(x: 2, y: rect.midY))
            bar.line(to: NSPoint(x: rect.width - 2, y: rect.midY))
            bar.lineWidth = 1.6
            bar.lineCapStyle = .round
            NSColor.black.setStroke()
            bar.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}
