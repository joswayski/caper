import AppKit
import CaperCore

enum MessageAction { case reaction(String), reactors(String), addReaction, retryReaction, dismissReaction, pin, retryPin, dismissPin, history, reply, replyInThread, openThread }

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
    private var canInteract = false
    private var reactionErrors: [String: String] = [:]
    private var pinErrors: [String: String] = [:]
    private var pendingPins: Set<String> = []
    private var followEnd = true
    private var selection: [String: (text: String, range: NSRange)] = [:]
    private var focusedID: String?
    private var pendingFocus: String?
    var inThread = false
    /// The first unread message of this visit: it wears the "New messages"
    /// line and starts a new author group, as on every other client. Set it
    /// before `update`.
    var newMessagesID: String?
    /// The divider the rows were last laid out with.
    private var shownNewMessagesID: String?
    var canLoadOlder = false
    /// The loaded history reaches the live end (no newer page or jump window).
    /// Set it before `update`.
    var live = true
    /// "Jump to latest": whether it shows, and other people's messages that
    /// arrived at the live end while it did.
    private(set) var jump = (shows: false, count: 0)
    var jumpChanged: (() -> Void)?
    private var jumpAnimation: Timer?
    var loadOlder: (() -> Void)?
    var visibleMessagesChanged: ((Set<String>) -> Void)?
    var react: ((String, String) -> Void)?
    var openAuthor: ((ChatAuthor, NSView) -> Void)?
    var openMention: ((MessageMention, NSView, NSRect) -> Void)?
    /// Text typed after a click in the history goes to the composer.
    var typeToCompose: ((NSEvent) -> Bool)?
    var makeAttachments: ((ChatMessage) -> AppKitAttachmentSurface?)?
    var makeReply: ((ChatMessage) -> AppKitAttachmentSurface?)?
    var makePending: ((PendingMessage) -> AppKitAttachmentSurface?)?
    var messageMenu: ((ChatMessage, NSView) -> NSMenu?)?
    var messageAction: ((ChatMessage, MessageAction, NSView) -> Void)?
    var hoverReaction: ((ChatMessage, String, Bool, NSView) -> Void)?
    var dismissHover: (() -> Void)?

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
        NotificationCenter.default.addObserver(self, selector: #selector(handleThemeChange),
            name: CaperThemeStore.didChangeNotification, object: nil)
        setAccessibilityIdentifier("message-timeline")
        setAccessibilityLabel("Messages")
        document.setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NotificationCenter.default.removeObserver(self) }

    @objc private func handleThemeChange() {
        backgroundColor = NativeStyle.conversation
        document.applyTheme()
        for row in rows.values {
            row.applyTheme()
            row.needsDisplay = true
        }
        needsDisplay = true
    }

    func update(_ messages: [ChatMessage], conversationID: String?, viewerID: String? = nil, blocked: Set<String> = [], pending: PendingMessage? = nil, canInteract: Bool = false, reactionErrors: [String: String] = [:], pinErrors: [String: String] = [:], pendingPins: Set<String> = []) {
        let changedConversation = self.conversationID != conversationID
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        let anchorTextTop = anchor.flatMap { rows[$0.id]?.textTop }
        if changedConversation { revealed = [] }
        let next = BlockedMessages.entries(messages, blocked: blocked, viewerID: viewerID, revealed: revealed).flatMap { entry -> [TimelineEntry] in
            if case let .blocked(run) = entry, run.revealed { return [entry] + run.messages.map(TimelineEntry.message) }
            return [entry]
        }
        // Other people's messages that arrive at the live end while the reader
        // is up in the history count on Jump to latest.
        let arrived = changedConversation || followEnd || !live ? 0
            : JumpToLatest.arrivals(messages, previousIDs: Set(self.messages.map(\.id)), viewerID: viewerID)
        let previous = Dictionary(uniqueKeysWithValues: entries.enumerated().map { (key($0.element), $0.offset) })
        var invalid = Set<String>()
        for (index, entry) in next.enumerated() {
            let id = key(entry)
            guard let oldIndex = previous[id] else { invalid.insert(id); continue }
            let oldPredecessor = oldIndex > 0 ? entries[oldIndex - 1] : nil
            let predecessor = index > 0 ? next[index - 1] : nil
            // Deleting or inserting a predecessor can change grouping even
            // when this message itself has not changed.
            if entries[oldIndex] != entry || oldPredecessor != predecessor
                || continues(at: oldIndex, in: entries) != continues(at: index, in: next) || self.reactionErrors[id] != reactionErrors[id]
                || self.pinErrors[id] != pinErrors[id] || self.pendingPins.contains(id) != pendingPins.contains(id)
                || self.canInteract != canInteract { invalid.insert(id) }
        }
        if shownNewMessagesID != newMessagesID {
            // The divider's row gains or loses its line and header, and the row
            // above it its hairline of padding.
            for (id, list) in [(shownNewMessagesID, entries), (newMessagesID, next)] {
                guard let id, let index = list.firstIndex(where: { key($0) == id }) else { continue }
                invalid.insert(id)
                if index > 0 { invalid.insert(key(list[index - 1])) }
            }
            shownNewMessagesID = newMessagesID
        }
        if self.pending != pending, let pending { invalid.insert("pending-\(pending.id)") }
        self.pending = pending
        self.messages = messages
        entries = next
        self.blocked = blocked
        self.conversationID = conversationID
        self.viewerID = viewerID
        self.canInteract = canInteract
        self.reactionErrors = reactionErrors
        self.pinErrors = pinErrors
        self.pendingPins = pendingPins
        if arrived > 0 { jump.count += arrived }
        if changedConversation {
            dismissHover?()
            stopJumpAnimation()
            jump.count = 0
            geometry = NativeTimelineLayout()
            followEnd = true
            focusedID = nil
            pendingFocus = nil
            selection.removeAll()
            rows.values.forEach { $0.removeFromSuperview() }
            rows.removeAll()
        }
        geometry.replace(ids: rowIDs, width: contentSize.width, invalidating: invalid)
        realize(anchor: changedConversation ? nil : anchor, anchorTextTop: anchorTextTop)
        let currentIDs = Set(rowIDs)
        selection = selection.filter { currentIDs.contains($0.key) }
    }

    /// A jump measures only its destination viewport, not the intervening
    /// history. Revealing a blocked target follows the shipping quote behavior.
    @discardableResult func focusMessage(_ id: String) -> Bool {
        guard messages.contains(where: { $0.id == id }) else { return false }
        guard contentSize.width > 0, contentSize.height > 0 else { pendingFocus = id; return true }
        pendingFocus = nil
        followEnd = false
        focusedID = id
        revealed.insert(id)
        update(messages, conversationID: conversationID, viewerID: viewerID, blocked: blocked,
            pending: pending, canInteract: canInteract, reactionErrors: reactionErrors, pinErrors: pinErrors, pendingPins: pendingPins)
        guard let index = rowIDs.firstIndex(of: id) else { return false }
        realize(anchor: geometry.anchor(at: geometry.origin(of: index)))
        let offset = max(0, geometry.origin(of: index) - max(0, (contentSize.height - geometry.height(at: index)) / 2))
        realize(anchor: geometry.anchor(at: offset))
        return true
    }

    func showLatest() {
        stopJumpAnimation()
        focusedID = nil
        pendingFocus = nil
        followEnd = true
        realize(anchor: nil)
    }

    /// Jump to latest: scrolls to the newest message (instantly with Reduce
    /// Motion or from many screens away), then follows new ones again.
    func jumpToLatest() {
        let viewport = contentSize.height
        let start = contentView.bounds.minY
        guard JumpToLatest.animates(distanceFromEnd: geometry.contentHeight - contentView.bounds.maxY, viewport: viewport,
                                    reduceMotion: CaperAccessibility.reduceMotion) else { showLatest(); return }
        stopJumpAnimation()
        focusedID = nil
        pendingFocus = nil
        jump.count = 0
        let began = CACurrentMediaTime()
        // Rows measure as they come into view, so the end is read again each frame.
        let timer = Timer(timeInterval: 1.0 / 120, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let t = min(1, (CACurrentMediaTime() - began) / 0.26)
                let eased = 1 - pow(1 - t, 3)
                let end = max(0, self.geometry.contentHeight - self.contentSize.height)
                self.contentView.scroll(to: NSPoint(x: 0, y: start + (end - start) * eased))
                self.reflectScrolledClipView(self.contentView)
                if t >= 1 { self.showLatest() }
            }
        }
        jumpAnimation = timer
        RunLoop.main.add(timer, forMode: .common)
        refreshJump()
    }

    private func stopJumpAnimation() {
        jumpAnimation?.invalidate()
        jumpAnimation = nil
    }

    /// Shows Jump to latest more than half a screen above the newest loaded
    /// message of a live history; anywhere closer hides it and clears its count.
    private func refreshJump() {
        let shows = jumpAnimation == nil && !followEnd && JumpToLatest.shows(
            distanceFromEnd: geometry.contentHeight - contentView.bounds.maxY, viewport: contentSize.height, live: live)
        let next = (shows: shows, count: shows ? jump.count : 0)
        guard next != jump else { return }
        jump = next
        jumpChanged?()
    }

    /// The row after `index` groups under it, so this row keeps only a
    /// hairline of padding below (web's `chat-message-continued`).
    private func continues(at index: Int, in list: [TimelineEntry]) -> Bool {
        guard index + 1 < list.count, case .message(let message) = list[index], case .message(let next) = list[index + 1],
              next.id != newMessagesID, !BlockedMessages.isHidden(next, blocked: blocked, viewerID: viewerID) else { return false }
        // The root is not a predecessor of its first reply.
        return NativePresentation.grouped(next, after: inThread && message.threadRootId == nil ? nil : message, inThread: inThread)
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
        if let pendingFocus { focusMessage(pendingFocus) }
        else { realize(anchor: anchor) }
    }

    @objc private func scrolled() {
        guard !rendering else { return }
        dismissHover?()
        followEnd = contentView.bounds.maxY >= geometry.contentHeight - 2
        realize(anchor: geometry.anchor(at: contentView.bounds.minY))
        if !followEnd { requestOlderIfNeeded() }
    }

    override func scrollWheel(with event: NSEvent) {
        // The reader's own scrolling takes over from a jump in progress.
        if jumpAnimation != nil, event.scrollingDeltaY != 0 { stopJumpAnimation() }
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
        // Measures and places the visible rows at one width, and returns them.
        func place() -> Range<Int> {
            let width = contentSize.width
            // Measuring changes the estimated visible interval. Converge by measuring
            // only newly exposed rows, never the rest of the history at startup.
            var needed = geometry.visibleRange(offset: offset(), height: viewport, overscan: 100)
            // A recreated cell still needs its text/container and leaf geometry,
            // even when the virtual timeline remembers its previous outer height.
            while needed.contains(where: { !geometry.isMeasured($0) || rows[ids[$0]] == nil }) {
                for index in needed where !geometry.isMeasured(index) || rows[ids[index]] == nil {
                    let row = cell(at: index)
                    geometry.measure(index, height: row.measuredHeight(width: width))
                }
                needed = geometry.visibleRange(offset: offset(), height: viewport, overscan: 100)
            }
            let retained = Set(needed.map { ids[$0] })
            for id in Array(rows.keys) where !retained.contains(id) {
                guard let row = rows.removeValue(forKey: id) else { continue }
                if row.body.selectedRange().length > 0 { selection[id] = (row.body.string, row.body.selectedRange()) }
                row.removeFromSuperview()
            }
            document.setFrameSize(NSSize(width: width, height: max(viewport, geometry.contentHeight)))
            // A conversation shorter than the pane rests on the composer, as
            // on web, instead of hanging under the header.
            let lead = max(0, viewport - geometry.contentHeight)
            for index in needed {
                let row = cell(at: index)
                row.frame = NSRect(x: 0, y: lead + geometry.origin(of: index), width: width, height: geometry.height(at: index))
                // NSTextView resizing can scroll its selection into view. Finish
                // that layout before restoring the reader's anchor, not after it.
                row.layoutSubtreeIfNeeded()
            }
            contentView.scroll(to: NSPoint(x: 0, y: offset()))
            reflectScrolledClipView(contentView)
            return needed
        }
        var needed = place()
        // A scroll bar that takes room (a mouse, or "Show scroll bars: Always")
        // narrows the rows when it appears and widens them when it hides, and
        // placing rows can cause either. Rows measured at the earlier width
        // would keep its wrapping and heights, so place them again.
        var passes = 0
        while geometry.width != max(1, contentSize.width), passes < 3 {
            geometry.replace(ids: ids, width: contentSize.width)
            needed = place()
            passes += 1
        }
        rows.values.forEach { $0.refreshHover() }
        // These are real, selectable NSTextViews/buttons, not label-only mirrors.
        document.setAccessibilityChildren(needed.compactMap { rows[ids[$0]] })
        let visible = Set(needed.filter {
            let bottom = geometry.origin(of: $0) + geometry.height(at: $0)
            return bottom > contentView.bounds.minY && bottom <= contentView.bounds.maxY + 1
        }.map { ids[$0] })
        visibleMessagesChanged?(visible)
        refreshJump()
    }

    private func cell(at index: Int) -> MessageCell {
        let id = index < entries.count ? key(entries[index]) : "pending-\(pending!.id)"
        let row: MessageCell
        if let existing = rows[id] { row = existing }
        else {
            row = MessageCell()
            row.openAuthor = { [weak self] author, view in self?.openAuthor?(author, view) }
            row.body.openMention = { [weak self] mention, view, rect in self?.openMention?(mention, view, rect) }
            row.body.typeToCompose = { [weak self] event in self?.typeToCompose?(event) ?? false }
            row.makeAttachments = { [weak self] in self?.makeAttachments?($0) }
            row.makeReply = { [weak self] in self?.makeReply?($0) }
            row.messageMenu = { [weak self] message, view in self?.messageMenu?(message, view) }
            row.messageAction = { [weak self] message, action, view in self?.messageAction?(message, action, view) }
            row.hoverReaction = { [weak self] message, emoji, entered, view in self?.hoverReaction?(message, emoji, entered, view) }
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
            // The root is not a predecessor of its first reply.
            let prior = inThread && predecessor?.threadRootId == nil ? nil : predecessor
            let newMessages = message.id == newMessagesID && !revealedBlocked
            row.configure(message, grouped: !newMessages && NativePresentation.grouped(message, after: prior, inThread: inThread),
                continued: continues(at: index, in: entries), day: revealedBlocked ? nil : day, newMessages: newMessages,
                viewerID: viewerID, revealedBlocked: revealedBlocked, canInteract: canInteract, inThread: inThread,
                reactionError: reactionErrors[message.id], pinError: pinErrors[message.id], pinPending: pendingPins.contains(message.id), react: react)
        case .blocked(let run):
            row.configure(run, day: day) { [weak self] in
                guard let self else { return }
                self.revealed = BlockedMessages.toggling(run, in: self.revealed)
                self.followEnd = false
                self.update(self.messages, conversationID: self.conversationID, viewerID: self.viewerID, blocked: self.blocked,
                    pending: self.pending, canInteract: self.canInteract, reactionErrors: self.reactionErrors,
                    pinErrors: self.pinErrors, pendingPins: self.pendingPins)
            }
        }
        row.layer?.borderColor = NativeStyle.accent.cgColor
        row.layer?.borderWidth = id == focusedID ? 1 : 0
        if let saved = selection.removeValue(forKey: id), saved.text == row.body.string,
           NSMaxRange(saved.range) <= row.body.string.utf16.count {
            row.body.setSelectedRange(saved.range)
        }
        return row
    }
}

