import CaperCore
import SwiftUI
import UIKit

/// One timeline row's display state. Built per snapshot from the model; cells
/// read it only when they are realized.
struct MessageRowState: Equatable {
    var id: String
    /// The server's id; pending rows have none.
    var messageID: String
    var authorName: String
    var avatarID: Int?
    var picture: CaperPicture?
    var createdAt: String
    var text: String
    var mentions: [MessageMention]
    var grouped: Bool
    var continued: Bool
    var mentionsViewer: Bool
    var edited: Bool
    var deleted: Bool
    /// Files, a GIF, link cards or a forward, drawn by CaperCore's SwiftUI leaves.
    var extras: ChatMessage?
    var pendingAttachments: [ChatAttachment]
    var reactions: [ReactionChipsView.Chip] = []
    var canInteract = false
    /// A failed reaction or pin, with Retry and Dismiss.
    var reactionError: String?
    var pinError: String?
    /// The message this one replies to, shown above it.
    var replyTo: MessageReply?
    /// Under a channel row: its thread's replies, or "Replied to a thread".
    var thread: ThreadLink?
    /// Opened from the thread view: no thread link of its own.
    var inThread = false
    var pending: Bool
    var pendingError: String?
    var pendingRejected: Bool
    /// "you" or the pinner's name, on a pinned message.
    var pinnedBy: String?
    var pinMine = false
    /// The jump target (a link or Go to message): terracotta, like web.
    var focused = false
    /// A blocked author's message the reader chose to show.
    var revealedBlocked = false
    /// The avatar and name open the author's card; guests have none.
    var authorProfile = false
    /// Pins list rows: the full date, and the card paints the background.
    var inPins = false

    @MainActor
    init(message: ChatMessage, grouped: Bool, continued: Bool, viewerID: String?, focused: Bool = false, inPins: Bool = false,
         chat: ChatModel? = nil, inThread: Bool = false) {
        id = message.clientMessageId
        messageID = message.id
        authorName = message.author.name
        authorProfile = !message.author.isGuest
        avatarID = message.author.avatarId
        picture = message.author.avatarPicture
        createdAt = message.createdAt
        text = message.isDeleted ? "" : message.content.text
        mentions = message.content.mentions
        self.grouped = grouped
        self.continued = continued
        mentionsViewer = NativePresentation.mentionsViewer(message, viewerID: viewerID)
        edited = message.forward == nil && !message.isDeleted && (message.revision ?? 1) > 1
        deleted = message.isDeleted
        extras = UIKitMessageExtras.shows(message) ? message : nil
        pendingAttachments = []
        pending = false
        pendingError = nil
        pendingRejected = false
        if let pin = message.pin, !message.isDeleted {
            pinMine = pin.author.id == viewerID
            pinnedBy = pinMine ? "you" : pin.author.name
        }
        self.focused = focused && !inPins
        self.inPins = inPins
        if !message.isDeleted {
            reactions = (message.reactions ?? []).map { reaction in
                .init(emoji: reaction.emoji, count: reaction.authorIds.count, own: viewerID.map(reaction.authorIds.contains) ?? false)
            }
        }
        self.inThread = inThread
        replyTo = message.isDeleted ? nil : message.replyTo
        if !inThread && !inPins {
            if message.threadRootId == nil, let summary = message.thread, summary.replyCount > 0 {
                thread = ThreadLink(rootID: message.id, replyCount: summary.replyCount,
                                    participants: summary.participants, unread: chat?.threadUnread(message.id) ?? false)
            } else if let root = message.threadRootId {
                thread = ThreadLink(rootID: root, replyCount: 0, participants: [], unread: chat?.threadUnread(root) ?? false)
            }
        }
        if let chat {
            canInteract = chat.canInteract
            reactionError = chat.reactionErrors[message.id]
            pinError = message.isDeleted ? nil : chat.pinErrors[message.id]
        }
    }

    init(pending: PendingMessage, author: ChatAuthor?, grouped: Bool, error: String?, rejected: Bool) {
        id = pending.id
        messageID = ""
        authorName = author?.name ?? "Guest"
        avatarID = author?.avatarId
        picture = author?.avatarPicture
        createdAt = pending.createdAt
        text = pending.text
        mentions = []
        self.grouped = grouped
        continued = false
        mentionsViewer = false
        edited = false
        deleted = false
        extras = nil
        pendingAttachments = pending.attachments
        self.pending = true
        pendingError = error
        pendingRejected = rejected
    }
}

