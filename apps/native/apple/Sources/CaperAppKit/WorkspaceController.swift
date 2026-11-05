import AppKit
import CaperCore

@MainActor final class WorkspaceController: NSViewController, NSTextViewDelegate {
    private let model: AppModel
    private let navigationObservation = NativeObservation()
    private let timelineObservation = NativeObservation()
    private let composerObservation = NativeObservation()
    private let membersObservation = NativeObservation()
    private let rail = NativeView(NativeStyle.blackout)
    private let sidebar = NativeView(NativeStyle.sidebar)
    private let members = NativeView(NativeStyle.sidebar)
    private let stage = NativeView(NativeStyle.conversation)
    private let timeline = MessageTimeline(frame: .zero)
    private let header = NativeStyle.label("", size: 16, bold: true)
    private let status = NativeStyle.label("", size: 12)
    private let account = NativeStyle.label("")
    private let composer = ComposerTextView()
    private let composerScroll = NSScrollView()
    private let channels = NativeView()
    private let channelScroll = NSScrollView()
    private let memberList = NativeView()
    private let memberScroll = NSScrollView()
    private lazy var send = NativeButton("Send", primary: true) { [weak self] in self?.submit() }
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
    private lazy var logout = NativeButton("Sign out") { [weak self] in
        guard let self else { return }; Task { await self.model.logout() }
    }
    private lazy var mute = NativeButton("Mute") { [weak self] in
        guard let self else { return }; Task { await self.model.voice.setMuted(!self.model.voice.muted) }
    }
    private lazy var deafen = NativeButton("Deafen") { [weak self] in
        guard let self else { return }; Task { await self.model.voice.setDeafened(!self.model.voice.deafened) }
    }
    private var spaceButtons: [NativeButton] = []

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() { view = NativeView(NativeStyle.blackout) }
    override func viewDidLoad() {
        super.viewDidLoad()
        [rail, sidebar, stage, members].forEach { view.addSubview($0) }
        [timeline, header, status, retry, editRejected, composerScroll, send].forEach { stage.addSubview($0) }
        [account, mute, deafen, logout, channelScroll].forEach { sidebar.addSubview($0) }
        setupList(channels, in: channelScroll)
        setupList(memberList, in: memberScroll)
        members.addSubview(memberScroll)
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
        navigationObservation.start { [weak self] in self?.updateNavigation() }
        timelineObservation.start { [weak self] in
            guard let self else { return }
            let chat = self.model.chat
            self.timeline.canLoadOlder = chat.hasMore && !chat.loading && !chat.loadingOlder && chat.olderError == nil
            let visible = chat.displayedMessages.filter {
                $0.isChannelMessage && !chat.blockedAuthorIDs.contains($0.author.id)
            }
            self.timeline.update(visible, conversationID: self.model.selectedDirectMessageID ?? self.model.selectedChannelID)
        }
        composerObservation.start { [weak self] in self?.updateComposer() }
        membersObservation.start { [weak self] in self?.updateMembers() }
    }

    private func setupList(_ list: NSView, in scroll: NSScrollView) {
        scroll.documentView = list
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.drawsBackground = false
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        let width = view.bounds.width, height = view.bounds.height
        let memberWidth: CGFloat = width >= 1100 && model.selectedDirectMessageID == nil ? 220 : 0
        rail.frame = NSRect(x: 0, y: 0, width: 64, height: height)
        sidebar.frame = NSRect(x: 64, y: 0, width: 280, height: height)
        members.frame = NSRect(x: width - memberWidth, y: 0, width: memberWidth, height: height)
        members.isHidden = memberWidth == 0
        stage.frame = NSRect(x: 344, y: 0, width: width - 344 - memberWidth, height: height)
        let center = stage.bounds.width
        header.frame = NSRect(x: 20, y: 14, width: center - 40, height: 26)
        timeline.frame = NSRect(x: 0, y: 48, width: center, height: height - 48 - 138)
        status.frame = NSRect(x: 20, y: height - 132, width: center - 40, height: 24)
        composerScroll.frame = NSRect(x: 20, y: height - 102, width: center - 116, height: 70)
        composer.frame.size.width = composerScroll.contentSize.width
        composer.minSize = NSSize(width: 0, height: 70)
        send.frame = NSRect(x: center - 84, y: height - 70, width: 64, height: 38)
        retry.frame = NSRect(x: center - 84, y: height - 108, width: 64, height: 30)
        editRejected.frame = NSRect(x: 20, y: height - 168, width: 200, height: 30)
        channelScroll.frame = NSRect(x: 0, y: 0, width: 280, height: height - 116)
        account.frame = NSRect(x: 16, y: height - 110, width: 248, height: 24)
        mute.frame = NSRect(x: 16, y: height - 76, width: 110, height: 30)
        deafen.frame = NSRect(x: 138, y: height - 76, width: 126, height: 30)
        logout.frame = NSRect(x: 16, y: height - 38, width: 248, height: 26)
        memberScroll.frame = members.bounds
        layoutList(channels, width: channelScroll.contentSize.width)
        layoutList(memberList, width: memberScroll.contentSize.width)
        for (index, button) in spaceButtons.enumerated() {
            button.frame = NSRect(x: 10, y: 18 + CGFloat(index) * 54, width: 44, height: 44)
        }
    }

