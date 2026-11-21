import SwiftUI

#if os(macOS)
import AppKit

@MainActor final class AppKitMessageGIF: NSControl {
    private let image = NativeKlipyImage()
    private let label = AppKitKit.label("GIF unavailable", size: 12, color: AppKitKit.muted)
    private let show: (MediaViewerRequest) -> Void
    private var gif: ChatGif
    private var failed = false
    private var measuredWidth: CGFloat = 0
    init(_ gif: ChatGif, show: @escaping (MediaViewerRequest) -> Void) {
        self.gif = gif; self.show = show; super.init(frame: .zero)
        wantsLayer = true; layer?.addSublayer(image.layer); addSubview(label); label.isHidden = true
        layer?.masksToBounds = true; layer?.cornerRadius = gif.kind == .sticker ? 0 : 8
        image.failed = { [weak self] failed in self?.failed = failed; self?.label.isHidden = !failed; self?.invalidateIntrinsicContentSize() }
        setAccessibilityIdentifier("message-gif"); translatesAutoresizingMaskIntoConstraints = false
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    func update(_ gif: ChatGif) { guard self.gif != gif else { return }; self.gif = gif; load(); invalidateIntrinsicContentSize() }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); if window != nil { load() } else { image.stop() } }
    private func load() { image.set(sources: gif.animatedSources, still: gif.still.flatMap(URL.init(string:)), maximum: 600, limit: 24 * 1024 * 1024) }
    override var intrinsicContentSize: NSSize {
        let maximum: CGFloat = gif.kind == .sticker ? 160 : 300
        let scale = min(1, maximum / CGFloat(max(gif.width, gif.height)))
        let width = min(max(1, bounds.width), CGFloat(gif.width) * scale)
        return NSSize(width: NSView.noIntrinsicMetric, height: failed ? 32 : width * CGFloat(gif.height) / CGFloat(gif.width))
    }
    override var isFlipped: Bool { true }
    override func layout() {
        super.layout(); let maxWidth = CGFloat(gif.width) * min(1, (gif.kind == .sticker ? 160 : 300) / CGFloat(max(gif.width, gif.height)))
        CATransaction.begin(); CATransaction.setDisableActions(true)
        image.layer.frame = CGRect(x: 0, y: 0, width: min(bounds.width, maxWidth), height: intrinsicContentSize.height); CATransaction.commit()
        label.frame = bounds
        if measuredWidth != bounds.width { measuredWidth = bounds.width; invalidateIntrinsicContentSize() }
    }
    /// The drawn GIF. The control spans the row; only the picture opens it.
    private var pictureRect: NSRect {
        failed ? bounds : NSRect(x: 0, y: 0, width: min(bounds.width, CGFloat(gif.width) * min(1, (gif.kind == .sticker ? 160 : 300) / CGFloat(max(gif.width, gif.height)))), height: bounds.height)
    }
    override func hitTest(_ point: NSPoint) -> NSView? { pictureRect.contains(convert(point, from: superview)) ? super.hitTest(point) : nil }
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityFrame() -> NSRect { window?.convertToScreen(convert(pictureRect, to: nil)) ?? super.accessibilityFrame() }
    override func accessibilityHitTest(_ point: NSPoint) -> Any? { self }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityLabel() -> String? { failed ? "GIF unavailable" : "Open \(gif.accessibilityLabel)" }
    override func accessibilityPerformPress() -> Bool { open(); return true }
    override func mouseDown(with event: NSEvent) { open() }
    private func open() { if let file = gif.mediaViewerAttachment { show(MediaViewerRequest(items: [file], start: 0, chat: nil, hideSpoiler: { _ in })) } }
}