/// Selectable message text drawn by TextKit 2. It sizes to its text, so the
/// collection view measures a row only when the row is realized.
final class MessageTextView: UITextView, NSTextLayoutManagerDelegate, UITextViewDelegate {
    /// The message text without decorative link symbols, for copying and VoiceOver.
    var plainText: String { LinkPills.plain(text ?? "") }
    /// A tapped person pill: its user id.
    var openMention: ((String) -> Void)?
    static let mentionScheme = "caper-mention"

    static func mentionID(_ url: URL) -> String? {
        url.scheme == mentionScheme ? URLComponents(url: url, resolvingAgainstBaseURL: false)?.path : nil
    }

    func textView(_ textView: UITextView, primaryActionFor textItem: UITextItem, defaultAction: UIAction) -> UIAction? {
        guard case let .link(url) = textItem.content, let id = Self.mentionID(url) else { return defaultAction }
        return UIAction { [weak self] _ in self?.openMention?(id) }
    }

    func textView(_ textView: UITextView, menuConfigurationFor textItem: UITextItem, defaultMenu: UIMenu) -> UITextItem.MenuConfiguration? {
        // A held pill opens message actions, like any held text.
        if case let .link(url) = textItem.content, Self.mentionID(url) != nil { return nil }
        return .init(menu: defaultMenu)
    }

    init() {
        super.init(frame: .zero, textContainer: nil)
        // TextKit 2; never touch layoutManager, which would fall back to TextKit 1.
        // Lines sit at ascent + descent without the font's line gap, as SwiftUI's Text sets them.
        textLayoutManager?.usesFontLeading = false
        textLayoutManager?.delegate = self
        isEditable = false
        isSelectable = true
        isScrollEnabled = false
        showsVerticalScrollIndicator = false
        showsHorizontalScrollIndicator = false
        backgroundColor = .clear
        textContainerInset = .zero
        textContainer.lineFragmentPadding = 0
        dataDetectorTypes = []
        adjustsFontForContentSizeCategory = false
        // Links carry their own colour, so person pills (links too) keep theirs.
        linkTextAttributes = [:]
        delegate = self
        translatesAutoresizingMaskIntoConstraints = false
        setContentCompressionResistancePriority(.required, for: .vertical)
        accessibilityTraits.insert(.staticText)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func textLayoutManager(_ textLayoutManager: NSTextLayoutManager, textLayoutFragmentFor location: NSTextLocation,
                           in textElement: NSTextElement) -> NSTextLayoutFragment {
        let range = textElement.elementRange
        guard let paragraph = textElement as? NSTextParagraph,
              paragraph.attributedString.length > 0 else { return NSTextLayoutFragment(textElement: textElement, range: range) }
        var hasPill = false
        paragraph.attributedString.enumerateAttribute(.caperLinkPill, in: NSRange(location: 0, length: paragraph.attributedString.length)) { value, _, stop in
            if value != nil { hasPill = true; stop.pointee = true }
        }
        return hasPill ? LinkPillFragment(textElement: textElement, range: range) : NSTextLayoutFragment(textElement: textElement, range: range)
    }

    override func copy(_ sender: Any?) {
        guard let range = selectedTextRange, let selected = text(in: range) else { return super.copy(sender) }
        UIPasteboard.general.string = LinkPills.plain(selected)
    }
}

/// A row's link into its thread.
struct ThreadLink: Equatable {
    let rootID: String
    /// Zero on a reply sent to the channel too: "Replied to a thread".
    let replyCount: Int
    let participants: [ChatAuthor]
    let unread: Bool
}

/// What a row's hosted SwiftUI leaves need from the conversation.
struct RowHost {
    let chat: ChatModel
    let present: UIKitHosting.Present
    let openForward: (ChatMessage) -> Void
    /// The long-press sheet; `true` opens straight to the emoji picker.
    var actions: (_ messageID: String, _ picker: Bool) -> Void = { _, _ in }
    var reactors: (_ messageID: String, _ emoji: String) -> Void = { _, _ in }
    var openThread: (_ rootID: String) -> Void = { _ in }
    /// A person pill or the author's avatar or name: their profile card.
    var openMention: (_ messageID: String, _ userID: String) -> Void = { _, _ in }
    var openAuthor: (_ messageID: String) -> Void = { _ in }
}

/// One message: avatar or grouped gutter, header, text and its notes. The
/// timeline's cells and the Pins cards host it.
final class MessageRowView: UIView, UIGestureRecognizerDelegate {
    private let avatar = AvatarView(side: 34)
    private let gutter = UIView()
    private let name = UIKitStyle.label(size: 13, weight: .bold)
    private let time = UIKitStyle.label(size: 10, color: UIKitStyle.muted)
    private let edited = UIKitStyle.label("(edited)", size: 10, color: UIKitStyle.muted)
    private let header = UIStackView()
    private let body = MessageTextView()
    private let note = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let extras = HostedView()
    private let chips = ReactionChipsView()
    private let errors = UIStackView()
    private let errorText = UIKitStyle.label(size: 11, weight: .medium, color: UIKitStyle.error)
    private lazy var errorRetry = CaperButton("Retry", kind: .plain, size: 11) { [weak self] in self?.retryError() }
    private lazy var errorDismiss = CaperButton("Dismiss", kind: .plain, size: 11) { [weak self] in self?.dismissError() }
    /// The text column's width, for laying out reaction chips.
    var columnWidth: CGFloat = 0
    /// The model and a presenter for the hosted extras (media viewer, forwards).
    var host: RowHost?
    private let column = UIStackView()
    private let accent = UIView()
    private let tint = UIView()
    private let pinBadge = UIKitStyle.label(size: 10, weight: .medium, color: UIKitStyle.muted)
    private let replyContext = HostedView()
    private let above = UIStackView()
    private let threadLink = ThreadLinkButton()
    private var top: NSLayoutConstraint!
    private var badgeTop: NSLayoutConstraint!
    private var columnUnderBadge: NSLayoutConstraint!
    private var bottom: NSLayoutConstraint!
    private var avatarWidth: NSLayoutConstraint!
    private var state: MessageRowState?
    private var authorTaps: [UITapGestureRecognizer] = []
    var retry: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        tint.translatesAutoresizingMaskIntoConstraints = false
        accent.translatesAutoresizingMaskIntoConstraints = false
        accent.backgroundColor = UIKitStyle.terracotta
        self.addSubview(tint)
        self.addSubview(accent)

