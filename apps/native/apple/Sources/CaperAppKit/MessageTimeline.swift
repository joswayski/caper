import AppKit
import CaperCore

/// A scroll document with only realized native text views. Unlike NSTableView,
/// it never manufactures an AX row for every message in the loaded history.
@MainActor final class MessageTimeline: NSScrollView {
    private let document = NativeView(NativeStyle.conversation)
    private var geometry = NativeTimelineLayout()
    private var messages: [ChatMessage] = []
    private var entries: [TimelineEntry] = []
    private var pending: PendingMessage?
    private var rowIDs: [String] { entries.map(key) + (pending.map { ["pending-\($0.id)"] } ?? []) }
    private var blocked: Set<String> = []
    private var revealed: Set<String> = []
    private var rows: [String: MessageCell] = [:]
    private var rendering = false
    private var conversationID: String?
    private var viewerID: String?
    private var followEnd = true
    private var selection: [String: (text: String, range: NSRange)] = [:]
    var canLoadOlder = false
    var loadOlder: (() -> Void)?
    var react: ((String, String) -> Void)?
    var openAuthor: ((ChatAuthor, NSView) -> Void)?
    var openMention: ((MessageMention, NSView, NSRect) -> Void)?
    var makeAttachments: ((ChatMessage) -> AppKitAttachmentSurface?)?
    var makePending: ((PendingMessage) -> AppKitAttachmentSurface?)?

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

    func update(_ messages: [ChatMessage], conversationID: String?, viewerID: String? = nil, blocked: Set<String> = [], pending: PendingMessage? = nil) {
        let changedConversation = self.conversationID != conversationID
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        let anchorTextTop = anchor.flatMap { rows[$0.id]?.textTop }
        if changedConversation { revealed = [] }
        let next = BlockedMessages.entries(messages, blocked: blocked, viewerID: viewerID, revealed: revealed).flatMap { entry -> [TimelineEntry] in
            if case let .blocked(run) = entry, run.revealed { return [entry] + run.messages.map(TimelineEntry.message) }
            return [entry]
        }
        let previous = Dictionary(uniqueKeysWithValues: entries.enumerated().map { (key($0.element), $0.offset) })
        var invalid = Set<String>()
        for (index, entry) in next.enumerated() {
            let id = key(entry)
            guard let oldIndex = previous[id] else { invalid.insert(id); continue }
            let oldPredecessor = oldIndex > 0 ? entries[oldIndex - 1] : nil
            let predecessor = index > 0 ? next[index - 1] : nil
            // Deleting or inserting a predecessor can change grouping even
            // when this message itself has not changed.
            if entries[oldIndex] != entry || oldPredecessor != predecessor { invalid.insert(id) }
        }
        if self.pending != pending, let pending { invalid.insert("pending-\(pending.id)") }
        self.pending = pending
        self.messages = messages
        entries = next
        self.blocked = blocked
        self.conversationID = conversationID
        self.viewerID = viewerID
        if changedConversation {
            geometry = NativeTimelineLayout()
            followEnd = true
            selection.removeAll()
            rows.values.forEach { $0.removeFromSuperview() }
            rows.removeAll()
        }
        geometry.replace(ids: rowIDs, width: contentSize.width, invalidating: invalid)
        realize(anchor: changedConversation ? nil : anchor, anchorTextTop: anchorTextTop)
        let currentIDs = Set(rowIDs)
        selection = selection.filter { currentIDs.contains($0.key) }
    }

    /// A revealed run keeps its header alongside its messages; their native
    /// identities must be distinct even though SwiftUI nests them in one row.
    private func key(_ entry: TimelineEntry) -> String {
        switch entry {
        case .message(let message): message.id
        case .blocked(let run): run.id
        }
    }

