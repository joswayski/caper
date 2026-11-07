import AppKit
import CaperCore
import Observation

/// Each region observes only the model properties read by its update closure.
/// Composer edits must not invalidate the timeline or navigation views.
@MainActor final class NativeObservation {
    private var track: (() -> Void)?

    func start(_ update: @escaping () -> Void) {
        start(reading: update, update: { _ in })
    }

    /// Read before rendering child controllers so their model reads do not
    /// accidentally subscribe the root controller to every child region.
    func start<Value>(reading: @escaping () -> Value, update: @escaping (Value) -> Void) {
        track = { [weak self] in
            let value = withObservationTracking(reading) { [weak self] in
                Task { @MainActor [weak self] in self?.track?() }
            }
            update(value)
        }
        track?()
    }
}

class NativeView: NSView {
    override var isFlipped: Bool { true }
    init(_ color: NSColor = .clear) {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = color.cgColor
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

enum NativeStyle {
    // Derive from the shipping tokens until the shared token generator lands.
    static var blackout: NSColor { NSColor(CaperTheme.blackout) }
    static var sidebar: NSColor { NSColor(CaperTheme.sidebar) }
    static var conversation: NSColor { NSColor(CaperTheme.conversation) }
    static var composer: NSColor { NSColor(CaperTheme.composer) }
    static var text: NSColor { NSColor(CaperTheme.text) }
    static var muted: NSColor { NSColor(CaperTheme.muted) }
    static var border: NSColor { NSColor(CaperTheme.border) }
    static var accent: NSColor { NSColor(CaperTheme.terracotta) }
    static func font(_ size: CGFloat, bold: Bool = false) -> NSFont {
        NSFont(name: bold ? "Satoshi-Bold" : "Satoshi-Regular", size: size) ?? .systemFont(ofSize: size)
    }
    static func label(_ text: String, size: CGFloat = 14, bold: Bool = false) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = font(size, bold: bold)
        label.textColor = self.text
        label.lineBreakMode = .byTruncatingTail
        return label
    }
}

final class NativeButton: NSButton {
    private let actionHandler: () -> Void
    var focusChanged: ((Bool) -> Void)?
    init(_ title: String, primary: Bool = false, action: @escaping () -> Void) {
        actionHandler = action
        super.init(frame: .zero)
        self.title = title
        target = self; self.action = #selector(pressed)
        bezelStyle = .rounded
        isBordered = false
        font = NativeStyle.font(14)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.backgroundColor = (primary ? NSColor(CaperTheme.action) : NativeStyle.composer).cgColor
        contentTintColor = primary ? NativeStyle.blackout : NativeStyle.text
        setAccessibilityLabel(title)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func pressed() { actionHandler() }
    override var isEnabled: Bool { didSet { alphaValue = isEnabled ? 1 : 0.45 } }

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        if accepted { focusChanged?(true) }
        return accepted
    }
    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        if accepted { focusChanged?(false) }
        return accepted
    }

    static func icon(_ label: String, symbol: String, action: @escaping () -> Void) -> NativeButton {
        let button = NativeButton(label, action: action)
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        button.imagePosition = .imageOnly
        button.layer?.backgroundColor = NSColor.clear.cgColor
        button.toolTip = label
        return button
    }
}