        header.axis = .horizontal
        header.alignment = .firstBaseline
        header.spacing = 7
        [name, time, edited].forEach(header.addArrangedSubview)
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        name.lineBreakMode = .byTruncatingTail
        time.setContentHuggingPriority(.required, for: .horizontal)
        edited.setContentHuggingPriority(.required, for: .horizontal)
        header.addArrangedSubview(UIView())
        // VoiceOver reads the header as one element: who and when.
        header.isAccessibilityElement = true
        header.accessibilityTraits = .header

        note.numberOfLines = 0
        column.axis = .vertical
        column.spacing = 4
        column.alignment = .fill
        errorText.numberOfLines = 0
        [errorText, errorRetry, errorDismiss].forEach(errors.addArrangedSubview)
        errors.spacing = 8
        errors.alignment = .firstBaseline
        errors.isHidden = true
        [header, body, extras, chips, errors, threadLink, note].forEach(column.addArrangedSubview)
        // Holding a row opens its actions, as on SwiftUI's iPhone layout.
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(held(_:)))
        hold.minimumPressDuration = 0.5
        hold.delegate = self
        addGestureRecognizer(hold)
        // The avatar and name open the author's card; pills open theirs.
        for view in [gutter, name] as [UIView] {
            let tap = UITapGestureRecognizer(target: self, action: #selector(openAuthor))
            view.addGestureRecognizer(tap)
            view.isUserInteractionEnabled = true
            authorTaps.append(tap)
        }
        body.openMention = { [weak self] id in
            guard let self, let messageID = self.state?.messageID, !messageID.isEmpty else { return }
            self.host?.openMention(messageID, id)
        }
        chips.toggle = { [weak self] emoji in self?.toggle(emoji) }
        chips.showReactors = { [weak self] emoji in
            guard let self, let id = self.state?.messageID, !id.isEmpty else { return }
            self.host?.reactors(id, emoji)
        }
        chips.addReaction = { [weak self] in
            guard let self, let id = self.state?.messageID, !id.isEmpty else { return }
            self.host?.actions(id, true)
        }
        column.translatesAutoresizingMaskIntoConstraints = false

        gutter.translatesAutoresizingMaskIntoConstraints = false
        gutter.addSubview(avatar)
        // The header names the author; the picture adds nothing for VoiceOver.
        gutter.accessibilityElementsHidden = true
        self.addSubview(gutter)
        self.addSubview(column)
        pinBadge.isHidden = true
        replyContext.isHidden = true
        above.axis = .vertical
        above.alignment = .leading
        above.spacing = 5
        above.translatesAutoresizingMaskIntoConstraints = false
        [pinBadge, replyContext].forEach(above.addArrangedSubview)
        replyContext.widthAnchor.constraint(equalTo: above.widthAnchor).isActive = true
        self.addSubview(above)
        threadLink.addAction(UIAction { [weak self] _ in
            guard let self, let link = self.state?.thread else { return }
            self.host?.openThread(link.rootID)
        }, for: .touchUpInside)

        top = column.topAnchor.constraint(equalTo: self.topAnchor, constant: 10)
        badgeTop = above.topAnchor.constraint(equalTo: self.topAnchor, constant: 10)
        columnUnderBadge = column.topAnchor.constraint(equalTo: above.bottomAnchor, constant: 5)
        bottom = column.bottomAnchor.constraint(equalTo: self.bottomAnchor, constant: -10)
        bottom.priority = .init(999)
        avatarWidth = gutter.widthAnchor.constraint(equalToConstant: 34)
        NSLayoutConstraint.activate([
            tint.leadingAnchor.constraint(equalTo: self.leadingAnchor),
            tint.trailingAnchor.constraint(equalTo: self.trailingAnchor),
            tint.topAnchor.constraint(equalTo: self.topAnchor),
            tint.bottomAnchor.constraint(equalTo: self.bottomAnchor),
            accent.leadingAnchor.constraint(equalTo: self.leadingAnchor),
            accent.widthAnchor.constraint(equalToConstant: 3),
            accent.topAnchor.constraint(equalTo: self.topAnchor),
            accent.bottomAnchor.constraint(equalTo: self.bottomAnchor),
            gutter.leadingAnchor.constraint(equalTo: self.leadingAnchor, constant: 18),
            gutter.topAnchor.constraint(equalTo: column.topAnchor),
            avatarWidth,
            avatar.topAnchor.constraint(equalTo: gutter.topAnchor),
            avatar.leadingAnchor.constraint(equalTo: gutter.leadingAnchor),
            gutter.heightAnchor.constraint(greaterThanOrEqualToConstant: 1),
            column.leadingAnchor.constraint(equalTo: gutter.trailingAnchor, constant: 10),
            column.trailingAnchor.constraint(equalTo: self.trailingAnchor, constant: -18),
            above.leadingAnchor.constraint(equalTo: column.leadingAnchor),
            above.trailingAnchor.constraint(equalTo: column.trailingAnchor),
            badgeTop, top, bottom,
            self.heightAnchor.constraint(greaterThanOrEqualTo: gutter.heightAnchor),
        ])
        // The row contains its header and text as separate VoiceOver elements,
        // so readers can move into the text and select it.
        isAccessibilityElement = false
        accessibilityContainerType = .semanticGroup
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ row: MessageRowState) {
        state = row
        authorTaps.forEach { $0.isEnabled = row.authorProfile && !row.pending }
        // SwiftUI's grouped rows read about 4pt further apart than their 2pt padding.
        let topPadding: CGFloat = row.grouped ? 6 : 10
        top.constant = topPadding
        badgeTop.constant = topPadding
        bottom.constant = row.continued ? -2 : -10
        // "Pinned by …" heads the row, over the text column.
        pinBadge.isHidden = row.pinnedBy == nil
        if let host, let reply = row.replyTo {
            replyContext.show(UIKitReplyContext(reference: reply, motionKey: row.id, chat: host.chat))
            replyContext.isHidden = false
        } else {
            replyContext.show(Optional<EmptyView>.none)
            replyContext.isHidden = true
        }
        let hasAbove = !pinBadge.isHidden || !replyContext.isHidden
        // Deactivate before activating, so the two never conflict for a frame.
        if hasAbove { top.isActive = false; columnUnderBadge.isActive = true }
        else { columnUnderBadge.isActive = false; top.isActive = true }
        if let pinnedBy = row.pinnedBy {
            pinBadge.attributedText = Self.pinBadge("Pinned by \(pinnedBy)")
            pinBadge.accessibilityLabel = "Pinned by \(pinnedBy)"
        }
        header.isHidden = row.grouped
        avatar.isHidden = row.grouped
        if !row.grouped { avatar.show(name: row.authorName, avatarID: row.avatarID, picture: row.picture) }
        name.text = row.authorName
        let timeText = row.inPins ? Self.fullDate(row.createdAt) : NativePresentation.time(row.createdAt)
        time.text = row.pending ? (row.pendingError == nil ? "Sending…" : "") : timeText
        edited.isHidden = !row.edited
        header.accessibilityLabel = [row.authorName, row.pending ? "sending" : timeText, row.edited ? "edited" : ""]
            .filter { !$0.isEmpty }.joined(separator: ", ")

        tint.backgroundColor = Self.background(row)
        let highlighted = row.focused || (row.pinnedBy == nil && row.mentionsViewer)
        accent.isHidden = !(highlighted || row.revealedBlocked)
        accent.backgroundColor = highlighted ? UIKitStyle.terracotta : UIKitStyle.border

        if let host, let message = row.extras {
            extras.show(UIKitMessageExtras(message: message, chat: host.chat, present: host.present, openForward: { host.openForward(message) }))
        } else if let host, !row.pendingAttachments.isEmpty {
            extras.show(UIKitMessageExtras(pending: row.pendingAttachments, chat: host.chat, present: host.present))
        } else {
            extras.show(Optional<EmptyView>.none)
        }
        extras.isHidden = row.extras == nil && row.pendingAttachments.isEmpty
        chips.configure(row.reactions, messageID: row.messageID, canInteract: row.canInteract, width: max(columnWidth, 120))
        chipsWidth = columnWidth
        errorText.text = row.reactionError ?? row.pinError
        threadLink.isHidden = row.thread == nil
        if let link = row.thread { threadLink.configure(link) }
        errors.isHidden = errorText.text == nil
        errorRetry.isEnabled = row.canInteract

        if row.deleted {
            body.isHidden = true
            note.isHidden = false
            note.text = "This message was deleted."
        } else {
            body.isHidden = row.text.isEmpty
            if !row.text.isEmpty { body.attributedText = Self.render(row) }
            var lines: [String] = []
            if let error = row.pendingError { lines.append("\(row.pendingRejected ? "Not sent." : "Not confirmed yet.") \(error)") }
            note.isHidden = lines.isEmpty
            note.text = lines.joined(separator: "\n")
            note.textColor = row.pendingError == nil ? UIKitStyle.muted : UIKitStyle.error
        }
        self.alpha = row.pending && row.pendingError == nil ? 0.6 : 1
        body.accessibilityLabel = row.grouped ? "\(row.authorName), \(timeText): \(body.plainText)" : body.plainText
        body.accessibilityIdentifier = "message-text-\(row.id)"
        body.accessibilityCustomActions = actions(for: row)
    }

