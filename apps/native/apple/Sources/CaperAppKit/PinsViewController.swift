import AppKit
import CaperCore

/// Pin cards reuse the timeline's TextKit selection, spoilers and message
/// controls. The existing presenter owns popover placement and focus restoration.
@MainActor final class PinsViewController: NSViewController {
    private let model: AppModel
    private let configure: (MessageCell) -> Void
    private let react: (String, String) -> Void
    private let close: () -> Void
    private let observation = NativeObservation()
    private let scroll = NSScrollView()
    private let document = NativeView()
    private let titleLabel = NativeStyle.label("Pins", size: 15, bold: true)
    private let closeButton: NativeButton
    private let rule = CALayer()
    private let error = NativeStyle.label("", size: 12)
    private let empty = NativeStyle.label("No pinned messages", size: 17, bold: true)
    private let hint = NSTextField(wrappingLabelWithString: "Pinned messages in this channel will appear here.")
    private var cards: [String: PinCard] = [:]
    private var ordered: [PinCard] = []
    private var active = false

    init(model: AppModel, configure: @escaping (MessageCell) -> Void,
         react: @escaping (String, String) -> Void, close: @escaping () -> Void) {
        self.model = model; self.configure = configure; self.react = react; self.close = close
        closeButton = NativeButton.icon("Close pins", symbol: "xmark", action: close)
        super.init(nibName: nil, bundle: nil)
        preferredContentSize = NSSize(width: 470, height: 360)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NativeView(NativeStyle.conversation)
        view.frame = NSRect(origin: .zero, size: preferredContentSize)
        view.setAccessibilityElement(true); view.setAccessibilityRole(.group); view.setAccessibilityIdentifier("pinned-messages")
        titleLabel.alignment = .center
        closeButton.setAccessibilityIdentifier("close-pins"); closeButton.layer?.backgroundColor = NSColor.clear.cgColor
        rule.backgroundColor = NativeStyle.border.cgColor; view.layer?.addSublayer(rule)
        error.textColor = NSColor(CaperTheme.error)
        error.maximumNumberOfLines = 0; error.lineBreakMode = .byWordWrapping
        empty.alignment = .center
        hint.font = NativeStyle.font(13); hint.textColor = NativeStyle.muted; hint.alignment = .center
        scroll.documentView = document; scroll.drawsBackground = false
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled),
            name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        [titleLabel, closeButton, scroll, error, empty, hint].forEach { view.addSubview($0) }
    }
    override func viewDidAppear() {
        super.viewDidAppear(); active = true
        let chat = model.chat
        let context = chat.editingContext, focus = chat.focusRevision, replies = chat.replyTargets, request = chat.composerFocusRequest
        observation.start { [weak self] in
            guard let self, self.active else { return }
            guard chat.editingContext == context, chat.focusRevision == focus, chat.replyTargets == replies,
                  chat.composerFocusRequest == request, !self.model.whatsNew.workspaceModalOpen else { self.close(); return }
            self.render()
        }
    }
    override func viewDidDisappear() { super.viewDidDisappear(); active = false }
    @objc private func scrolled() { ordered.forEach { $0.row.refreshHover() } }
    override func cancelOperation(_ sender: Any?) { close() }

    private func render() {
        let chat = model.chat, messages = model.chat.displayedPins
        let ids = Set(messages.map(\.id))
        for id in cards.keys.filter({ !ids.contains($0) }) { cards.removeValue(forKey: id)?.removeFromSuperview() }
        ordered = messages.map { message in
            let card: PinCard
            if let previous = cards[message.id] { card = previous }
            else {
                card = PinCard { [weak self] in self?.goTo(message.id) }
                configure(card.row)
                card.row.heightChanged = { [weak self] in self?.view.needsLayout = true }
                cards[message.id] = card; document.addSubview(card)
            }
            card.update(message, chat: chat, viewer: chat.viewerID ?? model.account?.id, react: react)
            return card
        }
        error.stringValue = chat.jumpError ?? ""; error.isHidden = chat.jumpError == nil
        empty.isHidden = !messages.isEmpty; hint.isHidden = !messages.isEmpty
        document.setAccessibilityChildren(ordered)
        view.needsLayout = true
    }
    override func viewDidLayout() {
        super.viewDidLayout()
        let width = max(1, view.bounds.width - 24)
        let errorHeight = error.isHidden ? 0 : ceil(error.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width, height: 1000)).height ?? 0) + 24
        titleLabel.frame = NSRect(x: 56, y: 15, width: max(1, view.bounds.width - 112), height: 24)
        closeButton.frame = NSRect(x: max(0, view.bounds.width - 48), y: 3, width: 36, height: 44)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        rule.frame = NSRect(x: 0, y: 49, width: view.bounds.width, height: 1)
        CATransaction.commit()
        error.frame = NSRect(x: 12, y: 62, width: width, height: max(0, errorHeight - 24))
        scroll.frame = NSRect(x: 0, y: 50 + errorHeight, width: view.bounds.width, height: max(0, view.bounds.height - 50 - errorHeight))
        empty.frame = NSRect(x: 12, y: max(50 + errorHeight, scroll.frame.midY - 25), width: width, height: 24)
        hint.frame = NSRect(x: 24, y: empty.frame.maxY + 8, width: max(1, width - 24), height: 56)
        let cardWidth = max(1, scroll.contentSize.width - 24)
        var y: CGFloat = 12
        for card in ordered {
            let height = card.height(for: cardWidth)
            card.frame = NSRect(x: 12, y: y, width: cardWidth, height: height)
            y += height + 12
        }
        document.setFrameSize(NSSize(width: scroll.contentSize.width, height: y))
    }
    private func goTo(_ id: String) {
        let chat = model.chat
        guard !chat.jumpingToMessage, let message = chat.displayedPins.first(where: { $0.id == id }) else { return }
        Task { [weak self] in if await chat.goToMessage(message) { self?.close() } }
    }
}

