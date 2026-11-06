import AppKit
import CaperCore

/// Requests and blocked DMs cannot expose a composer. Operations stay scoped
/// to their conversation even when the reader navigates while awaiting a reply.
@MainActor final class DirectConversationBar: NativeView {
    private let model: AppModel
    private let notice = NativeStyle.label("", size: 13)
    private let errorLabel = NativeStyle.label("", size: 12)
    private var pending: (id: String, action: String)?
    private var failure: (id: String, message: String)?
    var changed: (() -> Void)?
    var block: ((BlockTarget) -> Void)?
    private lazy var accept = NativeButton("Accept", primary: true) { [weak self] in
        self?.run("accept") { model, conversation in try await model.acceptRequest(conversation) }
    }
    private lazy var decline = NativeButton("Decline") { [weak self] in
        self?.run("decline") { model, conversation in try await model.declineRequest(conversation) }
    }
    private lazy var blockButton = NativeButton("Block") { [weak self] in
        guard let self, let conversation = self.model.selectedDirectMessage, conversation.peer.canBlock else { return }
        self.block?(BlockTarget(peer: conversation.peer))
    }
    private lazy var unblock = NativeButton("Unblock") { [weak self] in
        self?.run("unblock") { model, conversation in try await model.unblock(accountID: conversation.peer.id) }
    }

    init(model: AppModel) {
        self.model = model
        super.init()
        [notice, errorLabel, accept, decline, blockButton, unblock].forEach { addSubview($0) }
        [notice, errorLabel].forEach { $0.lineBreakMode = .byWordWrapping; $0.maximumNumberOfLines = 0 }
        errorLabel.textColor = NSColor(CaperTheme.error)
        accept.setAccessibilityIdentifier("message-request-accept")
        decline.setAccessibilityIdentifier("message-request-decline")
        blockButton.setAccessibilityIdentifier("message-request-block")
        unblock.setAccessibilityIdentifier("dm-blocked-unblock")
        errorLabel.setAccessibilityIdentifier("direct-conversation-error")
        update()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update() {
        let geometry = (isHidden, notice.stringValue, errorLabel.stringValue)
        let conversation = model.selectedDirectMessage
        let incoming = conversation?.status == .incoming
        let blocked = conversation?.blocked == true && conversation?.peer.id != model.account?.id
        isHidden = !incoming && !blocked
        setAccessibilityIdentifier(incoming ? "message-request-bar" : "dm-blocked-bar")
        notice.stringValue = conversation.flatMap(NativePresentation.directMessageNotice) ?? ""
        notice.setAccessibilityIdentifier(incoming ? "message-request-prompt" : "dm-blocked-notice")
        errorLabel.stringValue = failure?.id == conversation?.id ? failure?.message ?? "" : ""
        errorLabel.isHidden = errorLabel.stringValue.isEmpty
        accept.isHidden = !incoming; decline.isHidden = !incoming
        blockButton.isHidden = !incoming || conversation?.peer.canBlock != true
        unblock.isHidden = incoming || !blocked
        let busy = pending != nil && pending?.id == conversation?.id
        [accept, decline, blockButton, unblock].forEach { $0.isEnabled = !busy }
        accept.title = busy && pending?.action == "accept" ? "Accepting…" : "Accept"
        decline.title = busy && pending?.action == "decline" ? "Declining…" : "Decline"
        unblock.title = busy ? "Unblocking…" : "Unblock"
        // Draft observation also reaches this gate. Typing must not resize the
        // conversation unless its notice, error or visibility actually changes.
        if geometry != (isHidden, notice.stringValue, errorLabel.stringValue) {
            needsLayout = true
            changed?()
        }
    }

    func preferredHeight(width: CGFloat) -> CGFloat {
        76 + textHeight(notice, width: width) + (errorLabel.isHidden ? 0 : textHeight(errorLabel, width: width) + 8)
    }

    private func textHeight(_ field: NSTextField, width: CGFloat) -> CGFloat {
        ceil(field.attributedStringValue.boundingRect(with: NSSize(width: max(1, width), height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]).height) + 4
    }

    override func layout() {
        super.layout()
        notice.frame = NSRect(x: 0, y: 8, width: bounds.width, height: textHeight(notice, width: bounds.width))
        let top = notice.frame.maxY + 10
        accept.frame = NSRect(x: 0, y: top, width: 128, height: 38)
        decline.frame = NSRect(x: 136, y: top, width: 100, height: 38)
        blockButton.frame = NSRect(x: 244, y: top, width: 88, height: 38)
        unblock.frame = NSRect(x: 0, y: top, width: 128, height: 38)
        errorLabel.frame = NSRect(x: 0, y: top + 46, width: bounds.width, height: textHeight(errorLabel, width: bounds.width))
    }

    private func run(_ action: String, _ operation: @escaping @MainActor (AppModel, DirectMessageConversation) async throws -> Void) {
        guard let conversation = model.selectedDirectMessage, pending?.id != conversation.id else { return }
        pending = (conversation.id, action); failure = nil; update()
        Task {
            do { try await operation(model, conversation) }
            catch { if model.selectedDirectMessageID == conversation.id { failure = (conversation.id, NativePresentation.errorMessage(error)) } }
            if pending?.id == conversation.id { pending = nil }
            update()
        }
    }
}
