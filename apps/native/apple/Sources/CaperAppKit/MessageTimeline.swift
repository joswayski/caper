import AppKit
import CaperCore

/// A scroll document with only realized native text views. Unlike NSTableView,
/// it never manufactures an AX row for every message in the loaded history.
@MainActor final class MessageTimeline: NSScrollView {
    private let document = NativeView(NativeStyle.conversation)
    private var geometry = NativeTimelineLayout()
    private var messages: [ChatMessage] = []
    private var rows: [String: MessageCell] = [:]
    private var rendering = false
    private var conversationID: String?
    private var followEnd = true
    private var selection: [String: (text: String, range: NSRange)] = [:]
    var react: ((String, String) -> Void)?

    override init(frame: NSRect) {
        super.init(frame: frame)
        documentView = document
        drawsBackground = true
        backgroundColor = NativeStyle.conversation
        hasVerticalScroller = true
        autohidesScrollers = true
        contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled),
            name: NSView.boundsDidChangeNotification, object: contentView)
        setAccessibilityIdentifier("message-timeline")
        setAccessibilityLabel("Messages")
        document.setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NotificationCenter.default.removeObserver(self) }

    func update(_ messages: [ChatMessage], conversationID: String?) {
        let changedConversation = self.conversationID != conversationID
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        let previous = Dictionary(uniqueKeysWithValues: self.messages.enumerated().map { ($0.element.id, $0.offset) })
        var invalid = Set<String>()
        for (index, message) in messages.enumerated() {
            guard let oldIndex = previous[message.id] else { invalid.insert(message.id); continue }
            let oldPredecessor = oldIndex > 0 ? self.messages[oldIndex - 1] : nil
            let predecessor = index > 0 ? messages[index - 1] : nil
            // Deleting or inserting a predecessor can change grouping even
            // when this message itself has not changed.
            if self.messages[oldIndex] != message || oldPredecessor != predecessor { invalid.insert(message.id) }
        }
        self.messages = messages
        self.conversationID = conversationID
        if changedConversation {
            geometry = NativeTimelineLayout()
            followEnd = true
            selection.removeAll()
            rows.values.forEach { $0.removeFromSuperview() }
            rows.removeAll()
        }
        geometry.replace(ids: messages.map(\.id), width: contentSize.width, invalidating: invalid)
        realize(anchor: changedConversation ? nil : anchor)
        let currentIDs = Set(messages.map(\.id))
        selection = selection.filter { currentIDs.contains($0.key) }
    }

    override func layout() {
        super.layout()
        guard !rendering else { return }
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        if geometry.width != contentSize.width {
            geometry.replace(ids: messages.map(\.id), width: contentSize.width)
        }
        realize(anchor: anchor)
    }

    @objc private func scrolled() {
        guard !rendering else { return }
        followEnd = contentView.bounds.maxY >= geometry.contentHeight - 2
        realize(anchor: geometry.anchor(at: contentView.bounds.minY))
    }

    private func realize(anchor: NativeTimelineLayout.Anchor?) {
        guard !rendering, contentSize.width > 0, contentSize.height > 0 else { return }
        rendering = true
        defer { rendering = false }
        let viewport = contentSize.height
        func offset() -> CGFloat {
            let wanted = followEnd ? geometry.contentHeight - viewport : anchor.flatMap { geometry.offset(for: $0) } ?? contentView.bounds.minY
            return min(max(0, wanted), max(0, geometry.contentHeight - viewport))
        }
        // Measuring changes the estimated visible interval. Converge by measuring
        // only newly exposed rows, never the rest of the history at startup.
        var needed = geometry.visibleRange(offset: offset(), height: viewport, overscan: 100)
        while needed.contains(where: { !geometry.isMeasured($0) }) {
            for index in needed where !geometry.isMeasured(index) {
                let row = cell(at: index)
                geometry.measure(index, height: row.measuredHeight(width: contentSize.width))
            }
            needed = geometry.visibleRange(offset: offset(), height: viewport, overscan: 100)
        }
        let retained = Set(needed.map { messages[$0].id })
        for id in Array(rows.keys) where !retained.contains(id) {
            guard let row = rows.removeValue(forKey: id) else { continue }
            if row.body.selectedRange().length > 0 { selection[id] = (row.body.string, row.body.selectedRange()) }
            row.removeFromSuperview()
        }
        document.setFrameSize(NSSize(width: contentSize.width, height: max(viewport, geometry.contentHeight)))
        for index in needed {
            let row = cell(at: index)
            row.frame = NSRect(x: 0, y: geometry.origin(of: index), width: contentSize.width, height: geometry.height(at: index))
        }
        contentView.scroll(to: NSPoint(x: 0, y: offset()))
        reflectScrolledClipView(contentView)
        // These are real, selectable NSTextViews/buttons, not label-only mirrors.
        document.setAccessibilityChildren(needed.compactMap { rows[messages[$0].id] })
    }

    private func cell(at index: Int) -> MessageCell {
        let message = messages[index]
        let row: MessageCell
        if let existing = rows[message.id] { row = existing }
        else {
            row = MessageCell()
            rows[message.id] = row
            document.addSubview(row)
        }
        row.configure(message, grouped: NativePresentation.grouped(message, after: index > 0 ? messages[index - 1] : nil), react: react)
        if let saved = selection.removeValue(forKey: message.id), saved.text == row.body.string,
           NSMaxRange(saved.range) <= row.body.string.utf16.count {
            row.body.setSelectedRange(saved.range)
        }
        return row
    }
}

