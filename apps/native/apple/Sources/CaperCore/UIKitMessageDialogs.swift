#if os(iOS)
import UIKit
import SwiftUI
import Observation

/// The existing sheet router owns presentation; its contents are UIKit.
struct UIKitMessageDialogSheet: View {
    @State private var controller: UIKitMessageDialog
    init(_ controller: UIKitMessageDialog) { _controller = State(initialValue: controller) }
    var body: some View {
        UIKitMessageDialogBridge(controller: controller)
            .interactiveDismissDisabled(controller.dismissDisabled)
            .presentationDetents([.large]).preferredColorScheme(CaperTheme.colorScheme)
    }
}

private struct UIKitMessageDialogBridge: UIViewControllerRepresentable {
    let controller: UIKitMessageDialog
    func makeUIViewController(context: Context) -> UIKitMessageDialog { controller }
    func updateUIViewController(_ controller: UIKitMessageDialog, context: Context) {}
    static func dismantleUIViewController(_ controller: UIKitMessageDialog, coordinator: ()) { controller.stop() }
}

@MainActor class UIKitMessageDialog: UIViewController, UIAdaptivePresentationControllerDelegate {
    let body = UIStackView()
    let close: () -> Void
    private var active = true
    private var closeButton: UIButton?
    private var originals: [String: (message: ChatMessage, view: UIKitOriginalMessage)] = [:]
    var dismissDisabled: Bool { false }
    /// False for a dialog whose own footer button already closes it.
    var showsCloseButton: Bool { true }
    /// A Settings sub-page: the navigation bar's Back leaves it, so it has no
    /// Close button and the bar repeats no title. Set before it loads.
    var embedded = false

    private let closeLabel: String
    private var header: UIStackView?
    private var titleLabel: UILabel?
    private var scrollBottom: NSLayoutConstraint!
    private var scroll: UIScrollView!
    private var footer: UIView?
    var idealHeight: CGFloat?

