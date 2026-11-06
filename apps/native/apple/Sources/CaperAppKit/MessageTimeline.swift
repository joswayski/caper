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
    private var viewerID: String?
    private var followEnd = true
    private var selection: [String: (text: String, range: NSRange)] = [:]
    var canLoadOlder = false
    var loadOlder: (() -> Void)?
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

    func update(_ messages: [ChatMessage], conversationID: String?, viewerID: String? = nil) {
        let changedConversation = self.conversationID != conversationID
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        let anchorTextTop = anchor.flatMap { rows[$0.id]?.textTop }
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
        self.viewerID = viewerID
        if changedConversation {
            geometry = NativeTimelineLayout()
            followEnd = true
            selection.removeAll()
            rows.values.forEach { $0.removeFromSuperview() }
            rows.removeAll()
        }
        geometry.replace(ids: messages.map(\.id), width: contentSize.width, invalidating: invalid)
        realize(anchor: changedConversation ? nil : anchor, anchorTextTop: anchorTextTop)
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
        if !followEnd { requestOlderIfNeeded() }
    }

    override func scrollWheel(with event: NSEvent) {
        super.scrollWheel(with: event)
        // A short page may not move the clip view at all. An upward gesture
        // must still reach history, without fetching every page on first paint.
        if event.scrollingDeltaY > 0 {
            requestOlderIfNeeded()
        }
    }

    private func requestOlderIfNeeded() {
        guard canLoadOlder, !messages.isEmpty, contentView.bounds.minY < 128, let loadOlder else { return }
        // Disarm synchronously; the model observation rearms after the request.
        canLoadOlder = false
        followEnd = false
        loadOlder()
    }

    private func realize(anchor: NativeTimelineLayout.Anchor?, anchorTextTop: CGFloat? = nil) {
        guard !rendering, contentSize.width > 0, contentSize.height > 0 else { return }
        rendering = true
        defer { rendering = false }
        let viewport = contentSize.height
        func offset() -> CGFloat {
            // Prepending can group the anchor under its new predecessor and
            // hide its header. Anchor the text, not just the row's outer box.
            let textShift = anchor.flatMap { rows[$0.id]?.textTop }.flatMap { top in anchorTextTop.map { top - $0 } } ?? 0
            let wanted = followEnd ? geometry.contentHeight - viewport : (anchor.flatMap { geometry.offset(for: $0) } ?? contentView.bounds.minY) + textShift
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
            // NSTextView resizing can scroll its selection into view. Finish
            // that layout before restoring the reader's anchor, not after it.
            row.layoutSubtreeIfNeeded()
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
        let previous = index > 0 ? messages[index - 1] : nil
        // The divider belongs to this realized row, so prepending can remove
        // it using the same text anchor as an author-header grouping change.
        let day = previous.map { NativePresentation.sameDay($0.createdAt, message.createdAt) } == true
            ? nil : NativePresentation.dayLabel(message.createdAt)
        row.configure(message, grouped: NativePresentation.grouped(message, after: previous), day: day,
            viewerID: viewerID, react: react)
        if let saved = selection.removeValue(forKey: message.id), saved.text == row.body.string,
           NSMaxRange(saved.range) <= row.body.string.utf16.count {
            row.body.setSelectedRange(saved.range)
        }
        return row
    }
}

@MainActor private final class MessageCell: NativeView {
    let body = NSTextView()
    private let author = NativeStyle.label("", size: 13, bold: true)
    private let timestamp = NativeStyle.label("", size: 10)
    private let avatar = NSImageView()
    private let detail = NativeStyle.label("", size: 12)
    private let day = NativeStyle.label("", size: 10, bold: true)
    private let dayLeading = CALayer()
    private let dayTrailing = CALayer()
    private let mentionTint = CALayer()
    private let mentionBar = CALayer()
    private var reactions: [NativeButton] = []
    private var message: ChatMessage?
    private var grouped = false
    private var viewerID: String?
    private var dayHeight: CGFloat { day.isHidden ? 0 : ceil(day.cell?.cellSize.height ?? 14) + 16 }
    var textTop: CGFloat { dayHeight + (grouped ? 4 : 32) }

