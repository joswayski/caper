import AppKit
import CaperCore
import CoreImage
import QuartzCore

enum MessageAction { case reaction(String), reactors(String), addReaction, retryReaction, dismissReaction, pin, retryPin, dismissPin, history, reply, replyInThread, openThread }
enum PendingAction { case retry, edit, dismiss }

/// A scroll document with only realized native text views. Unlike NSTableView,
/// it never manufactures an AX row for every message in the loaded history.
@MainActor final class MessageTimeline: NSScrollView {
    private let document = NativeView(NativeStyle.conversation)
    private var geometry = NativeTimelineLayout()
    private let arrivals = MessageArrivals()
    private var messages: [ChatMessage] = []
    private var entries: [TimelineEntry] = []
    private var pending: PendingMessage?
    private var pendingAuthor: ChatAuthor?
    private var pendingError: String?
    private var sendRejected = false
    private var sendUploadFailed = false
    private var canEditPending = true
    private var activePending: PendingMessage? {
        guard let pending, !entries.contains(where: { key($0) == pending.id }) else { return nil }
        return pending
    }
    private var rowIDs: [String] { entries.map(key) + (activePending.map { [$0.id] } ?? []) }
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
    private var activeMessageID: String?
    private weak var observedWindow: NSWindow?
    var inThread = false
    /// The first unread message of this visit: it wears the "New messages"
    /// line and starts a new author group, as on every other client. Set it
    /// before `update`.
    var newMessagesID: String?
    /// The divider the rows were last laid out with.
    private var shownNewMessagesID: String?
    var canLoadOlder = false
    /// The loaded history reaches the live end (no newer page after a jump).
    /// Set it before `update`.
    var live = true
    /// The "You’re viewing older messages" toast: whether it shows, and other
    /// people's messages that arrived at the live end while it did. A
    /// detached window (not `live`) shows it wherever the reader is, uncounted.
    private(set) var jump = (shows: false, count: 0)
    /// Below the last row, as Load newer in a detached window: 8 points above
    /// it and 68 below, so the toast floating over the history never covers it.
    var footer: NSView? {
        didSet {
            guard footer !== oldValue else { return }
            oldValue?.removeFromSuperview()
            if let footer { document.addSubview(footer) }
            realize(anchor: geometry.anchor(at: contentView.bounds.minY))
        }
    }
    private var footerHeight: CGFloat { footer == nil ? 8 : 8 + 28 + 68 }
    /// The end of the scrollable content: the last row, and the footer under it.
    private var contentEnd: CGFloat { geometry.contentHeight + footerHeight }
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
    var makePendingAttachments: ((PendingMessage) -> AppKitAttachmentSurface?)?
    var makePendingReply: ((PendingMessage) -> AppKitAttachmentSurface?)?
    var pendingAction: ((PendingMessage, PendingAction) -> Void)?
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

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let observedWindow {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didBecomeKeyNotification, object: observedWindow)
            NotificationCenter.default.removeObserver(self, name: NSWindow.didResignKeyNotification, object: observedWindow)
        }
        observedWindow = window
        if let window {
            NotificationCenter.default.addObserver(self, selector: #selector(windowDidBecomeKey),
                name: NSWindow.didBecomeKeyNotification, object: window)
            NotificationCenter.default.addObserver(self, selector: #selector(windowDidResignKey),
                name: NSWindow.didResignKeyNotification, object: window)
        }
    }

    @objc private func windowDidBecomeKey() { refreshHover() }
    @objc private func windowDidResignKey() { rows.values.forEach { $0.clearHover() } }

    func refreshHover() { rows.values.forEach { $0.refreshHover() } }

    /// Keeps a message row highlighted while an anchored reaction picker is open.
    func setActiveMessageID(_ id: String?) {
        guard activeMessageID != id else { return }
        let previous = activeMessageID
        activeMessageID = id
        if let previous { rows[previous]?.setActionActive(false) }
        if let id { rows[id]?.setActionActive(true) }
        if id == nil {
            DispatchQueue.main.async { [weak self] in self?.refreshHover() }
        }
    }

    @objc private func handleThemeChange() {
        backgroundColor = NativeStyle.conversation
        document.applyTheme()
        for row in rows.values {
            row.applyTheme()
            row.needsDisplay = true
        }
        needsDisplay = true
    }

    func update(_ messages: [ChatMessage], conversationID: String?, viewerID: String? = nil, blocked: Set<String> = [],
                pending: PendingMessage? = nil, pendingAuthor: ChatAuthor? = nil, pendingError: String? = nil,
                sendRejected: Bool = false, sendUploadFailed: Bool = false, canEditPending: Bool = true,
                canInteract: Bool = false, reactionErrors: [String: String] = [:], pinErrors: [String: String] = [:], pendingPins: Set<String> = []) {
        let changedConversation = self.conversationID != conversationID
        let anchor = geometry.anchor(at: contentView.bounds.minY)
        let anchorTextTop = anchor.flatMap { rows[$0.id]?.textTop }
        let motionEnabled = !changedConversation && live && followEnd && !CaperAccessibility.reduceMotion
        var previousVisualY: [String: CGFloat] = [:]
        if motionEnabled {
            let scrollY = contentView.bounds.minY
            for (id, row) in rows {
                let translationY = (row.layer?.presentation()?.value(forKeyPath: "transform.translation.y") as? NSNumber)
                    .map { CGFloat($0.doubleValue) } ?? 0
                previousVisualY[id] = row.frame.minY - scrollY + translationY
            }
        }
        if changedConversation { revealed = [] }
        let next = BlockedMessages.entries(messages, blocked: blocked, viewerID: viewerID, revealed: revealed).flatMap { entry -> [TimelineEntry] in
            if case let .blocked(run) = entry, run.revealed { return [entry] + run.messages.map(TimelineEntry.message) }
            return [entry]
        }
        let oldActivePending = activePending
        let oldPendingAuthor = self.pendingAuthor
        let oldEntriesEmpty = entries.isEmpty
        let nextActivePending: PendingMessage? = pending.flatMap { item in
            next.contains(where: { key($0) == item.id }) ? nil : item
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
            let messageID: String? = if case .message(let m) = entry { m.id } else { nil }
            let errorsChanged = messageID.map {
                self.reactionErrors[$0] != reactionErrors[$0]
                    || self.pinErrors[$0] != pinErrors[$0]
                    || self.pendingPins.contains($0) != pendingPins.contains($0)
            } ?? false
            // Deleting or inserting a predecessor (or pending send) can change
            // grouping even when this message itself has not changed.
            if entries[oldIndex] != entry || oldPredecessor != predecessor
                || continues(at: oldIndex, in: entries, pending: oldActivePending, author: oldPendingAuthor)
                    != continues(at: index, in: next, pending: nextActivePending, author: pendingAuthor)
                || errorsChanged || self.canInteract != canInteract { invalid.insert(id) }
        }
        if shownNewMessagesID != newMessagesID {
            // The divider's row gains or loses its line and header, and the row
            // above it its hairline of padding.
            for (id, list) in [(shownNewMessagesID, entries), (newMessagesID, next)] {
                guard let id, let index = list.firstIndex(where: { if case .message(let m) = $0 { return m.id == id }; return false }) else { continue }
                invalid.insert(key(list[index]))
                if index > 0 { invalid.insert(key(list[index - 1])) }
            }
            shownNewMessagesID = newMessagesID
        }
        if let nextActivePending,
           oldActivePending != nextActivePending || self.pendingAuthor != pendingAuthor
            || self.pendingError != pendingError || self.sendRejected != sendRejected
            || self.sendUploadFailed != sendUploadFailed || self.canEditPending != canEditPending
            || entries.last != next.last {
            invalid.insert(nextActivePending.id)
        }
        self.pending = pending
        self.pendingAuthor = pendingAuthor
        self.pendingError = pendingError
        self.sendRejected = sendRejected
        self.sendUploadFailed = sendUploadFailed
        self.canEditPending = canEditPending
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
            rows.values.forEach { $0.cancelArrivalMotion(); $0.removeFromSuperview() }
            rows.removeAll()
        }
        let ids = rowIDs
        arrivals.observe("timeline", key: conversationID ?? "", ids: ids, live: live && followEnd)
        geometry.replace(ids: ids, width: contentSize.width, invalidating: invalid)
        realize(anchor: changedConversation ? nil : anchor, anchorTextTop: anchorTextTop)
        if motionEnabled {
            var incoming = Set(rows.keys.filter { arrivals.claim($0) })
            if oldEntriesEmpty, let p = nextActivePending, oldActivePending?.id != p.id, rows[p.id] != nil {
                incoming.insert(p.id)
            }
            if !incoming.isEmpty {
                let scrollY = contentView.bounds.minY
                for id in incoming { rows[id]?.animateArrival() }
                for (id, row) in rows where !incoming.contains(id) {
                    guard let oldY = previousVisualY[id] else { continue }
                    let delta = oldY - (row.frame.minY - scrollY)
                    if abs(delta) > 0.5 { row.animateGlide(fromOffsetY: delta) }
                }
            }
        }
        let currentIDs = Set(ids)
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
            pending: pending, pendingAuthor: pendingAuthor, pendingError: pendingError,
            sendRejected: sendRejected, sendUploadFailed: sendUploadFailed, canEditPending: canEditPending,
            canInteract: canInteract, reactionErrors: reactionErrors, pinErrors: pinErrors, pendingPins: pendingPins)
        guard let index = entries.firstIndex(where: { if case .message(let m) = $0 { return m.id == id }; return false }) else { return false }
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
        rows.values.forEach { $0.cancelArrivalMotion() }
        let viewport = contentSize.height
        let start = contentView.bounds.minY
        guard JumpToLatest.animates(distanceFromEnd: contentEnd - contentView.bounds.maxY, viewport: viewport,
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
                let end = max(0, self.contentEnd - self.contentSize.height)
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

    /// Shows the toast more than half a screen above the newest loaded message
    /// of a live history; anywhere closer hides it and clears its count. A
    /// detached window shows it wherever the reader is, without a count.
    private func refreshJump() {
        let shows = !live || (jumpAnimation == nil && !followEnd && JumpToLatest.shows(
            distanceFromEnd: contentEnd - contentView.bounds.maxY, viewport: contentSize.height, live: true))
        let next = (shows: shows, count: shows && live ? jump.count : 0)
        guard next != jump else { return }
        jump = next
        jumpChanged?()
    }

    /// The row after `index` groups under it, so this row keeps only a
    /// hairline of padding below (web's `chat-message-continued`).
    private func continues(at index: Int, in list: [TimelineEntry], pending: PendingMessage?, author: ChatAuthor?) -> Bool {
        guard index < list.count, case .message(let message) = list[index] else { return false }
        let prior = inThread && message.threadRootId == nil ? nil : message
        if index + 1 < list.count {
            guard case .message(let next) = list[index + 1],
                  next.id != newMessagesID, !BlockedMessages.isHidden(next, blocked: blocked, viewerID: viewerID) else { return false }
            return NativePresentation.grouped(next, after: prior, inThread: inThread)
        }
        guard let pending, !BlockedMessages.isHidden(message, blocked: blocked, viewerID: viewerID) else { return false }
        return NativePresentation.grouped(pending, author: author, after: prior, inThread: inThread)
    }

    /// Stable row identity: clientMessageId keeps a pending send and its
    /// confirmed server message on the exact same cell, with zero layout shift.
    private func key(_ entry: TimelineEntry) -> String {
        switch entry {
        case .message(let message): message.clientMessageId.isEmpty ? message.id : message.clientMessageId
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
        followEnd = contentView.bounds.maxY >= contentEnd - 2
        realize(anchor: geometry.anchor(at: contentView.bounds.minY))
        if !followEnd { requestOlderIfNeeded() }
    }

    override func scrollWheel(with event: NSEvent) {
        // The reader's own scrolling takes over from any jump or arrival glide.
        if event.scrollingDeltaY != 0 {
            if jumpAnimation != nil { stopJumpAnimation() }
            rows.values.forEach { $0.cancelArrivalMotion() }
        }
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
            let wanted = followEnd ? contentEnd - viewport : (anchor.flatMap { geometry.offset(for: $0) } ?? contentView.bounds.minY) + textShift
            return min(max(0, wanted), max(0, contentEnd - viewport))
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
                row.cancelArrivalMotion()
                if row.body.selectedRange().length > 0 { selection[id] = (row.body.string, row.body.selectedRange()) }
                row.removeFromSuperview()
            }
            document.setFrameSize(NSSize(width: width, height: max(viewport, contentEnd)))
            // A conversation shorter than the pane rests on the composer, as
            // on web, instead of hanging under the header.
            let lead = max(0, viewport - contentEnd)
            for index in needed {
                let row = cell(at: index)
                row.frame = NSRect(x: 0, y: lead + geometry.origin(of: index), width: width, height: geometry.height(at: index))
                // NSTextView resizing can scroll its selection into view. Finish
                // that layout before restoring the reader's anchor, not after it.
                row.layoutSubtreeIfNeeded()
            }
            if let footer {
                let footerWidth = min(180, max(0, width - 40))
                footer.frame = NSRect(x: (width - footerWidth) / 2, y: lead + geometry.contentHeight + 8, width: footerWidth, height: 28)
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
        var children: [NSView] = needed.compactMap { rows[ids[$0]] }
        if let footer { children.append(footer) }
        document.setAccessibilityChildren(children)
        let visible = Set(needed.compactMap { index -> String? in
            guard index < entries.count, case .message(let message) = entries[index] else { return nil }
            let bottom = geometry.origin(of: index) + geometry.height(at: index)
            return bottom > contentView.bounds.minY && bottom <= contentView.bounds.maxY + 1 ? message.id : nil
        })
        visibleMessagesChanged?(visible)
        refreshJump()
    }

    private func cell(at index: Int) -> MessageCell {
        let id = index < entries.count ? key(entries[index]) : activePending!.id
        let row: MessageCell
        if let existing = rows[id] { row = existing }
        else {
            row = MessageCell()
            row.openAuthor = { [weak self] author, view in self?.openAuthor?(author, view) }
            row.body.openMention = { [weak self] mention, view, rect in self?.openMention?(mention, view, rect) }
            row.body.typeToCompose = { [weak self] event in self?.typeToCompose?(event) ?? false }
            row.makeAttachments = { [weak self] in self?.makeAttachments?($0) }
            row.makeReply = { [weak self] in self?.makeReply?($0) }
            row.makePendingAttachments = { [weak self] in self?.makePendingAttachments?($0) }
            row.makePendingReply = { [weak self] in self?.makePendingReply?($0) }
            row.pendingAction = { [weak self] pending, action in self?.pendingAction?(pending, action) }
            row.messageMenu = { [weak self] message, view in self?.messageMenu?(message, view) }
            row.messageAction = { [weak self] message, action, view in self?.messageAction?(message, action, view) }
            row.hoverReaction = { [weak self] message, emoji, entered, view in self?.hoverReaction?(message, emoji, entered, view) }
            row.menuClosed = { [weak self] in self?.refreshHover() }
            row.heightChanged = { [weak self, weak row] in
                guard let self, let row, self.rows[id] === row else { return }
                let anchor = self.geometry.anchor(at: self.contentView.bounds.minY)
                self.geometry.replace(ids: self.rowIDs, width: self.contentSize.width, invalidating: [id])
                self.realize(anchor: anchor)
            }
            rows[id] = row
            document.addSubview(row)
        }
        if index == entries.count, let pending = activePending {
            let previous = entries.last
            let day = previous.map { NativePresentation.sameDay($0.lastCreatedAt, pending.createdAt) } == true
                ? nil : NativePresentation.dayLabel(pending.createdAt)
            let predecessor: ChatMessage? = if case .message(let prior) = previous { prior } else { nil }
            let prior = inThread && predecessor?.threadRootId == nil ? nil : predecessor
            let grouped = NativePresentation.grouped(pending, author: pendingAuthor, after: prior, inThread: inThread)
            row.configure(pending, author: pendingAuthor, grouped: grouped, day: day, error: pendingError,
                rejected: sendRejected, uploadFailed: sendUploadFailed, canEdit: canEditPending)
            return row
        }
        let entry = entries[index]
        let previous = index > 0 ? entries[index - 1] : nil
        // The divider belongs to this realized row, so prepending can remove
        // it using the same text anchor as an author-header grouping change.
        let day = previous.map { NativePresentation.sameDay($0.lastCreatedAt, entry.firstCreatedAt) } == true
            ? nil : NativePresentation.dayLabel(entry.firstCreatedAt)
        var focused = false
        switch entry {
        case .message(let message):
            let predecessor: ChatMessage? = if case .message(let prior) = previous { prior } else { nil }
            let revealedBlocked = BlockedMessages.isHidden(message, blocked: blocked, viewerID: viewerID)
            // The root is not a predecessor of its first reply.
            let prior = inThread && predecessor?.threadRootId == nil ? nil : predecessor
            let newMessages = message.id == newMessagesID && !revealedBlocked
            focused = focusedID != nil && message.id == focusedID
            row.configure(message, grouped: !newMessages && NativePresentation.grouped(message, after: prior, inThread: inThread),
                continued: continues(at: index, in: entries, pending: activePending, author: pendingAuthor), day: revealedBlocked ? nil : day, newMessages: newMessages,
                viewerID: viewerID, revealedBlocked: revealedBlocked, canInteract: canInteract, inThread: inThread,
                reactionError: reactionErrors[message.id], pinError: pinErrors[message.id], pinPending: pendingPins.contains(message.id), react: react)
            row.setActionActive(message.id == activeMessageID)
        case .blocked(let run):
            row.configure(run, day: day) { [weak self] in
                guard let self else { return }
                self.revealed = BlockedMessages.toggling(run, in: self.revealed)
                self.followEnd = false
                self.update(self.messages, conversationID: self.conversationID, viewerID: self.viewerID, blocked: self.blocked,
                    pending: self.pending, pendingAuthor: self.pendingAuthor, pendingError: self.pendingError,
                    sendRejected: self.sendRejected, sendUploadFailed: self.sendUploadFailed, canEditPending: self.canEditPending,
                    canInteract: self.canInteract, reactionErrors: self.reactionErrors,
                    pinErrors: self.pinErrors, pendingPins: self.pendingPins)
            }
        }
        if focused { row.markJumpTarget() }
        if let saved = selection.removeValue(forKey: id), saved.text == row.body.string,
           NSMaxRange(saved.range) <= row.body.string.utf16.count {
            row.body.setSelectedRange(saved.range)
        }
        return row
    }
}

@MainActor private final class MessageBadge: NativeView {
    private let label: NSTextField
    private let isAdmin: Bool

    init(_ text: String, admin: Bool, accessibilityLabel: String) {
        self.isAdmin = admin
        self.label = NativeStyle.label(text, size: 9, bold: true)
        super.init()
        layer?.cornerRadius = 4
        layer?.borderWidth = 1
        label.alignment = .center
        label.setAccessibilityElement(false)
        addSubview(label)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel(accessibilityLabel)
        setAccessibilityValue(accessibilityLabel)
        applyTheme()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var preferredWidth: CGFloat { ceil(label.cell?.cellSize.width ?? 32) + 10 }

    override func applyTheme() {
        super.applyTheme()
        if isAdmin {
            label.textColor = NativeStyle.pinGold
            layer?.backgroundColor = NativeStyle.pinGold.withAlphaComponent(0.1).cgColor
            layer?.borderColor = NativeStyle.pinGold.withAlphaComponent(0.55).cgColor
        } else {
            label.textColor = NativeStyle.muted
            layer?.backgroundColor = NSColor.clear.cgColor
            layer?.borderColor = NativeStyle.border.cgColor
        }
    }

    override func layout() {
        super.layout()
        label.frame = NSRect(x: 0, y: 1, width: bounds.width, height: max(0, bounds.height - 2))
    }
}

@MainActor final class MessageCell: NativeView {
    let body = MessageTextView()
    private let author = NativeStyle.label("", size: 13, bold: true)
    private let adminBadge = MessageBadge("ADMIN", admin: true, accessibilityLabel: "Admin")
    private let guestBadge = MessageBadge("GUEST", admin: false, accessibilityLabel: "Guest")
    private let timestamp = NativeStyle.label("", size: 10)
    private let avatar = NativeAvatar()
    private var showsAdmin = false
    private var showsGuest = false
    private var arrivalGeneration = 0
    var openAuthor: ((ChatAuthor, NSView) -> Void)?
    var messageMenu: ((ChatMessage, NSView) -> NSMenu?)?
    var messageAction: ((ChatMessage, MessageAction, NSView) -> Void)?
    var pendingAction: ((PendingMessage, PendingAction) -> Void)?
    var hoverReaction: ((ChatMessage, String, Bool, NSView) -> Void)?
    var menuClosed: (() -> Void)?
    private let controls = MessageActionBar()
    private var tracking: NSTrackingArea?
    private var trackingRect: NSRect?
    private var hovered = false
    private var menuOpen = false
    private var actionActive = false
    private weak var activeMenu: CaperMenu?
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
    private let pendingFailure = NativeStyle.label("", size: 11)
    private lazy var retrySend = NativeButton("Retry send") { [weak self] in
        guard let self, let pending = self.pending else { return }
        self.pendingAction?(pending, .retry)
    }
    private lazy var editPending = NativeButton("Edit") { [weak self] in
        guard let self, let pending = self.pending else { return }
        self.pendingAction?(pending, .edit)
    }
    private lazy var dismissPending = NativeButton("Dismiss") { [weak self] in
        guard let self, let pending = self.pending else { return }
        self.pendingAction?(pending, .dismiss)
    }
    private var pinPending = false
    private var layoutWidth: CGFloat = 0
    private var compact: Bool { layoutWidth < 260 }
    private var textInset: CGFloat { compact ? 20 : 68 }
    private var stackedRecovery: Bool { layoutWidth - textInset - 20 < 132 }
    private var reactionFailureHeight: CGFloat { reactionFailure.isHidden ? 0 : stackedRecovery ? 94 : 66 }
    private var pinFailureHeight: CGFloat { pinFailure.isHidden ? 0 : stackedRecovery ? 94 : 66 }
    private var pendingFailureHeight: CGFloat { pendingFailure.isHidden ? 0 : stackedRecovery ? 94 : 66 }
    private var failureHeight: CGFloat { reactionFailureHeight + pinFailureHeight + pendingFailureHeight }
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
    var makePendingAttachments: ((PendingMessage) -> AppKitAttachmentSurface?)?
    private var quote: AppKitAttachmentSurface?
    var makeReply: ((ChatMessage) -> AppKitAttachmentSurface?)?
    var makePendingReply: ((PendingMessage) -> AppKitAttachmentSurface?)?
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
    let hoverTint = CALayer()
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
    private var pendingAuthor: ChatAuthor?
    private var pendingErrorText: String?
    private var pendingRejected = false
    private var pendingUploadFailed = false
    private var pendingCanEdit = true
    private var grouped = false
    /// The next row groups under this one.
    private var continued = false
    private var viewerID: String?
    private var dayHeight: CGFloat { day.isHidden ? 0 : ceil(day.cell?.cellSize.height ?? 14) + 16 }
    private var unreadHeight: CGFloat { unread.isHidden ? 0 : ceil(unread.cell?.cellSize.height ?? 15) + 16 }
    /// The dividers above the message: its day, then the unread line.
    private var dividerHeight: CGFloat { dayHeight + unreadHeight }
    /// Inter-group breathing room sits outside `messageBounds` so hover, mention,
    /// reply and pin highlights stay vertically symmetric on every row.
    private var groupGap: CGFloat { (grouped || blockedRun != nil || dividerHeight > 0) ? 0 : 12 }
    private var topInset: CGFloat { dividerHeight + groupGap }
    private var messageBounds: NSRect {
        NSRect(x: 0, y: topInset, width: bounds.width, height: max(0, bounds.height - topInset))
    }
    var textTop: CGFloat {
        // Symmetric vertical padding inside the highlight box: 3pt on grouped
        // rows, 4pt above the avatar/header on group starts (20pt header + 2pt gap).
        return topInset + pinHeight + quoteHeight + (grouped ? 3 : compact ? 46 : 26)
    }
    /// Symmetric bottom padding inside the row highlight box.
    private var bottomPadding: CGFloat { grouped ? 3 : 4 }

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
        hoverTint.backgroundColor = NativeStyle.hoverWash.cgColor
        hoverTint.isHidden = true
        mentionBar.backgroundColor = NativeStyle.accent.cgColor
        blockedBar.backgroundColor = NativeStyle.border.cgColor
        blockedBar.isHidden = true
        blockedToggle.alignment = .left; blockedToggle.font = NativeStyle.font(12, bold: true)
        blockedToggle.contentTintColor = NativeStyle.accent
        blockedToggle.layer?.backgroundColor = NSColor.clear.cgColor
        blockedToggle.setAccessibilityIdentifier("blocked-run-toggle")
        blockedToggle.isHidden = true
        [blockedBar, mentionTint, hoverTint, mentionBar, dayLeading, dayTrailing, unreadLeading, unreadTrailing].forEach { layer?.addSublayer($0) }
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
        adminBadge.isHidden = true
        guestBadge.isHidden = true
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
        pendingFailure.textColor = NativeStyle.error
        pendingFailure.maximumNumberOfLines = 2; pendingFailure.lineBreakMode = .byWordWrapping
        for item in [pendingFailure, retrySend, editPending, dismissPending] { item.isHidden = true }
        thread.isHidden = true; thread.alignment = .left
        thread.font = NativeStyle.font(11, bold: true); thread.contentTintColor = NativeStyle.accent
        thread.layer?.backgroundColor = NSColor.clear.cgColor
        [body, author, adminBadge, guestBadge, timestamp, avatar, day, unread, authorAction, avatarAction, blockedToggle, edited, controls,
            reactionFailure, retryReaction, dismissReaction, pinnedBy, pinFailure, retryPin, dismissPin,
            pendingFailure, retrySend, editPending, dismissPending, thread].forEach { addSubview($0) }
        controls.action = { [weak self] action, anchor in
            guard let self, let message = self.message else { return }
            self.messageAction?(message, action, anchor)
        }
        controls.messageMenu = { [weak self] anchor in
            guard let self, let message = self.message else { return nil }
            return self.messageMenu?(message, anchor)
        }
        controls.menuOpened = { [weak self] menu in self?.trackMenu(menu) }
        controls.focusChanged = { [weak self] in self?.updateControlsVisibility() }
        body.messageMenu = { [weak self] in
            guard let self, let message = self.message else { return nil }
            return self.messageMenu?(message, self)
        }
        body.menuOpened = { [weak self] menu in self?.trackMenu(menu) }
        body.revealSpoilers = { [weak self] in self?.revealSpoilers($0) }
        setAccessibilityCustomActions([NSAccessibilityCustomAction(name: "Message actions") { [weak self] in
            guard let self, self.message != nil else { return false }
            self.controls.showMenu(); return true
        }])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// A newly sent or received row fades in and rises smoothly from below with
    /// a brief vertical motion blur, easing out quickly without shifting layout.
    func animateArrival() {
        guard !CaperAccessibility.reduceMotion, let layer else { return }
        arrivalGeneration += 1
        let gen = arrivalGeneration
        let duration: CFTimeInterval = 0.18
        let timing = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)

        layer.removeAnimation(forKey: "caperArrivalOpacity")
        layer.removeAnimation(forKey: "caperArrivalRise")
        layer.removeAnimation(forKey: "caperArrivalBlur")

        let blur = CIFilter(name: "CIMotionBlur")
        if let blur {
            blur.name = "caperSendBlur"
            blur.setValue(0.0, forKey: "inputRadius")
            blur.setValue(Double.pi / 2, forKey: "inputAngle")
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layerUsesCoreImageFilters = true
            layer.filters = [blur]
            CATransaction.commit()
        }

        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in
            Task { @MainActor [weak self] in
                guard let self, self.arrivalGeneration == gen else { return }
                self.layer?.filters = nil
                self.layerUsesCoreImageFilters = false
            }
        }

        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0.0
        fade.toValue = 1.0
        fade.duration = duration
        fade.timingFunction = timing
        layer.add(fade, forKey: "caperArrivalOpacity")

        let rise = CABasicAnimation(keyPath: "transform.translation.y")
        rise.fromValue = 12.0
        rise.toValue = 0.0
        rise.duration = duration
        rise.timingFunction = timing
        layer.add(rise, forKey: "caperArrivalRise")

        if blur != nil {
            let blurAnim = CABasicAnimation(keyPath: "filters.caperSendBlur.inputRadius")
            blurAnim.fromValue = 3.5
            blurAnim.toValue = 0.0
            blurAnim.duration = duration
            blurAnim.timingFunction = timing
            layer.add(blurAnim, forKey: "caperArrivalBlur")
        }

        CATransaction.commit()
    }

    /// Carries an existing row smoothly up from its previous on-screen position
    /// as a new message arrives beneath it.
    func animateGlide(fromOffsetY delta: CGFloat) {
        guard !CaperAccessibility.reduceMotion, let layer else { return }
        let glide = CABasicAnimation(keyPath: "transform.translation.y")
        glide.fromValue = delta
        glide.toValue = 0.0
        glide.duration = 0.18
        glide.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
        layer.add(glide, forKey: "caperArrivalRise")
    }

    func cancelArrivalMotion() {
        arrivalGeneration += 1
        layer?.removeAnimation(forKey: "caperArrivalOpacity")
        layer?.removeAnimation(forKey: "caperArrivalRise")
        layer?.removeAnimation(forKey: "caperArrivalBlur")
        layer?.filters = nil
        layerUsesCoreImageFilters = false
    }

    override func applyTheme() {
        super.applyTheme()
        dayLeading.backgroundColor = NativeStyle.border.cgColor
        dayTrailing.backgroundColor = NativeStyle.border.cgColor
        unreadLeading.backgroundColor = NativeStyle.accent.cgColor
        unreadTrailing.backgroundColor = NativeStyle.accent.cgColor
        hoverTint.backgroundColor = NativeStyle.hoverWash.cgColor
        mentionBar.backgroundColor = NativeStyle.accent.cgColor
        blockedBar.backgroundColor = NativeStyle.border.cgColor
        adminBadge.applyTheme()
        guestBadge.applyTheme()
        body.linkTextAttributes = [.foregroundColor: NativeStyle.link, .cursor: NSCursor.pointingHand]
        if let message {
            body.textStorage?.setAttributedString(bodyText(message))
            layer?.backgroundColor = revealedBlocked ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
            for button in reactions {
                let selected = button.state == .on
                button.layer?.backgroundColor = (selected ? NativeStyle.accent.withAlphaComponent(0.16) : NativeStyle.surface).cgColor
                button.layer?.borderColor = (selected ? NativeStyle.accent : NativeStyle.border).cgColor
            }
        } else if let pending {
            body.textStorage?.setAttributedString(bodyText(pending))
            layer?.backgroundColor = NSColor.clear.cgColor
        } else if let blockedRun {
            layer?.backgroundColor = blockedRun.revealed ? NativeStyle.surface.cgColor : NSColor.clear.cgColor
        }
        body.needsDisplay = true
    }

    func configure(_ pending: PendingMessage, author: ChatAuthor?, grouped: Bool, day: String?, error: String?, rejected: Bool, uploadFailed: Bool, canEdit: Bool) {
        guard self.pending != pending || self.pendingAuthor != author || self.grouped != grouped
            || self.day.stringValue != (day ?? "") || self.pendingErrorText != error
            || self.pendingRejected != rejected || self.pendingUploadFailed != uploadFailed
            || self.pendingCanEdit != canEdit || self.message != nil || self.blockedRun != nil else { return }
        if self.pending?.replyTo != pending.replyTo || self.message != nil {
            quote?.removeFromSuperview()
            quote = makePendingReply?(pending)
            if let quote {
                quote.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(quote, positioned: .below, relativeTo: controls)
            }
        }
        if self.pending?.attachments != pending.attachments || self.pending?.gif != pending.gif
            || self.pending?.embeds != pending.embeds || self.message != nil {
            attachments?.removeFromSuperview()
            attachments = makePendingAttachments?(pending)
            if let attachments {
                attachments.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(attachments, positioned: .below, relativeTo: controls)
            }
        }
        self.pending = pending
        self.pendingAuthor = author
        self.pendingErrorText = error
        self.pendingRejected = rejected
        self.pendingUploadFailed = uploadFailed
        self.pendingCanEdit = canEdit
        self.message = nil
        self.blockedRun = nil
        self.revealedBlocked = false
        self.grouped = grouped
        self.continued = false
        self.showsAdmin = author?.isAdmin == true
        self.showsGuest = author?.isGuest == true
        blockedToggle.isHidden = true
        body.isHidden = false
        controls.isHidden = true
        hovered = false; menuOpen = false; actionActive = false; activeMenu = nil
        controls.setMenuOpen(false); controls.setReactionPickerOpen(false)
        edited.isHidden = true
        thread.isHidden = true
        pinnedBy.isHidden = true
        for item in [reactionFailure, retryReaction, dismissReaction, pinFailure, retryPin, dismissPin] { item.isHidden = true }
        reactions.forEach { $0.removeFromSuperview() }; reactions = []
        self.day.stringValue = day ?? ""
        self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(pending.id)")
        unread.isHidden = true
        setAccessibilityIdentifier("pending-message-\(pending.id)")
        setAccessibilityHelp(nil)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = NSColor.clear.cgColor
        blockedBar.isHidden = true
        mentionTint.isHidden = true
        hoverTint.isHidden = true
        mentionBar.isHidden = true
        CATransaction.commit()
        self.author.stringValue = author?.name ?? "Guest"
        self.author.setAccessibilityElement(true)
        authorAction.isHidden = true
        avatarAction.isHidden = true
        adminBadge.isHidden = grouped || !showsAdmin
        guestBadge.isHidden = grouped || !showsGuest
        timestamp.stringValue = error == nil ? "Sending…" : ""
        body.textStorage?.setAttributedString(bodyText(pending))
        body.configureMentions([])
        body.setAccessibilityIdentifier("pending-text-\(pending.id)")
        avatar.update(
            name: author?.name ?? "Guest",
            avatarId: author?.avatarId,
            picture: grouped ? nil : author?.avatarPicture
        )
        pendingFailure.isHidden = error == nil
        pendingFailure.stringValue = error.map { uploadFailed ? $0 : "\(rejected ? "Not sent." : "Not confirmed yet.") \($0)" } ?? ""
        retrySend.title = uploadFailed ? "Retry" : "Retry send"
        retrySend.isHidden = error == nil || rejected
        editPending.isHidden = error == nil || !rejected || pending.gif != nil
        editPending.isEnabled = canEdit
        editPending.toolTip = canEdit ? nil : "Clear your current draft to edit this message."
        dismissPending.isHidden = error == nil || !rejected
        needsLayout = true
    }

    func configure(_ message: ChatMessage, grouped: Bool, continued: Bool = false, day: String?, newMessages: Bool = false, viewerID: String?, revealedBlocked: Bool, canInteract: Bool, inThread: Bool, inPins: Bool = false, reactionError: String?, pinError: String?, pinPending: Bool, react: ((String, String) -> Void)?) {
        let wasPending = self.pending != nil
        guard wasPending || self.message != message || self.grouped != grouped || self.continued != continued || self.day.stringValue != (day ?? "") || unread.isHidden == newMessages || self.viewerID != viewerID || self.revealedBlocked != revealedBlocked || self.canInteract != canInteract || reactionFailure.stringValue != (reactionError ?? "") || pinFailure.stringValue != (pinError ?? "") || self.pinPending != pinPending else { return }
        let range = body.selectedRange()
        // Another message, or edited text, hides its spoilers again.
        if self.message?.id != message.id || self.message?.content.text != message.content.text { revealedSpoilers = [] }
        if wasPending || self.message?.replyTo != message.replyTo || self.message?.isDeleted != message.isDeleted {
            quote?.removeFromSuperview()
            quote = makeReply?(message)
            if let quote {
                quote.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(quote, positioned: .below, relativeTo: controls)
            }
        }
        if wasPending || self.message?.content.attachments != message.content.attachments || self.message?.content.gif != message.content.gif
            || self.message?.content.embeds != message.content.embeds || self.message?.forward != message.forward || self.message?.isDeleted != message.isDeleted {
            attachments?.removeFromSuperview()
            attachments = makeAttachments?(message)
            if let attachments {
                attachments.heightChanged = { [weak self] in self?.heightChanged?() }
                addSubview(attachments, positioned: .below, relativeTo: controls)
            }
        }
        self.pending = nil
        self.pendingAuthor = nil
        self.pendingErrorText = nil
        self.pendingRejected = false
        self.pendingUploadFailed = false
        self.pendingCanEdit = true
        for item in [pendingFailure, retrySend, editPending, dismissPending] { item.isHidden = true }
        self.message = message; self.grouped = grouped; self.continued = continued; self.viewerID = viewerID
        self.canInteract = canInteract
        self.pinPending = pinPending
        self.showsAdmin = message.author.isAdmin
        self.showsGuest = message.author.isGuest
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
        adminBadge.isHidden = grouped || !showsAdmin
        guestBadge.isHidden = grouped || !showsGuest
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

    /// Pending text uses the exact same 20pt pitch and formatting metrics as
    /// confirmed messages so server confirmation causes zero vertical shift.
    private func bodyText(_ pending: PendingMessage) -> NSAttributedString {
        let text = NSMutableAttributedString(attributedString: NativePresentation.messageText(pending.text, mentions: []))
        text.addAttribute(.foregroundColor, value: NativeStyle.muted, range: NSRange(location: 0, length: text.length))
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
        guard blockedRun != run || self.day.stringValue != (day ?? "") || pending != nil else { return }
        blockedRun = run; message = nil; pending = nil; pendingAuthor = nil; pendingErrorText = nil
        grouped = true; continued = false; showsAdmin = false; showsGuest = false
        self.day.stringValue = day ?? ""; self.day.isHidden = day == nil
        self.day.setAccessibilityIdentifier("date-\(run.id)")
        unread.isHidden = true
        setAccessibilityIdentifier(run.id)
        setAccessibilityHelp(nil)
        body.isHidden = true; body.string = ""
        authorAction.isHidden = true; avatarAction.isHidden = true
        adminBadge.isHidden = true; guestBadge.isHidden = true
        avatar.update(name: "", avatarId: nil, picture: nil)
        controls.isHidden = true; edited.isHidden = true
        hovered = false; menuOpen = false; actionActive = false; activeMenu = nil
        controls.setMenuOpen(false); controls.setReactionPickerOpen(false)
        reactionFailure.isHidden = true; retryReaction.isHidden = true; dismissReaction.isHidden = true
        pinnedBy.isHidden = true; pinFailure.isHidden = true; retryPin.isHidden = true; dismissPin.isHidden = true
        pendingFailure.isHidden = true; retrySend.isHidden = true; editPending.isHidden = true; dismissPending.isHidden = true
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
        mentionTint.isHidden = true; hoverTint.isHidden = true; mentionBar.isHidden = true
        CATransaction.commit()
        needsLayout = true
    }

    func measuredHeight(width: CGFloat) -> CGFloat {
        layoutWidth = width
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
        let avatarMinHeight = grouped ? 0 : topInset + pinHeight + quoteHeight + 4 + 36 + bottomPadding
        let height = max(avatarMinHeight, textTop + textHeight + attachmentHeight + reactionHeight + failureHeight + threadHeight + bottomPadding)
        guard pending == nil else { return height }
        let controlsWidth = min(controls.preferredWidth, max(30, width - 30))
        // A wrapped hover bar must not steal the next grouped row's hit area.
        return controlsWidth < controls.preferredWidth
            ? max(height, topInset + pinHeight + quoteHeight + controls.height(for: controlsWidth) + 11) : height
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
        let dayInset = dayHeight, inset = dividerHeight, contentInset = topInset
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
        // Date dividers and inter-group gaps remain neutral when hovered or tinted.
        let messageRect = messageBounds
        mentionTint.frame = messageRect
        hoverTint.frame = messageRect
        mentionBar.frame = NSRect(x: 0, y: messageRect.minY, width: 3, height: messageRect.height)
        blockedBar.frame = NSRect(x: 0, y: messageRect.minY, width: 2, height: messageRect.height)
        CATransaction.commit()
        if trackingRect != messageRect { updateTrackingAreas() }
        avatar.isHidden = grouped; author.isHidden = grouped; timestamp.isHidden = grouped
        adminBadge.isHidden = grouped || !showsAdmin
        guestBadge.isHidden = grouped || !showsGuest
        if blockedRun != nil {
            blockedToggle.frame = NSRect(x: 18, y: inset + 4, width: bounds.width - 36, height: 36)
            return
        }
        let textWidth = max(40, bounds.width - textInset - 20)
        pinnedBy.frame = NSRect(x: textInset, y: contentInset, width: textWidth, height: pinHeight)
        quote?.frame = NSRect(x: 20, y: contentInset + pinHeight, width: max(40, bounds.width - 40), height: quoteHeight)
        let messageTop = contentInset + pinHeight + quoteHeight
        avatar.frame = NSRect(x: 20, y: messageTop + 4, width: 36, height: 36)
        // Intrinsic size describes the alignment rect, excluding NSTextField's
        // cell insets. Manual frames need cellSize or even short names truncate.
        // Badges and time stay beside the name at every width, as on web and
        // the desktop clients; a long name gives way and ends in "…".
        let timeWidth = timestamp.stringValue.isEmpty ? 0
            : min(ceil(timestamp.cell?.cellSize.width ?? timestamp.intrinsicContentSize.width), max(0, bounds.width - 95))
        let adminWidth: CGFloat = adminBadge.isHidden ? 0 : adminBadge.preferredWidth + 6
        let guestWidth: CGFloat = guestBadge.isHidden ? 0 : guestBadge.preferredWidth + 6
        let trailingWidth = adminWidth + guestWidth + (timeWidth > 0 ? timeWidth + 7 : 0)
        let authorWidth = min(ceil(author.cell?.cellSize.width ?? author.intrinsicContentSize.width), max(0, bounds.width - 88 - trailingWidth))
        // A label's cell draws its text 2 points in; the body has no inset.
        // Start the name 2 points early so it lines up with the message.
        author.frame = NSRect(x: 66, y: messageTop + 4, width: authorWidth, height: 20)
        var headerX = author.frame.maxX
        if !adminBadge.isHidden {
            adminBadge.frame = NSRect(x: headerX + 6, y: messageTop + 6, width: adminBadge.preferredWidth, height: 15)
            headerX = adminBadge.frame.maxX
        }
        if !guestBadge.isHidden {
            guestBadge.frame = NSRect(x: headerX + 6, y: messageTop + 6, width: guestBadge.preferredWidth, height: 15)
            headerX = guestBadge.frame.maxX
        }
        timestamp.frame = NSRect(x: headerX + 7, y: messageTop + 7, width: timeWidth, height: 17)
        // The hover wash and focus ring get room around the name, which
        // otherwise touches their left edge.
        authorAction.frame = author.frame.insetBy(dx: -4, dy: -1)
        avatarAction.frame = avatar.frame
        let controlsWidth = min(controls.preferredWidth, max(30, bounds.width - 30))
        controls.frame = NSRect(x: max(0, bounds.width - controlsWidth - 15), y: messageTop + (grouped ? -2 : 2),
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
        pendingFailure.frame = NSRect(x: textInset, y: pinFailure.frame.minY + pinFailureHeight, width: textWidth, height: 30)
        let retryWidth = max(58, ceil(retrySend.cell?.cellSize.width ?? 76) + 12)
        retrySend.frame = NSRect(x: textInset, y: pendingFailure.frame.maxY + 4, width: retryWidth, height: 24)
        let editOriginX = retrySend.isHidden ? textInset : (stackedRecovery ? textInset : retrySend.frame.maxX + 6)
        let editOriginY = retrySend.isHidden ? pendingFailure.frame.maxY + 4 : retrySend.frame.minY + (stackedRecovery ? 28 : 0)
        editPending.frame = NSRect(x: editOriginX, y: editOriginY, width: 54, height: 24)
        let dismissOriginX = editPending.isHidden ? textInset : (stackedRecovery ? textInset : editPending.frame.maxX + 6)
        let dismissOriginY = editPending.isHidden ? editOriginY : editPending.frame.minY + (stackedRecovery ? 28 : 0)
        dismissPending.frame = NSRect(x: dismissOriginX, y: dismissOriginY, width: 68, height: 24)
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
        let rect = messageBounds
        trackingRect = rect
        let hasTopInset = topInset > 0
        let options: NSTrackingArea.Options = hasTopInset
            ? [.mouseEnteredAndExited, .activeInKeyWindow]
            : [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow]
        let tracking = NSTrackingArea(rect: hasTopInset ? rect : .zero, options: options, owner: self, userInfo: nil)
        self.tracking = tracking; addTrackingArea(tracking)
    }
    override func mouseEntered(with event: NSEvent) { refreshHover() }
    override func mouseExited(with event: NSEvent) {
        if let window, window.isKeyWindow {
            refreshHover()
        } else {
            clearHover()
        }
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            hovered = false; menuOpen = false; actionActive = false; activeMenu = nil
            controls.setMenuOpen(false); controls.setReactionPickerOpen(false)
            updateControlsVisibility()
        }
    }
    /// The message a link, quote or search opened: a selection, as on web —
    /// a 12% accent wash and 3pt edge under its date divider, never an outline.
    func markJumpTarget() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        mentionTint.backgroundColor = NativeStyle.accent.withAlphaComponent(0.12).cgColor
        mentionTint.isHidden = false
        mentionBar.isHidden = false
        CATransaction.commit()
    }
    func clearHover() {
        guard hovered else { return }
        hovered = false; updateControlsVisibility()
    }
    func setActionActive(_ active: Bool) {
        guard actionActive != active else { return }
        actionActive = active
        controls.setReactionPickerOpen(active)
        updateControlsVisibility()
    }
    /// Keeps the row highlighted while a context or options menu is open over it.
    private func trackMenu(_ menu: CaperMenu) {
        activeMenu = menu
        menuOpen = true
        controls.setMenuOpen(true)
        updateControlsVisibility()
        let previous = menu.onClose
        menu.onClose = { [weak self, weak menu] in
            previous?()
            guard let self else { return }
            if self.activeMenu === menu || self.activeMenu == nil {
                self.activeMenu = nil
                self.menuOpen = false
                self.controls.setMenuOpen(false)
                self.updateControlsVisibility()
                if let menuClosed = self.menuClosed { menuClosed() } else { self.refreshHover() }
            }
        }
    }
    /// AppKit sends no mouseExited when scrolling moves a row out from under a
    /// still pointer, so its scroll view re-reads the pointer after each move.
    func refreshHover() {
        guard let window, window.isKeyWindow else { return }
        guard message != nil else { clearHover(); return }
        // A row's visibleRect can be the whole visible history (views need
        // not clip to their bounds), so keep to the message area itself.
        let point = convert(window.mouseLocationOutsideOfEventStream, from: nil)
        let inside = messageBounds.intersection(visibleRect).contains(point)
            || (hovered && !controls.isHidden && controls.frame.contains(point))
        guard inside != hovered else { return }
        hovered = inside; updateControlsVisibility()
    }
    /// Exposed for view tests; true when hover, focus, menu or reaction picker marks this row.
    var isRowHighlighted: Bool { message != nil && (hovered || controls.hasFocus || menuOpen || actionActive) }
    private func updateControlsVisibility() {
        let lit = isRowHighlighted
        if lit, controls.alphaValue == 0 { controls.refreshQuick() }
        controls.alphaValue = lit ? 1 : 0
        CATransaction.begin(); CATransaction.setDisableActions(true)
        hoverTint.isHidden = !lit
        CATransaction.commit()
    }
    override func rightMouseDown(with event: NSEvent) {
        guard let message, let menu = messageMenu?(message, self) else { super.rightMouseDown(with: event); return }
        if let shown = CaperMenu.show(menu, at: convert(event.locationInWindow, from: nil), in: self) {
            trackMenu(shown)
        }
    }
}

