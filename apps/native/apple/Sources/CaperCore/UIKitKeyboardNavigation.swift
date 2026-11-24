#if os(iOS)
import UIKit

/// The quick switcher's field: Up and Down move the highlighted result and
/// Escape closes, ahead of the text field's own handling of those keys.
@MainActor private final class UIKitSwitcherInput: UITextField {
    var handleKey: ((String) -> Void)?
    override var keyCommands: [UIKeyCommand]? {
        guard markedTextRange == nil else { return super.keyCommands }
        return [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, UIKeyCommand.inputEscape].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(key(_:)))
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
    }
    @objc private func key(_ command: UIKeyCommand) { if let input = command.input { handleKey?(input) } }
}

/// ⌘K: channels, DMs, spaces and people, unread first, best match first;
/// Return opens the highlighted one.
@MainActor final class UIKitQuickSwitcher: UIKitMessageDialog, UITextFieldDelegate {
    private let model: AppModel
    private let field = UIKitSwitcherInput()
    private let list = UIStackView()
    private let empty = UIKitMessageDialog.label("No matches")
    private var results: [SwitcherItem] = []
    private var buttons: [UIButton] = []
    private var selected = 0
    private var query = ""

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model
        super.init(title: "Jump to", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        field.placeholder = "Channels, DMs, spaces and people"
        field.accessibilityLabel = "Jump to"
        field.accessibilityIdentifier = "quick-switcher-field"
        field.font = UIFont(name: "Figtree-Regular", size: 14)
        field.borderStyle = .roundedRect
        field.autocapitalizationType = .none
        field.autocorrectionType = .no
        field.returnKeyType = .go
        field.delegate = self
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        field.addAction(UIAction { [weak self] _ in
            // Let marked text finish before the results follow it.
            guard let self, self.field.markedTextRange == nil else { return }
            self.query = self.field.text ?? ""
            self.selected = 0
            self.render()
        }, for: .editingChanged)
        field.handleKey = { [weak self] input in
            guard let self else { return }
            if input == UIKeyCommand.inputEscape { self.close(); return }
            self.move(down: input == UIKeyCommand.inputDownArrow)
        }
        list.axis = .vertical
        list.accessibilityIdentifier = "quick-switcher-results"
        empty.textColor = UIColor(CaperTheme.muted)
        body.addArrangedSubview(field)
        body.addArrangedSubview(list)
        body.addArrangedSubview(empty)
        observe()
        Task { [model] in await model.refreshPeople() }
        // Other spaces' channels join the list as they load (as on web).
        Task { [model] in await model.loadSwitcherSpaces() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        field.becomeFirstResponder()
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if results.indices.contains(selected) { open(results[selected]) }
        return false
    }

    override func render() {
        guard isViewLoaded else { return }
        let next = model.rankedSwitcherItems(model.switcherItems(), query: query)
        empty.isHidden = !next.isEmpty
        if next.map(\.id) != results.map(\.id) || next.map(\.unread) != results.map(\.unread) {
            let previous = results.indices.contains(selected) ? results[selected].id : nil
            results = next
            selected = previous.flatMap { id in next.firstIndex { $0.id == id } } ?? 0
            rebuild()
        }
        highlight()
    }

    private func rebuild() {
        list.arrangedSubviews.forEach { list.removeArrangedSubview($0); $0.removeFromSuperview() }
        buttons = results.map { item in
            var config = UIButton.Configuration.plain()
            config.title = item.name
            config.subtitle = item.unread ? "\(item.detail) · Unread" : item.detail
            config.titleAlignment = .leading
            config.baseForegroundColor = UIColor(CaperTheme.text)
            config.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 10, bottom: 7, trailing: 10)
            config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
                var value = value
                value.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: item.unread ? "Figtree-Bold" : "Figtree-Medium", size: 14) ?? .systemFont(ofSize: 14))
                return value
            }
            config.subtitleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
                var value = value
                value.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(for: UIFont(name: "Figtree-Regular", size: 11) ?? .systemFont(ofSize: 11))
                value.foregroundColor = UIColor(CaperTheme.muted)
                return value
            }
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.open(item) })
            button.contentHorizontalAlignment = .fill
            button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
            button.accessibilityLabel = item.unread ? "\(item.name), \(item.detail), unread" : "\(item.name), \(item.detail)"
            button.accessibilityIdentifier = "quick-switcher-\(item.id)"
            list.addArrangedSubview(button)
            return button
        }
    }

    private func move(down: Bool) {
        guard !results.isEmpty else { return }
        selected = (selected + (down ? 1 : results.count - 1)) % results.count
        highlight()
    }

    private func highlight() {
        for (offset, button) in buttons.enumerated() {
            let on = offset == selected
            button.configuration?.background.backgroundColor = on ? UIColor(CaperTheme.terracotta).withAlphaComponent(0.32) : .clear
            if on { button.accessibilityTraits.insert(.selected) } else { button.accessibilityTraits.remove(.selected) }
        }
        if buttons.indices.contains(selected) { buttons[selected].scrollIntoViewIfNested() }
    }

    private func open(_ item: SwitcherItem) {
        close()
        Task { [model] in await model.open(item) }
    }
}

private extension UIView {
    func scrollIntoViewIfNested() {
        var ancestor = superview
        while let view = ancestor, !(view is UIScrollView) { ancestor = view.superview }
        guard let scroll = ancestor as? UIScrollView else { return }
        scroll.scrollRectToVisible(convert(bounds, to: scroll), animated: false)
    }
}

/// ⌘/: every keyboard shortcut, as `KeyboardNavigation.shortcuts` lists them.
@MainActor final class UIKitShortcutHelp: UIKitMessageDialog {
    init(close: @escaping () -> Void) { super.init(title: "Keyboard shortcuts", close: close) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var canBecomeFirstResponder: Bool { true }
    override var keyCommands: [UIKeyCommand]? {
        let escape = UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(escape))
        escape.wantsPriorityOverSystemBehavior = true
        return [escape]
    }
    @objc private func escape() { close() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.accessibilityIdentifier = "shortcut-help"
        for section in KeyboardNavigation.shortcuts {
            let title = Self.label(section.title, size: 13, bold: true)
            title.textColor = UIColor(CaperTheme.muted)
            title.accessibilityTraits.insert(.header)
            body.addArrangedSubview(title)
            for row in section.rows {
                let action = Self.label(row.action, size: 14)
                let keys = Self.label(row.keys, size: 14, bold: true)
                keys.textAlignment = .right
                let line = Self.header(action, keys)
                line.isAccessibilityElement = true
                line.accessibilityLabel = "\(row.action): \(row.keys)"
                body.addArrangedSubview(line)
            }
        }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        becomeFirstResponder()
    }
}
#endif
