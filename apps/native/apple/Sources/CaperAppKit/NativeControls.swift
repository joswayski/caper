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
    var hoverChanged: ((Bool) -> Void)?
    var profileFeedback = false {
        didSet {
            guard profileFeedback != oldValue else { return }
            if profileFeedback {
                let wash = CALayer()
                wash.cornerRadius = 8; wash.isHidden = true
                layer?.addSublayer(wash); hoverWash = wash
                focusRingType = .none
            } else {
                hoverWash?.removeFromSuperlayer(); hoverWash = nil
                focusRingType = .default
            }
            needsLayout = true
        }
    }
    private var tracking: NSTrackingArea?
    private var hovered = false
    private var hoverWash: CALayer?
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
    override var isEnabled: Bool { didSet { alphaValue = isEnabled ? 1 : 0.45; needsLayout = true } }

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        if accepted { needsLayout = true; focusChanged?(true) }
        return accepted
    }
    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        if accepted { needsLayout = true; focusChanged?(false) }
        return accepted
    }

    override func layout() {
        super.layout()
        guard let hoverWash else { return }
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let focused = window?.firstResponder === self
        hoverWash.frame = bounds
        hoverWash.backgroundColor = (hovered ? NSColor.white.withAlphaComponent(0.06) : .clear).cgColor
        hoverWash.borderColor = NativeStyle.accent.cgColor
        hoverWash.borderWidth = focused ? 2 : 0
        hoverWash.isHidden = (!hovered && !focused) || !isEnabled
        CATransaction.commit()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        tracking = nil
        guard profileFeedback || hoverChanged != nil else { return }
        let tracking = NSTrackingArea(rect: .zero, options: [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow], owner: self, userInfo: nil)
        self.tracking = tracking; addTrackingArea(tracking)
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; needsLayout = true; hoverChanged?(true) }
    override func mouseExited(with event: NSEvent) { hovered = false; needsLayout = true; hoverChanged?(false) }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { hovered = false; hoverChanged?(false) }
    }
    override func resetCursorRects() {
        super.resetCursorRects()
        if profileFeedback && isEnabled { addCursorRect(bounds, cursor: .pointingHand) }
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

/// The host owns the preferred width. Pointer moves only preview it; releasing
/// the pointer, keyboard adjustments and AX adjustments commit a preference.
@MainActor final class NativePanelResizeHandle: NativeView {
    private let defaultWidth: CGFloat
    private let widensToLeft: Bool
    private let resized: (CGFloat, Bool) -> Void
    private var width: CGFloat
    private var maximum: CGFloat = 440

    init(_ label: String, identifier: String, defaultWidth: CGFloat, widensToLeft: Bool,
         resized: @escaping (CGFloat, Bool) -> Void) {
        self.defaultWidth = defaultWidth; self.width = defaultWidth
        self.widensToLeft = widensToLeft; self.resized = resized
        super.init()
        setAccessibilityElement(true); setAccessibilityRole(.slider)
        setAccessibilityLabel(label); setAccessibilityIdentifier(identifier)
        setAccessibilityOrientation(.horizontal)
        setAccessibilityHelp(widensToLeft
            ? "Drag to resize. Left widens and Right narrows by 10 pixels; Home and End select the bounds. Double-click resets."
            : "Drag to resize. Arrow keys adjust by 10 pixels; Home and End select the bounds. Double-click resets.")
        update(width: defaultWidth, maximum: 440)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var acceptsFirstResponder: Bool { true }
    override func becomeFirstResponder() -> Bool { needsDisplay = true; return true }
    override func resignFirstResponder() -> Bool { needsDisplay = true; return true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
    override func draw(_ dirtyRect: NSRect) {
        guard window?.firstResponder === self else { return }
        NativeStyle.accent.setFill()
        NSRect(x: floor(bounds.midX) - 1, y: 0, width: 2, height: bounds.height).fill()
    }

    func update(width: CGFloat, maximum: CGFloat) {
        self.maximum = max(220, maximum)
        let next = min(self.maximum, max(220, width))
        let changed = self.width != next
        self.width = next
        setAccessibilityMinValue(220); setAccessibilityMaxValue(Double(self.maximum))
        setAccessibilityValue("\(Int(next)) pixels")
        if changed { NSAccessibility.post(element: self, notification: .valueChanged) }
    }

    private func resize(_ value: CGFloat, commit: Bool = true) {
        update(width: commit ? value.rounded() : value, maximum: maximum)
        resized(width, commit)
    }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        window.makeFirstResponder(self)
        if event.clickCount == 2 { resize(defaultWidth); return }
        let start = event.locationInWindow.x, initialWidth = width
        var dragged = false
        while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            if next.type == .leftMouseUp {
                if dragged { resize(width) }
                break
            }
            dragged = true
            let delta = next.locationInWindow.x - start
            resize(initialWidth + (widensToLeft ? -delta : delta), commit: false)
        }
    }
    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123: resize(width + (widensToLeft ? 10 : -10))
        case 124: resize(width + (widensToLeft ? -10 : 10))
        case 115: resize(220)
        case 119: resize(maximum)
        default: super.keyDown(with: event)
        }
    }
    override func accessibilityPerformIncrement() -> Bool { resize(width + 10); return true }
    override func accessibilityPerformDecrement() -> Bool { resize(width - 10); return true }
}