    private var chipsWidth: CGFloat = 0

    override func layoutSubviews() {
        super.layoutSubviews()
        // A rotation or a different card width rewraps the chips.
        let width = column.bounds.width
        if width > 0, abs(width - chipsWidth) > 0.5, let state {
            columnWidth = width
            chips.configure(state.reactions, messageID: state.messageID, canInteract: state.canInteract, width: width)
            chipsWidth = width
        }
    }

    /// A hold opens message actions rather than the text's loupe; double-tap
    /// still selects words, and the sheet offers Copy text.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
        other is UILongPressGestureRecognizer && other.view?.isDescendant(of: body) == true
    }

    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard gestureRecognizer.view === self else { return super.gestureRecognizerShouldBegin(gestureRecognizer) }
        // Chips have their own hold (who reacted).
        let point = gestureRecognizer.location(in: chips)
        return chips.isHidden || !chips.bounds.contains(point)
    }

    @objc private func openAuthor() {
        guard let state, state.authorProfile, !state.messageID.isEmpty else { return }
        host?.openAuthor(state.messageID)
    }

    @objc private func held(_ gesture: UILongPressGestureRecognizer) {
        guard gesture.state == .began, let state, !state.pending, !state.messageID.isEmpty else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        host?.actions(state.messageID, false)
    }

    private func toggle(_ emoji: String) {
        guard let state, let chat = host?.chat, !state.messageID.isEmpty, state.canInteract else { return }
        let own = state.reactions.first { $0.emoji == emoji }?.own ?? false
        Task { await chat.setReaction(messageID: state.messageID, emoji: emoji, active: !own) }
    }

    private func retryError() {
        guard let state, let chat = host?.chat else { return }
        if state.reactionError != nil { Task { await chat.retryReaction(messageID: state.messageID) } }
        else if state.pinError != nil { Task { await chat.retryPin(messageID: state.messageID) } }
    }

    private func dismissError() {
        guard let state, let chat = host?.chat else { return }
        if state.reactionError != nil { chat.reactionErrors[state.messageID] = nil }
        else { chat.pinErrors[state.messageID] = nil }
    }

    /// SwiftUI's rowBackground: the jump target, then pins, then mentions of you.
    private static func background(_ row: MessageRowState) -> UIColor {
        if row.inPins { return .clear }
        if row.focused { return UIKitStyle.terracotta.withAlphaComponent(0.12) }
        if row.pinnedBy != nil { return row.pinMine ? UIKitStyle.terracotta.withAlphaComponent(0.12) : UIKitStyle.pinGold.withAlphaComponent(0.06) }
        if row.mentionsViewer { return UIKitStyle.terracotta.withAlphaComponent(0.08) }
        return row.revealedBlocked ? UIKitStyle.surface : .clear
    }

    private static func pinBadge(_ text: String) -> NSAttributedString {
        let result = NSMutableAttributedString()
        if let icon = UIKitStyle.icon("pin")?.resized(to: 12).withTintColor(UIKitStyle.muted, renderingMode: .alwaysOriginal) {
            let attachment = NSTextAttachment(image: icon)
            attachment.bounds = CGRect(x: 0, y: -2, width: 12, height: 12)
            result.append(NSAttributedString(attachment: attachment))
            result.append(NSAttributedString(string: " "))
        }
        result.append(NSAttributedString(string: text))
        return result
    }

    /// Pins show when, not just the time: "Nov 5, 2026 at 2:14 AM".
    static func fullDate(_ value: String) -> String {
        guard let date = UIKitStyle.date(value) else { return NativePresentation.time(value) }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    /// Plain runs in the body colour; pills and links keep their own.
    private static func render(_ row: MessageRowState) -> NSAttributedString {
        // Satoshi 14 at the reader's text size, as SwiftUI's custom fonts scale.
        let size = UIFontMetrics.default.scaledValue(for: 14)
        let text = NSMutableAttributedString(attributedString: NativePresentation.messageText(row.text, size: size, mentions: row.mentions))
        text.enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if value == nil { text.addAttribute(.foregroundColor, value: row.pending ? UIKitStyle.muted : UIKitStyle.body, range: range) }
        }
        text.enumerateAttribute(.link, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if value != nil { text.addAttribute(.foregroundColor, value: UIKitStyle.link, range: range) }
        }
        LinkPills.decorate(text, size: size)
        // Resolved people open their card; after the link pills, which they never get.
        guard !row.pending else { return text }
        text.enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            guard let id = value as? String, !id.isEmpty else { return }
            var components = URLComponents()
            components.scheme = MessageTextView.mentionScheme
            components.path = id
            if let url = components.url { text.addAttribute(.link, value: url, range: range) }
        }
        return text
    }

    private func actions(for row: MessageRowState) -> [UIAccessibilityCustomAction] {
        var actions: [UIAccessibilityCustomAction] = []
        if !row.text.isEmpty {
            actions.append(UIAccessibilityCustomAction(name: "Copy text") { [weak self] _ in
                UIPasteboard.general.string = self?.body.plainText
                return true
            })
        }
        if row.authorProfile, !row.messageID.isEmpty {
            actions.append(UIAccessibilityCustomAction(name: "Open profile for \(row.authorName)") { [weak self] _ in
                self?.host?.openAuthor(row.messageID); return true
            })
        }
        let shown = body.attributedText ?? NSAttributedString()
        shown.enumerateAttribute(.link, in: NSRange(location: 0, length: shown.length)) { value, range, _ in
            guard let url = value as? URL else { return }
            if let id = MessageTextView.mentionID(url) {
                let name = shown.attributedSubstring(from: range).string.trimmingCharacters(in: CharacterSet(charactersIn: "@ "))
                actions.append(UIAccessibilityCustomAction(name: "Open profile for \(name)") { [weak self] _ in
                    self?.host?.openMention(row.messageID, id); return true
                })
                return
            }
            actions.append(UIAccessibilityCustomAction(name: "Open \(url.host ?? url.absoluteString)") { _ in
                UIApplication.shared.open(url)
                return true
            })
        }
        if row.pendingError != nil, !row.pendingRejected, let retry {
            actions.append(UIAccessibilityCustomAction(name: "Retry send") { _ in retry(); return true })
        }
        // As SwiftUI's "Message actions": the long-press sheet.
        if !row.pending, !row.messageID.isEmpty {
            actions.append(UIAccessibilityCustomAction(name: "Message actions") { [weak self] _ in
                self?.host?.actions(row.messageID, false); return true
            })
        }
        return actions
    }

    override var accessibilityElements: [Any]? {
        get { [pinBadge, replyContext, header, body, extras, chips, errors, threadLink, note].filter { !$0.isHidden } }
        set {}
    }

    /// Messages that arrive while the reader watches the live end rise in;
    /// everything else, and everything with Reduce Motion, appears in place.
    func arrive() {
        guard !UIKitStyle.reduceMotion else { return }
        self.alpha = 0
        self.transform = CGAffineTransform(translationX: 0, y: 8)
        let target = state?.pending == true && state?.pendingError == nil ? 0.6 : 1
        let animator = UIViewPropertyAnimator(duration: UIKitStyle.motionFast, timingParameters: UIKitStyle.easeOut)
        animator.addAnimations { [weak self] in
            self?.alpha = target
            self?.transform = .identity
        }
        animator.startAnimation()
    }

    func prepareForReuse() {
        self.layer.removeAllAnimations()
        self.transform = .identity
        retry = nil
    }
}