@MainActor final class MessageCell: NativeView {
    let body = MessageTextView()
    private let author = NativeStyle.label("", size: 13, bold: true)
    private let timestamp = NativeStyle.label("", size: 10)
    private let avatar = NativeAvatar()
    var openAuthor: ((ChatAuthor, NSView) -> Void)?
    var messageMenu: ((ChatMessage, NSView) -> NSMenu?)?
    var messageAction: ((ChatMessage, MessageAction, NSView) -> Void)?
    var hoverReaction: ((ChatMessage, String, Bool, NSView) -> Void)?
    private let controls = MessageActionBar()
    private var tracking: NSTrackingArea?
    private var hovered = false
    private var canInteract = false
    private lazy var edited = NativeButton("(edited)") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .history, self)
    }
    private let reactionFailure = NativeStyle.label("", size: 11)
    private lazy var retryReaction = NativeButton("Retry") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .retryReaction, self)
    }
    private lazy var dismissReaction = NativeButton("Dismiss") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .dismissReaction, self)
    }
    private lazy var pinnedBy: NativeButton = NativeButton("") { [weak self] in
        guard let self, let message = self.message, !message.isDeleted, let pin = message.pin else { return }
        self.openAuthor?(pin.author, self.pinnedBy)
    }
    private var pinHeight: CGFloat { pinnedBy.isHidden ? 0 : 22 }
    private let pinFailure = NativeStyle.label("", size: 11)
    private lazy var retryPin = NativeButton("Retry") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .retryPin, self)
    }
    private lazy var dismissPin = NativeButton("Dismiss") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .dismissPin, self)
    }
    private var pinPending = false
    private var layoutWidth: CGFloat = 0
    private var compact: Bool { layoutWidth < 260 }
    private var textInset: CGFloat { compact ? 20 : 68 }
    private var stackedRecovery: Bool { layoutWidth - textInset - 20 < 132 }
    private var reactionFailureHeight: CGFloat { reactionFailure.isHidden ? 0 : stackedRecovery ? 94 : 66 }
    private var pinFailureHeight: CGFloat { pinFailure.isHidden ? 0 : stackedRecovery ? 94 : 66 }
    private var failureHeight: CGFloat { reactionFailureHeight + pinFailureHeight }
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
    private var quote: AppKitAttachmentSurface?
    var makeReply: ((ChatMessage) -> AppKitAttachmentSurface?)?
    private var quoteHeight: CGFloat = 0
    private lazy var thread = NativeButton("") { [weak self] in
        guard let self, let message = self.message else { return }
        self.messageAction?(message, .openThread, self)
    }
    private var threadHeight: CGFloat { thread.isHidden ? 0 : 32 }
    var heightChanged: (() -> Void)?
    private var attachmentHeight: CGFloat = 0
    private let day = NativeStyle.label("", size: 10, bold: true)
    private let dayLeading = CALayer()
    private let dayTrailing = CALayer()
    /// "New messages": a terracotta line above the first unread message,
    /// under its date divider when it has one.
    private let unread = NativeStyle.label("New messages", size: 11)
    private let unreadLeading = CALayer()
    private let unreadTrailing = CALayer()
    private let mentionTint = CALayer()
    private let mentionBar = CALayer()
    private let blockedBar = CALayer()
    private var blockedRun: BlockedRun?
    private var revealedBlocked = false
    private var toggleBlocked: (() -> Void)?
    private lazy var blockedToggle = NativeButton("") { [weak self] in self?.toggleBlocked?() }
    private var reactions: [NativeButton] = []
    private var message: ChatMessage?
    /// Spoiler groups the reader revealed in this row (not persisted).
    private var revealedSpoilers = Set<Int>()
    private var pending: PendingMessage?
    private var grouped = false
    /// The next row groups under this one.
    private var continued = false
    private var viewerID: String?
    private var dayHeight: CGFloat { day.isHidden ? 0 : ceil(day.cell?.cellSize.height ?? 14) + 16 }
    private var unreadHeight: CGFloat { unread.isHidden ? 0 : ceil(unread.cell?.cellSize.height ?? 15) + 16 }
    /// The dividers above the message: its day, then the unread line.
    private var dividerHeight: CGFloat { dayHeight + unreadHeight }
    var textTop: CGFloat {
        guard pending == nil else { return 0 }
        // A grouped row sits right under its author's previous one, as on
        // web; its hover actions float over the row's top edge.
        return dividerHeight + pinHeight + quoteHeight + (grouped ? 2 : compact ? 52 : 32)
    }
    /// Space under the row: a hairline when the next row continues the run.
    private var bottomPadding: CGFloat { continued ? 2 : 12 }

    init() {
        super.init()
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        day.textColor = NativeStyle.muted
        day.isHidden = true
        day.alignment = .center
        dayLeading.backgroundColor = NativeStyle.border.cgColor
        dayTrailing.backgroundColor = NativeStyle.border.cgColor
        unread.font = NSFont(name: "Figtree-Medium", size: 11) ?? NativeStyle.font(11)
        unread.textColor = NativeStyle.accent
        unread.isHidden = true
        unread.alignment = .center
        unread.setAccessibilityIdentifier("channel-new-messages")
        unreadLeading.backgroundColor = NativeStyle.accent.cgColor
        unreadTrailing.backgroundColor = NativeStyle.accent.cgColor
        mentionTint.backgroundColor = NativeStyle.accent.withAlphaComponent(0.08).cgColor
        mentionBar.backgroundColor = NativeStyle.accent.cgColor
        blockedBar.backgroundColor = NativeStyle.border.cgColor
        blockedBar.isHidden = true
        blockedToggle.alignment = .left; blockedToggle.font = NativeStyle.font(12, bold: true)
        blockedToggle.contentTintColor = NativeStyle.accent
        blockedToggle.layer?.backgroundColor = NSColor.clear.cgColor
        blockedToggle.setAccessibilityIdentifier("blocked-run-toggle")
        blockedToggle.isHidden = true
        [blockedBar, mentionTint, mentionBar, dayLeading, dayTrailing, unreadLeading, unreadTrailing].forEach { layer?.addSublayer($0) }
        body.isEditable = false; body.isSelectable = true
        body.isRichText = true; body.drawsBackground = false
        body.textContainerInset = .zero
        body.textContainer?.lineFragmentPadding = 0
        // Draws spoiler fills and quote bars, as in the composer.
        body.textContainer?.replaceLayoutManager(DraftLinkLayoutManager())
        body.isHorizontallyResizable = false
        body.isVerticallyResizable = false
        body.layoutManager?.usesFontLeading = false
        body.linkTextAttributes = [.foregroundColor: NativeStyle.link, .cursor: NSCursor.pointingHand]
        timestamp.textColor = NativeStyle.muted
        authorAction.layer?.backgroundColor = NSColor.clear.cgColor
        avatarAction.layer?.backgroundColor = NSColor.clear.cgColor
        authorAction.profileFeedback = true; avatarAction.profileFeedback = true
        edited.layer?.backgroundColor = NSColor.clear.cgColor
        edited.font = NativeStyle.font(10); edited.contentTintColor = NativeStyle.muted
        edited.isHidden = true
        reactionFailure.textColor = NativeStyle.error
        reactionFailure.maximumNumberOfLines = 2; reactionFailure.lineBreakMode = .byWordWrapping
        pinnedBy.isHidden = true; pinnedBy.alignment = .left
        pinnedBy.font = NativeStyle.font(10); pinnedBy.contentTintColor = NativeStyle.muted
        pinnedBy.layer?.backgroundColor = NSColor.clear.cgColor
        pinnedBy.image = NSImage(systemSymbolName: "pin.fill", accessibilityDescription: nil)
        pinnedBy.imagePosition = .imageLeading
        pinnedBy.hoverChanged = { [weak self] entered in
            guard entered, let self, let message = self.message, !message.isDeleted, let pin = message.pin else { return }
            self.openAuthor?(pin.author, self.pinnedBy)
        }
        pinFailure.textColor = NativeStyle.error
        pinFailure.maximumNumberOfLines = 2; pinFailure.lineBreakMode = .byWordWrapping
        thread.isHidden = true; thread.alignment = .left
        thread.font = NativeStyle.font(11, bold: true); thread.contentTintColor = NativeStyle.accent
        thread.layer?.backgroundColor = NSColor.clear.cgColor
        [body, author, timestamp, avatar, day, unread, authorAction, avatarAction, blockedToggle, edited, controls,
            reactionFailure, retryReaction, dismissReaction, pinnedBy, pinFailure, retryPin, dismissPin, thread].forEach { addSubview($0) }
        controls.action = { [weak self] action, anchor in
            guard let self, let message = self.message else { return }
            self.messageAction?(message, action, anchor)
        }
        controls.messageMenu = { [weak self] anchor in
            guard let self, let message = self.message else { return nil }
            return self.messageMenu?(message, anchor)
        }
        controls.focusChanged = { [weak self] in self?.updateControlsVisibility() }
        body.messageMenu = { [weak self] in
            guard let self, let message = self.message else { return nil }
            return self.messageMenu?(message, self)
        }
        body.revealSpoilers = { [weak self] in self?.revealSpoilers($0) }
        setAccessibilityCustomActions([NSAccessibilityCustomAction(name: "Message actions") { [weak self] in
            guard let self, self.message != nil else { return false }
            self.controls.showMenu(); return true
        }])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func applyTheme() {
        super.applyTheme()
        dayLeading.backgroundColor = NativeStyle.border.cgColor
        dayTrailing.backgroundColor = NativeStyle.border.cgColor
        unreadLeading.backgroundColor = NativeStyle.accent.cgColor
        unreadTrailing.backgroundColor = NativeStyle.accent.cgColor
        mentionBar.backgroundColor = NativeStyle.accent.cgColor
        blockedBar.backgroundColor = NativeStyle.border.cgColor
        body.linkTextAttributes = [.foregroundColor: NativeStyle.link, .cursor: NSCursor.pointingHand]
        if let message {
            body.textStorage?.setAttributedString(bodyText(message))
            layer?.backgroundColor = revealedBlocked ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
            for button in reactions {
                let selected = button.state == .on
                button.layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : NativeStyle.surface).cgColor
                button.layer?.borderColor = (selected ? NativeStyle.accent : NativeStyle.border).cgColor
            }
        } else if let blockedRun {
            layer?.backgroundColor = blockedRun.revealed ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
        }
        body.needsDisplay = true
    }

    func configure(_ pending: PendingMessage, make: ((PendingMessage) -> AppKitAttachmentSurface?)?) {
        guard self.pending != pending else { return }
        self.pending = pending
        // Pending commands have a separate identity from server messages;
        // this cell is never reused as a confirmed or blocked row.
        subviews.forEach { $0.isHidden = true }
        [mentionTint, mentionBar, blockedBar, dayLeading, dayTrailing, unreadLeading, unreadTrailing].forEach { $0.isHidden = true }
        attachments?.removeFromSuperview()
        attachments = make?(pending)
        if let attachments {
            attachments.heightChanged = { [weak self] in self?.heightChanged?() }
            addSubview(attachments)
        }
        setAccessibilityIdentifier("pending-row-\(pending.id)")
    }

    func configure(_ message: ChatMessage, grouped: Bool, continued: Bool = false, day: String?, newMessages: Bool = false, viewerID: String?, revealedBlocked: Bool, canInteract: Bool, inThread: Bool, inPins: Bool = false, reactionError: String?, pinError: String?, pinPending: Bool, react: ((String, String) -> Void)?) {
        guard self.message != message || self.grouped != grouped || self.continued != continued || self.day.stringValue != (day ?? "") || unread.isHidden == newMessages || self.viewerID != viewerID || self.revealedBlocked != revealedBlocked || self.canInteract != canInteract || reactionFailure.stringValue != (reactionError ?? "") || pinFailure.stringValue != (pinError ?? "") || self.pinPending != pinPending else { return }
        let range = body.selectedRange()
        // Another message, or edited text, hides its spoilers again.
        if self.message?.id != message.id || self.message?.content.text != message.content.text { revealedSpoilers = [] }
        if self.message?.replyTo != message.replyTo || self.message?.isDeleted != message.isDeleted {
            quote?.removeFromSuperview()
            quote = makeReply?(message)
            if let quote {
                quote.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(quote, positioned: .below, relativeTo: controls)
            }
        }
        if self.message?.content.attachments != message.content.attachments || self.message?.content.gif != message.content.gif
            || self.message?.forward != message.forward || self.message?.isDeleted != message.isDeleted {
            attachments?.removeFromSuperview()
            attachments = makeAttachments?(message)
            if let attachments {
                attachments.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(attachments, positioned: .below, relativeTo: controls)
            }
        }
        self.message = message; self.grouped = grouped; self.continued = continued; self.viewerID = viewerID
        self.canInteract = canInteract
        self.pinPending = pinPending
        blockedRun = nil; self.revealedBlocked = revealedBlocked
        blockedToggle.isHidden = true; body.isHidden = false
        setAccessibilityIdentifier("message-row-\(message.id)")
        controls.isHidden = false
        controls.configure(message, canInteract: canInteract, viewerID: viewerID)
        updateControlsVisibility()
        thread.isHidden = inThread || (message.threadRootId == nil && (message.thread?.replyCount ?? 0) == 0)
        let count = message.thread?.replyCount ?? 0
        thread.title = message.threadRootId != nil ? "View thread" : "\(count) \(count == 1 ? "reply" : "replies")"
        thread.setAccessibilityLabel(thread.title)
        thread.setAccessibilityIdentifier("thread-summary-\(message.id)")
        edited.isHidden = !Self.showsEdited(message)
        edited.setAccessibilityLabel("Message history")
        edited.setAccessibilityIdentifier("message-history-\(message.id)")
        reactionFailure.stringValue = reactionError ?? ""
        for item in [reactionFailure, retryReaction, dismissReaction] { item.isHidden = message.isDeleted || reactionError == nil }
        retryReaction.isEnabled = canInteract
        retryReaction.setAccessibilityIdentifier("reaction-retry-\(message.id)")
        dismissReaction.setAccessibilityIdentifier("reaction-dismiss-\(message.id)")
        pinnedBy.isHidden = message.isDeleted || message.pin == nil
        if let pin = message.pin {
            pinnedBy.title = "Pinned by \(pin.author.id == viewerID ? "you" : pin.author.name)"
            pinnedBy.setAccessibilityLabel("Open profile for \(pin.author.name)")
            pinnedBy.toolTip = pinnedBy.title
        }
        pinnedBy.setAccessibilityIdentifier("pinned-by-\(message.id)")
        pinFailure.stringValue = pinError ?? ""
        pinFailure.setAccessibilityIdentifier("pin-error-\(message.id)")
        for item in [pinFailure, retryPin, dismissPin] { item.isHidden = message.isDeleted || pinError == nil }
        retryPin.isEnabled = canInteract && !pinPending
        retryPin.setAccessibilityIdentifier("pin-retry-\(message.id)")
        dismissPin.setAccessibilityIdentifier("pin-dismiss-\(message.id)")
        self.day.stringValue = day ?? ""
        self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(message.id)")
        unread.isHidden = !newMessages
        let mentionsViewer = !message.isDeleted && NativePresentation.mentionsViewer(message, viewerID: viewerID)
        setAccessibilityHelp(mentionsViewer ? "Mentions you" : nil)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = revealedBlocked ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
        blockedBar.isHidden = !revealedBlocked
        let pin = message.isDeleted ? nil : message.pin
        mentionTint.isHidden = !mentionsViewer && pin == nil
        mentionTint.backgroundColor = pin.map { ($0.author.id == viewerID
            ? NativeStyle.accent.withAlphaComponent(0.12) : NativeStyle.pinGold.withAlphaComponent(0.06)).cgColor }
            ?? NativeStyle.accent.withAlphaComponent(0.08).cgColor
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
        timestamp.stringValue = NativePresentation.time(message.createdAt, includingDate: inPins)
        body.textStorage?.setAttributedString(bodyText(message))
        body.configureMentions(message.isDeleted ? [] : message.content.mentions)
        body.setAccessibilityIdentifier("message-\(message.id)")
        if NSMaxRange(range) <= body.string.utf16.count { body.setSelectedRange(range) }
        avatar.update(
            name: message.author.name,
            avatarId: message.author.avatarId,
            picture: grouped ? nil : message.author.avatarPicture
        )
        let previousReactions = reactions
        reactions = (message.isDeleted ? [] : message.reactions ?? []).map { reaction in
            let id = "reaction-\(message.id)-\(reaction.emoji)"
            let button = previousReactions.first { $0.accessibilityIdentifier() == id }
                ?? NativeButton("") { react?(message.id, reaction.emoji) }
            let selected = viewerID.map { reaction.authorIds.contains($0) } ?? false
            // Web's chip: the same Twemoji artwork as the hover bubble and
            // picker, then a bold count; terracotta when it includes you.
            let count = "\(reaction.authorIds.count)"
            if let art = NativePresentation.emojiImage(reaction.emoji) {
                button.image = NSImage(cgImage: art, size: NSSize(width: 18, height: 18))
                button.imagePosition = .imageLeading
                button.imageHugsTitle = true
            }
            button.attributedTitle = NSAttributedString(string: button.image == nil ? "\(reaction.emoji) \(count)" : " \(count)", attributes: [
                .font: NativeStyle.font(12, bold: true),
                .foregroundColor: selected ? NativeStyle.accentBright : NativeStyle.muted,
            ])
            button.setAccessibilityLabel("\(reaction.emoji) \(count)")
            button.isEnabled = canInteract
            button.setAccessibilityIdentifier(id)
            button.hoverChanged = { [weak self, weak button] entered in
                guard let self, let button, let message = self.message else { return }
                self.hoverReaction?(message, reaction.emoji, entered, button)
            }
            let showPeople = { [weak self, weak button] in
                guard let self, let button, let message = self.message else { return }
                self.messageAction?(message, .reactors(reaction.emoji), button)
            }
            button.menu = NavigationMenus.menu([NavigationMenuItem("Show who reacted", action: showPeople)])
            button.setAccessibilityCustomActions([NSAccessibilityCustomAction(name: "Show who reacted") { showPeople(); return true }])
            button.state = selected ? .on : .off
            button.layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : NativeStyle.surface).cgColor
            button.layer?.borderWidth = 1
            button.layer?.borderColor = (selected ? NativeStyle.accent : NativeStyle.border).cgColor
            return button
        }
        for button in previousReactions where !reactions.contains(where: { $0 === button }) { button.removeFromSuperview() }
        for button in reactions where button.superview == nil { addSubview(button) }
        needsLayout = true
    }

    private static func showsEdited(_ message: ChatMessage) -> Bool {
        !message.isDeleted && message.forward == nil && (message.revision ?? 1) > 1 && !message.content.text.isEmpty
    }

    /// The message's text: spoilers the reader has not revealed stay hidden.
    private func bodyText(_ message: ChatMessage) -> NSAttributedString {
        let text = NSMutableAttributedString(attributedString: NativePresentation.messageText(
            message.isDeleted ? MessageDeletion.placeholder : message.content.text,
            mentions: message.isDeleted ? [] : message.content.mentions))
        if message.isDeleted {
            text.addAttribute(.foregroundColor, value: NativeStyle.muted, range: NSRange(location: 0, length: text.length))
        } else {
            NativePresentation.concealSpoilers(text, revealed: revealedSpoilers)
        }
        // Emoji fallback fonts have larger metrics than Inrond. Use the same
        // 20-point pitch for measurement and TextKit drawing, not per-glyph height.
        // Quoted lines keep their indent.
        var styles: [(NSRange, NSParagraphStyle?)] = []
        text.enumerateAttribute(.paragraphStyle, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            styles.append((range, value as? NSParagraphStyle))
        }
        for (range, style) in styles {
            let paragraph = (style?.mutableCopy() as? NSMutableParagraphStyle) ?? NSMutableParagraphStyle()
            paragraph.minimumLineHeight = 20; paragraph.maximumLineHeight = 20
            text.addAttribute(.paragraphStyle, value: paragraph, range: range)
        }
        return text
    }

    /// Shows the spoiler groups in place: the same glyphs, so the row keeps its height.
    private func revealSpoilers(_ groups: Set<Int>) {
        guard let message, !message.isDeleted, !groups.isSubset(of: revealedSpoilers) else { return }
        revealedSpoilers.formUnion(groups)
        let range = body.selectedRange()
        body.textStorage?.setAttributedString(bodyText(message))
        body.configureMentions(message.content.mentions)
        if NSMaxRange(range) <= body.string.utf16.count { body.setSelectedRange(range) }
        NSAccessibility.post(element: body, notification: .valueChanged)
    }

    func configure(_ run: BlockedRun, day: String?, toggle: @escaping () -> Void) {
        toggleBlocked = toggle
        guard blockedRun != run || self.day.stringValue != (day ?? "") else { return }
        blockedRun = run; message = nil; grouped = true; continued = false
        self.day.stringValue = day ?? ""; self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(run.id)")
        unread.isHidden = true
        setAccessibilityIdentifier(run.id)
        setAccessibilityHelp(nil)
        body.isHidden = true; body.string = ""
        authorAction.isHidden = true; avatarAction.isHidden = true
        avatar.update(name: "", avatarId: nil, picture: nil)
        controls.isHidden = true; edited.isHidden = true
        reactionFailure.isHidden = true; retryReaction.isHidden = true; dismissReaction.isHidden = true
        pinnedBy.isHidden = true; pinFailure.isHidden = true; retryPin.isHidden = true; dismissPin.isHidden = true
        reactions.forEach { $0.removeFromSuperview() }; reactions = []
        attachments?.removeFromSuperview(); attachments = nil; attachmentHeight = 0
        quote?.removeFromSuperview(); quote = nil; quoteHeight = 0
        thread.isHidden = true
        blockedToggle.isHidden = false
        blockedToggle.title = "\(run.label) — \(run.revealed ? "Hide" : "Show")"
        blockedToggle.setAccessibilityLabel("\(run.revealed ? "Hide" : "Show") \(run.label)")
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = run.revealed ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
        blockedBar.isHidden = !run.revealed
        mentionTint.isHidden = true; mentionBar.isHidden = true
        CATransaction.commit()
        needsLayout = true
    }

    func measuredHeight(width: CGFloat) -> CGFloat {
        layoutWidth = width
        if pending != nil { return attachments?.height(for: width) ?? 0 }
        if blockedRun != nil { return dayHeight + 48 }
        let textWidth = max(40, width - textInset - 20)
        body.textContainer?.containerSize = NSSize(width: textWidth, height: .greatestFiniteMagnitude)
        let text = body.attributedString()
        let coreTextHeight = NativePresentation.textHeight(text, width: textWidth)
        body.layoutManager?.ensureLayout(for: body.textContainer!)
        let nativeHeight = body.layoutManager?.usedRect(for: body.textContainer!).height ?? 0
        attachmentHeight = attachments?.height(for: textWidth) ?? 0
        quoteHeight = quote?.height(for: max(40, width - 40)) ?? 0
        let textHeight = body.string.isEmpty && attachments != nil ? 0
            : max(20, max(coreTextHeight, ceil(nativeHeight))) + (editedFrame(width: textWidth)?.ownLine == true ? 18 : 0)
        let height = textTop + textHeight + attachmentHeight + reactionHeight + failureHeight + threadHeight + bottomPadding
        let controlsWidth = min(controls.preferredWidth, max(30, width - 30))
        // A wrapped hover bar must not steal the next grouped row's hit area.
        return controlsWidth < controls.preferredWidth
            ? max(height, dividerHeight + pinHeight + quoteHeight + controls.height(for: controlsWidth) + 11) : height
    }

    private var reactionFrames: [NSRect] {
        let available = max(40, layoutWidth - textInset - 20)
        var x: CGFloat = 0, y: CGFloat = 0
        return reactions.map { reaction in
            let width = min(available, max(54, reaction.intrinsicContentSize.width + 12))
            if x > 0, x + width > available { x = 0; y += 32 }
            defer { x += width + 6 }
            return NSRect(x: x, y: y, width: width, height: 28)
        }
    }
    private var reactionHeight: CGFloat { reactionFrames.last.map { $0.maxY + 4 } ?? 0 }

    override func layout() {
        super.layout()
        if pending != nil { attachments?.frame = bounds; return }
        let dayInset = dayHeight, inset = dividerHeight
        let dayWidth = min(ceil(day.cell?.cellSize.width ?? 0), max(0, bounds.width - 36))
        day.frame = NSRect(x: (bounds.width - dayWidth) / 2, y: 8, width: dayWidth, height: max(0, dayInset - 16))
        let unreadWidth = min(ceil(unread.cell?.cellSize.width ?? 0), max(0, bounds.width - 32))
        unread.frame = NSRect(x: (bounds.width - unreadWidth) / 2, y: dayInset + 8, width: unreadWidth, height: max(0, inset - dayInset - 16))
        CATransaction.begin(); CATransaction.setDisableActions(true)
        dayLeading.isHidden = day.isHidden; dayTrailing.isHidden = day.isHidden
        dayLeading.frame = NSRect(x: 18, y: floor(dayInset / 2), width: max(0, day.frame.minX - 28), height: 1)
        dayTrailing.frame = NSRect(x: day.frame.maxX + 10, y: floor(dayInset / 2), width: max(0, bounds.width - day.frame.maxX - 28), height: 1)
        unreadLeading.isHidden = unread.isHidden; unreadTrailing.isHidden = unread.isHidden
        let unreadLine = dayInset + floor((inset - dayInset) / 2)
        unreadLeading.frame = NSRect(x: 16, y: unreadLine, width: max(0, unread.frame.minX - 24), height: 1)
        unreadTrailing.frame = NSRect(x: unread.frame.maxX + 8, y: unreadLine, width: max(0, bounds.width - unread.frame.maxX - 24), height: 1)
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
        let textWidth = max(40, bounds.width - textInset - 20)
        pinnedBy.frame = NSRect(x: textInset, y: inset, width: textWidth, height: pinHeight)
        quote?.frame = NSRect(x: 20, y: inset + pinHeight, width: max(40, bounds.width - 40), height: quoteHeight)
        let messageTop = inset + pinHeight + quoteHeight
        avatar.frame = NSRect(x: 20, y: messageTop + 10, width: 36, height: 36)
        // Intrinsic size describes the alignment rect, excluding NSTextField's
        // cell insets. Manual frames need cellSize or even short names truncate.
        // The time stays beside the name at every width, as on web and the
        // desktop clients; a long name gives way and ends in "…".
        let timeWidth = min(ceil(timestamp.cell?.cellSize.width ?? timestamp.intrinsicContentSize.width), max(0, bounds.width - 95))
        let authorWidth = min(ceil(author.cell?.cellSize.width ?? author.intrinsicContentSize.width), max(0, bounds.width - 88 - timeWidth - 7))
        // A label's cell draws its text 2 points in; the body has no inset.
        // Start the name 2 points early so it lines up with the message.
        author.frame = NSRect(x: 66, y: messageTop + 10, width: authorWidth, height: 20)
        timestamp.frame = NSRect(x: author.frame.maxX + 7, y: messageTop + 13, width: timeWidth, height: 17)
        // The hover wash and focus ring get room around the name, which
        // otherwise touches their left edge.
        authorAction.frame = author.frame.insetBy(dx: -4, dy: -1)
        avatarAction.frame = avatar.frame
        let controlsWidth = min(controls.preferredWidth, max(30, bounds.width - 30))
        controls.frame = NSRect(x: max(0, bounds.width - controlsWidth - 15), y: messageTop + (grouped ? -3 : 3),
            width: controlsWidth, height: controls.height(for: controlsWidth))
        let footer = attachmentHeight + reactionHeight + failureHeight + threadHeight
        let top = textTop
        body.frame = NSRect(x: textInset, y: top, width: textWidth, height: max(0, bounds.height - top - footer - bottomPadding))
        edited.frame = editedFrame(width: textWidth).map { $0.frame.offsetBy(dx: body.frame.minX, dy: body.frame.minY) } ?? .zero
        attachments?.frame = NSRect(x: textInset, y: body.frame.maxY, width: textWidth, height: attachmentHeight)
        for (reaction, frame) in zip(reactions, reactionFrames) {
            reaction.frame = frame.offsetBy(dx: textInset, dy: bounds.height - failureHeight - threadHeight - reactionHeight - bottomPadding + 4)
        }
        thread.frame = NSRect(x: textInset, y: bounds.height - threadHeight - bottomPadding + 4, width: textWidth, height: 28)
        reactionFailure.frame = NSRect(x: textInset, y: bounds.height - failureHeight - threadHeight - bottomPadding + 4, width: textWidth, height: 30)
        retryReaction.frame = NSRect(x: textInset, y: reactionFailure.frame.maxY + 4, width: 58, height: 24)
        dismissReaction.frame = NSRect(x: stackedRecovery ? textInset : textInset + 64, y: retryReaction.frame.minY + (stackedRecovery ? 28 : 0), width: 68, height: 24)
        pinFailure.frame = NSRect(x: textInset, y: reactionFailure.frame.minY + reactionFailureHeight, width: textWidth, height: 30)
        retryPin.frame = NSRect(x: textInset, y: pinFailure.frame.maxY + 4, width: 58, height: 24)
        dismissPin.frame = NSRect(x: stackedRecovery ? textInset : textInset + 64, y: retryPin.frame.minY + (stackedRecovery ? 28 : 0), width: 68, height: 24)
    }

    /// The (edited) button follows the last word as on web, in body
    /// coordinates, or starts its own line when the last line is full.
    private func editedFrame(width: CGFloat) -> (frame: NSRect, ownLine: Bool)? {
        guard !edited.isHidden, let layoutManager = body.layoutManager, layoutManager.numberOfGlyphs > 0 else { return nil }
        let size = NSSize(width: ceil(edited.cell?.cellSize.width ?? 44), height: 16)
        let last = layoutManager.extraLineFragmentTextContainer != nil ? layoutManager.extraLineFragmentUsedRect
            : layoutManager.lineFragmentUsedRect(forGlyphAt: layoutManager.numberOfGlyphs - 1, effectiveRange: nil)
        // Lowered 4 points so the 10-point label sits on the 14-point text's baseline.
        if last.maxX + 2 + size.width <= width { return (NSRect(origin: NSPoint(x: last.maxX + 2, y: last.minY + 4), size: size), false) }
        return (NSRect(origin: NSPoint(x: -2, y: last.maxY), size: size), true)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let tracking = NSTrackingArea(rect: .zero, options: [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow], owner: self, userInfo: nil)
        self.tracking = tracking; addTrackingArea(tracking)
    }
    override func mouseEntered(with event: NSEvent) { refreshHover() }
    override func mouseExited(with event: NSEvent) { refreshHover() }
    /// AppKit sends no mouseExited when scrolling moves a row out from under a
    /// still pointer, so its scroll view re-reads the pointer after each move.
    /// Like the tracking area, it waits while another window (a picker) is key.
    func refreshHover() {
        guard let window, window.isKeyWindow else { return }
        // A row's visibleRect can be the whole visible history (views need
        // not clip to their bounds), so keep to the row itself.
        let inside = bounds.intersection(visibleRect).contains(convert(window.mouseLocationOutsideOfEventStream, from: nil))
        guard inside != hovered else { return }
        hovered = inside; updateControlsVisibility()
    }
    private func updateControlsVisibility() { controls.alphaValue = hovered || controls.hasFocus ? 1 : 0 }
    override func rightMouseDown(with event: NSEvent) {
        guard let message, let menu = messageMenu?(message, self) else { super.rightMouseDown(with: event); return }
        CaperMenu.show(menu, at: convert(event.locationInWindow, from: nil), in: self)
    }
}

