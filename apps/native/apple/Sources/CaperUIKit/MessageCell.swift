import CaperCore
import UIKit

/// One timeline row's display state. Built per snapshot from the model; cells
/// read it only when they are realized.
struct MessageRowState: Equatable {
    var id: String
    var authorName: String
    var avatarID: Int?
    var createdAt: String
    var text: String
    var mentions: [MessageMention]
    var grouped: Bool
    var continued: Bool
    var mentionsViewer: Bool
    var edited: Bool
    var deleted: Bool
    var attachmentCount: Int
    var pending: Bool
    var pendingError: String?
    var pendingRejected: Bool

    init(message: ChatMessage, grouped: Bool, continued: Bool, viewerID: String?) {
        id = message.clientMessageId
        authorName = message.author.name
        avatarID = message.author.avatarId
        createdAt = message.createdAt
        text = message.isDeleted ? "" : message.content.text
        mentions = message.content.mentions
        self.grouped = grouped
        self.continued = continued
        mentionsViewer = NativePresentation.mentionsViewer(message, viewerID: viewerID)
        edited = message.forward == nil && !message.isDeleted && (message.revision ?? 1) > 1
        deleted = message.isDeleted
        attachmentCount = message.isDeleted ? 0 : message.content.attachments?.count ?? 0
        pending = false
        pendingError = nil
        pendingRejected = false
    }

