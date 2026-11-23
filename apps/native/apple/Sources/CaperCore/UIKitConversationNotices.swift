#if os(iOS)
import UIKit
import Observation

/// Self-sizing native notices. The controller owns observation while attached;
/// no presentation root or SwiftUI sizing pass is needed above the composer.
@MainActor public final class UIKitConversationNotices: UIViewController {
    private let model: AppModel
    private let inThread: Bool
    private let body = UIStackView()
    private var generation = 0
    public init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func loadView() {
        view = body; body.axis = .vertical; body.spacing = 6
        body.translatesAutoresizingMaskIntoConstraints = false
        body.isLayoutMarginsRelativeArrangement = true
        body.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)
        body.setContentHuggingPriority(.required, for: .vertical)
        body.setContentCompressionResistancePriority(.required, for: .vertical)
    }
    public override func didMove(toParent parent: UIViewController?) {
        super.didMove(toParent: parent); generation += 1
        if parent != nil { loadViewIfNeeded(); observe(generation) }
    }
    private func observe(_ token: Int) {
        guard token == generation, parent != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func label(_ text: String, error: Bool = false, id: String? = nil) -> UILabel {
        let label = UIKitMessageDialog.label(text, size: 11, error: error); label.accessibilityIdentifier = id; return label
    }
    private func row(_ views: [UIView]) -> UIStackView {
        let row = UIStackView(arrangedSubviews: views); row.alignment = .center; row.spacing = 8; return row
    }
    private func render() {
        let chat = model.chat, voice = model.voice
        var rows: [UIView] = []
        if inThread {
            if chat.isPreview {
                rows.append(label(model.selectedDirectMessageID != nil ? "You can’t reply in this conversation." : "Join the channel to reply.", id: "thread-preview-note"))
            } else {
                if let error = chat.error, chat.pendingMessage == nil { rows.append(label(error, error: true)) }
                if let error = chat.sessionError { rows.append(sessionError(error)) }
                if let pending = chat.pendingMessage, pending.threadRootId != chat.threadRootID,
                   !chat.sending, chat.error != nil || chat.sendRejected {
                    rows.append(label("Confirm or dismiss the pending message before sending a reply.", error: true, id: "thread-pending-note"))
                }
            }
        } else {
            if let error = chat.sessionError { rows.append(sessionError(error)) }
            if let error = chat.error, chat.pendingMessage == nil, !chat.loadFailed, !chat.displayedMessages.isEmpty {
                rows.append(label(error, error: true, id: "chat-error"))
            }
            if let error = voice.error {
                let dismiss = UIKitMessageDialog.button("×") { voice.error = nil }; dismiss.accessibilityLabel = "Dismiss voice error"
                dismiss.widthAnchor.constraint(equalToConstant: 44).isActive = true
                let notice = row([label(error, error: true), dismiss]); notice.accessibilityIdentifier = "voice-error-inline"
                rows.append(notice)
            }
            if let conversation = model.selectedDirectMessage, conversation.peer.id != model.account?.id, conversation.status == .outgoing {
                rows.append(label(MessageRequests.waitingNotice(for: conversation.peer), id: "message-request-waiting"))
            }
            if let root = chat.pendingMessage?.threadRootId, !chat.sending,
               chat.error != nil || chat.sendRejected, chat.threadRootID != root {
                rows.append(row([label(chat.sendRejected ? "A thread reply wasn’t sent." : "A thread reply couldn’t be confirmed.", error: true),
                    UIKitMessageDialog.button("Review reply") { Task { await chat.openThread(root) } }]))
            }
        }
        body.arrangedSubviews.forEach { body.removeArrangedSubview($0); $0.removeFromSuperview() }
        rows.forEach(body.addArrangedSubview)
        body.invalidateIntrinsicContentSize()
    }
    private func sessionError(_ text: String) -> UIView {
        row([label(text, error: true), UIKitMessageDialog.button("Retry session") { [model] in Task { await model.chat.retrySession() } }])
    }
}