/// TextKit still owns selection, including drags starting on a mention. Only a
/// single stationary click on an actual resolved-person glyph opens a card.
@MainActor final class MessageTextView: NSTextView {
    var openMention: ((MessageMention, NSView, NSRect) -> Void)?
    var messageMenu: (() -> NSMenu?)?
    var menuOpened: ((CaperMenu) -> Void)?
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
        if let shown = CaperMenu.show(items + CaperMenuItem.items(from: actions), at: convert(event.locationInWindow, from: nil), in: self) {
            menuOpened?(shown)
        }
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
    var menuOpened: ((CaperMenu) -> Void)?
    var focusChanged: (() -> Void)?
    private var menuOpen = false
    private var reactionPickerOpen = false
    private var quick: [NativeButton] = []
    /// The account's quick reactions the buttons show (`NativePresentation.quickReactions`).
    private var quickEmoji: [String] = []
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
        quick = (0..<3).map { index in
            NativeButton("") { [weak self] in
                guard let self, self.quickEmoji.indices.contains(index) else { return }
                self.action?(.reaction(self.quickEmoji[index]), self)
            }
        }
        showQuick(NativePresentation.quickReactions)
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
        applyTriggerStates()
    }

    func setMenuOpen(_ open: Bool) {
        guard menuOpen != open else { return }
        menuOpen = open
        applyTriggerStates()
    }

    func setReactionPickerOpen(_ open: Bool) {
        guard reactionPickerOpen != open else { return }
        reactionPickerOpen = open
        applyTriggerStates()
    }

    private func applyTriggerStates() {
        for (button, active) in [(more, menuOpen), (add, reactionPickerOpen)] {
            button.layer?.backgroundColor = (active ? NativeStyle.accent.withAlphaComponent(0.16) : .clear).cgColor
            button.layer?.borderWidth = active ? 1 : 0
            button.layer?.borderColor = (active ? NativeStyle.accent : .clear).cgColor
            button.contentTintColor = active ? NativeStyle.text : NativeStyle.muted
        }
    }

    /// Dresses the quick-reaction buttons for `emoji` (the account's most used).
    private func showQuick(_ emoji: [String]) {
        quickEmoji = emoji
        for (emoji, button) in zip(emoji, quick) {
            let artwork = NativePresentation.emojiImage(emoji)
            button.title = artwork == nil ? emoji : ""
            button.image = artwork.map { NSImage(cgImage: $0, size: NSSize(width: 18, height: 18)) }
            button.imagePosition = artwork == nil ? .noImage : .imageOnly
            button.setAccessibilityLabel("React with \(emoji)"); button.toolTip = "React with \(emoji)"
        }
    }

    private var configured: (message: ChatMessage, canInteract: Bool, viewerID: String?)?

    /// As the bar appears: a newer quick-reaction list (never while it shows,
    /// so a button can't change under the pointer).
    func refreshQuick() {
        guard quickEmoji != NativePresentation.quickReactions, let configured else { return }
        showQuick(NativePresentation.quickReactions)
        configure(configured.message, canInteract: configured.canInteract, viewerID: configured.viewerID)
    }

    func configure(_ message: ChatMessage, canInteract: Bool, viewerID: String?) {
        configured = (message, canInteract, viewerID)
        if alphaValue == 0, quickEmoji != NativePresentation.quickReactions { showQuick(NativePresentation.quickReactions) }
        for (emoji, button) in zip(quickEmoji, quick) {
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
        guard let menu = messageMenu?(more), let shown = CaperMenu.show(menu, below: more) else { return }
        menuOpened?(shown)
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
