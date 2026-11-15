import SwiftUI
import Combine

// These dialogs have no mention source. Emoji completion and formatting use
// the existing TextKit coordinator, including its IME and undo handling.
#if os(macOS)
import AppKit

@MainActor final class NativeDialogComposer: NSView {
    let controller: ComposerAutocompleteController
    private var configuration: NativeMessageTextEditor!
    private var coordinator: NativeMessageTextEditor.Coordinator!
    private var editor: NSScrollView!
    private let toolbar = NSStackView()
    private let suggestions = AppKitKit.stack([], spacing: 0)
    private var suggestionScroll: NSScrollView!
    private var suggestionHeight: NSLayoutConstraint!
    private var changes: AnyCancellable?
    private var buttons: [AppKitKitButton] = []
    private var editorHeight: NSLayoutConstraint!
    private let grows: Bool

    init(text: Binding<String>, placeholder: String, identifier: String, height: CGFloat,
         controller: ComposerAutocompleteController? = nil, submit: (() -> Void)? = nil,
         pasteAttachments: (() -> Bool)? = nil, highlightSource: MentionSource? = nil, grows: Bool = false) {
        let controller = controller ?? ComposerAutocompleteController()
        self.controller = controller; self.grows = grows
        super.init(frame: .zero)
        configuration = NativeMessageTextEditor(text: text, placeholder: placeholder, controller: controller,
            mentions: .empty, submit: submit ?? {}, submitOnReturn: submit != nil, accessibilityID: identifier,
            pasteAttachments: pasteAttachments, highlightSource: highlightSource)
        coordinator = configuration.makeCoordinator()
        editor = configuration.makeNativeView(coordinator: coordinator)
        (editor.documentView as? ComposerTextView)?.placeholder = placeholder
        toolbar.orientation = .horizontal; toolbar.spacing = 2
        for item in MessageFormattingAction.all {
            let button = AppKitKitButton("", kind: .plain) { [weak self] in self?.controller.formatAction?(item.style) }
            button.image = NSImage(systemSymbolName: item.symbol, accessibilityDescription: item.title)
            button.setAccessibilityLabel(item.title); button.toolTip = item.title
            button.widthAnchor.constraint(equalToConstant: 36).isActive = true
            button.heightAnchor.constraint(equalToConstant: 36).isActive = true
            toolbar.addArrangedSubview(button); buttons.append(button)
        }
        suggestionScroll = appKitKitScroll(suggestions, insets: NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0))
        suggestionScroll.setAccessibilityIdentifier("emoji-suggestions")
        suggestionHeight = suggestionScroll.heightAnchor.constraint(equalToConstant: 0)
        suggestionHeight.isActive = true
        editor.translatesAutoresizingMaskIntoConstraints = false
        editorHeight = editor.heightAnchor.constraint(equalToConstant: height); editorHeight.isActive = true
        let stack = AppKitKit.stack([toolbar, editor, suggestionScroll], spacing: 4)
        AppKitKit.fillWidth(editor, of: stack); AppKitKit.fillWidth(suggestionScroll, of: stack)
        wantsLayer = true; layer?.backgroundColor = AppKitKit.composer.cgColor
        layer?.cornerRadius = 8; layer?.borderWidth = 1; layer?.borderColor = AppKitKit.border.cgColor
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8), stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 6), stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
        changes = controller.objectWillChange.sink { [weak self] in
            DispatchQueue.main.async { [weak self] in self?.renderControls() }
        }
        update(editable: true); renderControls()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(editable: Bool) {
        configuration.editable = editable
        configuration.updateNativeView(editor, coordinator: coordinator)
        renderControls(); needsLayout = true
    }
    func focus() { controller.restoreFocus() }

    override func layout() {
        super.layout()
        guard grows, let text = editor.documentView as? NSTextView, let container = text.textContainer, let layout = text.layoutManager else { return }
        layout.ensureLayout(for: container)
        let height = min(140, max(42, ceil(layout.usedRect(for: container).height + text.textContainerInset.height * 2)))
        if editorHeight.constant != height { editorHeight.constant = height }
    }

    private func renderControls() {
        toolbar.isHidden = !controller.formattingSelection
        for (button, item) in zip(buttons, MessageFormattingAction.all) {
            button.isEnabled = configuration.editable
            button.state = controller.formattingStyles.contains(item.style) ? .on : .off
            button.setAccessibilityValue(button.state == .on ? 1 : 0)
            button.layer?.backgroundColor = (button.state == .on ? AppKitKit.border : .clear).cgColor
        }
        suggestions.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard configuration.editable, case .emoji(let match) = controller.match else {
            suggestionScroll.isHidden = true; suggestionHeight.constant = 0; return
        }
        suggestionScroll.isHidden = false; suggestionHeight.constant = min(176, CGFloat(match.choices.count) * 44)
        for (index, entry) in match.choices.enumerated() {
            let button = AppKitKitButton(":\(entry.name.replacingOccurrences(of: " ", with: "_")):", kind: .plain) { [weak self] in
                self?.controller.accept(index); self?.focus()
            }
            if let image = NativePresentation.emojiImage(for: entry) { button.image = NSImage(cgImage: image, size: NSSize(width: 24, height: 24)) }
            button.imagePosition = .imageLeft; button.alignment = .left; button.minimumHeight = 44
            button.setAccessibilityLabel("\(entry.name), emoji"); button.setAccessibilityIdentifier("emoji-suggestion-\(entry.id)")
            button.setAccessibilityValue(index == controller.selected ? "Selected" : "Not selected")
            button.layer?.backgroundColor = (index == controller.selected ? AppKitKit.border : .clear).cgColor
            suggestions.addArrangedSubview(button); AppKitKit.fillWidth(button, of: suggestions)
        }
        suggestions.layoutSubtreeIfNeeded()
        if suggestions.arrangedSubviews.indices.contains(controller.selected) {
            suggestions.arrangedSubviews[controller.selected].scrollToVisible(suggestions.arrangedSubviews[controller.selected].bounds)
        }
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class NativeDialogComposer: UIView {
    let controller: ComposerAutocompleteController
    private var configuration: NativeMessageTextEditor!
    private var coordinator: NativeMessageTextEditor.Coordinator!
    private var editor: UITextView!
    private let placeholderLabel = UILabel()
    private let toolbar = UIStackView()
    private let suggestions = UIStackView()
    private let suggestionScroll = UIScrollView()
    private let toolbarScroll = UIScrollView()
    private var suggestionHeight: NSLayoutConstraint!
    private var changes: AnyCancellable?
    private var buttons: [UIButton] = []
    private var editorHeight: NSLayoutConstraint!
    private let grows: Bool
    private var textChanges: NSObjectProtocol?
    /// Called after a growing editor changes height, so its host re-measures.
    var heightChanged: (() -> Void)?

    init(text: Binding<String>, placeholder: String, identifier: String, height: CGFloat,
         controller: ComposerAutocompleteController? = nil, submit: (() -> Void)? = nil,
         pasteAttachments: (() -> Bool)? = nil, highlightSource: MentionSource? = nil, grows: Bool = false) {
        let controller = controller ?? ComposerAutocompleteController()
        self.controller = controller; self.grows = grows
        super.init(frame: .zero)
        configuration = NativeMessageTextEditor(text: text, placeholder: placeholder, controller: controller,
            mentions: .empty, submit: submit ?? {}, submitOnReturn: submit != nil, accessibilityID: identifier,
            pasteAttachments: pasteAttachments, highlightSource: highlightSource)
        coordinator = configuration.makeCoordinator()
        editor = configuration.makeNativeView(coordinator: coordinator)
        placeholderLabel.text = placeholder; placeholderLabel.font = editor.font
        placeholderLabel.textColor = UIColor(CaperTheme.muted)
        placeholderLabel.isUserInteractionEnabled = false; placeholderLabel.accessibilityElementsHidden = true
        placeholderLabel.translatesAutoresizingMaskIntoConstraints = false
        editor.addSubview(placeholderLabel)
        NSLayoutConstraint.activate([
            placeholderLabel.leadingAnchor.constraint(equalTo: editor.leadingAnchor, constant: 5),
            placeholderLabel.topAnchor.constraint(equalTo: editor.topAnchor, constant: 11),
            placeholderLabel.trailingAnchor.constraint(lessThanOrEqualTo: editor.trailingAnchor, constant: -7),
        ])
        for item in MessageFormattingAction.all {
            let button = UIButton(type: .system)
            button.setImage(UIImage(systemName: item.symbol), for: .normal)
            button.accessibilityLabel = item.title; button.tintColor = UIColor(CaperTheme.text)
            button.addAction(UIAction { [weak self] _ in self?.controller.formatAction?(item.style) }, for: .touchUpInside)
            button.widthAnchor.constraint(equalToConstant: 44).isActive = true
            toolbar.addArrangedSubview(button); buttons.append(button)
        }
        toolbarScroll.addSubview(toolbar); toolbar.translatesAutoresizingMaskIntoConstraints = false
        toolbarScroll.heightAnchor.constraint(equalToConstant: 44).isActive = true
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: toolbarScroll.contentLayoutGuide.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: toolbarScroll.contentLayoutGuide.trailingAnchor),
            toolbar.topAnchor.constraint(equalTo: toolbarScroll.contentLayoutGuide.topAnchor),
            toolbar.bottomAnchor.constraint(equalTo: toolbarScroll.contentLayoutGuide.bottomAnchor),
            toolbar.heightAnchor.constraint(equalTo: toolbarScroll.frameLayoutGuide.heightAnchor),
        ])
        suggestions.axis = .vertical; suggestions.translatesAutoresizingMaskIntoConstraints = false
        suggestionScroll.accessibilityIdentifier = "emoji-suggestions"
        suggestionScroll.addSubview(suggestions)
        NSLayoutConstraint.activate([
            suggestions.leadingAnchor.constraint(equalTo: suggestionScroll.contentLayoutGuide.leadingAnchor),
            suggestions.trailingAnchor.constraint(equalTo: suggestionScroll.contentLayoutGuide.trailingAnchor),
            suggestions.topAnchor.constraint(equalTo: suggestionScroll.contentLayoutGuide.topAnchor),
            suggestions.bottomAnchor.constraint(equalTo: suggestionScroll.contentLayoutGuide.bottomAnchor),
            suggestions.widthAnchor.constraint(equalTo: suggestionScroll.frameLayoutGuide.widthAnchor),
        ])
        suggestionHeight = suggestionScroll.heightAnchor.constraint(equalToConstant: 0); suggestionHeight.isActive = true
        editorHeight = editor.heightAnchor.constraint(equalToConstant: height); editorHeight.isActive = true
        let stack = UIStackView(arrangedSubviews: [toolbarScroll, editor, suggestionScroll])
        stack.axis = .vertical; stack.spacing = 4; stack.translatesAutoresizingMaskIntoConstraints = false
        backgroundColor = UIColor(CaperTheme.composer); layer.cornerRadius = 8
        layer.borderWidth = 1; layer.borderColor = UIColor(CaperTheme.border).cgColor
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8), stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 6), stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
        // Typing does not lay this view out, so a growing editor re-measures here.
        if grows {
            textChanges = NotificationCenter.default.addObserver(forName: UITextView.textDidChangeNotification, object: editor, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.setNeedsLayout() }
            }
        }
        changes = controller.objectWillChange.sink { [weak self] in
            DispatchQueue.main.async { [weak self] in self?.renderControls() }
        }
        update(editable: true); renderControls()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(editable: Bool) {
        configuration.editable = editable
        configuration.updateNativeView(editor, coordinator: coordinator)
        editor.accessibilityHint = configuration.submitOnReturn ? "Return sends. Shift-Return adds a new line."
            : "Return adds a new line. Use the dialog's button to save or forward."
        renderControls(); setNeedsLayout()
    }
    func focus() { controller.restoreFocus() }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard grows, editor.bounds.width > 0 else { return }
        let height = min(140, max(42, ceil(editor.sizeThatFits(CGSize(width: editor.bounds.width, height: .greatestFiniteMagnitude)).height)))
        if editorHeight.constant != height {
            editorHeight.constant = height
            DispatchQueue.main.async { [weak self] in self?.heightChanged?() }
        }
    }

    private func renderControls() {
        placeholderLabel.isHidden = !configuration.text.isEmpty
        toolbarScroll.isHidden = !controller.formattingSelection
        for (button, item) in zip(buttons, MessageFormattingAction.all) {
            button.isEnabled = configuration.editable
            button.isSelected = controller.formattingStyles.contains(item.style)
            button.backgroundColor = button.isSelected ? UIColor(CaperTheme.border) : .clear
            button.accessibilityTraits = button.isSelected ? [.button, .selected] : .button
        }
        suggestions.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard configuration.editable, case .emoji(let match) = controller.match else {
            suggestionScroll.isHidden = true; suggestionHeight.constant = 0; return
        }
        suggestionScroll.isHidden = false; suggestionHeight.constant = min(176, CGFloat(match.choices.count) * 44)
        for (index, entry) in match.choices.enumerated() {
            var config = UIButton.Configuration.plain()
            config.title = ":\(entry.name.replacingOccurrences(of: " ", with: "_")):"
            if let image = NativePresentation.emojiImage(for: entry) {
                config.image = UIImage(cgImage: image, scale: CGFloat(image.width) / 24, orientation: .up).withRenderingMode(.alwaysOriginal)
            }
            config.imagePadding = 12; config.baseForegroundColor = UIColor(CaperTheme.text)
            config.background.backgroundColor = index == controller.selected ? UIColor(CaperTheme.border) : .clear
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
                self?.controller.accept(index); self?.focus()
            })
            button.contentHorizontalAlignment = .leading
            button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
            button.accessibilityLabel = "\(entry.name), emoji"; button.accessibilityIdentifier = "emoji-suggestion-\(entry.id)"
            button.accessibilityTraits = index == controller.selected ? [.button, .selected] : .button
            suggestions.addArrangedSubview(button)
        }
        suggestions.layoutIfNeeded()
        if suggestions.arrangedSubviews.indices.contains(controller.selected) {
            suggestionScroll.scrollRectToVisible(suggestions.arrangedSubviews[controller.selected].frame, animated: false)
        }
    }
}
#endif