    override func layout() {
        super.layout()
        guard !rendering else { return }
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        if geometry.width != contentSize.width {
            geometry.replace(ids: rowIDs, width: contentSize.width)
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
        let ids = rowIDs
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
        let retained = Set(needed.map { ids[$0] })
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
        document.setAccessibilityChildren(needed.compactMap { rows[ids[$0]] })
    }

    private func cell(at index: Int) -> MessageCell {
        let id = index < entries.count ? key(entries[index]) : "pending-\(pending!.id)"
        let row: MessageCell
        if let existing = rows[id] { row = existing }
        else {
            row = MessageCell()
            row.openAuthor = { [weak self] author, view in self?.openAuthor?(author, view) }
            row.body.openMention = { [weak self] mention, view, rect in self?.openMention?(mention, view, rect) }
            row.makeAttachments = { [weak self] in self?.makeAttachments?($0) }
            row.heightChanged = { [weak self, weak row] in
                guard let self, let row, self.rows[id] === row else { return }
                let anchor = self.geometry.anchor(at: self.contentView.bounds.minY)
                self.geometry.replace(ids: self.rowIDs, width: self.contentSize.width, invalidating: [id])
                self.realize(anchor: anchor)
            }
            rows[id] = row
            document.addSubview(row)
        }
        if index == entries.count, let pending {
            row.configure(pending, make: makePending)
            return row
        }
        let entry = entries[index]
        let previous = index > 0 ? entries[index - 1] : nil
        // The divider belongs to this realized row, so prepending can remove
        // it using the same text anchor as an author-header grouping change.
        let day = previous.map { NativePresentation.sameDay($0.lastCreatedAt, entry.firstCreatedAt) } == true
            ? nil : NativePresentation.dayLabel(entry.firstCreatedAt)
        switch entry {
        case .message(let message):
            let predecessor: ChatMessage? = if case .message(let prior) = previous { prior } else { nil }
            let revealedBlocked = BlockedMessages.isHidden(message, blocked: blocked, viewerID: viewerID)
            row.configure(message, grouped: NativePresentation.grouped(message, after: predecessor), day: revealedBlocked ? nil : day,
                viewerID: viewerID, revealedBlocked: revealedBlocked, react: react)
        case .blocked(let run):
            row.configure(run, day: day) { [weak self] in
                guard let self else { return }
                self.revealed = BlockedMessages.toggling(run, in: self.revealed)
                self.followEnd = false
                self.update(self.messages, conversationID: self.conversationID, viewerID: self.viewerID, blocked: self.blocked, pending: self.pending)
            }
        }
        if let saved = selection.removeValue(forKey: id), saved.text == row.body.string,
           NSMaxRange(saved.range) <= row.body.string.utf16.count {
            row.body.setSelectedRange(saved.range)
        }
        return row
    }
}

@MainActor private final class MessageCell: NativeView {
    let body = MessageTextView()
    private let author = NativeStyle.label("", size: 13, bold: true)
    private let timestamp = NativeStyle.label("", size: 10)
    private let avatar = NSImageView()
    var openAuthor: ((ChatAuthor, NSView) -> Void)?
    private lazy var authorAction: NativeButton = NativeButton("") { [weak self] in
        guard let self, let message = self.message, !message.author.isGuest else { return }
        self.openAuthor?(message.author, self.authorAction)
    }
    private lazy var avatarAction: NativeButton = NativeButton("") { [weak self] in
        guard let self, let message = self.message, !message.author.isGuest else { return }
        self.openAuthor?(message.author, self.avatarAction)
    }
    private var attachments: AppKitAttachmentSurface?
    var makeAttachments: ((ChatMessage) -> AppKitAttachmentSurface?)?
    var heightChanged: (() -> Void)?
    private var attachmentHeight: CGFloat = 0
    private let day = NativeStyle.label("", size: 10, bold: true)
    private let dayLeading = CALayer()
    private let dayTrailing = CALayer()
    private let mentionTint = CALayer()
    private let mentionBar = CALayer()
    private let blockedBar = CALayer()
    private var blockedRun: BlockedRun?
    private var revealedBlocked = false
    private var toggleBlocked: (() -> Void)?
    private lazy var blockedToggle = NativeButton("") { [weak self] in self?.toggleBlocked?() }
    private var reactions: [NativeButton] = []
    private var message: ChatMessage?
    private var pending: PendingMessage?
    private var grouped = false
    private var viewerID: String?
    private var dayHeight: CGFloat { day.isHidden ? 0 : ceil(day.cell?.cellSize.height ?? 14) + 16 }
    var textTop: CGFloat { pending == nil ? dayHeight + (grouped ? 4 : 32) : 0 }

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
        blockedBar.backgroundColor = NativeStyle.border.cgColor
        blockedBar.isHidden = true
        blockedToggle.alignment = .left; blockedToggle.font = NativeStyle.font(12, bold: true)
        blockedToggle.contentTintColor = NativeStyle.accent
        blockedToggle.layer?.backgroundColor = NSColor.clear.cgColor
        blockedToggle.setAccessibilityIdentifier("blocked-run-toggle")
        blockedToggle.isHidden = true
        [blockedBar, mentionTint, mentionBar, dayLeading, dayTrailing].forEach { layer?.addSublayer($0) }
        body.isEditable = false; body.isSelectable = true
        body.isRichText = true; body.drawsBackground = false
        body.textContainerInset = .zero
        body.textContainer?.lineFragmentPadding = 0
        body.isHorizontallyResizable = false
        body.isVerticallyResizable = false
        body.layoutManager?.usesFontLeading = false
        body.linkTextAttributes = [.foregroundColor: NSColor(CaperTheme.link)]
        timestamp.textColor = NativeStyle.muted
        avatar.wantsLayer = true; avatar.layer?.cornerRadius = 10; avatar.layer?.masksToBounds = true
        avatar.setAccessibilityElement(false)
        authorAction.layer?.backgroundColor = NSColor.clear.cgColor
        avatarAction.layer?.backgroundColor = NSColor.clear.cgColor
        [body, author, timestamp, avatar, day, authorAction, avatarAction, blockedToggle].forEach { addSubview($0) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ pending: PendingMessage, make: ((PendingMessage) -> AppKitAttachmentSurface?)?) {
        guard self.pending != pending else { return }
        self.pending = pending
        // Pending commands have a separate identity from server messages;
        // this cell is never reused as a confirmed or blocked row.
        subviews.forEach { $0.isHidden = true }
        [mentionTint, mentionBar, blockedBar, dayLeading, dayTrailing].forEach { $0.isHidden = true }
        attachments?.removeFromSuperview()
        attachments = make?(pending)
        if let attachments {
            attachments.heightChanged = { [weak self] in self?.heightChanged?() }
            addSubview(attachments)
        }
        setAccessibilityIdentifier("pending-row-\(pending.id)")
    }