final class MessageCell: UICollectionViewCell {
    static let reuse = "message"
    let row = MessageRowView()
    var retry: (() -> Void)? {
        get { row.retry }
        set { row.retry = newValue }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        row.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: contentView.topAnchor),
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            row.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
        isAccessibilityElement = false
        contentView.isAccessibilityElement = false
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ state: MessageRowState) {
        row.configure(state)
        accessibilityIdentifier = state.pending ? "pending-message-\(state.id)" : "message-row-\(state.id)"
    }

    func arrive() { row.arrive() }

    override func prepareForReuse() {
        super.prepareForReuse()
        row.prepareForReuse()
    }
}


/// "Beginning of conversation", a history spinner, or a retry after a failed page.
final class TimelineEdgeCell: UICollectionViewCell {
    static let reuse = "edge"
    private let label = UIKitStyle.label(size: 11, weight: .medium, color: UIKitStyle.muted)
    private let button = CaperButton("Retry", kind: .plain, size: 11) {}
    var retry: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        let stack = UIStackView(arrangedSubviews: [label, button])
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        button.addAction(UIAction { [weak self] _ in self?.retry?() }, for: .primaryActionTriggered)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: contentView.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            contentView.heightAnchor.constraint(equalToConstant: 44),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ edge: TimelineEdge) {
        switch edge {
        case .beginning: label.text = "Beginning of conversation"
        case .loading: label.text = "Loading older messages…"
        case .failed: label.text = "Couldn’t load older messages."
        }
        button.isHidden = edge != .failed
        accessibilityIdentifier = "timeline-edge"
    }
}