/// TextKit still owns selection, including drags starting on a mention. Only a
/// single stationary click on an actual resolved-person glyph opens a card.
@MainActor final class MessageTextView: NSTextView {
    var openMention: ((MessageMention, NSView, NSRect) -> Void)?
    var messageMenu: (() -> NSMenu?)?
    var typeToCompose: ((NSEvent) -> Bool)?
    /// Reveals hidden spoiler groups (a click, or VoiceOver's "Spoiler, reveal").
    var revealSpoilers: ((Set<Int>) -> Void)?
    private var people: [String: MessageMention] = [:]

    // Read-only message bodies are drag sources, never destinations. Let the
    // conversation receive files even when a drag also offers plain text.
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { [] }

    override func rightMouseDown(with event: NSEvent) {
        guard let actions = messageMenu?() else { super.rightMouseDown(with: event); return }
        // Copy for a selection comes first, then the message menu (its own
        // Copy text, links and actions), all in one Caper menu.
        var items: [CaperMenuItem] = []
        let selected = selectedRanges.map(\.rangeValue).filter { $0.length > 0 }
            .map { (string as NSString).substring(with: $0) }.joined(separator: "\n")
        if !selected.isEmpty {
            items += [.action("Copy") {
                NSPasteboard.general.clearContents(); NSPasteboard.general.setString(selected, forType: .string)
            }, .separator]
        }
        CaperMenu.show(items + CaperMenuItem.items(from: actions), at: convert(event.locationInWindow, from: nil), in: self)
    }

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
        let hidden = NativePresentation.hiddenSpoilers(in: text)
        if !hidden.isEmpty {
            actions.insert(NSAccessibilityCustomAction(name: "Spoiler, reveal") { [weak self] in
                guard let self, let reveal = self.revealSpoilers else { return false }
                reveal(hidden)
                return true
            }, at: 0)
        }
        setAccessibilityCustomActions(actions)
        window?.invalidateCursorRects(for: self)
    }

    /// VoiceOver reads "Spoiler" in place of each hidden spoiler.
    override func accessibilityValue() -> String? {
        guard let text = textStorage, !NativePresentation.hiddenSpoilers(in: text).isEmpty else { return super.accessibilityValue() }
        return NativePresentation.spokenText(text)
    }

    /// A printable key on selected message text types into the composer;
    /// ⌘/⌃ shortcuts and navigation keys stay with the selection.
    override func keyDown(with event: NSEvent) {
        if Self.composes(event), typeToCompose?(event) == true { return }
        super.keyDown(with: event)
    }
    /// Read-only text otherwise drops focus on Tab; move on as the composer does.
    override func insertTab(_ sender: Any?) { KeyboardTabOrder.step(in: window, forward: true, from: self) }
    override func insertBacktab(_ sender: Any?) { KeyboardTabOrder.step(in: window, forward: false, from: self) }

    static func composes(_ event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection([.command, .control, .function]).isEmpty,
              let characters = event.characters, !characters.isEmpty else { return false }
        // Control characters (Escape, Tab, Return, Delete) and the private-use
        // function-key range (arrows, Page Up) are not text.
        return characters.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) && !(0xF700...0xF8FF).contains($0.value) }
    }

    override func mouseDown(with event: NSEvent) {
        let start = convert(event.locationInWindow, from: nil)
        // A click on a hidden spoiler reveals its whole group, and nothing else.
        if event.clickCount == 1, let group = hiddenSpoiler(at: start) { revealSpoilers?([group]); return }
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

    private func hiddenSpoiler(at point: NSPoint) -> Int? {
        guard let layoutManager, let textContainer, !string.isEmpty else { return nil }
        let origin = textContainerOrigin
        let local = NSPoint(x: point.x - origin.x, y: point.y - origin.y)
        let glyph = layoutManager.glyphIndex(for: local, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs,
              layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer).insetBy(dx: -2, dy: -1).contains(local) else { return nil }
        return textStorage?.attribute(NativePresentation.hiddenSpoilerAttribute, at: layoutManager.characterIndexForGlyph(at: glyph), effectiveRange: nil) as? Int
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        guard let layoutManager, let textContainer else { return }
        attributedString().enumerateAttribute(NativePresentation.hiddenSpoilerAttribute, in: NSRange(location: 0, length: string.utf16.count)) { value, range, _ in
            guard value != nil else { return }
            let glyphs = layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            layoutManager.enumerateEnclosingRects(forGlyphRange: glyphs, withinSelectedGlyphRange: NSRange(location: NSNotFound, length: 0), in: textContainer) { rect, _ in
                self.addCursorRect(rect.offsetBy(dx: self.textContainerOrigin.x, dy: self.textContainerOrigin.y), cursor: .pointingHand)
            }
        }
        attributedString().enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: string.utf16.count)) { value, range, _ in
            guard let id = value as? String, self.people[id] != nil else { return }
            let glyphs = layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            layoutManager.enumerateEnclosingRects(forGlyphRange: glyphs, withinSelectedGlyphRange: NSRange(location: NSNotFound, length: 0), in: textContainer) { rect, _ in
                self.addCursorRect(rect.offsetBy(dx: self.textContainerOrigin.x, dy: self.textContainerOrigin.y), cursor: .pointingHand)
            }
        }
    }
}

