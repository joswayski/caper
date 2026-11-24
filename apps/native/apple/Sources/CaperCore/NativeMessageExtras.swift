import Observation
import SwiftUI

#if os(macOS)
import AppKit

@MainActor final class AppKitMessageExtras: NSStackView {
    var heightChanged: (() -> Void)?
    private let files: AppKitMessageFiles
    private let embeds = AppKitMessageEmbeds()
    init(_ message: ChatMessage, chat: ChatModel, show: @escaping (MediaViewerRequest) -> Void) {
        let available = {
            chat.channelID == message.channelId && (chat.displayedMessages.first { $0.id == message.id }
                ?? chat.displayedPins.first { $0.id == message.id })?.isDeleted == false
        }
        files = AppKitMessageFiles(chat: chat, available: available, show: show)
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        files.update(message.content.attachments ?? [])
        if !(message.content.attachments ?? []).isEmpty { append(files) }
        if let gif = message.content.gif { append(AppKitMessageGIF(gif, show: show)) }
        if !(message.content.embeds ?? []).isEmpty { embeds.update(message.content.embeds ?? []); append(embeds) }
        if MentionAutocomplete.asksFeedback(message) {
            let label = AppKitKit.label(FeedbackInbox.notified, size: 11, color: AppKitKit.muted)
            label.setAccessibilityIdentifier("feedback-notified-\(message.id)"); append(label)
        }
        if let forward = message.forward {
            let card = AppKitKit.stack([AppKitKit.label("Forwarded", size: 11, color: AppKitKit.muted)], spacing: 8)
            card.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
            card.wantsLayer = true; card.layer?.backgroundColor = AppKitKit.surface.cgColor
            card.layer?.cornerRadius = 8; card.layer?.borderWidth = 1; card.layer?.borderColor = AppKitKit.border.cgColor
            card.setAccessibilityElement(true); card.setAccessibilityRole(.group); card.setAccessibilityLabel("Forwarded message"); card.setAccessibilityIdentifier("forward-card-\(message.id)")
            if let original = forward.message {
                let preview = AppKitOriginalMessage(original, available: {
                    available() && (chat.displayedMessages.first { $0.id == message.id } ?? chat.displayedPins.first { $0.id == message.id })?.forward?.message != nil
                }, show: show)
                card.addArrangedSubview(preview); AppKitKit.fillWidth(preview, of: card)
                preview.heightChanged = { [weak self] in self?.heightChanged?() }
                let replies = original.thread?.replyCount ?? 0
                let open = AppKitKitButton((replies > 0 ? "\(replies) \(replies == 1 ? "reply" : "replies") · " : "") + "View conversation") {
                    Task {
                        guard available(), let current = chat.displayedMessages.first(where: { $0.id == message.id })
                            ?? chat.displayedPins.first(where: { $0.id == message.id }), current.forward?.message != nil else { return }
                        _ = await chat.onOpenForward?(current)
                    }
                }
                open.setAccessibilityIdentifier("open-forward-\(message.id)"); card.addArrangedSubview(open)
            } else { card.addArrangedSubview(AppKitKit.label("Original conversation unavailable.", size: 12, color: AppKitKit.muted)) }
            append(card)
        }
        // Message links in the text: a live card of each original this
        // account can see (the API decides), else a neutral notice.
        for linked in MessageLinkCards.shared.cards(for: message) {
            let target = linked.target
            guard case let .original(original) = linked.card else {
                let locked = AppKitKit.label("Message you can\u{2019}t view", size: 12, color: AppKitKit.muted)
                locked.setAccessibilityIdentifier("message-link-locked-\(target.messageID)"); append(locked); continue
            }
            let card = AppKitKit.stack([AppKitKit.label("Linked message", size: 11, color: AppKitKit.muted)], spacing: 8)
            card.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
            card.wantsLayer = true; card.layer?.backgroundColor = AppKitKit.surface.cgColor
            card.layer?.cornerRadius = 8; card.layer?.borderWidth = 1; card.layer?.borderColor = AppKitKit.border.cgColor
            card.setAccessibilityElement(true); card.setAccessibilityRole(.group); card.setAccessibilityLabel("Linked message")
            card.setAccessibilityIdentifier("message-link-card-\(target.messageID)")
            let preview = AppKitOriginalMessage(original, show: show)
            card.addArrangedSubview(preview); AppKitKit.fillWidth(preview, of: card)
            preview.heightChanged = { [weak self] in self?.heightChanged?() }
            if !original.isDeleted, let cards = original.content.embeds, !cards.isEmpty {
                let embeds = AppKitMessageEmbeds(); embeds.update(cards)
                card.addArrangedSubview(embeds); AppKitKit.fillWidth(embeds, of: card)
            }
            let jump = AppKitKitButton("Jump to message") { MessageLinkCards.shared.jump(target) }
            jump.setAccessibilityIdentifier("message-link-jump-\(target.messageID)"); card.addArrangedSubview(jump)
            append(card)
        }
        files.heightChanged = { [weak self] in self?.heightChanged?() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    private func append(_ view: NSView) { addArrangedSubview(view); AppKitKit.fillWidth(view, of: self) }
}

@MainActor final class AppKitPendingExtras: NSStackView {
    var heightChanged: (() -> Void)?
    init(_ pending: PendingMessage, show: @escaping (MediaViewerRequest) -> Void) {
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 8
        if !pending.attachments.isEmpty {
            let files = AppKitMessageFiles(chat: nil, show: show)
            files.update(pending.attachments)
            files.alphaValue = 0.7
            addArrangedSubview(files); AppKitKit.fillWidth(files, of: self)
            files.heightChanged = { [weak self] in self?.heightChanged?() }
        }
        if let gif = pending.gif {
            let view = AppKitMessageGIF(gif, show: show)
            view.alphaValue = 0.7
            addArrangedSubview(view); AppKitKit.fillWidth(view, of: self)
        }
        if !pending.embeds.isEmpty {
            let embeds = AppKitMessageEmbeds()
            embeds.update(pending.embeds)
            addArrangedSubview(embeds); AppKitKit.fillWidth(embeds, of: self)
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

@MainActor final class AppKitPendingRow: NSStackView {
    var heightChanged: (() -> Void)?
    private let pending: PendingMessage
    private let chat: ChatModel
    private let observation = AppKitObservation()
    private let error = AppKitKit.label("", size: 11, color: AppKitKit.error)
    private let actions = NSStackView()
    private var retry: AppKitKitButton!, edit: AppKitKitButton!, dismiss: AppKitKitButton!
    init(_ pending: PendingMessage, chat: ChatModel, show: @escaping (MediaViewerRequest) -> Void) {
        self.pending = pending; self.chat = chat
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 5
        edgeInsets = NSEdgeInsets(top: 10, left: 18, bottom: 10, right: 18)
        if let reply = pending.replyTo {
            let quote = AppKitReplyContext(reference: reply, motionKey: pending.id, chat: chat)
            addArrangedSubview(quote); AppKitKit.fillWidth(quote, of: self)
            quote.heightChanged = { [weak self] in self?.heightChanged?() }
        }
        let avatar = AppKitAvatar(); avatar.translatesAutoresizingMaskIntoConstraints = false
        avatar.update(name: chat.currentAuthor?.name ?? "Guest", avatarId: chat.currentAuthor?.avatarId, picture: chat.currentAuthor?.avatarPicture)
        NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 34), avatar.heightAnchor.constraint(equalToConstant: 34)])
        let author = chat.currentAuthor
        let name = AppKitKit.label((author?.name ?? "Guest") + (author?.isAdmin == true ? " · ADMIN" : "") + (author?.isGuest == true ? " · GUEST" : ""), size: 13, weight: .bold, wraps: false)
        let words = AppKitKit.stack([name], spacing: 4)
        AppKitKit.fillWidth(name, of: words)
        if !pending.text.isEmpty { let text = AppKitKit.label(pending.text, size: 14, color: AppKitKit.muted); words.addArrangedSubview(text); AppKitKit.fillWidth(text, of: words) }
        if !pending.attachments.isEmpty {
            let files = AppKitMessageFiles(chat: nil, show: show); files.update(pending.attachments); files.alphaValue = 0.7
            words.addArrangedSubview(files); AppKitKit.fillWidth(files, of: words); files.heightChanged = { [weak self] in self?.heightChanged?() }
        }
        if let gif = pending.gif { let view = AppKitMessageGIF(gif, show: show); view.alphaValue = 0.7; words.addArrangedSubview(view); AppKitKit.fillWidth(view, of: words) }
        if !pending.embeds.isEmpty { let embeds = AppKitMessageEmbeds(); embeds.update(pending.embeds); words.addArrangedSubview(embeds); AppKitKit.fillWidth(embeds, of: words) }
        retry = AppKitKitButton("Retry send", kind: .plain) { Task { if chat.pendingMessage?.id == pending.id { await chat.send(inThread: pending.threadRootId != nil, retryPending: true) } } }
        edit = AppKitKitButton("Edit", kind: .plain) { if chat.pendingMessage?.id == pending.id { _ = chat.discardRejected(edit: true) } }
        dismiss = AppKitKitButton("Dismiss", kind: .plain) { if chat.pendingMessage?.id == pending.id { _ = chat.discardRejected() } }
        [retry!, edit!, dismiss!].forEach(actions.addArrangedSubview); actions.spacing = 12
        words.addArrangedSubview(error); AppKitKit.fillWidth(error, of: words); words.addArrangedSubview(actions)
        let row = NSStackView(views: [avatar, words]); row.orientation = .horizontal; row.alignment = .top; row.spacing = 10
        words.widthAnchor.constraint(equalTo: row.widthAnchor, constant: -44).isActive = true
        addArrangedSubview(row); AppKitKit.fillWidth(row, of: self); setAccessibilityElement(true); setAccessibilityRole(.group); setAccessibilityIdentifier("pending-message-\(pending.id)")
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) } else { observation.stop() } }
    private func render() {
        error.isHidden = chat.error == nil; actions.isHidden = error.isHidden
        error.stringValue = chat.error.map { chat.sendUploadFailed ? $0 : "\(chat.sendRejected ? "Not sent." : "Not confirmed yet.") \($0)" } ?? ""
        retry.title = chat.sendUploadFailed ? "Retry" : "Retry send"; retry.isHidden = chat.sendRejected
        edit.isHidden = !chat.sendRejected || pending.gif != nil; dismiss.isHidden = !chat.sendRejected
        edit.isEnabled = pending.threadRootId == nil ? chat.draft.isEmpty && chat.attachmentDrafts.isEmpty
            : pending.threadRootId == chat.threadRootID && chat.threadDraft.isEmpty && chat.threadAttachmentDrafts.isEmpty
        edit.toolTip = edit.isEnabled ? nil : "Clear your current draft to edit this message."
        heightChanged?()
    }
}
#elseif os(iOS)
import UIKit

