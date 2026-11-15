#if os(iOS)
import UIKit

/// Username suggestions stay in the dialog's layout, never in a detached
/// presentation. Return picks a suggestion; forms decide what an empty list does.
@MainActor final class UIKitPeopleField: UIStackView, UITextFieldDelegate {
    let field = UIKitPeopleInput()
    private let list = UIStackView(), scroll = UIScrollView()
    private var listHeight: NSLayoutConstraint!
    private var state = PeopleSuggestionState()
    private var choices: [MentionCandidate] = []
    private var buttons: [UIButton] = []
    private let normalize: (String) -> String
    private(set) var text = ""
    var candidates: [MentionCandidate] = [] { didSet { refresh() } }
    var editable = true { didSet { refresh() } }
    var changed: ((String) -> Void)?
    var submit: (() -> Void)?
    var heightChanged: (() -> Void)?
    init(identifier: String, listIdentifier: String, normalize: @escaping (String) -> String) {
        self.normalize = normalize; super.init(frame: .zero); axis = .vertical; spacing = 6
        field.placeholder = "Exact username"; field.accessibilityLabel = "Exact username"; field.accessibilityIdentifier = identifier
        field.font = UIFont(name: "Satoshi-Regular", size: 14); field.borderStyle = .roundedRect
        field.autocapitalizationType = .none; field.autocorrectionType = .no; field.delegate = self
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        field.addAction(UIAction { [weak self] _ in self?.edited() }, for: .editingChanged)
        field.handleKey = { [weak self] input in self?.key(input) }
        list.axis = .vertical; list.spacing = 0; list.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(list); scroll.accessibilityIdentifier = listIdentifier; scroll.backgroundColor = UIColor(CaperTheme.surface)
        scroll.layer.cornerRadius = 8; scroll.layer.borderWidth = 1; scroll.layer.borderColor = UIColor(CaperTheme.border).cgColor
        listHeight = scroll.heightAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([list.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            list.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor), list.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            list.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor), list.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor), listHeight])
        addArrangedSubview(field); addArrangedSubview(scroll); scroll.isHidden = true
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func setText(_ value: String) { text = value; field.text = value; state.textChanged(to: value); refresh() }
    func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool { editable }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if let candidate = state.highlightedChoice(in: choices) { pick(candidate) } else if editable { submit?() }; return false
    }
    private func edited() {
        guard editable else { field.text = text; return }
        // Let marked text finish before normalization changes its range.
        guard field.markedTextRange == nil else { return }
        let next = normalize(field.text ?? "")
        if field.text != next { field.text = next }
        guard text != next else { return }; text = next; state.textChanged(to: next); refresh(); changed?(next)
    }
    private func key(_ key: String) {
        guard !choices.isEmpty else { return }
        if key == UIKeyCommand.inputEscape { state.close(for: text); refresh() }
        else { state.move(down: key == UIKeyCommand.inputDownArrow, count: choices.count); highlight() }
    }
    private func pick(_ candidate: MentionCandidate) {
        guard editable else { return }
        text = candidate.username; field.text = text; state.close(for: text); refresh(); changed?(text)
    }
    private func refresh() {
        let next = state.choices(text: text, candidates: candidates, enabled: editable)
        field.hasChoices = !next.isEmpty
        guard choices != next else { highlight(); return }; choices = next
        list.arrangedSubviews.forEach { list.removeArrangedSubview($0); $0.removeFromSuperview() }; buttons = []
        for candidate in choices {
            var config = UIButton.Configuration.plain(); config.title = candidate.displayName
            config.subtitle = "@\(candidate.username)\(candidate.invited ? " · Already invited" : "")"
            config.titleAlignment = .leading; config.baseForegroundColor = UIColor(CaperTheme.text)
            config.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 48, bottom: 7, trailing: 8)
            config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
                var value = value; value.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Satoshi-Medium", size: 12) ?? .systemFont(ofSize: 12)); return value
            }
            config.subtitleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
                var value = value; value.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(for: UIFont(name: "Satoshi-Regular", size: 11) ?? .systemFont(ofSize: 11)); return value
            }
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.pick(candidate) })
            let avatar = UIKitAvatar(side: 30); avatar.show(name: candidate.displayName, avatarID: candidate.avatarId, picture: candidate.picture)
            avatar.isUserInteractionEnabled = false; button.addSubview(avatar)
            NSLayoutConstraint.activate([avatar.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 8), avatar.centerYAnchor.constraint(equalTo: button.centerYAnchor)])
            button.contentHorizontalAlignment = .fill; button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
            button.accessibilityLabel = candidate.accessibilityLabel; button.accessibilityIdentifier = "person-suggestion-\(candidate.username.lowercased())"
            buttons.append(button); list.addArrangedSubview(button)
        }
        listHeight.constant = min(CGFloat(choices.count) * 44, 176); scroll.isHidden = choices.isEmpty
        highlight(); heightChanged?()
    }
    private func highlight() {
        let index = state.highlightedIndex(count: choices.count)
        for (offset, button) in buttons.enumerated() {
            button.configuration?.background.backgroundColor = offset == index ? UIColor(CaperTheme.terracotta).withAlphaComponent(0.32) : .clear
            if offset == index { button.accessibilityTraits.insert(.selected) } else { button.accessibilityTraits.remove(.selected) }
        }
        if buttons.indices.contains(index) { scroll.scrollRectToVisible(buttons[index].convert(buttons[index].bounds, to: scroll), animated: false) }
    }
}