enum TimelineEdge: Hashable { case beginning, loading, failed }

/// A full-date divider between days, as on every client.
/// What a blocked-run row shows; Equatable so unchanged runs aren't reconfigured.
struct BlockedRunState: Equatable {
    let key: String
    let label: String
    let revealed: Bool
    let ids: [String]

    init(entry: UIKitTimelineEntry) {
        key = "blocked-\(entry.messages.first?.id ?? "")"
        label = entry.label; revealed = entry.revealed; ids = entry.messages.map(\.id)
    }

    func toggling(_ revealed: Set<String>) -> Set<String> {
        self.revealed ? revealed.subtracting(ids) : revealed.union(ids)
    }
}

/// (Ban icon) "2 blocked messages — Show": a run of blocked authors' messages.
final class BlockedRunCell: UICollectionViewCell {
    static let reuse = "blocked-run"
    private let label = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let toggle = UIButton(type: .system)
    private var action: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        let icon = UIImageView(image: UIKitStyle.icon("ban")?.resized(to: 16))
        icon.tintColor = UIKitStyle.muted
        icon.isAccessibilityElement = false
        label.isAccessibilityElement = false
        toggle.titleLabel?.font = UIKitStyle.font(12, .bold)
        toggle.setTitleColor(UIKitStyle.terracottaBright, for: .normal)
        toggle.accessibilityIdentifier = "blocked-run-toggle"
        toggle.addAction(UIAction { [weak self] _ in self?.action?() }, for: .primaryActionTriggered)
        let stack = UIStackView(arrangedSubviews: [icon, label, toggle, UIView()])
        stack.spacing = 6
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 18),
            stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -18),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 4),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -4),
            toggle.heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ run: BlockedRunState, toggle action: @escaping () -> Void) {
        self.action = action
        label.text = "\(run.label) —"
        let title = run.revealed ? "Hide" : "Show"
        toggle.setTitle(title, for: .normal)
        toggle.accessibilityLabel = "\(title) \(run.label)"
        contentView.backgroundColor = run.revealed ? UIKitStyle.surface : .clear
        accessibilityIdentifier = run.key
    }
}