/// The original is immutable delivery content from another conversation. Its
/// media must not refresh URLs through the currently open channel's ChatModel.
@MainActor final class AppKitOriginalMessage: NSStackView {
    var heightChanged: (() -> Void)?
    private let avatar = AppKitAvatar()
    private let name = AppKitKit.label("", size: 13, weight: .bold)
    private let badges = AppKitKit.label("", size: 10, color: AppKitKit.muted)
    private let text = AppKitMessageBody()
    private let reactions = AppKitKit.label("", size: 12)
    private let files: AppKitMessageFiles
    private var gif: AppKitMessageGIF?
    private let show: (MediaViewerRequest) -> Void
    init(_ message: ChatMessage, available: (() -> Bool)? = nil, show: @escaping (MediaViewerRequest) -> Void) {
        self.show = show; files = AppKitMessageFiles(chat: nil, available: available, show: show)
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        avatar.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 24), avatar.heightAnchor.constraint(equalToConstant: 24)])
        let header = NSStackView(views: [avatar, name, badges]); header.orientation = .horizontal; header.spacing = 8; header.alignment = .centerY
        for row in [header, text, files, reactions] { addArrangedSubview(row); AppKitKit.fillWidth(row, of: self) }
        files.heightChanged = { [weak self] in self?.heightChanged?() }
        update(message)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ message: ChatMessage) {
        let author = message.author
        avatar.update(name: author.name, avatarId: author.avatarId, picture: author.avatarPicture); name.stringValue = author.name
        badges.stringValue = [author.isAdmin ? "ADMIN" : "", message.editedAt != nil && !message.isDeleted ? "edited" : ""].filter { !$0.isEmpty }.joined(separator: " · ")
        text.update(NativePresentation.messageText(message.isDeleted ? MessageDeletion.placeholder : message.content.text))
        text.isHidden = !message.isDeleted && message.content.text.isEmpty
        files.update(message.isDeleted ? [] : message.content.attachments ?? []); files.isHidden = message.isDeleted || (message.content.attachments ?? []).isEmpty
        if !message.isDeleted, let value = message.content.gif {
            if let gif { gif.update(value) } else {
                let gif = AppKitMessageGIF(value, show: show); self.gif = gif
                insertArrangedSubview(gif, at: 3); AppKitKit.fillWidth(gif, of: self)
            }
        } else { gif?.removeFromSuperview(); gif = nil }
        let list = message.isDeleted ? [] : message.reactions ?? []
        let font = reactions.font ?? AppKitKit.font(12)
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: reactions.textColor ?? AppKitKit.text]
        let side = ceil(font.pointSize * 4 / 3)
        let line = NSMutableAttributedString()
        for (index, reaction) in list.enumerated() {
            if index > 0 { line.append(NSAttributedString(string: "   ", attributes: attributes)) }
            if let artwork = NativePresentation.emojiImage(reaction.emoji) {
                let attachment = NSTextAttachment()
                attachment.image = NSImage(cgImage: artwork, size: NSSize(width: side, height: side))
                attachment.bounds = CGRect(x: 0, y: (font.capHeight - side) / 2, width: side, height: side)
                line.append(NSAttributedString(attachment: attachment))
            } else {
                line.append(NSAttributedString(string: reaction.emoji, attributes: attributes))
            }
            line.append(NSAttributedString(string: " \(reaction.authorIds.count)", attributes: attributes))
        }
        reactions.attributedStringValue = line
        reactions.setAccessibilityLabel(list.map { "\($0.emoji), \($0.authorIds.count) reactions" }.joined(separator: ", "))
        reactions.isHidden = list.isEmpty
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitMessageGIF: UIControl {
    private let image = NativeKlipyImage()
    private let label = UIKitMessageDialog.label("GIF unavailable")
    private let show: (MediaViewerRequest) -> Void
    private var gif: ChatGif
    private var failed = false
    private var measuredWidth: CGFloat = 0
    init(_ gif: ChatGif, show: @escaping (MediaViewerRequest) -> Void) {
        self.gif = gif; self.show = show; super.init(frame: .zero)
        layer.addSublayer(image.layer); addSubview(label); label.isHidden = true; clipsToBounds = true
        layer.cornerRadius = gif.kind == .sticker ? 0 : 8
        image.failed = { [weak self] failed in
            guard let self else { return }
            self.failed = failed; self.label.isHidden = !failed
            self.accessibilityLabel = failed ? "GIF unavailable" : "Open \(self.gif.accessibilityLabel)"; self.invalidateIntrinsicContentSize()
        }
        accessibilityIdentifier = "message-gif"; isAccessibilityElement = true; accessibilityTraits = .button
        accessibilityLabel = "Open \(gif.accessibilityLabel)"; addTarget(self, action: #selector(open), for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ gif: ChatGif) { guard self.gif != gif else { return }; self.gif = gif; load(); invalidateIntrinsicContentSize() }
    override func didMoveToWindow() { super.didMoveToWindow(); if window != nil { load() } else { image.stop() } }
    private func load() { image.set(sources: gif.animatedSources, still: gif.still.flatMap(URL.init(string:)), maximum: 600, limit: 24 * 1024 * 1024) }
    override var intrinsicContentSize: CGSize {
        let maximum: CGFloat = gif.kind == .sticker ? 160 : 300
        let width = min(max(1, bounds.width), CGFloat(gif.width) * min(1, maximum / CGFloat(max(gif.width, gif.height))))
        return CGSize(width: UIView.noIntrinsicMetric, height: failed ? 32 : width * CGFloat(gif.height) / CGFloat(gif.width))
    }
    override func layoutSubviews() {
        super.layoutSubviews(); let maxWidth = CGFloat(gif.width) * min(1, (gif.kind == .sticker ? 160 : 300) / CGFloat(max(gif.width, gif.height)))
        CATransaction.begin(); CATransaction.setDisableActions(true)
        image.layer.frame = CGRect(x: 0, y: 0, width: min(bounds.width, maxWidth), height: intrinsicContentSize.height); CATransaction.commit()
        label.frame = bounds
        if measuredWidth != bounds.width { measuredWidth = bounds.width; invalidateIntrinsicContentSize() }
    }
    @objc private func open() { if !SwipeTapGuard.swiping, let file = gif.mediaViewerAttachment { show(MediaViewerRequest(items: [file], start: 0, chat: nil, hideSpoiler: { _ in })) } }
}

@MainActor final class UIKitOriginalMessage: UIStackView {
    private let avatar = UIKitAvatar(side: 24)
    private let name = UIKitMessageDialog.label("", size: 13, bold: true)
    private let badges = UIKitMessageDialog.label("", size: 10)
    private let text = UIKitMessageBody()
    private let reactions = UIKitMessageDialog.label("", size: 12)
    private let files: UIKitMessageFiles
    private var gif: UIKitMessageGIF?
    private let show: (MediaViewerRequest) -> Void
    init(_ message: ChatMessage, available: (() -> Bool)? = nil, show: @escaping (MediaViewerRequest) -> Void) {
        self.show = show; files = UIKitMessageFiles(chat: nil, available: available, show: show)
        super.init(frame: .zero); axis = .vertical; spacing = 8
        let header = UIStackView(arrangedSubviews: [avatar, name, badges]); header.spacing = 8; header.alignment = .center
        [header, text, files, reactions].forEach(addArrangedSubview)
        update(message)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ message: ChatMessage) {
        let author = message.author
        avatar.show(name: author.name, avatarID: author.avatarId, picture: author.avatarPicture); name.text = author.name
        badges.text = [author.isAdmin ? "ADMIN" : "", message.editedAt != nil && !message.isDeleted ? "edited" : ""].filter { !$0.isEmpty }.joined(separator: " · ")
        text.update(NativePresentation.messageText(message.isDeleted ? MessageDeletion.placeholder : message.content.text))
        text.isHidden = !message.isDeleted && message.content.text.isEmpty
        files.update(message.isDeleted ? [] : message.content.attachments ?? []); files.isHidden = message.isDeleted || (message.content.attachments ?? []).isEmpty
        if !message.isDeleted, let value = message.content.gif {
            if let gif { gif.update(value) } else { let gif = UIKitMessageGIF(value, show: show); self.gif = gif; insertArrangedSubview(gif, at: 3) }
        } else { gif?.removeFromSuperview(); gif = nil }
        let list = message.isDeleted ? [] : message.reactions ?? []
        let font: UIFont = reactions.font
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: reactions.textColor as UIColor]
        let side = ceil(font.pointSize * 4 / 3)
        let line = NSMutableAttributedString()
        for (index, reaction) in list.enumerated() {
            if index > 0 { line.append(NSAttributedString(string: "   ", attributes: attributes)) }
            if let artwork = NativePresentation.emojiImage(reaction.emoji) {
                let attachment = NSTextAttachment()
                attachment.image = UIImage(cgImage: artwork, scale: CGFloat(artwork.width) / side, orientation: .up)
                attachment.bounds = CGRect(x: 0, y: (font.capHeight - side) / 2, width: side, height: side)
                line.append(NSAttributedString(attachment: attachment))
            } else {
                line.append(NSAttributedString(string: reaction.emoji, attributes: attributes))
            }
            line.append(NSAttributedString(string: " \(reaction.authorIds.count)", attributes: attributes))
        }
        reactions.attributedText = line
        reactions.accessibilityLabel = list.map { "\($0.emoji), \($0.authorIds.count) reactions" }.joined(separator: ", ")
        reactions.isHidden = list.isEmpty
    }
}
#endif
