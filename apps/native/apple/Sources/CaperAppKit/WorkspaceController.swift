import AppKit
import CaperCore

@MainActor final class WorkspaceController: NSViewController, NSTextViewDelegate {
    private let model: AppModel
    private let inThread: Bool
    private var presentationParent: NSViewController { parent as? WorkspaceController ?? self }
    private var threadPane: WorkspaceController?
    private let threadObservation = NativeObservation()
    private var preferredThreadWidth = CGFloat(UserDefaults.standard.object(forKey: "caper.threadPanelWidth") as? Double ?? 340)
    private lazy var threadDivider = NativeThreadDivider { [weak self] delta in
        guard let self else { return }
        let maximum = min(600, self.view.bounds.width - self.navigationWidth - 300)
        self.preferredThreadWidth = min(maximum, max(300, self.preferredThreadWidth + delta))
        UserDefaults.standard.set(Double(self.preferredThreadWidth), forKey: "caper.threadPanelWidth")
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
    private let attachmentObservation = NativeObservation()
    private let forwardingObservation = NativeObservation()
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
    private lazy var pins = AppKitPinsPresenter(model: model, media: media)
    private lazy var picker = AppKitComposerPicker(model: model, inThread: inThread)
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
        self.pins.show(from: self.pinsButton, composer: self.composer)
    }
    private lazy var membersToggle = NativeButton.icon("Show members", symbol: "person.2") { [weak self] in
        guard let self else { return }
        self.membersPreference = self.members.isHidden
        self.view.needsLayout = true
    }
    private lazy var stage = AppKitFileDropView(model: model, inThread: inThread)
    private let timeline = MessageTimeline(frame: .zero)
    private let header = NativeStyle.label("", size: 16, bold: true)
    private let status = NativeStyle.label("", size: 12)
    private let composer = ComposerTextView()
    private let composerScroll = NSScrollView()
    private let attachmentScroll = NSScrollView()
    private var attachmentTray: AppKitAttachmentSurface?
    private var replyDraft: AppKitAttachmentSurface?
    private lazy var back = NativeButton("Back to channel") { [weak self] in self?.closeThread() }
    private lazy var broadcast: NativeButton = NativeButton("") { [weak self] in
        guard let self else { return }; self.model.chat.threadBroadcast = self.broadcast.state == .on
    }
    private lazy var newer = NativeButton("Load newer messages") { [weak self] in
        guard let self else { return }
        Task {
            if self.inThread { await self.model.chat.loadThread(newer: true) }
            else { await self.model.chat.loadNewer() }
        }
    }
    private lazy var latest = NativeButton("Back to latest") { [weak self] in
        guard let self else { return }
        let context = self.dialogs.messageContext, root = self.model.chat.threadRootID
        let channel = self.model.chat.channelID, viewer = self.model.chat.viewerID
        Task {
            if self.inThread { await self.model.chat.loadThread() }
            else { await self.model.chat.retryLoad() }
            guard self.inThread ? self.dialogs.messageContext == context && self.model.chat.threadRootID == root
                : self.model.chat.channelID == channel && self.model.chat.viewerID == viewer else { return }
            guard self.inThread ? self.model.chat.threadError == nil : !self.model.chat.loadFailed else { return }
            self.model.chat.focusedMessageID = nil
            self.timeline.showLatest()
        }
    }
    private var uploadConversationID: String?
    private var voicePreviewChecked = false
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
    private var formattingDraft = false
    /// The style web carries between typed keys (shared/messages/composer.rs).
    private var typing = ComposerTyping()
    private lazy var send = NativeButton("Send", primary: true) { [weak self] in self?.submit() }
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
        [timeline, header, pinsButton, membersToggle, back, broadcast, newer, latest, status, retry, editRejected, attachmentScroll, attach, composerScroll, emoji, gif, send, counter, join, directBar, suggestions].forEach { stage.addSubview($0) }
        pinsButton.setAccessibilityIdentifier("channel-pins")
        pinsButton.isHidden = inThread
        timeline.inThread = inThread
        if inThread { timeline.setAccessibilityIdentifier("thread-timeline"); timeline.setAccessibilityLabel("Thread replies") }
        back.isHidden = !inThread; back.font = NativeStyle.font(11, bold: true)
        back.setAccessibilityIdentifier("close-thread")
        broadcast.isHidden = !inThread; broadcast.setButtonType(.switch)
        broadcast.font = NativeStyle.font(11); broadcast.layer?.backgroundColor = NSColor.clear.cgColor
        broadcast.setAccessibilityIdentifier("thread-broadcast")
        newer.font = NativeStyle.font(11); latest.font = NativeStyle.font(11)
        newer.setAccessibilityIdentifier(inThread ? "load-newer-replies" : "load-newer-messages")
        latest.setAccessibilityIdentifier(inThread ? "thread-latest" : "channel-latest")
        attachmentScroll.drawsBackground = false
        attachmentScroll.hasVerticalScroller = true
        attachmentScroll.autohidesScrollers = true
        attachmentScroll.isHidden = true
        attach.setAccessibilityIdentifier("attach-files-button")
        emoji.setAccessibilityIdentifier("composer-emoji")
        gif.setAccessibilityIdentifier("composer-gif")
        gif.setAccessibilityLabel("Send a GIF"); gif.toolTip = "GIFs and stickers"
        gif.font = NativeStyle.font(11, bold: true)
        gif.layer?.backgroundColor = NSColor.clear.cgColor
        counter.setAccessibilityIdentifier("composer-counter")
        counter.alignment = .center
        join.setAccessibilityIdentifier("join-channel")
        if !inThread {
            navigation.show = { [weak self] item in
                guard let self else { return }; self.dialogs.show(item, in: self)
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
            }
            members.openProfile = { [weak self] member, view in
                self?.profiles.show(member: member, relativeTo: view.bounds, of: view)
            }
        }
        timeline.openAuthor = { [weak self] author, view in
            self?.profiles.show(author: author, relativeTo: view.bounds, of: view)
        }
        timeline.openMention = { [weak self] mention, view, rect in
            self?.profiles.show(mention: mention, relativeTo: rect, of: view)
        }
        membersToggle.setAccessibilityIdentifier("toggle-members")
        status.textColor = NativeStyle.muted
        composer.textContainer?.replaceLayoutManager(DraftLinkLayoutManager())
        composer.layoutManager?.delegate = composer.composerGlyphs
        composer.font = NativeStyle.font(14)
        composer.textColor = NativeStyle.text
        composer.insertionPointColor = NativeStyle.text
        composer.backgroundColor = NativeStyle.composer
        composer.isRichText = false
        composer.allowsUndo = true
        composer.attachmentDropDestination = stage
        stage.acceptedDrop = { [weak self] in
            guard let self, self.composer.isEditable else { return }
            self.view.window?.makeFirstResponder(self.composer)
        }
        composer.updateDragTypeRegistration()
        composer.textContainerInset = NSSize(width: 12, height: 12)
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
            if focused { self?.refreshSuggestions() } else { self?.suggestions.update(nil) }
        }
        composer.setAccessibilityHelp("Return sends. Shift-Return adds a new line. Use arrows and Return or Tab to choose a suggestion; Escape dismisses.")
        suggestions.accept = { [weak self] in self?.acceptSuggestion($0) }
        composerScroll.documentView = composer
        composerScroll.hasVerticalScroller = true
        composerScroll.autohidesScrollers = true
        composerScroll.drawsBackground = true
        composerScroll.backgroundColor = NativeStyle.composer
        composerScroll.wantsLayer = true; composerScroll.layer?.cornerRadius = 8
        send.setAccessibilityIdentifier(inThread ? "thread-send" : "send-message")
        send.image = NSImage(systemSymbolName: "paperplane.fill", accessibilityDescription: nil)
        send.imagePosition = .imageOnly
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
        timeline.makePending = { [weak self] pending in self?.media.pendingMessage(pending) }
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
        timeline.react = { [weak self] id, emoji in
            guard let self,
                  let message = self.model.chat.displayedMessages.first(where: { $0.id == id }) else { return }
            self.messageAction(message, action: .reaction(emoji), anchor: self.timeline)
        }
        navigationObservation.start { [weak self] in
            guard let self else { return }
            self.header.stringValue = self.inThread ? "Thread" : self.model.selectedDirectMessage?.peer.displayName ?? "# \(self.model.chat.channelName)"
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
            let identity = [chat.channelID ?? "", root ?? "channel"].joined(separator: ":")
            self.timeline.update(visible, conversationID: identity,
                viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs,
                pending: chat.pendingMessage.flatMap { $0.threadRootId == root ? $0 : nil },
                canInteract: chat.canInteract, reactionErrors: chat.reactionErrors, pinErrors: chat.pinErrors, pendingPins: chat.pendingPins)
            if let id = chat.focusedMessageID, self.focusedRevision != chat.focusRevision || self.focusedConversation != identity,
               visible.contains(where: { $0.id == id }) {
                self.focusedRevision = chat.focusRevision; self.focusedConversation = identity
                self.timeline.focusMessage(id)
            }
            DispatchQueue.main.async { [weak self] in self?.acknowledgeVisibleThread() }
        }
        composerObservation.start { [weak self] in self?.updateComposer() }
        attachmentObservation.start(reading: { [weak self] in
            guard let self else { return false }
            let chat = self.model.chat
            return !self.model.previewingChannel && (self.model.selectedDirectMessage.map { $0.status != .incoming && !$0.blocked } ?? true)
                && (self.inThread ? !chat.threadAttachmentDrafts.isEmpty || chat.threadAttachmentNotice != nil
                    : !chat.attachmentDrafts.isEmpty || chat.attachmentNotice != nil)
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
        picker.close(restoringFocus: false)
        pins.close(restoringFocus: false)
        reactionTooltip.close()
    }

    private func updateThreadPane(_ root: String?) {
        pins.close(restoringFocus: false)
        if root != nil, threadPane == nil {
            let pane = WorkspaceController(model: model, inThread: true)
            addChild(pane)
            view.addSubview(pane.view, positioned: .below, relativeTo: members)
            threadPane = pane
            if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0; fade.toValue = 1; fade.duration = 0.14
                fade.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                pane.view.layer?.add(fade, forKey: "entrance")
            }
        } else if root == nil, let pane = threadPane {
            let returnFocus = (view.window?.firstResponder as? NSView).map { $0.isDescendant(of: pane.view) } == true
            pane.picker.close(restoringFocus: false)
            pane.view.removeFromSuperview(); pane.removeFromParent()
            threadPane = nil
            stage.isHidden = false
            model.threadVisibilityChanged(rootID: nil, seq: nil)
            if returnFocus { view.window?.makeFirstResponder(composer) }
        }
        view.needsLayout = true
    }

    private func closeThread() {
        picker.close(restoringFocus: false)
        model.chat.closeThread()
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
        let width = view.bounds.width, height = view.bounds.height
        let navigationWidth = self.navigationWidth
        let docked = width > 760 && width - navigationWidth >= 540
        let hasMembers = !inThread && threadPane == nil && model.selectedDirectMessageID == nil && model.visit == nil
        let visible = hasMembers && (membersPreference ?? docked)
        if !visible { membersDragWidth = nil }
        let maximum = min(440, max(220, width - navigationWidth - (docked ? 320 : 64)))
        // Clamping a small window never overwrites the user's saved width.
        let memberWidth = min(maximum, max(220, membersDragWidth ?? preferredMembersWidth))
        if inThread { stage.frame = view.bounds }
        else {
            navigation.frame = NSRect(x: 0, y: 0, width: navigationWidth, height: height)
            sidebarResize.frame = NSRect(x: navigationWidth - 8, y: 0, width: 8, height: height)
            sidebarResize.update(width: navigationWidth - NavigationSidebar.railWidth, maximum: sidebarMaximum)
            // Narrow windows keep the editor width and show one conversation
            // at a time; the native channel timeline retains its scroll anchor.
            let available = max(0, width - navigationWidth - (visible && docked ? memberWidth : 0))
            let split = available >= 900
            let threadWidth: CGFloat = threadPane == nil ? 0 : split ? min(min(600, available - 300), max(300, preferredThreadWidth)) : available
            stage.isHidden = threadPane != nil && !split
            stage.frame = NSRect(x: navigationWidth, y: 0, width: max(0, available - (split ? threadWidth : 0)), height: height)
            threadPane?.view.frame = NSRect(x: width - threadWidth, y: 0, width: threadWidth, height: height)
            threadDivider.isHidden = threadPane == nil || !split
            threadDivider.frame = NSRect(x: width - threadWidth - 3, y: 0, width: 6, height: height)
            threadDivider.setAccessibilityValue("\(Int(threadWidth)) points")
            members.frame = NSRect(x: width - memberWidth, y: docked ? 0 : 50, width: memberWidth, height: height - (docked ? 0 : 50))
            members.setVisible(visible)
            membersResize.isHidden = !visible
            membersResize.frame = NSRect(x: members.frame.minX, y: members.frame.minY, width: 8, height: members.frame.height)
            membersResize.update(width: memberWidth, maximum: maximum)
        }
        if stage.frame != previousStage || stage.isHidden { reactionTooltip.close() }
        let center = stage.bounds.width
        back.frame = NSRect(x: center - 170, y: 10, width: 150, height: 28)
        membersToggle.frame = NSRect(x: center - 48, y: 8, width: 32, height: 32)
        membersToggle.isHidden = !hasMembers
        pinsButton.frame = NSRect(x: center - (membersToggle.isHidden ? 48 : 88), y: 8, width: 32, height: 32)
        header.frame = NSRect(x: 20, y: 14, width: max(0, center - (inThread ? 195 : membersToggle.isHidden ? 80 : 120)), height: 26)
        membersToggle.setAccessibilityLabel(visible ? "Hide members" : "Show members")
        membersToggle.toolTip = visible ? "Hide members" : "Show members"
        membersToggle.contentTintColor = visible ? NativeStyle.accent : NativeStyle.muted
        let base = height - (broadcast.isHidden ? 0 : 28)
        let compact = center < 500
        let composerTop = base - (compact ? 150 : 102)
        let attachmentHeight = attachmentTray?.height(for: max(1, center - 40)) ?? 0
        let trayHeight = min(attachmentHeight, max(80, height * 0.3))
        let quoteHeight = replyDraft?.height(for: max(1, center - 40)) ?? 0
        attachmentScroll.frame = NSRect(x: 20, y: composerTop - 8 - trayHeight, width: center - 40, height: trayHeight)
        attachmentTray?.frame = NSRect(x: 0, y: 0, width: center - 40, height: attachmentHeight)
        let quoteBottom = attachmentScroll.frame.minY - (trayHeight > 0 ? 8 : 0)
        replyDraft?.frame = NSRect(x: 20, y: quoteBottom - quoteHeight, width: center - 40, height: quoteHeight)
        let recoveryHeight: CGFloat = editRejected.isHidden ? 0 : 34
        let statusY = directBar.isHidden ? quoteBottom - quoteHeight - recoveryHeight - 30 : height - max(138, directBar.preferredHeight(width: center - 40) + 32) + 6
        let top: CGFloat = newer.isHidden && latest.isHidden ? 48 : 88
        timeline.frame = NSRect(x: 0, y: top, width: center, height: max(0, statusY - 6 - top))
        newer.frame = NSRect(x: 20, y: 52, width: 150, height: 28)
        latest.frame = NSRect(x: center - 130, y: 52, width: 110, height: 28)
        status.frame = NSRect(x: 20, y: statusY, width: center - (retry.isHidden ? 40 : 116), height: 24)
        directBar.frame = NSRect(x: 20, y: status.frame.maxY, width: center - 40, height: max(0, height - status.frame.maxY - 8))
        attach.frame = NSRect(x: 16, y: base - 72, width: 38, height: 42)
        send.frame = NSRect(x: center - 56, y: base - 70, width: 36, height: 38)
        emoji.frame = NSRect(x: send.frame.minX - 40, y: base - 72, width: 32, height: 42)
        gif.frame = NSRect(x: emoji.frame.minX - 40, y: base - 72, width: 32, height: 42)
        let editorRight = (gif.isHidden ? emoji.frame.minX : gif.frame.minX) - 12
        composerScroll.frame = NSRect(x: compact ? 20 : 58, y: composerTop, width: compact ? center - 40 : editorRight - 58, height: 70)
        composer.frame.size.width = composerScroll.contentSize.width
        composer.minSize = NSSize(width: 0, height: 70)
        counter.frame = NSRect(x: 20, y: base - 24, width: center - 40, height: 16)
        broadcast.frame = NSRect(x: 20, y: height - 26, width: center - 40, height: 22)
        join.frame = NSRect(x: 20, y: height - 80, width: 140, height: 38)
        layoutSuggestions()
        retry.frame = NSRect(x: center - 84, y: statusY, width: 64, height: 24)
        editRejected.frame = NSRect(x: 20, y: quoteBottom - quoteHeight - recoveryHeight, width: 200, height: 30)
        focusComposerIfRequested()
    }

    private func updateComposer() {
        let chat = model.chat
        if inThread { directBar.isHidden = true } else { directBar.update() }
        let root = inThread ? chat.threadRootID : nil
        let identity = [dialogs.messageContext, root ?? "channel"].joined(separator: ":")
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
        }
        composer.placeholder = inThread ? "Reply to thread…" : "Message \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        let ready = !inThread || (root != nil && !chat.threadLoading)
        pinsButton.isEnabled = chat.channelID != nil
        composer.isEditable = ready && directBar.isHidden && chat.canInteract && !chat.sending && !chat.sendRejected && chat.pendingMessage == nil
        send.isEnabled = ready && directBar.isHidden && chat.canInteract && chat.canPressSend(inThread: inThread)
        send.title = chat.sending ? "Sending…" : inThread ? "Send reply" : "Send"
        send.setAccessibilityLabel(send.title); send.toolTip = send.title
        send.layer?.backgroundColor = (send.isEnabled ? NSColor(CaperTheme.action) : NativeStyle.composer).cgColor
        send.contentTintColor = send.isEnabled ? NativeStyle.blackout : NativeStyle.muted
        composerScroll.isHidden = model.previewingChannel || !directBar.isHidden
        send.isHidden = composerScroll.isHidden
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
        counter.isHidden = composerScroll.isHidden || count < 3000
        counter.stringValue = "\(count.formatted()) / 4,000"
        counter.textColor = AppKitComposerCounter.color(count)
        join.isHidden = inThread || !model.previewingChannel
        join.isEnabled = joiningChannelID == nil
        join.title = joiningChannelID == model.selectedChannelID ? "Joining…" : "Join channel"
        broadcast.isHidden = !inThread || composerScroll.isHidden
        broadcast.title = "Also send to \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        broadcast.setAccessibilityLabel(broadcast.title)
        broadcast.state = chat.threadBroadcast ? .on : .off
        broadcast.isEnabled = ready && chat.canInteract && chat.pendingMessage == nil
        let hasQuote = !composerScroll.isHidden && chat.replyTargets[root ?? "channel"] != nil
        if hasQuote, replyDraft == nil {
            let banner = media.replyDraft(inThread: inThread)
            banner.heightChanged = { [weak self] in self?.view.needsLayout = true }
            stage.addSubview(banner, positioned: .below, relativeTo: suggestions)
            replyDraft = banner
        } else if !hasQuote { replyDraft?.removeFromSuperview(); replyDraft = nil }
        newer.isHidden = !(inThread ? chat.threadHasNewer : chat.hasNewer)
        newer.title = inThread ? "Load newer replies" : "Load newer messages"
        newer.setAccessibilityLabel(newer.title)
        newer.isEnabled = inThread ? !chat.threadLoading : !chat.loadingNewer
        latest.isHidden = newer.isHidden && chat.focusedMessageID == nil
        latest.isEnabled = inThread ? !chat.threadLoading : !chat.loading && !chat.jumpingToMessage
        // Keystrokes change only the composer, not timeline geometry.
        let pending = chat.pendingMessage.flatMap { $0.threadRootId == root ? $0 : nil }
        let historyError = inThread ? chat.threadError : chat.olderError
        retry.isHidden = !(chat.loadFailed || historyError != nil || chat.sessionError != nil || (pending != nil && !chat.sending && !chat.sendRejected))
        retry.isEnabled = inThread ? !chat.threadLoading : !chat.loadingOlder
        editRejected.isHidden = !chat.sendRejected || pending == nil
        let error = (chat.pendingMessage == nil || pending != nil ? chat.error : nil) ?? historyError ?? chat.jumpError ?? chat.sessionError ?? model.navigationError
        status.setAccessibilityIdentifier(inThread ? (error == nil ? "thread-status" : "thread-error") : (error == nil ? "conversation-status" : "conversation-error"))
        status.textColor = error == nil ? NativeStyle.muted : NSColor(CaperTheme.error)
        status.stringValue = error
            ?? (inThread ? (chat.threadLoading ? "Loading replies…" : chat.threadMessages.isEmpty ? "No replies yet. Start the thread." : "")
                : model.previewingChannel ? "Preview · Join #\(chat.channelName) to interact with people here" : chat.loading ? "Loading messages…" : chat.loadingOlder ? "Loading older messages…" : chat.typingNames.isEmpty ? (model.selectedDirectMessage?.status == .outgoing ? model.selectedDirectMessage.flatMap(NativePresentation.directMessageNotice) ?? "" : "") : "\(chat.typingNames.joined(separator: ", ")) typing…")
        // Layout consumes focus only once the requested pane is on screen.
        _ = chat.composerFocusRequest(inThread: inThread)
        view.needsLayout = true
        refreshSuggestions()
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
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: attach.bounds.maxY), in: attach)
    }

    @objc private func pickFiles() { media.pickFiles(inThread: inThread) }

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
            items += [.separator(), NavigationMenuItem("Delete message", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.deleteMessage($0, in: self.presentationParent)
            })]
        }
        if dialogs.canBlock(current.author) {
            items += [.separator(), NavigationMenuItem("Block \(current.author.name)…", action: perform { [weak self] in
                self?.confirmBlock(BlockTarget(author: $0.author))
            })]
        } else if dialogs.canUnblock(current.author) {
            items += [.separator(), NavigationMenuItem("Unblock \(current.author.name)", action: perform { chat.unblock($0.author) })]
        }
        return NavigationMenus.menu(items)
    }

    private func confirmBlock(_ target: BlockTarget) {
        guard let window = view.window else { return }
        profiles.confirmBlock(target, window: window)
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
    private func draftChanged() {
        composer.needsDisplay = true
        if inThread { model.chat.threadDraft = composer.string }
        else { model.chat.draft = composer.string; model.chat.draftChanged(composer.string) }
        suggestionsDismissed = false
        refreshSuggestions()
    }
    func textDidEndEditing(_ notification: Notification) {
        if notification.object as? NSTextView === composer { typing.end() }
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        guard !formattingDraft else { return }
        typing.observe(composer.string, selection: composer.selectedRange())
        suggestionsDismissed = false
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
            for key: NSAttributedString.Key in [.composerDelimiter, .obliqueness, .backgroundColor, .toolTip, .draftLink, .kern] {
                composer.typingAttributes.removeValue(forKey: key)
            }
            composer.typingAttributes[.font] = NativeStyle.font(14)
            composer.typingAttributes[.foregroundColor] = NativeStyle.text
            formattingDraft = false
        }
        let active = composer.isEditable && composer.window?.firstResponder === composer && !suggestionsDismissed
        suggestions.update(active ? NativePresentation.autocomplete(text: composer.string, selection: composer.selectedRange(),
            markedText: composer.hasMarkedText(), model: model) : nil)
        layoutSuggestions()
    }
    private func layoutSuggestions() {
        let width = suggestions.match?.isMention == true ? composerScroll.frame.width : min(260, composerScroll.frame.width)
        suggestions.frame = NSRect(x: composerScroll.frame.minX, y: composerScroll.frame.minY - suggestions.height - 8,
            width: width, height: suggestions.height)
    }
    private func formatDraft(_ marker: String) {
        guard composer.isEditable, !composer.hasMarkedText(),
              let result = MessageFormatting.toggle(composer.string, selection: composer.selectedRange(), marker: marker),
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
           let key = event.charactersIgnoringModifiers?.lowercased(), ["b", "i"].contains(key) {
            formatDraft(key == "b" ? "**" : "_"); return true
        }
        if handleSuggestionKey(event) { return true }
        if event.keyCode == 53, model.chat.threadRootID != nil { closeThread(); return true }
        if event.keyCode == 36, !event.modifierFlags.contains(.shift) { submit(); return true }
        return false
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
    private func submit() {
        guard send.isEnabled, !composer.hasMarkedText() else { return }
        timeline.showLatest()
        let context = dialogs.messageContext, root = model.chat.threadRootID
        Task {
            guard dialogs.messageContext == context, !inThread || model.chat.threadRootID == root else { return }
            await model.chat.send(inThread: inThread)
        }
    }
}

/// The same saved width as the shipping thread panel, with native pointer,
/// keyboard and accessibility adjustments. Width remains a host concern.
@MainActor private final class NativeThreadDivider: NativeView {
    private let resize: (CGFloat) -> Void
    init(resize: @escaping (CGFloat) -> Void) {
        self.resize = resize
        super.init()
        setAccessibilityElement(true); setAccessibilityRole(.splitter)
        setAccessibilityLabel("Resize thread"); setAccessibilityIdentifier("thread-divider")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var acceptsFirstResponder: Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
    override func draw(_ dirtyRect: NSRect) {
        NativeStyle.border.setFill()
        NSRect(x: floor(bounds.midX), y: 0, width: 1, height: bounds.height).fill()
    }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        window.makeFirstResponder(self)
        var previous = event.locationInWindow.x
        while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            if next.type == .leftMouseUp { break }
            resize(previous - next.locationInWindow.x)
            previous = next.locationInWindow.x
        }
    }
    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123: resize(16)
        case 124: resize(-16)
        default: super.keyDown(with: event)
        }
    }
    override func accessibilityPerformIncrement() -> Bool { resize(16); return true }
    override func accessibilityPerformDecrement() -> Bool { resize(-16); return true }
}