@MainActor private final class PinCard: NativeView {
    let row = MessageCell()
    private let go: NativeButton
    private let footer = CALayer()
    private var rowHeight: CGFloat = 0

    init(open: @escaping () -> Void) {
        go = NativeButton("Go to message", action: open)
        super.init()
        layer?.cornerRadius = 8; layer?.masksToBounds = true
        layer?.addSublayer(footer)
        addSubview(row); addSubview(go)
        setAccessibilityElement(true); setAccessibilityRole(.group)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ message: ChatMessage, chat: ChatModel, viewer: String?, react: @escaping (String, String) -> Void) {
        row.configure(message, grouped: false, day: nil, viewerID: viewer, revealedBlocked: false, canInteract: chat.canInteract,
            inThread: true, inPins: true, reactionError: chat.reactionErrors[message.id], pinError: chat.pinErrors[message.id],
            pinPending: chat.pendingPins.contains(message.id), react: react)
        footer.backgroundColor = (message.pin?.author.id == viewer
            ? NativeStyle.accent.withAlphaComponent(0.12) : NSColor(CaperTheme.pinGold).withAlphaComponent(0.06)).cgColor
        go.title = chat.jumpingToMessage ? "Loading message…" : "Go to message"
        go.setAccessibilityLabel(go.title); go.isEnabled = !chat.jumpingToMessage
        go.setAccessibilityIdentifier("go-to-message-\(message.id)"); setAccessibilityIdentifier("pinned-message-\(message.id)")
        needsLayout = true
    }
    func height(for width: CGFloat) -> CGFloat {
        rowHeight = row.measuredHeight(width: width)
        return rowHeight + 56
    }
    override func layout() {
        super.layout()
        row.frame = NSRect(x: 0, y: 0, width: bounds.width, height: rowHeight)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        footer.frame = NSRect(x: 0, y: rowHeight, width: bounds.width, height: bounds.height - rowHeight)
        CATransaction.commit()
        let width = min(max(1, bounds.width - 36), ceil(go.intrinsicContentSize.width) + 24)
        go.frame = NSRect(x: max(0, bounds.width - width - 18), y: rowHeight, width: width, height: 44)
    }
}
