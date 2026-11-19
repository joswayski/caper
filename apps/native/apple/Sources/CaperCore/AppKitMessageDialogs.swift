#if os(macOS)
import AppKit
import SwiftUI

@MainActor final class AppKitMessageEditor: AppKitSettingsDialog {
    private let state: NativeMessageEdit
    private let close: () -> Void
    private var composer: NativeDialogComposer!
    private let count = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var reload: AppKitKitButton!
    private var save: AppKitKitButton!
    private var cancel: AppKitKitButton!
    private var attached = false
    private var wasBusy = false
    override var dismissDisabled: Bool { state.busy }

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        let state = NativeMessageEdit(chat: chat, message: message, close: close)
        self.state = state; self.close = close
        super.init(title: "Edit message", detail: "Previous versions remain visible to people who can read this message.",
            closeLabel: "Close message editor", identifier: "message-editor", close: { if !state.busy { close() } })
        composer = NativeDialogComposer(text: Binding(get: { state.draft }, set: { state.draft = $0 }),
            placeholder: "Message", identifier: "message-edit-text", height: 150)
        reload = AppKitKitButton("Discard draft and load latest") { state.reload() }
        cancel = AppKitKitButton("Cancel") { if !state.busy { close() } }; cancel.keyEquivalent = "\u{1b}"
        save = AppKitKitButton("Save changes", kind: .primary) { state.save() }
        save.keyEquivalent = "\r"; save.keyEquivalentModifierMask = .command
        let space = NSView(); space.setContentHuggingPriority(.init(1), for: .horizontal)
        let actions = NSStackView(views: [space, cancel, save]); actions.orientation = .horizontal; actions.spacing = 8
        setRows([composer, count, error, reload, actions])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func render() {
        if state.chat.isDeleted(state.baseline.id) { observation.stop(); state.stop(); close(); return }
        composer.update(editable: !state.busy)
        count.stringValue = "\(state.draft.unicodeScalars.count.formatted()) / 4,000"
        error.stringValue = state.error ?? ""; error.isHidden = state.error == nil
        reload.isHidden = state.error == nil; reload.isEnabled = !state.busy
        cancel.isEnabled = !state.busy; save.isEnabled = state.canSave
        header.close.isEnabled = !state.busy
        save.title = state.busy ? "Saving…" : "Save changes"
        if wasBusy && !state.busy { composer.focus() }
        wasBusy = state.busy
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, !attached {
            attached = true
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window != nil else { return }
                self.observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
                self.composer.focus()
            }
        } else if window == nil && attached { observation.stop(); state.stop() }
    }
}

@MainActor final class AppKitForwardPicker: AppKitSettingsDialog, NSTextFieldDelegate {
    private let state: NativeForwardPicker
    private let search = AppKitKitField(height: 36)
    private let destinations = AppKitKit.stack([], spacing: 4)
    private var rows: [String: AppKitKitButton] = [:]
    private var visible: [String] = []
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var retry: AppKitKitButton!
    private var send: AppKitKitButton!
    private var composer: NativeDialogComposer!
    private var attached = false

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void, showMedia: @escaping (MediaViewerRequest) -> Void) {
        let state = NativeForwardPicker(chat: chat, message: message, close: close)
        self.state = state
        super.init(title: "Forward message", detail: "Shares this conversation, including future edits, reactions and replies. People in the destination can read and forward it.",
            closeLabel: "Close", identifier: "forward-picker", close: close)
        search.placeholderString = "Find a space, channel or DM"; search.delegate = self
        search.setAccessibilityLabel("Find a space, channel or DM")
        composer = NativeDialogComposer(text: Binding(get: { state.note }, set: { state.note = $0 }),
            placeholder: "Add a note (optional)", identifier: "forward-note", height: 64)
        retry = AppKitKitButton("Retry loading") { state.load() }
        send = AppKitKitButton("Forward (0)", kind: .primary) { state.send() }
        let preview = AppKitOriginalMessage(message.forward?.message ?? message, show: showMedia)
        setRows([preview, search, destinations, status,
                 composer, error, retry, send])
        preferHeight(560)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func controlTextDidChange(_ notification: Notification) { state.search = search.stringValue }

    private func render() {
        search.isEnabled = state.editable
        let next = state.visible.map(\.id)
        if next != visible {
            destinations.arrangedSubviews.forEach { $0.removeFromSuperview() }
            visible = next; rows = [:]
            for destination in state.visible {
                let row = AppKitKitButton("\(destination.direct ? "" : "# ")\(destination.name) · \(destination.spaceName)", kind: .plain) { [state] in
                    state.toggle(destination.id)
                }
                row.minimumHeight = 44; row.alignment = .left; row.imagePosition = .imageRight
                row.setAccessibilityIdentifier("forward-destination-\(destination.id)")
                destinations.addArrangedSubview(row); AppKitKit.fillWidth(row, of: destinations); rows[destination.id] = row
            }
        }
        for (id, row) in rows {
            let selected = state.selected.contains(id)
            row.image = NSImage(systemSymbolName: selected ? "checkmark.square.fill" : "square", accessibilityDescription: nil)
            row.contentTintColor = selected ? AppKitKit.terracottaBright : AppKitKit.muted
            row.setAccessibilityValue(selected ? "Selected" : "Not selected"); row.isEnabled = state.editable
        }
        status.stringValue = state.destinations == nil && state.error == nil ? "Loading destinations…"
            : state.destinations?.isEmpty == true ? "Join a channel or start a DM to forward here."
            : state.destinations != nil && state.visible.isEmpty ? "No matching destinations." : ""
        status.isHidden = status.stringValue.isEmpty
        composer.update(editable: state.editable)
        error.stringValue = state.error ?? ""; error.isHidden = state.error == nil
        retry.isHidden = state.destinations != nil || state.error == nil
        send.title = state.action; send.isEnabled = state.canSend
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, !attached {
            attached = true
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window != nil else { return }
                self.observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
                self.state.load()
            }
        } else if window == nil && attached { observation.stop(); state.stop() }
    }
}