/// Native controls remain in the keyboard/AX order when the pointer leaves.
/// Only their visual shell fades; no hover event rebuilds the row or its text.
@MainActor private final class MessageActionBar: NativeView {
    var action: ((MessageAction, NSView) -> Void)?
    var messageMenu: ((NSView) -> NSMenu?)?
    var focusChanged: (() -> Void)?
    private var quick: [NativeButton] = []
    private lazy var add: NativeButton = NativeButton.icon("Add reaction", symbol: "face.smiling") { [weak self] in
        guard let self else { return }; self.action?(.addReaction, self.add)
    }
    private lazy var reply: NativeButton = NativeButton.icon("Reply", symbol: "arrowshape.turn.up.left") { [weak self] in
        guard let self else { return }; self.action?(.reply, self.reply)
    }
    private lazy var more: NativeButton = NativeButton.icon("Message options", symbol: "ellipsis") { [weak self] in self?.showMenu() }
    var hasFocus: Bool { subviews.contains { $0 === window?.firstResponder } }
    // AppKit exposes a one-point focus/AX outset around these 24-point buttons.
    // Leave room between their hit regions, not only between their artwork.
    private let buttonPitch: CGFloat = 26
    var preferredWidth: CGFloat { CGFloat(subviews.filter { !$0.isHidden }.count - 1) * buttonPitch + 30 }
    private func columns(for width: CGFloat) -> Int { max(1, Int((width - 30) / buttonPitch) + 1) }
    func height(for width: CGFloat) -> CGFloat {
        let columns = columns(for: width), count = subviews.filter { !$0.isHidden }.count
        return CGFloat(max(0, (count - 1) / columns)) * buttonPitch + 30
    }