    func configure(_ message: ChatMessage, grouped: Bool, day: String?, viewerID: String?, revealedBlocked: Bool, react: ((String, String) -> Void)?) {
        guard self.message != message || self.grouped != grouped || self.day.stringValue != (day ?? "") || self.viewerID != viewerID || self.revealedBlocked != revealedBlocked else { return }
        let range = body.selectedRange()
        if self.message?.content.attachments != message.content.attachments || self.message?.isDeleted != message.isDeleted {
            attachments?.removeFromSuperview()
            attachments = makeAttachments?(message)
            if let attachments {
                attachments.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(attachments)
            }
        }
        self.message = message; self.grouped = grouped; self.viewerID = viewerID
        blockedRun = nil; self.revealedBlocked = revealedBlocked
        blockedToggle.isHidden = true; body.isHidden = false
        self.day.stringValue = day ?? ""
        self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(message.id)")
        let mentionsViewer = !message.isDeleted && NativePresentation.mentionsViewer(message, viewerID: viewerID)
        setAccessibilityHelp(mentionsViewer ? "Mentions you" : nil)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = revealedBlocked ? NSColor(CaperTheme.surface).cgColor : NSColor.clear.cgColor
        blockedBar.isHidden = !revealedBlocked
        mentionTint.isHidden = !mentionsViewer
        mentionBar.isHidden = !mentionsViewer || message.pin != nil
        CATransaction.commit()
        author.stringValue = message.author.name
        author.setAccessibilityElement(message.author.isGuest)
        authorAction.setAccessibilityLabel("Open profile for \(message.author.name)")
        authorAction.setAccessibilityIdentifier("profile-author-\(message.id)")
        authorAction.isHidden = grouped || message.author.isGuest
        avatarAction.setAccessibilityLabel("Open profile for \(message.author.name)")
        avatarAction.setAccessibilityIdentifier("profile-avatar-\(message.id)")
        avatarAction.isHidden = authorAction.isHidden
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
        body.configureMentions(message.isDeleted ? [] : message.content.mentions)
        body.setAccessibilityIdentifier("message-\(message.id)")
        if NSMaxRange(range) <= body.string.utf16.count { body.setSelectedRange(range) }
        if let id = CaperAvatar.index(for: message.author.avatarId) {
            avatar.image = Bundle(for: AppModel.self).image(forResource: "caper-avatar-\(id)")
        } else { avatar.image = nil }
        reactions.forEach { $0.removeFromSuperview() }
        reactions = (message.reactions ?? []).map { reaction in
            NativeButton("\(reaction.emoji) \(reaction.authorIds.count)") { react?(message.id, reaction.emoji) }
        }
        reactions.forEach { addSubview($0) }
        needsLayout = true
    }