    private func updateNavigation() {
        spaceButtons.forEach { $0.removeFromSuperview() }
        spaceButtons = model.spaces.map { space in
            let button = NativeButton(String(space.name.prefix(2)).uppercased()) { [weak self] in
                guard let self else { return }; Task { await self.model.select(space: space) }
            }
            button.setAccessibilityLabel(space.name)
            button.toolTip = space.name
            button.layer?.borderWidth = space.id == model.selectedSpaceID ? 2 : 0
            button.layer?.borderColor = NativeStyle.accent.cgColor
            rail.addSubview(button)
            return button
        }
        clear(channels)
        add(NativeStyle.label(model.detail?.space.name ?? "Browse", size: 17, bold: true), to: channels)
        for channel in model.detail?.channels ?? [] {
            let button = NativeButton("\(channel.private ? "♧" : "#")  \(channel.name)") { [weak self] in
                guard let self else { return }; Task { await self.model.select(channel: channel) }
            }
            button.setAccessibilityIdentifier("channel-\(channel.id)")
            button.alignment = .left
            button.layer?.backgroundColor = channel.id == model.selectedChannelID ? NativeStyle.composer.cgColor : NSColor.clear.cgColor
            add(button, to: channels)
        }
        add(NativeStyle.label("Direct messages", size: 12, bold: true), to: channels)
        for conversation in model.visibleDirectMessages {
            add(NativeButton(conversation.peer.displayName) { [weak self] in
                guard let self else { return }; Task { await self.model.select(directMessage: conversation) }
            }, to: channels)
        }
        if model.spacesLoaded, model.spaces.isEmpty {
            add(NativeButton("Create a space") { [weak self] in self?.createSpace() }, to: channels)
        }
        account.stringValue = model.account?.displayName ?? ""
        header.stringValue = model.selectedDirectMessage?.peer.displayName ?? "# \(model.chat.channelName)"
        view.needsLayout = true
    }

    private func updateMembers() {
        clear(memberList)
        let roster = model.presence.visibleMembers
        add(NativeStyle.label("MEMBERS — \(roster.count)", size: 12, bold: true), to: memberList)
        for snapshot in roster {
            let member = model.profiled(snapshot)
            let indicator = model.presence.status(for: member) == .online ? "●" : "○"
            add(NativeStyle.label("\(indicator)  \(member.displayName)"), to: memberList)
        }
        layoutList(memberList, width: memberScroll.contentSize.width)
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
        // Keystrokes change only the composer, not timeline geometry.
        retry.isHidden = !(chat.loadFailed || chat.olderError != nil || chat.sessionError != nil || (chat.pendingMessage != nil && !chat.sending && !chat.sendRejected))
        retry.isEnabled = !chat.loadingOlder
        editRejected.isHidden = !chat.sendRejected
        status.stringValue = chat.error ?? chat.olderError ?? chat.sessionError ?? model.navigationError
            ?? (chat.loading ? "Loading messages…" : chat.loadingOlder ? "Loading older messages…" : chat.typingNames.isEmpty ? "" : "\(chat.typingNames.joined(separator: ", ")) typing…")
        mute.title = model.voice.muted ? "Unmute" : "Mute"
        deafen.title = model.voice.deafened ? "Undeafen" : "Deafen"
        mute.setAccessibilityLabel(mute.title); deafen.setAccessibilityLabel(deafen.title)
    }

    func textDidChange(_ notification: Notification) {
        composer.needsDisplay = true
        model.chat.draft = composer.string
        model.chat.draftChanged(composer.string)
    }
    private func submit() {
        guard send.isEnabled, !composer.hasMarkedText() else { return }
        Task { await model.chat.send() }
    }
    private func createSpace() {
        let alert = NSAlert()
        alert.messageText = "Name your space"
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 300, height: 28))
        field.setAccessibilityLabel("Space name")
        alert.accessoryView = field
        alert.addButton(withTitle: "Create"); alert.addButton(withTitle: "Cancel")
        guard let window = view.window else { return }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self else { return }
            Task {
                do { try await self.model.createSpace(name: field.stringValue) }
                catch { self.model.error = error.localizedDescription }
            }
        }
    }
    private func clear(_ list: NSView) {
        list.subviews.forEach { $0.removeFromSuperview() }
    }
    private func add(_ child: NSView, to list: NSView) {
        list.addSubview(child)
    }
    private func layoutList(_ list: NSView, width: CGFloat) {
        var y: CGFloat = 12
        for child in list.subviews {
            let height: CGFloat = child is NSButton ? 34 : 28
            child.frame = NSRect(x: 12, y: y, width: max(0, width - 24), height: height)
            y += height + 6
        }
        list.setFrameSize(NSSize(width: width, height: y + 6))
    }
}

/// TextKit owns marked text, undo, caret and AX selection. Return sends only
/// outside composition; Shift-Return remains an ordinary newline.
private final class ComposerTextView: NSTextView {
    var submit: (() -> Void)?
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
        if event.keyCode == 36, !event.modifierFlags.contains(.shift), !hasMarkedText() {
            submit?()
        } else { super.keyDown(with: event) }
    }
}