/// Preview, incoming-request and blocked-peer actions retain their pending
/// state for this conversation and never apply an old error to another one.
@MainActor public final class UIKitNativeDirectBar: UIStackView {
    private let model: AppModel
    private let block: (BlockTarget) -> Void
    private var generation = 0
    private var context = ""
    private var pending: String?
    private var error: String?
    private var lastShown: String?
    public init(model: AppModel, block: @escaping (BlockTarget) -> Void) {
        self.model = model; self.block = block; super.init(frame: .zero)
        axis = .vertical; spacing = 8; isLayoutMarginsRelativeArrangement = true
        directionalLayoutMargins = NSDirectionalEdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    public override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1
        if window != nil { observe(generation) }
    }
    private func observe(_ token: Int) {
        guard token == generation, window != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func render() {
        let next = "\(model.selectedChannelID ?? "")|\(model.selectedDirectMessageID ?? "")"
        if context != next { context = next; pending = nil; error = nil }
        // Rebuild only when what shows changes: replacing the buttons on every
        // observed change drops a tap that lands mid-rebuild.
        let request = model.selectedDirectMessage
        let shown = [next, "\(model.previewingChannel)", model.chat.channelName, "\(request?.status == .incoming)", "\(request?.blocked == true)",
                     request.map { MessageRequests.prompt(for: $0.peer).name + MessageRequests.prompt(for: $0.peer).detail } ?? "",
                     "\(request?.peer.canBlock == true)", pending ?? "", error ?? ""].joined(separator: "|")
        guard shown != lastShown else { return }
        lastShown = shown
        arrangedSubviews.forEach { removeArrangedSubview($0); $0.removeFromSuperview() }
        accessibilityIdentifier = nil
        if model.previewingChannel, let channel = model.selectedChannel {
            addArrangedSubview(UIKitMessageDialog.label("Preview", bold: true))
            addArrangedSubview(UIKitMessageDialog.label("Join #\(model.chat.channelName) to interact with people here"))
            addArrangedSubview(action(pending == "join" ? "Joining…" : "Join channel", id: "join-channel", primary: true) { [weak self, model] in
                self?.run("join") { try await model.joinChannel(channel) }
            })
        } else if let request = model.selectedDirectMessage, request.status == .incoming {
            accessibilityIdentifier = "message-request-bar"
            let prompt = MessageRequests.prompt(for: request.peer)
            let label = UIKitMessageDialog.label(prompt.name + prompt.detail, size: 13); label.accessibilityIdentifier = "message-request-prompt"
            addArrangedSubview(label)
            let actions = UIStackView(); actions.spacing = 8
            actions.addArrangedSubview(action(pending == "accept" ? "Accepting…" : "Accept", id: "message-request-accept", primary: true) { [weak self, model] in
                self?.run("accept") { try await model.acceptRequest(request) }
            })
            actions.addArrangedSubview(action(pending == "decline" ? "Declining…" : "Decline", id: "message-request-decline") { [weak self, model] in
                self?.run("decline") { try await model.declineRequest(request) }
            })
            if request.peer.canBlock {
                actions.addArrangedSubview(action("Block", id: "message-request-block") { [block] in block(BlockTarget(peer: request.peer)) })
            }
            addArrangedSubview(actions)
        } else if let conversation = model.selectedDirectMessage, conversation.peer.id != model.account?.id, conversation.blocked {
            accessibilityIdentifier = "dm-blocked-bar"
            addArrangedSubview(UIKitMessageDialog.label(MessageRequests.blockedNotice(for: conversation.peer), size: 13))
            addArrangedSubview(action("Unblock", id: "dm-blocked-unblock") { [model] in UIKitDirectBar.unblock(model: model, accountID: conversation.peer.id) })
        }
        if let error {
            let label = UIKitMessageDialog.label(error, error: true); label.accessibilityIdentifier = "message-request-error"; addArrangedSubview(label)
        }
        invalidateIntrinsicContentSize()
    }
    private func action(_ title: String, id: String, primary: Bool = false, _ handler: @escaping () -> Void) -> UIButton {
        let button = UIKitMessageDialog.button(title, primary: primary, action: handler)
        button.accessibilityIdentifier = id; button.isEnabled = pending == nil; return button
    }
    private func run(_ name: String, _ operation: @escaping @MainActor () async throws -> Void) {
        guard pending == nil else { return }
        let target = context; pending = name; error = nil; render()
        Task { [weak self] in
            do { try await operation() } catch { if self?.context == target { self?.error = FriendlyError.message(for: error) } }
            if self?.context == target { self?.pending = nil; self?.render() }
        }
    }
}
#endif
