import AppKit
import CaperCore

@MainActor final class WorkspaceController: NSViewController, NSTextViewDelegate {
    private let model: AppModel
    private let navigationObservation = NativeObservation()
    private let timelineObservation = NativeObservation()
    private let composerObservation = NativeObservation()
    private lazy var navigation = NavigationSidebar(model: model)
    private lazy var dialogs = AppKitWorkspacePresenter(model: model)
    private lazy var members = MemberPanel(model: model)
    private lazy var profiles = AppKitProfilePresenter(model: model)
    private var membersPreference: Bool?
    private lazy var membersToggle = NativeButton.icon("Show members", symbol: "person.2") { [weak self] in
        guard let self else { return }
        self.membersPreference = self.members.isHidden
        self.view.needsLayout = true
    }
    private let stage = NativeView(NativeStyle.conversation)
    private let timeline = MessageTimeline(frame: .zero)
    private let header = NativeStyle.label("", size: 16, bold: true)
    private let status = NativeStyle.label("", size: 12)
    private let composer = ComposerTextView()
    private let composerScroll = NSScrollView()
    private let suggestions = ComposerSuggestions()
    private var suggestionsDismissed = false
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
        [timeline, header, membersToggle, status, retry, editRejected, composerScroll, send, join, suggestions].forEach { stage.addSubview($0) }
        join.setAccessibilityIdentifier("join-channel")
        navigation.show = { [weak self] item in
            guard let self else { return }; self.dialogs.show(item, in: self)
        }
        navigation.leaveChannel = { [weak self] channel in
            guard let self else { return }; self.dialogs.leaveChannel(channel, in: self)
        }
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
        composer.font = NativeStyle.font(14)
        composer.textColor = NativeStyle.text
        composer.insertionPointColor = NativeStyle.text
        composer.backgroundColor = NativeStyle.composer
        composer.isRichText = false
        composer.allowsUndo = true
        composer.textContainerInset = NSSize(width: 12, height: 12)
        composer.isHorizontallyResizable = false
        composer.isVerticallyResizable = true
        composer.autoresizingMask = [.width]
        composer.textContainer?.widthTracksTextView = true
        composer.setAccessibilityLabel("Message")
        composer.setAccessibilityIdentifier("message-composer")
        composer.delegate = self
        composer.submit = { [weak self] in self?.submit() }
        composer.handleKey = { [weak self] in self?.handleSuggestionKey($0) == true }
        composer.focusChanged = { [weak self] in self?.refreshSuggestions() }
        composer.setAccessibilityHelp("Return sends. Shift-Return adds a new line. Use arrows and Return or Tab to choose a suggestion; Escape dismisses.")
        suggestions.accept = { [weak self] in self?.acceptSuggestion($0) }
        composerScroll.documentView = composer
        composerScroll.hasVerticalScroller = true
        composerScroll.autohidesScrollers = true
        composerScroll.drawsBackground = true
        composerScroll.backgroundColor = NativeStyle.composer
        composerScroll.wantsLayer = true; composerScroll.layer?.cornerRadius = 8
        send.setAccessibilityIdentifier("send-message")
        retry.setAccessibilityIdentifier("conversation-retry")
        timeline.loadOlder = { [weak self] in
            guard let self else { return }
            Task { await self.model.chat.loadOlder() }
        }
        timeline.react = { [weak self] id, emoji in
            guard let self, self.model.chat.canInteract,
                  let message = self.model.chat.displayedMessages.first(where: { $0.id == id }) else { return }
            let active = !(message.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(self.model.chat.viewerID ?? "") ?? false)
            Task { await self.model.chat.setReaction(messageID: id, emoji: emoji, active: active) }
        }
        navigationObservation.start { [weak self] in
            guard let self else { return }
            self.header.stringValue = self.model.selectedDirectMessage?.peer.displayName ?? "# \(self.model.chat.channelName)"
            self.view.needsLayout = true
        }
        timelineObservation.start { [weak self] in
            guard let self else { return }
            let chat = self.model.chat
            self.timeline.canLoadOlder = chat.hasMore && !chat.loading && !chat.loadingOlder && chat.olderError == nil
            let visible = chat.displayedMessages.filter {
                $0.isChannelMessage && !chat.blockedAuthorIDs.contains($0.author.id)
            }
            self.timeline.update(visible, conversationID: self.model.selectedDirectMessageID ?? self.model.selectedChannelID,
                viewerID: chat.viewerID)
        }
        composerObservation.start { [weak self] in self?.updateComposer() }
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
        timeline.frame = NSRect(x: 0, y: 48, width: center, height: height - 48 - 138)
        status.frame = NSRect(x: 20, y: height - 132, width: center - 40, height: 24)
        composerScroll.frame = NSRect(x: 20, y: height - 102, width: center - 116, height: 70)
        composer.frame.size.width = composerScroll.contentSize.width
        composer.minSize = NSSize(width: 0, height: 70)
        send.frame = NSRect(x: center - 84, y: height - 70, width: 64, height: 38)
        join.frame = NSRect(x: 20, y: height - 80, width: 140, height: 38)
        layoutSuggestions()
        retry.frame = NSRect(x: center - 84, y: height - 108, width: 64, height: 30)
        editRejected.frame = NSRect(x: 20, y: height - 168, width: 200, height: 30)
    }

    private func updateComposer() {
        let chat = model.chat
        if composer.string != chat.draft, !composer.hasMarkedText() {
            composer.string = chat.draft
            composer.setSelectedRange(NSRange(location: composer.string.utf16.count, length: 0))
            composer.needsDisplay = true
        }
        composer.placeholder = "Message \(model.selectedDirectMessage?.peer.displayName ?? "#\(chat.channelName)")"
        composer.isEditable = chat.canInteract && !chat.sending && !chat.sendRejected && chat.pendingMessage == nil
        send.isEnabled = chat.canInteract && chat.canPressSend(inThread: false)
        send.title = chat.sending ? "Sending…" : "Send"
        composerScroll.isHidden = model.previewingChannel
        send.isHidden = model.previewingChannel
        join.isHidden = !model.previewingChannel
        join.isEnabled = joiningChannelID == nil
        join.title = joiningChannelID == model.selectedChannelID ? "Joining…" : "Join channel"
        // Keystrokes change only the composer, not timeline geometry.
        retry.isHidden = !(chat.loadFailed || chat.olderError != nil || chat.sessionError != nil || (chat.pendingMessage != nil && !chat.sending && !chat.sendRejected))
        retry.isEnabled = !chat.loadingOlder
        editRejected.isHidden = !chat.sendRejected
        status.stringValue = chat.error ?? chat.olderError ?? chat.sessionError ?? model.navigationError
            ?? (model.previewingChannel ? "Preview · Join #\(chat.channelName) to interact with people here" : chat.loading ? "Loading messages…" : chat.loadingOlder ? "Loading older messages…" : chat.typingNames.isEmpty ? "" : "\(chat.typingNames.joined(separator: ", ")) typing…")
        refreshSuggestions()
    }

    func textDidChange(_ notification: Notification) {
        composer.needsDisplay = true
        model.chat.draft = composer.string
        model.chat.draftChanged(composer.string)
        suggestionsDismissed = false
        refreshSuggestions()
    }
    func textViewDidChangeSelection(_ notification: Notification) {
        suggestionsDismissed = false
        refreshSuggestions()
    }
    private func refreshSuggestions() {
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

/// TextKit owns marked text, undo, caret and AX selection. Return sends only
/// outside composition; Shift-Return remains an ordinary newline.
private final class ComposerTextView: NSTextView {
    var submit: (() -> Void)?
    var handleKey: ((NSEvent) -> Bool)?
    var focusChanged: (() -> Void)?
    var placeholder = "" {
        didSet { if oldValue != placeholder { needsDisplay = true } }
    }
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        if string.isEmpty {
            (placeholder as NSString).draw(at: NSPoint(x: textContainerInset.width + 5, y: textContainerInset.height),
                withAttributes: [.font: font ?? NativeStyle.font(14), .foregroundColor: NativeStyle.muted])
        }
    }
    override func keyDown(with event: NSEvent) {
        if handleKey?(event) == true { return }
        if event.keyCode == 36, !event.modifierFlags.contains(.shift), !hasMarkedText() {
            submit?()
        } else { super.keyDown(with: event) }
    }
    override func becomeFirstResponder() -> Bool {
        let result = super.becomeFirstResponder()
        if result { focusChanged?() }
        return result
    }
    override func resignFirstResponder() -> Bool {
        let result = super.resignFirstResponder()
        if result { DispatchQueue.main.async { [weak self] in self?.focusChanged?() } }
        return result
    }
}