    func configure(_ run: BlockedRun, day: String?, toggle: @escaping () -> Void) {
        toggleBlocked = toggle
        guard blockedRun != run || self.day.stringValue != (day ?? "") else { return }
        blockedRun = run; message = nil; grouped = true
        self.day.stringValue = day ?? ""; self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(run.id)")
        setAccessibilityIdentifier(run.id)
        setAccessibilityHelp(nil)
        body.isHidden = true; body.string = ""
        authorAction.isHidden = true; avatarAction.isHidden = true
        reactions.forEach { $0.removeFromSuperview() }; reactions = []
        attachments?.removeFromSuperview(); attachments = nil; attachmentHeight = 0
        blockedToggle.isHidden = false
        blockedToggle.title = "\(run.label) — \(run.revealed ? "Hide" : "Show")"
        blockedToggle.setAccessibilityLabel("\(run.revealed ? "Hide" : "Show") \(run.label)")
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = run.revealed ? NSColor(CaperTheme.surface).cgColor : NSColor.clear.cgColor
        blockedBar.isHidden = !run.revealed
        mentionTint.isHidden = true; mentionBar.isHidden = true
        CATransaction.commit()
        needsLayout = true
    }

    func measuredHeight(width: CGFloat) -> CGFloat {
        if pending != nil { return attachments?.height(for: width) ?? 0 }
        if blockedRun != nil { return dayHeight + 48 }
        let textWidth = max(40, width - 88)
        body.textContainer?.containerSize = NSSize(width: textWidth, height: .greatestFiniteMagnitude)
        let text = body.attributedString()
        let coreTextHeight = NativePresentation.textHeight(text, width: textWidth)
        body.layoutManager?.ensureLayout(for: body.textContainer!)
        let nativeHeight = body.layoutManager?.usedRect(for: body.textContainer!).height ?? 0
        attachmentHeight = attachments?.height(for: textWidth) ?? 0
        let textHeight = body.string.isEmpty && attachments != nil ? 0 : max(20, max(coreTextHeight, ceil(nativeHeight)))
        return textTop + textHeight + attachmentHeight + (reactions.isEmpty ? 0 : 32) + 12
    }