    init(title: String, closeLabel: String = "Close", close: @escaping () -> Void) {
        self.close = close; self.closeLabel = closeLabel
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if presentationController?.delegate == nil { presentationController?.delegate = self }
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || isMovingFromParent { stop() }
    }
    func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool { !dismissDisabled }
    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { stop(); close() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(CaperTheme.raised); overrideUserInterfaceStyle = CaperTheme.interfaceStyle
        let title = Self.label(self.title ?? "", size: 20, bold: true)
        titleLabel = title
        let close = Self.button("Close") { [weak self] in if let self, !self.dismissDisabled { self.close() } }
        close.accessibilityLabel = closeLabel; closeButton = close
        close.isHidden = !showsCloseButton || embedded
        if embedded { navigationItem.title = "" }
        let header = Self.header(title, close); self.header = header
        let scroll = UIScrollView(); scroll.keyboardDismissMode = .interactive; self.scroll = scroll
        body.axis = .vertical; body.spacing = 14
        for child in [header, scroll] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        body.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(body)
        scrollBottom = scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor)
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            header.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            header.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            scroll.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 12),
            scroll.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            scrollBottom,
            body.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
            body.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            body.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 4),
            body.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -20),
            body.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
    }
    /// The height the header and every row need at this width, for hosts
    /// that size a dialog to its content rather than giving it a full sheet.
    func fittingHeight(width: CGFloat) -> CGFloat {
        loadViewIfNeeded()
        // Rows lay out inside the safe area, so a side inset wraps them onto more lines.
        let insets = view.safeAreaInsets
        let inner = CGSize(width: max(1, width - insets.left - insets.right - 32), height: UIView.layoutFittingCompressedSize.height)
        func fit(_ view: UIView?) -> CGFloat {
            view?.systemLayoutSizeFitting(inner, withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel).height ?? 0
        }
        return idealHeight ?? ceil(16 + fit(header) + 12 + 4 + fit(body) + 20 + fit(footer))
    }
    func setTitle(_ title: String) { self.title = title; titleLabel?.text = title }
    func setFooter(_ footer: UIView) {
        self.footer = footer; scrollBottom.isActive = false
        footer.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(footer)
        NSLayoutConstraint.activate([
            scroll.bottomAnchor.constraint(equalTo: footer.topAnchor),
            footer.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            footer.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            footer.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
        ])
    }
    func observe() {
        guard active else { return }
        withObservationTracking {
            render()
            updateDismissGuard()
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in self?.observe() }
        }
        var ancestor = parent
        while let controller = ancestor {
            if controller is UIKitWorkspaceFrame { controller.view.setNeedsLayout(); break }
            ancestor = controller.parent
        }
    }
    func render() {}
    func updateDismissGuard() { closeButton?.isEnabled = !dismissDisabled }
    func stop() { active = false }

    static func label(_ text: String, size: CGFloat = 12, bold: Bool = false, error: Bool = false) -> UILabel {
        let label = UILabel(); label.text = text; label.numberOfLines = 0
        label.font = UIFontMetrics(forTextStyle: .body).scaledFont(for:
            UIFont(name: bold ? "Figtree-Bold" : "Figtree-Regular", size: size) ?? .systemFont(ofSize: size))
        label.adjustsFontForContentSizeCategory = true
        label.textColor = UIColor(error ? CaperTheme.error : CaperTheme.text)
        return label
    }
    /// A dialog header: the title wraps at large text sizes instead of
    /// clipping, and the trailing button keeps its full width. At
    /// accessibility sizes the button goes below the title, so a long word
    /// such as "Notifications" keeps the whole width and never breaks mid-word.
    static func header(_ title: UILabel, _ trailing: UIView, spacing: CGFloat = 12) -> UIStackView {
        title.setContentHuggingPriority(.defaultLow, for: .horizontal)
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        trailing.setContentHuggingPriority(.required, for: .horizontal)
        trailing.setContentCompressionResistancePriority(.required, for: .horizontal)
        let header = UIStackView(arrangedSubviews: [title, trailing]); header.spacing = spacing
        func fit(_ header: UIStackView) {
            let stacked = header.traitCollection.preferredContentSizeCategory.isAccessibilityCategory
            header.axis = stacked ? .vertical : .horizontal; header.alignment = stacked ? .leading : .center
        }
        fit(header)
        header.registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (header: UIStackView, _) in fit(header) }
        return header
    }
    /// `icon`: an SF Symbol before the title, such as Copy's `doc.on.doc`.
    static func button(_ title: String, primary: Bool = false, icon: String? = nil, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.filled()
        config.title = title; config.cornerStyle = .fixed; config.background.cornerRadius = 8
        if let icon {
            config.image = UIImage(systemName: icon, withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold))
            config.imagePlacement = .leading; config.imagePadding = 6
        }
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            attributes.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Figtree-Bold", size: 13) ?? .systemFont(ofSize: 13))
            return attributes
        }
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        button.configurationUpdateHandler = { button in
            var config = button.configuration
            let filled = primary && button.isEnabled
            config?.baseBackgroundColor = UIColor(filled ? CaperTheme.action : CaperTheme.surface)
            config?.baseForegroundColor = UIColor(filled ? CaperTheme.onAction : button.isEnabled ? CaperTheme.text : CaperTheme.muted)
            button.configuration = config
        }
        button.setNeedsUpdateConfiguration()
        return button
    }
}