final class DayDividerCell: UICollectionViewCell {
    static let reuse = "day"
    private let label = UIKitStyle.label(size: 10, weight: .bold, color: UIKitStyle.muted)

    override init(frame: CGRect) {
        super.init(frame: frame)
        let left = UIView(), right = UIView()
        for line in [left, right] {
            line.backgroundColor = UIKitStyle.border
            line.translatesAutoresizingMaskIntoConstraints = false
            line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        }
        label.setContentHuggingPriority(.required, for: .horizontal)
        label.setContentCompressionResistancePriority(.required, for: .horizontal)
        let stack = UIStackView(arrangedSubviews: [left, label, right])
        stack.spacing = 10
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        left.widthAnchor.constraint(equalTo: right.widthAnchor).isActive = true
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 18),
            stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -18),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 8),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -8),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .header
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ text: String) {
        label.text = text
        accessibilityLabel = text
    }
}

/// "3 replies · View thread" with the repliers' faces, or "Replied to a
/// thread · View thread" under a reply also sent to the channel.
final class ThreadLinkButton: UIControl {
    private let faces = UIStackView()
    private let label = UILabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        faces.spacing = 5
        faces.isUserInteractionEnabled = false
        label.isUserInteractionEnabled = false
        let row = UIStackView(arrangedSubviews: [faces, label, UIView()])
        row.spacing = 8
        row.alignment = .center
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        layer.cornerRadius = 8
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// A bare UIControl has no default activation; VoiceOver taps fire like a touch.
    override func accessibilityActivate() -> Bool { sendActions(for: .touchUpInside); return true }

