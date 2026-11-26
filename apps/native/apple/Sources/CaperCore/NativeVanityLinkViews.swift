#if os(macOS)
import AppKit

/// Manage space's "Vanity link" section (web's VanityLinkSettings).
@MainActor final class AppKitVanityLink: NSStackView, NSTextFieldDelegate {
    private let state: NativeVanityLink
    private let observation = AppKitObservation()
    private let field = AppKitKitField(), error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var save: AppKitKitButton!, copy: AppKitKitButton!, off: AppKitKitButton!
    private var started = false
    init(state: NativeVanityLink) {
        self.state = state; super.init(frame: .zero); orientation = .vertical; spacing = 10; alignment = .leading
        field.placeholderString = "your-space"; field.delegate = self
        field.setAccessibilityLabel("Vanity link name"); field.setAccessibilityIdentifier("vanity-link-name")
        save = AppKitKitButton("Save") { [weak self] in self?.submit() }; save.setAccessibilityIdentifier("vanity-link-save")
        copy = AppKitKitButton("Copy", icon: "doc.on.doc") { [state] in state.copy() }
        off = AppKitKitButton("Turn off") { [weak self] in self?.confirm(off: true) }
        let actions = NSStackView(views: [save, copy, off]); actions.spacing = 8
        for view in [AppKitKit.label("Vanity link", size: 14, weight: .bold),
                     AppKitKit.label("A short link that never expires. Names you use stay reserved for this space, so no other space can take them.", size: 12, color: AppKitKit.muted),
                     AppKitKit.label(state.prefix, size: 12, color: AppKitKit.muted), field, actions, error] as [NSView] {
            addArrangedSubview(view); AppKitKit.fillWidth(view, of: self)
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
            if !started { started = true; Task { [state] in await state.load() } }
        } else { observation.stop() }
    }
    func controlTextDidChange(_ notification: Notification) { state.setName(field.stringValue); if field.stringValue != state.name { field.stringValue = state.name } }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard selector == #selector(NSResponder.insertNewline(_:)) else { return false }
        submit(); return true
    }
    private func render() {
        if field.stringValue != state.name, field.currentEditor() == nil { field.stringValue = state.name }
        field.isEnabled = state.loaded
        save.title = state.current == nil ? "Save" : "Change"; save.isEnabled = !state.pending && state.changed && !state.name.isEmpty
        let showsLive = state.current != nil && !state.changed
        copy.isHidden = !showsLive; off.isHidden = !showsLive; off.isEnabled = !state.pending
        copy.title = state.copied ? "Copied" : "Copy"
        error.stringValue = state.error ?? ""; error.isHidden = state.error == nil
    }
    private func submit() {
        switch state.check() {
        case nil: state.apply(state.name)
        case "confirm": confirm(off: false)
        default: break
        }
    }
    private func confirm(off: Bool) {
        guard !state.pending, let window else { return }
        let alert = NSAlert(); alert.messageText = off ? "Turn off vanity link?" : "Change vanity link?"
        alert.informativeText = "\(state.currentURL) will stop working. The name stays reserved for this space."
        alert.addButton(withTitle: "Cancel"); alert.addButton(withTitle: off ? "Turn off" : "Change link")
        alert.beginSheetModal(for: window) { [state] response in
            if response == .alertSecondButtonReturn { state.apply(off ? nil : state.name) }
        }
    }
}
#elseif os(iOS)
import UIKit
import Observation

/// Manage space's "Vanity link" section (web's VanityLinkSettings).
@MainActor final class UIKitVanityLink: UIStackView {
    private let state: NativeVanityLink
    private let present: (UIViewController) -> Void
    private let field = UIKitMessageDialog.workspaceField("your-space", label: "Vanity link name")
    private let error = UIKitMessageDialog.label("", error: true)
    private var save: UIButton!, copy: UIButton!, off: UIButton!
    private var generation = 0
    private var started = false
    init(state: NativeVanityLink, present: @escaping (UIViewController) -> Void) {
        self.state = state; self.present = present; super.init(frame: .zero); axis = .vertical; spacing = 10
        field.autocapitalizationType = .none; field.autocorrectionType = .no; field.keyboardType = .URL; field.accessibilityIdentifier = "vanity-link-name"
        let prefix = UILabel(); prefix.text = state.prefix; prefix.font = .systemFont(ofSize: 12); prefix.textColor = UIColor(CaperTheme.muted); prefix.sizeToFit()
        field.leftView = prefix; field.leftViewMode = .always
        field.addAction(UIAction { [weak self] _ in guard let self else { return }; self.state.setName(self.field.text ?? ""); if self.field.text != self.state.name { self.field.text = self.state.name } }, for: .editingChanged)
        field.addAction(UIAction { [weak self] _ in self?.submit() }, for: .editingDidEndOnExit)
        save = UIKitMessageDialog.button("Save", primary: true) { [weak self] in self?.submit() }; save.accessibilityIdentifier = "vanity-link-save"
        copy = UIKitMessageDialog.button("Copy", icon: "doc.on.doc") { [state] in state.copy() }
        off = UIKitMessageDialog.button("Turn off") { [weak self] in self?.confirm(off: true) }
        let actions = UIStackView(arrangedSubviews: [save, copy, off]); actions.spacing = 8
        [UIKitMessageDialog.label("Vanity link", size: 14, bold: true),
         UIKitMessageDialog.label("A short link that never expires. Names you use stay reserved for this space, so no other space can take them."),
         field, actions, error].forEach(addArrangedSubview)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1
        if window != nil { observe(generation); if !started { started = true; Task { [state] in await state.load() } } }
    }
    private func observe(_ token: Int) {
        guard token == generation, window != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func render() {
        if field.text != state.name, !field.isFirstResponder { field.text = state.name }
        field.isEnabled = state.loaded
        save.configuration?.title = state.current == nil ? "Save" : "Change"; save.isEnabled = !state.pending && state.changed && !state.name.isEmpty
        let showsLive = state.current != nil && !state.changed
        copy.isHidden = !showsLive; off.isHidden = !showsLive; off.isEnabled = !state.pending
        copy.configuration?.title = state.copied ? "Copied" : "Copy"
        error.text = state.error; error.isHidden = state.error == nil
    }
    private func submit() {
        switch state.check() {
        case nil: state.apply(state.name)
        case "confirm": confirm(off: false)
        default: break
        }
    }
    private func confirm(off: Bool) {
        guard !state.pending else { return }
        let alert = UIAlertController(title: off ? "Turn off vanity link?" : "Change vanity link?",
                                      message: "\(state.currentURL) will stop working. The name stays reserved for this space.", preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: off ? "Turn off" : "Change link", style: .destructive) { [state] _ in state.apply(off ? nil : state.name) })
        present(alert)
    }
}
#endif