@MainActor final class AppKitForwardConversation: AppKitSettingsDialog {
    private let state: NativeForwardConversation
    private let showMedia: (MediaViewerRequest) -> Void
    private let messages = AppKitKit.stack([], spacing: 16)
    private var previews: [String: (message: ChatMessage, host: AppKitOriginalMessage)] = [:]
    private let count = AppKitKit.label("", size: 13, weight: .bold)
    private let status = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var older: AppKitKitButton!
    private var retry: AppKitKitButton!
    private var attached = false

    init(chat: ChatModel, messageID: String, close: @escaping () -> Void, showMedia: @escaping (MediaViewerRequest) -> Void) {
        let state = NativeForwardConversation(chat: chat, messageID: messageID, close: close)
        self.state = state; self.showMedia = showMedia
        super.init(title: "Forwarded conversation", detail: "", closeLabel: "Close", identifier: "forward-conversation", close: close)
        older = AppKitKitButton("Load older replies") { state.older() }
        retry = AppKitKitButton("Retry") { state.load() }
        setRows([count, older, messages, status, error, retry])
        preferHeight(560)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func render() {
        state.refreshIfNeeded()
        let next = state.root.map { [$0] + (state.conversation?.messages ?? []) } ?? []
        let ids = Set(next.map(\.id))
        for id in previews.keys.filter({ !ids.contains($0) }) { previews.removeValue(forKey: id)?.host.removeFromSuperview() }
        for (index, message) in next.enumerated() {
            let host: AppKitOriginalMessage
            if let previous = previews[message.id] {
                host = previous.host
                if previous.message != message {
                    host.update(message)
                }
            } else {
                host = AppKitOriginalMessage(message, available: { [state] in state.available }, show: showMedia)
                messages.insertArrangedSubview(host, at: index)
                AppKitKit.fillWidth(host, of: messages)
            }
            previews[message.id] = (message, host)
            if !messages.arrangedSubviews.indices.contains(index) || messages.arrangedSubviews[index] !== host {
                messages.removeArrangedSubview(host); messages.insertArrangedSubview(host, at: index)
            }
        }
        let replies = state.conversation?.root?.thread?.replyCount ?? state.conversation?.messages.count ?? 0
        count.stringValue = "\(replies) \(replies == 1 ? "reply" : "replies")"; count.isHidden = replies == 0
        older.isHidden = state.conversation?.hasMore != true; older.isEnabled = !state.loading
        status.stringValue = state.loading ? "Loading replies…" : state.conversation?.messages.isEmpty == true ? "No replies yet." : ""
        status.isHidden = status.stringValue.isEmpty
        error.stringValue = state.error ?? ""; error.isHidden = state.error == nil; retry.isHidden = state.error == nil
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, !attached {
            attached = true
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window != nil else { return }
                self.observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
            }
        } else if window == nil && attached { observation.stop(); state.stop() }
    }
}

#endif