    override func layout() {
        super.layout()
        if pending != nil { attachments?.frame = bounds; return }
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
        blockedBar.frame = NSRect(x: 0, y: inset, width: 2, height: max(0, bounds.height - inset))
        CATransaction.commit()
        avatar.isHidden = grouped; author.isHidden = grouped; timestamp.isHidden = grouped
        if blockedRun != nil {
            blockedToggle.frame = NSRect(x: 18, y: inset + 4, width: bounds.width - 36, height: 36)
            return
        }
        avatar.frame = NSRect(x: 20, y: inset + 12, width: 36, height: 36)
        // Intrinsic size describes the alignment rect, excluding NSTextField's
        // cell insets. Manual frames need cellSize or even short names truncate.
        let timeWidth = ceil(timestamp.cell?.cellSize.width ?? timestamp.intrinsicContentSize.width)
        let authorWidth = min(ceil(author.cell?.cellSize.width ?? author.intrinsicContentSize.width), max(0, bounds.width - 88 - timeWidth - 7))
        author.frame = NSRect(x: 68, y: inset + 10, width: authorWidth, height: 20)
        timestamp.frame = NSRect(x: author.frame.maxX + 7, y: inset + 13, width: timeWidth, height: 17)
        authorAction.frame = author.frame
        avatarAction.frame = avatar.frame
        let footer = attachmentHeight + (reactions.isEmpty ? 0.0 : 32)
        let top = textTop
        body.frame = NSRect(x: 68, y: top, width: max(40, bounds.width - 88), height: max(0, bounds.height - top - footer - 12))
        attachments?.frame = NSRect(x: 68, y: body.frame.maxY, width: max(40, bounds.width - 88), height: attachmentHeight)
        var x: CGFloat = 68
        for reaction in reactions {
            let width = max(54, reaction.intrinsicContentSize.width + 12)
            reaction.frame = NSRect(x: x, y: bounds.height - 40, width: width, height: 28)
            x += width + 6
        }
    }
}

/// TextKit still owns selection, including drags starting on a mention. Only a
/// single stationary click on an actual resolved-person glyph opens a card.
@MainActor private final class MessageTextView: NSTextView {
    var openMention: ((MessageMention, NSView, NSRect) -> Void)?
    private var people: [String: MessageMention] = [:]

    // Read-only message bodies are drag sources, never destinations. Let the
    // conversation receive files even when a drag also offers plain text.
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { [] }

    func configureMentions(_ mentions: [MessageMention]) {
        people.removeAll()
        var actions: [NSAccessibilityCustomAction] = []
        let text = attributedString()
        text.enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: text.length)) { value, _, _ in
            guard let id = value as? String, !id.isEmpty, self.people[id] == nil,
                  let mention = mentions.first(where: { if case let .user(personID, _) = $0 { return personID == id }; return false }),
                  case let .user(_, username) = mention else { return }
            self.people[id] = mention
            actions.append(NSAccessibilityCustomAction(name: "Open profile for @\(username)") { [weak self] in
                guard let self, let openMention = self.openMention else { return false }
                openMention(mention, self, self.bounds)
                return true
            })
        }
        setAccessibilityCustomActions(actions)
        window?.invalidateCursorRects(for: self)
    }

    override func mouseDown(with event: NSEvent) {
        let start = convert(event.locationInWindow, from: nil)
        let hit = person(at: start)
        super.mouseDown(with: event)
        guard event.clickCount == 1, selectedRange().length == 0, let hit, let window else { return }
        let end = convert(window.mouseLocationOutsideOfEventStream, from: nil)
        guard hypot(end.x - start.x, end.y - start.y) < 3 else { return }
        openMention?(hit.0, self, hit.1)
    }

    private func person(at point: NSPoint) -> (MessageMention, NSRect)? {
        guard let layoutManager, let textContainer, !string.isEmpty else { return nil }
        let origin = textContainerOrigin
        let local = NSPoint(x: point.x - origin.x, y: point.y - origin.y)
        let glyph = layoutManager.glyphIndex(for: local, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs else { return nil }
        let rect = layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer)
        guard rect.contains(local) else { return nil }
        let index = layoutManager.characterIndexForGlyph(at: glyph)
        guard let id = textStorage?.attribute(NativePresentation.mentionAttribute, at: index, effectiveRange: nil) as? String,
              let mention = people[id] else { return nil }
        return (mention, rect.offsetBy(dx: origin.x, dy: origin.y))
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        guard let layoutManager, let textContainer else { return }
        attributedString().enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: string.utf16.count)) { value, range, _ in
            guard let id = value as? String, self.people[id] != nil else { return }
            let glyphs = layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            layoutManager.enumerateEnclosingRects(forGlyphRange: glyphs, withinSelectedGlyphRange: NSRange(location: NSNotFound, length: 0), in: textContainer) { rect, _ in
                self.addCursorRect(rect.offsetBy(dx: self.textContainerOrigin.x, dy: self.textContainerOrigin.y), cursor: .pointingHand)
            }
        }
    }
}
