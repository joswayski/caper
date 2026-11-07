import AppKit
import CaperCore

@MainActor final class WorkspaceController: NSViewController, NSTextViewDelegate {
    private let model: AppModel
    private let navigationObservation = NativeObservation()
    private let timelineObservation = NativeObservation()
    private let composerObservation = NativeObservation()
    private let attachmentObservation = NativeObservation()
    private lazy var navigation = NavigationSidebar(model: model)
    private lazy var dialogs = AppKitWorkspacePresenter(model: model)
    private lazy var media = AppKitMediaPresenter(model: model, parent: self)
    private lazy var picker = AppKitComposerPicker(model: model)
    private lazy var members = MemberPanel(model: model)
    private lazy var profiles = AppKitProfilePresenter(model: model)
    private var membersPreference: Bool?
    private lazy var membersToggle = NativeButton.icon("Show members", symbol: "person.2") { [weak self] in
        guard let self else { return }
        self.membersPreference = self.members.isHidden
        self.view.needsLayout = true
    }
    private lazy var stage = AppKitFileDropView(model: model)
    private let timeline = MessageTimeline(frame: .zero)
    private let header = NativeStyle.label("", size: 16, bold: true)
    private let status = NativeStyle.label("", size: 12)
    private let composer = ComposerTextView()
    private let composerScroll = NSScrollView()
    private let attachmentScroll = NSScrollView()
    private var attachmentTray: AppKitAttachmentSurface?
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
    /// The conversation (and account) the composer shows; another one starts typing afresh.
    private var composerChannel: String?
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
            if self.model.chat.loadFailed { await self.model.chat.retryLoad() }
            else if self.model.chat.olderError != nil { await self.model.chat.loadOlder() }
            else if self.model.chat.sessionError != nil { await self.model.chat.retrySession() }
            else { await self.model.chat.send(retryPending: true) }
        }
    }
    private lazy var editRejected = NativeButton("Edit rejected message") { [weak self] in
        _ = self?.model.chat.discardRejected(edit: true)
    }

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() { view = AppKitWorkspaceContainer() }
    override func viewDidLoad() {
        super.viewDidLoad()
        [navigation, stage, members].forEach { view.addSubview($0) }
        [timeline, header, membersToggle, status, retry, editRejected, attachmentScroll, attach, composerScroll, emoji, gif, send, counter, join, directBar, suggestions].forEach { stage.addSubview($0) }
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
        composer.setAccessibilityLabel("Message")
        composer.setAccessibilityIdentifier("message-composer")
        composer.delegate = self
        composer.handleKey = { [weak self] in self?.handleComposerKey($0) == true }
        composer.formatSelection = { [weak self] in self?.formatDraft($0) }
        composer.pasteAttachments = { [weak self] in
            guard let self, self.composer.isEditable else { return false }
            return self.model.chat.pasteAttachments()
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
        send.setAccessibilityIdentifier("send-message")
        send.image = NSImage(systemSymbolName: "paperplane.fill", accessibilityDescription: nil)
        send.imagePosition = .imageOnly
        retry.setAccessibilityIdentifier("conversation-retry")
        timeline.loadOlder = { [weak self] in
            guard let self else { return }
            Task { await self.model.chat.loadOlder() }
        }
        timeline.makeAttachments = { [weak self] message in self?.media.messageAttachments(message) }
        timeline.makePending = { [weak self] pending in self?.media.pendingMessage(pending) }
        timeline.messageMenu = { [weak self] message, anchor in self?.messageMenu(message, anchor: anchor) }
        timeline.messageAction = { [weak self] message, action, anchor in self?.messageAction(message, action: action, anchor: anchor) }
        timeline.react = { [weak self] id, emoji in
            guard let self,
                  let message = self.model.chat.displayedMessages.first(where: { $0.id == id }) else { return }
            self.messageAction(message, action: .reaction(emoji), anchor: self.timeline)
        }
        navigationObservation.start { [weak self] in
            guard let self else { return }
            self.header.stringValue = self.model.selectedDirectMessage?.peer.displayName ?? "# \(self.model.chat.channelName)"
            if !self.voicePreviewChecked, self.model.detail != nil {
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
                Task {
                    await self.model.chat.checkUploadAvailability()
                    await self.model.chat.checkKlipyAvailability()
                }
            }
            self.view.needsLayout = true
        }
        timelineObservation.start { [weak self] in
            guard let self else { return }
            let chat = self.model.chat
            self.timeline.canLoadOlder = chat.hasMore && !chat.loading && !chat.loadingOlder && chat.olderError == nil
            let visible = chat.displayedMessages.filter(\.isChannelMessage)
            self.timeline.update(visible, conversationID: self.model.selectedDirectMessageID ?? self.model.selectedChannelID,
                viewerID: chat.viewerID, blocked: chat.blockedAuthorIDs,
                pending: chat.pendingMessage.flatMap { $0.threadRootId == nil ? $0 : nil },
                canInteract: chat.canInteract, reactionErrors: chat.reactionErrors)
        }
        composerObservation.start { [weak self] in self?.updateComposer() }
        attachmentObservation.start(reading: { [weak self] in
            guard let self else { return false }
            let chat = self.model.chat
            return !self.model.previewingChannel && (self.model.selectedDirectMessage.map { $0.status != .incoming && !$0.blocked } ?? true)
                && (!chat.attachmentDrafts.isEmpty || chat.attachmentNotice != nil)
        }, update: { [weak self] visible in
            guard let self else { return }
            if visible, self.attachmentTray == nil {
                let tray = self.media.composerAttachments()
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
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        picker.close(restoringFocus: false)
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        let width = view.bounds.width, height = view.bounds.height
        let visible = (membersPreference ?? (width >= 1100)) && model.selectedDirectMessageID == nil
        let docked = width >= 1100
        let memberWidth: CGFloat = visible && docked ? 220 : 0
        navigation.frame = NSRect(x: 0, y: 0, width: 344, height: height)
        // Narrow windows keep the composer width and open members over the
        // conversation instead of making the member control disappear.
        members.frame = NSRect(x: width - 220, y: docked ? 0 : 48, width: 220, height: height - (docked ? 0 : 48))
        members.setVisible(visible)
        stage.frame = NSRect(x: 344, y: 0, width: width - 344 - memberWidth, height: height)
        let center = stage.bounds.width
        header.frame = NSRect(x: 20, y: 14, width: center - 80, height: 26)
        membersToggle.frame = NSRect(x: center - 48, y: 8, width: 32, height: 32)
        membersToggle.isHidden = model.selectedDirectMessageID != nil
        membersToggle.setAccessibilityLabel(visible ? "Hide members" : "Show members")
        membersToggle.toolTip = visible ? "Hide members" : "Show members"
        membersToggle.contentTintColor = visible ? NativeStyle.accent : NativeStyle.muted
        let attachmentHeight = attachmentTray?.height(for: max(1, center - 40)) ?? 0
        let trayHeight = min(attachmentHeight, max(80, height * 0.3))
        let bottom = directBar.isHidden ? 138 + (trayHeight > 0 ? trayHeight + 8 : 0) : max(138, directBar.preferredHeight(width: center - 40) + 32)
        timeline.frame = NSRect(x: 0, y: 48, width: center, height: max(0, height - 48 - bottom))
        status.frame = NSRect(x: 20, y: height - bottom + 6, width: center - 40, height: 24)
        directBar.frame = NSRect(x: 20, y: status.frame.maxY, width: center - 40, height: bottom - 32)
        attachmentScroll.frame = NSRect(x: 20, y: height - 110 - trayHeight, width: center - 40, height: trayHeight)
        attachmentTray?.frame = NSRect(x: 0, y: 0, width: center - 40, height: attachmentHeight)
        attach.frame = NSRect(x: 16, y: height - 72, width: 38, height: 42)
        send.frame = NSRect(x: center - 56, y: height - 70, width: 36, height: 38)
        emoji.frame = NSRect(x: send.frame.minX - 40, y: height - 72, width: 32, height: 42)
        gif.frame = NSRect(x: emoji.frame.minX - 40, y: height - 72, width: 32, height: 42)
        let editorRight = (gif.isHidden ? emoji.frame.minX : gif.frame.minX) - 12
        composerScroll.frame = NSRect(x: 58, y: height - 102, width: editorRight - 58, height: 70)
        composer.frame.size.width = composerScroll.contentSize.width
        composer.minSize = NSSize(width: 0, height: 70)
        counter.frame = NSRect(x: 20, y: height - 24, width: center - 40, height: 16)
        join.frame = NSRect(x: 20, y: height - 80, width: 140, height: 38)
        layoutSuggestions()
        retry.frame = NSRect(x: center - 84, y: height - 108, width: 64, height: 30)
        editRejected.frame = NSRect(x: 20, y: height - 168, width: 200, height: 30)
    }

    private func updateComposer() {
        let chat = model.chat
        directBar.update()
        if composerChannel != chat.editingContext {
            composerChannel = chat.editingContext
            typing.end()
        }
        if composer.string != chat.draft, !composer.hasMarkedText() {
            typing.end()
            composer.string = chat.draft
            composer.setSelectedRange(NSRange(location: composer.string.utf16.count, length: 0))
            composer.needsDisplay = true
        }
        composer.placeholder = "Message \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        composer.isEditable = directBar.isHidden && chat.canInteract && !chat.sending && !chat.sendRejected && chat.pendingMessage == nil
        send.isEnabled = directBar.isHidden && chat.canInteract && chat.canPressSend(inThread: false)
        send.title = chat.sending ? "Sending…" : "Send"
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
        gif.isEnabled = composer.isEditable && chat.canSendGif()
        picker.closeIfUnavailable(composer: composer)
        let count = chat.draft.unicodeScalars.count
        counter.isHidden = composerScroll.isHidden || count < 3000
        counter.stringValue = "\(count.formatted()) / 4,000"
        counter.textColor = AppKitComposerCounter.color(count)
        join.isHidden = !model.previewingChannel
        join.isEnabled = joiningChannelID == nil
        join.title = joiningChannelID == model.selectedChannelID ? "Joining…" : "Join channel"
        // Keystrokes change only the composer, not timeline geometry.
        retry.isHidden = !(chat.loadFailed || chat.olderError != nil || chat.sessionError != nil || (chat.pendingMessage != nil && !chat.sending && !chat.sendRejected))
        retry.isEnabled = !chat.loadingOlder
        editRejected.isHidden = !chat.sendRejected
        let error = chat.error ?? chat.olderError ?? chat.sessionError ?? model.navigationError
        status.setAccessibilityIdentifier(error == nil ? "conversation-status" : "conversation-error")
        status.textColor = error == nil ? NativeStyle.muted : NSColor(CaperTheme.error)
        status.stringValue = error
            ?? (model.previewingChannel ? "Preview · Join #\(chat.channelName) to interact with people here" : chat.loading ? "Loading messages…" : chat.loadingOlder ? "Loading older messages…" : chat.typingNames.isEmpty ? (model.selectedDirectMessage?.status == .outgoing ? model.selectedDirectMessage.flatMap(NativePresentation.directMessageNotice) ?? "" : "") : "\(chat.typingNames.joined(separator: ", ")) typing…")
        refreshSuggestions()
    }

    private func showAttachMenu() {
        let menu = NSMenu()
        let count = model.chat.attachmentDrafts.count
        if count >= AttachmentPolicy.maxAttachments {
            menu.addItem(withTitle: "You can attach up to \(AttachmentPolicy.maxAttachments) files.", action: nil, keyEquivalent: "")
        }
        let item = menu.addItem(withTitle: "Upload a file", action: #selector(pickFiles), keyEquivalent: "")
        item.target = self; item.isEnabled = count < AttachmentPolicy.maxAttachments
        menu.autoenablesItems = false
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: attach.bounds.maxY), in: attach)
    }

    @objc private func pickFiles() { media.pickFiles() }

    private func messageAction(_ message: ChatMessage, action: MessageAction, anchor: NSView) {
        let chat = model.chat
        guard let current = dialogs.currentMessage(message), !current.isDeleted else { return }
        let context = dialogs.messageContext
        switch action {
        case .history: dialogs.messageHistory(current, in: self)
        case .addReaction: picker.reaction(to: current, from: anchor, composer: composer)
        case .dismissReaction: chat.reactionErrors[current.id] = nil
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
        let chat = model.chat, context = dialogs.messageContext
        guard let current = dialogs.currentMessage(message) else { return nil }
        // A menu can outlive navigation or an incoming edit. Resolve again
        // on activation instead of applying the snapshot that opened it.
        func perform(_ action: @escaping (ChatMessage) -> Void) -> () -> Void {
            { [weak self] in
                guard let self, self.dialogs.messageContext == context,
                      let latest = self.dialogs.currentMessage(current) else { return }
                action(latest)
            }
        }
        func copy(_ text: String) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
        }
        let copyID = NavigationMenuItem("Copy message ID", action: perform { copy($0.id) })
        if current.isDeleted { return NavigationMenus.menu([copyID]) }
        var items: [NSMenuItem] = [NavigationMenuItem("Add reaction…", enabled: chat.canInteract, action: perform { [weak self, weak anchor] in
            guard let self, let anchor else { return }; self.messageAction($0, action: .addReaction, anchor: anchor)
        }), .separator()]
        if dialogs.canEdit(current) {
            items.append(NavigationMenuItem("Edit message", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.editMessage($0, in: self)
            }))
        }
        if current.forward == nil, (current.revision ?? 1) > 1 {
            items.append(NavigationMenuItem("Message history", action: perform { [weak self] in
                guard let self else { return }; self.dialogs.messageHistory($0, in: self)
            }))
        }
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
                guard let self else { return }; self.dialogs.deleteMessage($0, in: self)
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
        model.chat.draft = composer.string
        model.chat.draftChanged(composer.string)
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
        Task { await model.chat.send() }
    }
}