    init() {
        super.init(NativeStyle.surface)
        layer?.cornerRadius = 8; layer?.borderWidth = 1; layer?.borderColor = NativeStyle.border.cgColor
        quick = NativePresentation.quickReactions.map { emoji in
            let button = NativeButton("React with \(emoji)") { [weak self] in
                guard let self else { return }; self.action?(.reaction(emoji), self)
            }
            if let image = NativePresentation.emojiImage(emoji) {
                button.image = NSImage(cgImage: image, size: NSSize(width: 18, height: 18))
                button.imagePosition = .imageOnly
            }
            button.toolTip = "React with \(emoji)"
            return button
        }
        for button in quick + [add, reply, more] {
            button.layer?.backgroundColor = NSColor.clear.cgColor
            button.focusChanged = { [weak self] _ in
                // AppKit changes firstResponder after resignFirstResponder returns.
                DispatchQueue.main.async { [weak self] in self?.focusChanged?() }
            }
            addSubview(button)
        }
        alphaValue = 0
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func applyTheme() {
        super.applyTheme()
        layer?.borderColor = NativeStyle.border.cgColor
    }

    func configure(_ message: ChatMessage, canInteract: Bool, viewerID: String?) {
        for (emoji, button) in zip(NativePresentation.quickReactions, quick) {
            button.isHidden = !canInteract || message.isDeleted
            button.isEnabled = canInteract
            button.setAccessibilityIdentifier("quick-reaction-\(message.id)-\(emoji)")
            let selected = viewerID.map { id in message.reactions?.first(where: { $0.emoji == emoji })?.authorIds.contains(id) == true } ?? false
            button.state = selected ? .on : .off
            button.layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : .clear).cgColor
        }
        add.isHidden = message.isDeleted; add.isEnabled = canInteract
        add.setAccessibilityIdentifier("add-reaction-\(message.id)")
        reply.isHidden = message.isDeleted; reply.isEnabled = canInteract
        reply.setAccessibilityIdentifier("reply-message-\(message.id)")
        more.setAccessibilityIdentifier("message-options-\(message.id)")
        needsLayout = true
    }
    func showMenu() {
        guard let menu = messageMenu?(more) else { return }
        CaperMenu.show(menu, below: more)
    }
    override func layout() {
        super.layout()
        let columns = columns(for: bounds.width)
        for (index, button) in subviews.filter({ !$0.isHidden }).enumerated() {
            button.frame = NSRect(x: 3 + CGFloat(index % columns) * buttonPitch, y: 3 + CGFloat(index / columns) * buttonPitch, width: 24, height: 24)
        }
    }
    override func hitTest(_ point: NSPoint) -> NSView? { alphaValue == 0 ? nil : super.hitTest(point) }
}
