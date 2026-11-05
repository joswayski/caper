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
    var pending: Bool
    var pendingError: String?
    var pendingRejected: Bool
    /// "you" or the pinner's name, on a pinned message.
    var pinnedBy: String?
    var pinMine = false
    /// The jump target (a link or Go to message): terracotta, like web.
    var focused = false
    /// Pins list rows: the full date, and the card paints the background.
    var inPins = false

    init(message: ChatMessage, grouped: Bool, continued: Bool, viewerID: String?, focused: Bool = false, inPins: Bool = false) {
        id = message.clientMessageId
        messageID = message.id
        authorName = message.author.name
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
final class MessageTextView: UITextView, NSTextLayoutManagerDelegate {
    /// The message text without decorative link symbols, for copying and VoiceOver.
    var plainText: String { LinkPills.plain(text ?? "") }

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
        linkTextAttributes = [.foregroundColor: UIKitStyle.link]
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

/// What a row's hosted SwiftUI leaves need from the conversation.
struct RowHost {
    let chat: ChatModel
    let present: UIKitHosting.Present
    let openForward: (ChatMessage) -> Void
}

/// One message: avatar or grouped gutter, header, text and its notes. The
/// timeline's cells and the Pins cards host it.
final class MessageRowView: UIView {
    private let avatar = AvatarView(side: 34)
    private let gutter = UIView()
    private let name = UIKitStyle.label(size: 13, weight: .bold)
    private let time = UIKitStyle.label(size: 10, color: UIKitStyle.muted)
    private let edited = UIKitStyle.label("(edited)", size: 10, color: UIKitStyle.muted)
    private let header = UIStackView()
    private let body = MessageTextView()
    private let note = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let extras = HostedView()
    /// The model and a presenter for the hosted extras (media viewer, forwards).
    var host: RowHost?
    private let column = UIStackView()
    private let accent = UIView()
    private let tint = UIView()
    private let pinBadge = UIKitStyle.label(size: 10, weight: .medium, color: UIKitStyle.muted)
    private var top: NSLayoutConstraint!
    private var badgeTop: NSLayoutConstraint!
    private var columnUnderBadge: NSLayoutConstraint!
    private var bottom: NSLayoutConstraint!
    private var avatarWidth: NSLayoutConstraint!
    private var state: MessageRowState?
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
        [header, body, extras, note].forEach(column.addArrangedSubview)
        column.translatesAutoresizingMaskIntoConstraints = false

        gutter.translatesAutoresizingMaskIntoConstraints = false
        gutter.addSubview(avatar)
        // The header names the author; the picture adds nothing for VoiceOver.
        gutter.accessibilityElementsHidden = true
        self.addSubview(gutter)
        self.addSubview(column)
        pinBadge.isHidden = true
        self.addSubview(pinBadge)

        top = column.topAnchor.constraint(equalTo: self.topAnchor, constant: 10)
        badgeTop = pinBadge.topAnchor.constraint(equalTo: self.topAnchor, constant: 10)
        columnUnderBadge = column.topAnchor.constraint(equalTo: pinBadge.bottomAnchor, constant: 5)
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
            pinBadge.leadingAnchor.constraint(equalTo: column.leadingAnchor),
            pinBadge.trailingAnchor.constraint(lessThanOrEqualTo: column.trailingAnchor),
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
        // SwiftUI's grouped rows read about 4pt further apart than their 2pt padding.
        let topPadding: CGFloat = row.grouped ? 6 : 10
        top.constant = topPadding
        badgeTop.constant = topPadding
        bottom.constant = row.continued ? -2 : -10
        // "Pinned by …" heads the row, over the text column.
        pinBadge.isHidden = row.pinnedBy == nil
        top.isActive = row.pinnedBy == nil
        columnUnderBadge.isActive = row.pinnedBy != nil
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
        accent.isHidden = !(row.focused || (row.pinnedBy == nil && row.mentionsViewer))

        if let host, let message = row.extras {
            extras.show(UIKitMessageExtras(message: message, chat: host.chat, present: host.present, openForward: { host.openForward(message) }))
        } else if let host, !row.pendingAttachments.isEmpty {
            extras.show(UIKitMessageExtras(pending: row.pendingAttachments, chat: host.chat, present: host.present))
        } else {
            extras.show(Optional<EmptyView>.none)
        }
        extras.isHidden = row.extras == nil && row.pendingAttachments.isEmpty

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

    /// SwiftUI's rowBackground: the jump target, then pins, then mentions of you.
    private static func background(_ row: MessageRowState) -> UIColor {
        if row.inPins { return .clear }
        if row.focused { return UIKitStyle.terracotta.withAlphaComponent(0.12) }
        if row.pinnedBy != nil { return row.pinMine ? UIKitStyle.terracotta.withAlphaComponent(0.12) : UIKitStyle.pinGold.withAlphaComponent(0.06) }
        if row.mentionsViewer { return UIKitStyle.terracotta.withAlphaComponent(0.08) }
        return .clear
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
        LinkPills.decorate(text, size: size)
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
        body.attributedText?.enumerateAttribute(.link, in: NSRange(location: 0, length: body.attributedText?.length ?? 0)) { value, _, _ in
            guard let url = value as? URL else { return }
            actions.append(UIAccessibilityCustomAction(name: "Open \(url.host ?? url.absoluteString)") { _ in
                UIApplication.shared.open(url)
                return true
            })
        }
        if row.pendingError != nil, !row.pendingRejected, let retry {
            actions.append(UIAccessibilityCustomAction(name: "Retry send") { _ in retry(); return true })
        }
        return actions
    }

    override var accessibilityElements: [Any]? {
        get { [header, body, extras, note].filter { !$0.isHidden } }
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