@MainActor private final class MessageCell: NativeView {
    let body = NSTextView()
    private let author = NativeStyle.label("", bold: true)
    private let timestamp = NativeStyle.label("", size: 11)
    private let avatar = NSImageView()
    private let detail = NativeStyle.label("", size: 12)
    private var reactions: [NativeButton] = []
    private var message: ChatMessage?
    private var grouped = false

    init() {
        super.init()
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        body.isEditable = false; body.isSelectable = true
        body.isRichText = true; body.drawsBackground = false
        body.textContainerInset = .zero
        body.textContainer?.lineFragmentPadding = 0
        body.isHorizontallyResizable = false
        body.isVerticallyResizable = false
        body.linkTextAttributes = [.foregroundColor: NSColor(CaperTheme.link)]
        timestamp.textColor = NativeStyle.muted
        detail.textColor = NativeStyle.muted
        avatar.wantsLayer = true; avatar.layer?.cornerRadius = 10; avatar.layer?.masksToBounds = true
        avatar.setAccessibilityElement(false)
        [body, author, timestamp, avatar, detail].forEach { addSubview($0) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ message: ChatMessage, grouped: Bool, react: ((String, String) -> Void)?) {
        guard self.message != message || self.grouped != grouped else { return }
        let range = body.selectedRange()
        self.message = message; self.grouped = grouped
        author.stringValue = message.author.name
        timestamp.stringValue = NativePresentation.time(message.createdAt)
        body.textStorage?.setAttributedString(NativePresentation.messageText(message.isDeleted ? "Message deleted" : message.content.text))
        body.setAccessibilityIdentifier("message-\(message.id)")
        if NSMaxRange(range) <= body.string.utf16.count { body.setSelectedRange(range) }
        if let id = CaperAvatar.index(for: message.author.avatarId) {
            avatar.image = Bundle(for: AppModel.self).image(forResource: "caper-avatar-\(id)")
        } else { avatar.image = nil }
        let attachments = message.content.attachments ?? []
        detail.stringValue = attachments.isEmpty ? "" : "\(attachments.count) attachment(s) — media UI not yet ported"
        reactions.forEach { $0.removeFromSuperview() }
        reactions = (message.reactions ?? []).map { reaction in
            NativeButton("\(reaction.emoji) \(reaction.authorIds.count)") { react?(message.id, reaction.emoji) }
        }
        reactions.forEach { addSubview($0) }
        needsLayout = true
    }

    func measuredHeight(width: CGFloat) -> CGFloat {
        let textWidth = max(40, width - 88)
        body.textContainer?.containerSize = NSSize(width: textWidth, height: .greatestFiniteMagnitude)
        let text = body.attributedString()
        let coreTextHeight = NativePresentation.textHeight(text, width: textWidth)
        body.layoutManager?.ensureLayout(for: body.textContainer!)
        let nativeHeight = body.layoutManager?.usedRect(for: body.textContainer!).height ?? 0
        return (grouped ? 4 : 32) + max(20, max(coreTextHeight, ceil(nativeHeight)))
            + (detail.stringValue.isEmpty ? 0 : 24) + (reactions.isEmpty ? 0 : 32) + 12
    }

    override func layout() {
        super.layout()
        avatar.isHidden = grouped; author.isHidden = grouped; timestamp.isHidden = grouped
        avatar.frame = NSRect(x: 20, y: 12, width: 36, height: 36)
        author.frame = NSRect(x: 68, y: 10, width: min(200, bounds.width - 200), height: 20)
        timestamp.frame = NSRect(x: 278, y: 12, width: 100, height: 18)
        let footer = (detail.stringValue.isEmpty ? 0.0 : 24) + (reactions.isEmpty ? 0.0 : 32)
        let top = grouped ? 4.0 : 32
        body.frame = NSRect(x: 68, y: top, width: max(40, bounds.width - 88), height: max(20, bounds.height - top - footer - 12))
        detail.frame = NSRect(x: 68, y: body.frame.maxY, width: max(40, bounds.width - 88), height: 24)
        var x: CGFloat = 68
        for reaction in reactions {
            let width = max(54, reaction.intrinsicContentSize.width + 12)
            reaction.frame = NSRect(x: x, y: bounds.height - 40, width: width, height: 28)
            x += width + 6
        }
    }
}