    init() {
        super.init()
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        day.textColor = NativeStyle.muted
        day.isHidden = true
        day.alignment = .center
        dayLeading.backgroundColor = NativeStyle.border.cgColor
        dayTrailing.backgroundColor = NativeStyle.border.cgColor
        mentionTint.backgroundColor = NativeStyle.accent.withAlphaComponent(0.08).cgColor
        mentionBar.backgroundColor = NativeStyle.accent.cgColor
        [mentionTint, mentionBar, dayLeading, dayTrailing].forEach { layer?.addSublayer($0) }
        body.isEditable = false; body.isSelectable = true
        body.isRichText = true; body.drawsBackground = false
        body.textContainerInset = .zero
        body.textContainer?.lineFragmentPadding = 0
        body.isHorizontallyResizable = false
        body.isVerticallyResizable = false
        body.layoutManager?.usesFontLeading = false
        body.linkTextAttributes = [.foregroundColor: NSColor(CaperTheme.link)]
        timestamp.textColor = NativeStyle.muted
        detail.textColor = NativeStyle.muted
        avatar.wantsLayer = true; avatar.layer?.cornerRadius = 10; avatar.layer?.masksToBounds = true
        avatar.setAccessibilityElement(false)
        [body, author, timestamp, avatar, detail, day].forEach { addSubview($0) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ message: ChatMessage, grouped: Bool, day: String?, viewerID: String?, react: ((String, String) -> Void)?) {
        guard self.message != message || self.grouped != grouped || self.day.stringValue != (day ?? "") || self.viewerID != viewerID else { return }
        let range = body.selectedRange()
        self.message = message; self.grouped = grouped; self.viewerID = viewerID
        self.day.stringValue = day ?? ""
        self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(message.id)")
        let mentionsViewer = !message.isDeleted && NativePresentation.mentionsViewer(message, viewerID: viewerID)
        setAccessibilityHelp(mentionsViewer ? "Mentions you" : nil)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        mentionTint.isHidden = !mentionsViewer
        mentionBar.isHidden = !mentionsViewer || message.pin != nil
        CATransaction.commit()
        author.stringValue = message.author.name
        timestamp.stringValue = NativePresentation.time(message.createdAt)
        let text = NSMutableAttributedString(attributedString: NativePresentation.messageText(
            message.isDeleted ? MessageDeletion.placeholder : message.content.text,
            mentions: message.isDeleted ? [] : message.content.mentions))
        if message.isDeleted {
            text.addAttribute(.foregroundColor, value: NativeStyle.muted, range: NSRange(location: 0, length: text.length))
        }
        // Emoji fallback fonts have larger metrics than Satoshi. Use the same
        // 20-point pitch for measurement and TextKit drawing, not per-glyph height.
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = 20; paragraph.maximumLineHeight = 20
        text.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: text.length))
        body.textStorage?.setAttributedString(text)
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
        return textTop + max(20, max(coreTextHeight, ceil(nativeHeight)))
            + (detail.stringValue.isEmpty ? 0 : 24) + (reactions.isEmpty ? 0 : 32) + 12
    }

    override func layout() {
        super.layout()
        let inset = dayHeight
        let dayWidth = min(ceil(day.cell?.cellSize.width ?? 0), max(0, bounds.width - 36))
        day.frame = NSRect(x: (bounds.width - dayWidth) / 2, y: 8, width: dayWidth, height: max(0, inset - 16))
        CATransaction.begin(); CATransaction.setDisableActions(true)
        dayLeading.isHidden = day.isHidden; dayTrailing.isHidden = day.isHidden
        dayLeading.frame = NSRect(x: 18, y: floor(inset / 2), width: max(0, day.frame.minX - 28), height: 1)
        dayTrailing.frame = NSRect(x: day.frame.maxX + 10, y: floor(inset / 2), width: max(0, bounds.width - day.frame.maxX - 28), height: 1)
        // Date dividers remain neutral even when the following message mentions you.
        mentionTint.frame = NSRect(x: 0, y: inset, width: bounds.width, height: max(0, bounds.height - inset))
        mentionBar.frame = NSRect(x: 0, y: inset, width: 3, height: max(0, bounds.height - inset))
        CATransaction.commit()
        avatar.isHidden = grouped; author.isHidden = grouped; timestamp.isHidden = grouped
        avatar.frame = NSRect(x: 20, y: inset + 12, width: 36, height: 36)
        // Intrinsic size describes the alignment rect, excluding NSTextField's
        // cell insets. Manual frames need cellSize or even short names truncate.
        let timeWidth = ceil(timestamp.cell?.cellSize.width ?? timestamp.intrinsicContentSize.width)
        let authorWidth = min(ceil(author.cell?.cellSize.width ?? author.intrinsicContentSize.width), max(0, bounds.width - 88 - timeWidth - 7))
        author.frame = NSRect(x: 68, y: inset + 10, width: authorWidth, height: 20)
        timestamp.frame = NSRect(x: author.frame.maxX + 7, y: inset + 13, width: timeWidth, height: 17)
        let footer = (detail.stringValue.isEmpty ? 0.0 : 24) + (reactions.isEmpty ? 0.0 : 32)
        let top = textTop
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