@MainActor final class UIKitPeopleInput: UITextField {
    var hasChoices = false
    var handleKey: ((String) -> Void)?
    override var keyCommands: [UIKeyCommand]? {
        guard hasChoices else { return super.keyCommands }
        return [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, UIKeyCommand.inputEscape].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(moveChoice(_:))); command.wantsPriorityOverSystemBehavior = true; return command
        }
    }
    @objc private func moveChoice(_ command: UIKeyCommand) { if let input = command.input { handleKey?(input) } }
}

@MainActor final class UIKitNewDirectMessage: UIKitMessageDialog {
    private let model: AppModel
    private let people = UIKitPeopleField(identifier: "dm-username", listIdentifier: "dm-username-suggestions", normalize: ProfileValidation.normalizeUsername)
    private let status = UIKitMessageDialog.label("")
    private var button: UIButton!
    private var pending = false
    private var error: String?
    private var resolved: Person?
    override var dismissDisabled: Bool { pending }
    private var candidates: [MentionCandidate] { PeopleSuggestions.directMessage(people: model.people, accountID: model.account?.id) }
    private var recipientName: String? { resolved?.displayName ?? candidates.first { $0.username == people.text }?.displayName }
    private var valid: Bool { people.text.range(of: "^[a-z0-9_]{3,32}$", options: .regularExpression) != nil }
    init(model: AppModel, close: @escaping () -> Void) { self.model = model; super.init(title: "New direct message", close: close) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); body.addArrangedSubview(Self.label("Enter an account’s exact username."))
        body.addArrangedSubview(people); status.heightAnchor.constraint(greaterThanOrEqualToConstant: 48).isActive = true; body.addArrangedSubview(status)
        button = Self.button("Find person", primary: true) { [weak self] in self?.start() }; button.accessibilityIdentifier = "dm-create-submit"; body.addArrangedSubview(button)
        people.changed = { [weak self] _ in self?.error = nil; self?.resolved = nil; self?.render() }
        people.heightChanged = { [weak self] in self?.parent?.view.setNeedsLayout() }
        observe(); Task { [model] in await model.refreshPeople() }
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); people.field.becomeFirstResponder() }
    override func render() {
        people.candidates = candidates; people.editable = !pending
        let found = recipientName != nil
        let label = pending ? (found ? "Opening…" : "Finding…") : (found ? "Open conversation" : "Find person")
        button.configuration?.title = label; button.accessibilityLabel = label; button.isEnabled = !pending && valid
        status.text = error ?? recipientName.map { "\($0) (@\(resolved?.username ?? people.text))" }
        status.textColor = UIColor(error == nil ? CaperTheme.muted : CaperTheme.error)
        status.accessibilityIdentifier = error == nil ? nil : "dm-create-error"
        status.isAccessibilityElement = status.text != nil; updateDismissGuard()
    }
    private func start() {
        guard !pending, valid else { return }
        let submitted = resolved?.username ?? people.text, opening = recipientName != nil
        pending = true; error = nil; render()
        Task { [weak self, model] in
            do {
                if opening {
                    if try await model.createDirectMessage(username: submitted) { self?.pending = false; self?.close(); return }
                    self?.error = "Couldn’t open that conversation."
                } else { self?.resolved = try await model.lookupDirectPerson(username: submitted) }
            } catch { self?.error = FriendlyError.message(for: error) }
            self?.pending = false; self?.render()
        }
    }
}
#endif