@MainActor public final class UIKitNativeMessageExtras: UIStackView {
    private let chat: ChatModel
    private let present: UIKitHosting.Present
    private let openForward: (ChatMessage) -> Void
    private var message: ChatMessage?, pending: PendingMessage?
    private var files: UIKitMessageFiles?
    private var gif: UIKitMessageGIF?
    private let embeds = UIKitMessageEmbeds()
    private let notified = UIKitMessageDialog.label(FeedbackInbox.notified, size: 11)
    private var forward: UIStackView?, original: UIKitOriginalMessage?, forwardButton: UIButton?
    /// Message link cards, rebuilt only when their answers change.
    private var shownLinks: [MessageLinkCards.Card] = [], linkViews: [UIView] = []
    public init(chat: ChatModel, present: @escaping UIKitHosting.Present, openForward: @escaping (ChatMessage) -> Void) {
        self.chat = chat; self.present = present; self.openForward = openForward
        super.init(frame: .zero); axis = .vertical; spacing = 5
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public static func shows(_ message: ChatMessage) -> Bool {
        !message.isDeleted && (!(message.content.attachments ?? []).isEmpty || message.content.gif != nil || !(message.content.embeds ?? []).isEmpty || message.forward != nil || MentionAutocomplete.asksFeedback(message)
            || MessageLinkCards.hasLinks(message))
    }
    public static func shows(_ pending: PendingMessage) -> Bool { !pending.attachments.isEmpty || pending.gif != nil || !pending.embeds.isEmpty }
    public func update(_ message: ChatMessage) {
        if self.message?.id != message.id || pending != nil { reset() }
        self.message = message; pending = nil; render()
    }
    public func update(_ pending: PendingMessage) {
        if self.pending?.id != pending.id || message != nil { reset() }
        self.pending = pending; message = nil; render()
    }
    private func reset() {
        arrangedSubviews.forEach { $0.removeFromSuperview() }; files = nil; gif = nil; forward = nil; original = nil; forwardButton = nil
        shownLinks = []; linkViews = []
    }
    /// A message link's card: the original with its previews and Jump to
    /// message, or a neutral notice when this account can't see it.
    private func linkCard(_ target: MessageLinkTarget, _ card: MessageLinkCards.Card) -> UIView {
        guard case let .original(source) = card else {
            let locked = UIKitMessageDialog.label("Message you can\u{2019}t view", size: 12)
            locked.accessibilityIdentifier = "message-link-locked-\(target.messageID)"; return locked
        }
        let stack = UIStackView(); stack.axis = .vertical; stack.spacing = 8
        stack.isLayoutMarginsRelativeArrangement = true; stack.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 12, leading: 12, bottom: 12, trailing: 12)
        stack.backgroundColor = UIColor(CaperTheme.surface); stack.layer.cornerRadius = 8; stack.layer.borderWidth = 1; stack.layer.borderColor = UIColor(CaperTheme.border).cgColor
        stack.accessibilityIdentifier = "message-link-card-\(target.messageID)"
        stack.addArrangedSubview(UIKitMessageDialog.label("Linked message", size: 11))
        stack.addArrangedSubview(UIKitOriginalMessage(source, show: { [weak self] in self?.show($0) }))
        if !source.isDeleted, let cards = source.content.embeds, !cards.isEmpty {
            let embeds = UIKitMessageEmbeds(); embeds.update(cards); stack.addArrangedSubview(embeds)
        }
        let jump = UIKitMessageDialog.button("Jump to message") { if !SwipeTapGuard.swiping { MessageLinkCards.shared.jump(target) } }
        jump.accessibilityIdentifier = "message-link-jump-\(target.messageID)"; stack.addArrangedSubview(jump)
        return stack
    }
    private func available() -> Bool {
        guard let message else { return pending != nil }
        return chat.channelID == message.channelId && (chat.displayedMessages.first { $0.id == message.id }
            ?? chat.displayedPins.first { $0.id == message.id })?.isDeleted == false
    }
    private func show(_ request: MediaViewerRequest) {
        let box = ExtrasViewerBox()
        let viewer = UIKitMediaViewer(request: request) { box.value?.dismiss(animated: false) }; box.value = viewer
        present(viewer)
    }
    private func render() {
        let attachments = message?.isDeleted == true ? [] : message?.content.attachments ?? pending?.attachments ?? []
        if files == nil && !attachments.isEmpty {
            files = UIKitMessageFiles(chat: pending == nil ? chat : nil, available: { [weak self] in self?.available() == true }, show: { [weak self] in self?.show($0) })
        }
        files?.update(attachments); files?.alpha = pending == nil ? 1 : 0.7
        let value = message?.isDeleted == true ? nil : message?.content.gif ?? pending?.gif
        if let value {
            if let gif { gif.update(value) } else { gif = UIKitMessageGIF(value, show: { [weak self] in self?.show($0) }) }; gif?.alpha = pending == nil ? 1 : 0.7
        } else { gif?.removeFromSuperview(); gif = nil }
        let cards = message?.isDeleted == true ? [] : message?.content.embeds ?? pending?.embeds ?? []
        embeds.update(cards)
        var rows: [UIView] = []
        if !attachments.isEmpty, let files { rows.append(files) }; if let gif { rows.append(gif) }; if !cards.isEmpty { rows.append(embeds) }
        if let message, !message.isDeleted, MentionAutocomplete.asksFeedback(message) { notified.accessibilityIdentifier = "feedback-notified-\(message.id)"; rows.append(notified) }
        if let message, !message.isDeleted, let source = message.forward {
            if forward == nil {
                let card = UIStackView(); card.axis = .vertical; card.spacing = 8
                card.isLayoutMarginsRelativeArrangement = true; card.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 12, leading: 12, bottom: 12, trailing: 12)
                card.backgroundColor = UIColor(CaperTheme.surface); card.layer.cornerRadius = 8; card.layer.borderWidth = 1; card.layer.borderColor = UIColor(CaperTheme.border).cgColor
                card.accessibilityIdentifier = "forward-card-\(message.id)"; forward = card
            }
            if let source = source.message {
                if let original { original.update(source) }
                else { original = UIKitOriginalMessage(source, available: { [weak self] in self?.available() == true }, show: { [weak self] in self?.show($0) }) }
                if forwardButton == nil {
                    forwardButton = UIKitMessageDialog.button("View conversation") { [weak self] in
                        guard let self, available(), let message = self.message, let current = chat.displayedMessages.first(where: { $0.id == message.id })
                            ?? chat.displayedPins.first(where: { $0.id == message.id }), current.forward?.message != nil else { return }; openForward(current)
                    }
                }
                let replies = source.thread?.replyCount ?? 0
                forwardButton?.configuration?.title = (replies > 0 ? "\(replies) \(replies == 1 ? "reply" : "replies") · " : "") + "View conversation"
                forwardButton?.accessibilityIdentifier = "open-forward-\(message.id)"
                if original?.superview == nil {
                    forward?.arrangedSubviews.forEach { $0.removeFromSuperview() }
                    forward?.addArrangedSubview(UIKitMessageDialog.label("Forwarded", size: 11)); forward?.addArrangedSubview(original!); forward?.addArrangedSubview(forwardButton!)
                }
            } else {
                original?.removeFromSuperview(); original = nil; forwardButton = nil
                forward?.arrangedSubviews.forEach { $0.removeFromSuperview() }
                forward?.addArrangedSubview(UIKitMessageDialog.label("Forwarded", size: 11)); forward?.addArrangedSubview(UIKitMessageDialog.label("Original conversation unavailable."))
            }
            if let forward { rows.append(forward) }
        }
        let linked = message.map { MessageLinkCards.shared.cards(for: $0) } ?? []
        if linked.map({ $0.card }) != shownLinks {
            shownLinks = linked.map { $0.card }
            linkViews = linked.map { linkCard($0.target, $0.card) }
        }
        rows += linkViews
        for view in arrangedSubviews where !rows.contains(where: { $0 === view }) { view.removeFromSuperview() }
        for (index, view) in rows.enumerated() where !arrangedSubviews.indices.contains(index) || arrangedSubviews[index] !== view {
            removeArrangedSubview(view); insertArrangedSubview(view, at: index)
        }
    }
}

private final class ExtrasViewerBox { weak var value: UIViewController? }
#endif