    init(pending: PendingMessage, author: ChatAuthor?, grouped: Bool, error: String?, rejected: Bool) {
        id = pending.id
        authorName = author?.name ?? "Guest"
        avatarID = author?.avatarId
        createdAt = pending.createdAt
        text = pending.text
        mentions = []
        self.grouped = grouped
        continued = false
        mentionsViewer = false
        edited = false
        deleted = false
        attachmentCount = pending.attachments.count
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

final class MessageCell: UICollectionViewCell {
    static let reuse = "message"
    private let avatar = AvatarView(side: 34)
    private let gutter = UIView()
    private let name = UIKitStyle.label(size: 13, weight: .bold)
    private let time = UIKitStyle.label(size: 10, color: UIKitStyle.muted)
    private let edited = UIKitStyle.label("(edited)", size: 10, color: UIKitStyle.muted)
    private let header = UIStackView()
    private let body = MessageTextView()
    private let note = UIKitStyle.label(size: 12, color: UIKitStyle.muted)
    private let column = UIStackView()
    private let accent = UIView()
    private let tint = UIView()
    private var top: NSLayoutConstraint!
    private var bottom: NSLayoutConstraint!
    private var avatarWidth: NSLayoutConstraint!
    private var state: MessageRowState?
    var retry: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        tint.translatesAutoresizingMaskIntoConstraints = false
        accent.translatesAutoresizingMaskIntoConstraints = false
        accent.backgroundColor = UIKitStyle.terracotta
        contentView.addSubview(tint)
        contentView.addSubview(accent)

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
        [header, body, note].forEach(column.addArrangedSubview)
        column.translatesAutoresizingMaskIntoConstraints = false

        gutter.translatesAutoresizingMaskIntoConstraints = false
        gutter.addSubview(avatar)
        // The header names the author; the picture adds nothing for VoiceOver.
        gutter.accessibilityElementsHidden = true
        contentView.addSubview(gutter)
        contentView.addSubview(column)

        top = column.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10)
        bottom = column.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10)
        bottom.priority = .init(999)
        avatarWidth = gutter.widthAnchor.constraint(equalToConstant: 34)
        NSLayoutConstraint.activate([
            tint.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            tint.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            tint.topAnchor.constraint(equalTo: contentView.topAnchor),
            tint.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            accent.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            accent.widthAnchor.constraint(equalToConstant: 3),
            accent.topAnchor.constraint(equalTo: contentView.topAnchor),
            accent.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            gutter.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 18),
            gutter.topAnchor.constraint(equalTo: column.topAnchor),
            avatarWidth,
            avatar.topAnchor.constraint(equalTo: gutter.topAnchor),
            avatar.leadingAnchor.constraint(equalTo: gutter.leadingAnchor),
            gutter.heightAnchor.constraint(greaterThanOrEqualToConstant: 1),
            column.leadingAnchor.constraint(equalTo: gutter.trailingAnchor, constant: 10),
            column.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -18),
            top, bottom,
            contentView.heightAnchor.constraint(greaterThanOrEqualTo: gutter.heightAnchor),
        ])
        // The row contains its header and text as separate VoiceOver elements,
        // so readers can move into the text and select it.
        isAccessibilityElement = false
        contentView.isAccessibilityElement = false
        accessibilityContainerType = .semanticGroup
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ row: MessageRowState) {
        state = row
        accessibilityIdentifier = row.pending ? "pending-message-\(row.id)" : "message-row-\(row.id)"
        // SwiftUI's grouped rows read about 4pt further apart than their 2pt padding.
        top.constant = row.grouped ? 6 : 10
        bottom.constant = row.continued ? -2 : -10
        header.isHidden = row.grouped
        avatar.isHidden = row.grouped
        if !row.grouped { avatar.show(name: row.authorName, avatarID: row.avatarID) }
        name.text = row.authorName
        let timeText = NativePresentation.time(row.createdAt)
        time.text = row.pending ? (row.pendingError == nil ? "Sending…" : "") : timeText
        edited.isHidden = !row.edited
        header.accessibilityLabel = [row.authorName, row.pending ? "sending" : timeText, row.edited ? "edited" : ""]
            .filter { !$0.isEmpty }.joined(separator: ", ")

        tint.backgroundColor = row.mentionsViewer ? UIKitStyle.terracotta.withAlphaComponent(0.08) : .clear
        accent.isHidden = !row.mentionsViewer

        if row.deleted {
            body.isHidden = true
            note.isHidden = false
            note.text = "This message was deleted."
        } else {
            body.isHidden = row.text.isEmpty
            if !row.text.isEmpty { body.attributedText = Self.render(row) }
            let files = row.attachmentCount
            var lines: [String] = []
            // Attachments render in a later milestone; say they exist rather than hide them.
            if files > 0 { lines.append(files == 1 ? "1 attachment" : "\(files) attachments") }
            if let error = row.pendingError { lines.append("\(row.pendingRejected ? "Not sent." : "Not confirmed yet.") \(error)") }
            note.isHidden = lines.isEmpty
            note.text = lines.joined(separator: "\n")
            note.textColor = row.pendingError == nil ? UIKitStyle.muted : UIKitStyle.error
        }
        contentView.alpha = row.pending && row.pendingError == nil ? 0.6 : 1
        body.accessibilityLabel = row.grouped ? "\(row.authorName), \(timeText): \(body.plainText)" : body.plainText
        body.accessibilityIdentifier = "message-text-\(row.id)"
        body.accessibilityCustomActions = actions(for: row)
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
        get { [header, body, note].filter { !$0.isHidden } }
        set {}
    }

    /// Messages that arrive while the reader watches the live end rise in;
    /// everything else, and everything with Reduce Motion, appears in place.
    func arrive() {
        guard !UIKitStyle.reduceMotion else { return }
        contentView.alpha = 0
        contentView.transform = CGAffineTransform(translationX: 0, y: 8)
        let target = state?.pending == true && state?.pendingError == nil ? 0.6 : 1
        let animator = UIViewPropertyAnimator(duration: UIKitStyle.motionFast, timingParameters: UIKitStyle.easeOut)
        animator.addAnimations { [weak self] in
            self?.contentView.alpha = target
            self?.contentView.transform = .identity
        }
        animator.startAnimation()
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        contentView.layer.removeAllAnimations()
        contentView.transform = .identity
        retry = nil
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