    func configure(_ link: ThreadLink) {
        faces.arrangedSubviews.forEach { $0.removeFromSuperview() }
        let text = NSMutableAttributedString()
        let medium = UIKitStyle.font(11, .medium)
        if link.replyCount > 0 {
            for person in link.participants {
                let face = AvatarView(side: 24)
                face.show(name: person.name, avatarID: person.avatarId, picture: person.avatarPicture)
                faces.addArrangedSubview(face)
            }
            text.append(NSAttributedString(string: "\(link.replyCount) \(link.replyCount == 1 ? "reply" : "replies")",
                                           attributes: [.font: UIKitStyle.font(11, .bold), .foregroundColor: UIKitStyle.terracottaBright]))
            if link.unread {
                text.append(NSAttributedString(string: "  ● Unread", attributes: [.font: medium, .foregroundColor: UIKitStyle.pinGold]))
            }
            text.append(NSAttributedString(string: "  View thread", attributes: [.font: medium, .foregroundColor: UIKitStyle.muted]))
        } else {
            if let icon = UIKitStyle.icon("thread")?.resized(to: 14).withTintColor(UIKitStyle.terracottaBright, renderingMode: .alwaysOriginal) {
                let attachment = NSTextAttachment(image: icon)
                attachment.bounds = CGRect(x: 0, y: -3, width: 14, height: 14)
                text.append(NSAttributedString(attachment: attachment))
                text.append(NSAttributedString(string: " "))
            }
            text.append(NSAttributedString(string: "Replied to a thread · View thread",
                                           attributes: [.font: medium, .foregroundColor: UIKitStyle.terracottaBright]))
        }
        faces.isHidden = faces.arrangedSubviews.isEmpty
        label.attributedText = text
        accessibilityLabel = link.unread ? "View thread, unread" : "View thread"
        accessibilityValue = link.replyCount > 0 ? "\(link.replyCount) \(link.replyCount == 1 ? "reply" : "replies")" : nil
        accessibilityIdentifier = "view-thread-\(link.rootID)"
    }

    override var isHighlighted: Bool { didSet { backgroundColor = isHighlighted ? UIKitStyle.text.withAlphaComponent(0.08) : .clear } }
}

@MainActor extension RowHost {
    /// Person pills and authors open their profile card over `controller`;
    /// Block on the card closes it, then `block` asks to confirm.
    mutating func openProfiles(model: AppModel, controller: UIViewController, messages: @escaping @MainActor () -> [ChatMessage],
                               block: @escaping (BlockTarget) -> Void) {
        openMention = { [weak controller] messageID, userID in MainActor.assumeIsolated {
            guard let controller, let message = messages().first(where: { $0.id == messageID }),
                  let card = UIKitHosting.profileCard(model: model, userID: userID, in: message, requestBlock: block) else { return }
            controller.view.endEditing(true)
            controller.present(card, animated: !UIKitStyle.reduceMotion)
        } }
        openAuthor = { [weak controller] messageID in MainActor.assumeIsolated {
            guard let controller, let message = messages().first(where: { $0.id == messageID }), !message.author.isGuest else { return }
            controller.view.endEditing(true)
            controller.present(UIKitHosting.profileCard(model: model, author: message.author, requestBlock: block),
                               animated: !UIKitStyle.reduceMotion)
        } }
    }
}