@MainActor final class UIKitMessageEditor: UIKitMessageDialog {
    private let state: NativeMessageEdit
    private let context: String
    private var composer: NativeDialogComposer!
    private let count = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private var reload: UIButton!
    private var cancel: UIButton!
    private var save: UIButton!
    private var wasBusy = false
    override var dismissDisabled: Bool { state.busy }

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        state = NativeMessageEdit(chat: chat, message: message, close: close)
        context = chat.editingContext
        super.init(title: "Edit message", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        composer = NativeDialogComposer(text: Binding(get: { [state] in state.draft }, set: { [state] in state.draft = $0 }),
            placeholder: "Message", identifier: "message-edit-text", height: 150)
        reload = Self.button("Discard draft and load latest") { [state] in state.reload() }
        cancel = Self.button("Cancel") { [weak self] in if let self, !self.state.busy { self.close() } }
        save = Self.button("Save changes", primary: true) { [state] in state.save() }
        let actions = UIStackView(arrangedSubviews: [cancel, save]); actions.spacing = 8; actions.distribution = .fillEqually
        let rows: [UIView] = [Self.label("Previous versions remain visible to people who can read this message."), composer, count, error, reload, actions]
        rows.forEach { body.addArrangedSubview($0) }
        observe()
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); composer.focus() }
    override var keyCommands: [UIKeyCommand]? { [UIKeyCommand(input: "\r", modifierFlags: .command, action: #selector(saveMessage))] }
    @objc private func saveMessage() { state.save() }
    override func render() {
        if state.chat.editingContext != context || state.chat.isDeleted(state.baseline.id) { stop(); close(); return }
        composer.update(editable: !state.busy)
        let length = state.draft.unicodeScalars.count
        count.text = "\(length.formatted()) / 4,000"; count.isHidden = length < 3000
        count.textColor = ComposerCounter.uiColor(length)
        error.text = state.error; error.isHidden = state.error == nil
        reload.isHidden = state.error == nil; reload.isEnabled = !state.busy
        cancel.isEnabled = !state.busy; save.isEnabled = state.canSave
        save.configuration?.title = state.busy ? "Saving…" : "Save changes"
        isModalInPresentation = state.busy
        if wasBusy && !state.busy { composer.focus() }
        wasBusy = state.busy
    }
    override func stop() { super.stop(); state.stop() }
}

@MainActor final class UIKitForwardPicker: UIKitMessageDialog {
    private let state: NativeForwardPicker
    private let context: String
    private let search = UITextField()
    private let destinations = UIStackView()
    private var rows: [String: UIButton] = [:]
    private var visible: [String] = []
    private let status = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private var retry: UIButton!
    private var send: UIButton!
    private var composer: NativeDialogComposer!

    init(chat: ChatModel, message: ChatMessage, close: @escaping () -> Void) {
        state = NativeForwardPicker(chat: chat, message: message, close: close); context = chat.editingContext
        super.init(title: "Forward message", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        search.placeholder = "Find a space, channel or DM"; search.accessibilityLabel = search.placeholder
        search.borderStyle = .roundedRect; search.textColor = UIColor(CaperTheme.text)
        search.autocorrectionType = .no; search.autocapitalizationType = .none
        search.addTarget(self, action: #selector(searchChanged), for: .editingChanged)
        search.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        destinations.axis = .vertical; destinations.spacing = 4
        composer = NativeDialogComposer(text: Binding(get: { [state] in state.note }, set: { [state] in state.note = $0 }),
            placeholder: "Add a note (optional)", identifier: "forward-note", height: 64)
        retry = Self.button("Retry loading") { [state] in state.load() }
        send = Self.button("Forward (0)", primary: true) { [state] in state.send() }
        let preview = UIStackView(); preview.axis = .vertical
        let rows: [UIView] = [Self.label("Shares this conversation, including future edits, reactions and replies. People in the destination can read and forward it."),
                             preview, search, destinations, status, composer, error, retry, send]
        rows.forEach { body.addArrangedSubview($0) }
        setOriginals([state.message.forward?.message ?? state.message], in: preview)
        observe(); state.load()
    }
    @objc private func searchChanged() { state.search = search.text ?? "" }
    override func render() {
        if state.chat.editingContext != context { stop(); close(); return }
        search.isEnabled = state.editable
        let next = state.visible.map(\.id)
        if visible != next {
            destinations.arrangedSubviews.forEach { $0.removeFromSuperview() }; visible = next; rows = [:]
            for destination in state.visible {
                let row = Self.button("\(destination.direct ? "" : "# ")\(destination.name)") { [state] in state.toggle(destination.id) }
                row.configuration?.subtitle = destination.spaceName
                row.configuration?.imagePlacement = .trailing; row.configuration?.imagePadding = 12
                // Name and space on the left, the checkbox at the trailing edge, as on Mac.
                row.configuration?.titleAlignment = .leading
                row.contentHorizontalAlignment = .fill
                row.accessibilityIdentifier = "forward-destination-\(destination.id)"
                destinations.addArrangedSubview(row); rows[destination.id] = row
            }
        }
        for (id, row) in rows {
            let selected = state.selected.contains(id)
            row.configuration?.image = UIImage(systemName: selected ? "checkmark.square.fill" : "square")
            row.accessibilityValue = selected ? "Selected" : "Not selected"
            row.accessibilityTraits = selected ? [.button, .selected] : .button
            row.isEnabled = state.editable
        }
        status.text = state.destinations == nil && state.error == nil ? "Loading destinations…"
            : state.destinations?.isEmpty == true ? "Join a channel or start a DM to forward here."
            : state.destinations != nil && state.visible.isEmpty ? "No matching destinations." : ""
        status.isHidden = status.text?.isEmpty != false
        composer.update(editable: state.editable)
        error.text = state.error; error.isHidden = state.error == nil
        retry.isHidden = state.destinations != nil || state.error == nil
        send.configuration?.title = state.action; send.isEnabled = state.canSend
    }
    override func stop() { super.stop(); state.stop() }
}

@MainActor final class UIKitForwardConversation: UIKitMessageDialog {
    private let state: NativeForwardConversation
    private let context: String
    private let messages = UIStackView()
    private let count = UIKitMessageDialog.label("", size: 13, bold: true)
    private let status = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private var older: UIButton!
    private var retry: UIButton!

    init(chat: ChatModel, messageID: String, close: @escaping () -> Void) {
        state = NativeForwardConversation(chat: chat, messageID: messageID, close: close); context = chat.editingContext
        super.init(title: "Forwarded conversation", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        older = Self.button("Load older replies") { [state] in state.older() }
        retry = Self.button("Retry") { [state] in state.load() }
        messages.axis = .vertical; messages.spacing = 16
        let rows: [UIView] = [count, older, messages, status, error, retry]
        rows.forEach { body.addArrangedSubview($0) }
        observe()
    }
    override func render() {
        if state.chat.editingContext != context { stop(); close(); return }
        state.refreshIfNeeded()
        let next = state.root.map { [$0] + (state.conversation?.messages ?? []) } ?? []
        setOriginals(next, in: messages, available: { [state] in state.available })
        let replies = state.conversation?.root?.thread?.replyCount ?? state.conversation?.messages.count ?? 0
        count.text = "\(replies) \(replies == 1 ? "reply" : "replies")"; count.isHidden = replies == 0
        older.isHidden = state.conversation?.hasMore != true; older.isEnabled = !state.loading
        status.text = state.loading ? "Loading replies…" : state.conversation?.messages.isEmpty == true ? "No replies yet." : ""
        status.isHidden = status.text?.isEmpty != false
        error.text = state.error; error.isHidden = state.error == nil; retry.isHidden = state.error == nil
    }
    override func stop() { super.stop(); state.stop() }
}

extension UIKitMessageDialog {
    /// Retain realized originals across observation updates, including a
    /// revealed spoiler or a playing attachment inside the forward preview.
    fileprivate func setOriginals(_ messages: [ChatMessage], in stack: UIStackView, available: (() -> Bool)? = nil) {
        let ids = Set(messages.map(\.id))
        for id in originals.keys.filter({ !ids.contains($0) }) {
            originals.removeValue(forKey: id)?.view.removeFromSuperview()
        }
        for (index, message) in messages.enumerated() {
            let view: UIKitOriginalMessage
            if let previous = originals[message.id] {
                view = previous.view
                if previous.message != message { view.update(message) }
            } else {
                view = UIKitOriginalMessage(message, available: available) { [weak self] request in
                    guard let self, self.presentedViewController == nil else { return }
                    let viewer = UIKitMediaViewer(request: request) { [weak self] in self?.presentedViewController?.dismiss(animated: false) }
                    viewer.modalPresentationStyle = .fullScreen
                    self.present(viewer, animated: false)
                }
                stack.insertArrangedSubview(view, at: index)
            }
            originals[message.id] = (message, view)
            if !stack.arrangedSubviews.indices.contains(index) || stack.arrangedSubviews[index] !== view {
                stack.removeArrangedSubview(view); stack.insertArrangedSubview(view, at: index)
            }
        }
    }
}
#endif
