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
    private var backgroundColorProvider: () -> NSColor

    init(_ color: @autoclosure @escaping () -> NSColor = .clear) {
        self.backgroundColorProvider = color
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = backgroundColorProvider().cgColor
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleThemeDidChange),
            name: CaperThemeStore.didChangeNotification,
            object: nil
        )
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func handleThemeDidChange() {
        applyTheme()
    }

    func applyTheme() {
        layer?.backgroundColor = backgroundColorProvider().cgColor
        needsDisplay = true
    }
}

@MainActor enum NativeStyle {
    static var blackout: NSColor { CaperThemeStore.frameColor }
    static var surface: NSColor { CaperThemeStore.baseColor }
    static var sidebar: NSColor { CaperThemeStore.sidebarColor }
    static var sidebarRaised: NSColor { CaperThemeStore.sidebarRaisedColor }
    static var conversation: NSColor { CaperThemeStore.conversationColor }
    static var raised: NSColor { CaperThemeStore.raisedColor }
    static var composer: NSColor { CaperThemeStore.composerColor }
    static var text: NSColor { CaperThemeStore.textColor }
    static var message: NSColor { CaperThemeStore.messageColor }
    static var muted: NSColor { CaperThemeStore.mutedColor }
    static var link: NSColor { CaperThemeStore.linkColor }
    static var border: NSColor { CaperThemeStore.borderColor }
    static var accent: NSColor { CaperThemeStore.accentColor }
    static var accentBright: NSColor { CaperThemeStore.accentBrightColor }
    static var action: NSColor { CaperThemeStore.actionColor }
    static var actionHover: NSColor { CaperThemeStore.actionHoverColor }
    static var onAction: NSColor { CaperThemeStore.onActionColor }
    static var green: NSColor { CaperThemeStore.caperColor }
    static var voiceSessionGreen: NSColor { CaperThemeStore.voiceSessionGreenColor }
    static var pinGold: NSColor { CaperThemeStore.pinGoldColor }
    static var error: NSColor { CaperThemeStore.errorColor }
    static var voiceOff: NSColor { CaperThemeStore.voiceOffColor }
    static var hoverWash: NSColor { CaperThemeStore.hoverWashColor }
    static var surfaceRim: NSColor { CaperThemeStore.surfaceRimColor }
    static var composerOuterBg: NSColor { CaperThemeStore.composerOuterBgColor }
    static var composerOuterBorder: NSColor { CaperThemeStore.composerOuterBorderColor }
    /// A subtle olive (caper green mixed into the border) that replaces a panel's
    /// divider while its resize handle is dragged or keyboard-focused.
    static var resizeHighlight: NSColor { CaperThemeStore.resizeHighlightColor }
    static func font(_ size: CGFloat, bold: Bool = false) -> NSFont {
        NSFont(name: bold ? "Figtree-Bold" : "Figtree-Regular", size: size) ?? .systemFont(ofSize: size)
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
    private let isPrimary: Bool
    private var themeObserver: NSObjectProtocol?
    var focusChanged: ((Bool) -> Void)?
    var hoverChanged: ((Bool) -> Void)?
    /// Set false only for buttons with a custom subview hover treatment (like space rail tiles).
    var showsDefaultHover = true {
        didSet { needsLayout = true }
    }
    var profileFeedback = false {
        didSet {
            guard profileFeedback != oldValue else { return }
            focusRingType = profileFeedback ? .none : .default
            needsLayout = true
        }
    }
    private var tracking: NSTrackingArea?
    private var hovered = false
    private let hoverWash = CALayer()
    init(_ title: String, primary: Bool = false, action: @escaping () -> Void) {
        actionHandler = action
        isPrimary = primary
        super.init(frame: .zero)
        self.title = title
        target = self; self.action = #selector(pressed)
        bezelStyle = .rounded
        isBordered = false
        font = NativeStyle.font(14, bold: primary)
        wantsLayer = true
        layer?.cornerRadius = primary ? 16 : 8
        hoverWash.cornerRadius = primary ? 16 : 8
        hoverWash.isHidden = true
        layer?.addSublayer(hoverWash)
        applyThemeColors()
        setAccessibilityLabel(title)
        themeObserver = NotificationCenter.default.addObserver(
            forName: CaperThemeStore.didChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyThemeColors(); self?.needsLayout = true }
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit {
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
    }

    private func applyThemeColors() {
        if isPrimary {
            layer?.backgroundColor = (hovered ? NativeStyle.actionHover : NativeStyle.action).cgColor
            contentTintColor = NativeStyle.onAction
            layer?.borderWidth = 1
            layer?.borderColor = (CaperThemeStore.shared.active.isLight
                ? NSColor.black.withAlphaComponent(0.24)
                : NSColor.white.withAlphaComponent(0.55)).cgColor
        } else if imagePosition != .imageOnly && layer?.backgroundColor != NSColor.clear.cgColor {
            layer?.backgroundColor = NativeStyle.composer.cgColor
            contentTintColor = NativeStyle.text
        } else if contentTintColor == nil {
            contentTintColor = NativeStyle.text
        }
    }

    @objc private func pressed() { actionHandler() }
    override var isEnabled: Bool {
        didSet { alphaValue = isEnabled ? 1 : 0.45; needsLayout = true; window?.invalidateCursorRects(for: self) }
    }
    /// The fill is the bounds. NSButton otherwise reports its bezel's alignment
    /// rect, which makes a 24-point icon 31 points tall and over its neighbours.
    override func accessibilityFrame() -> NSRect {
        guard window != nil else { return super.accessibilityFrame() }
        return NSAccessibility.screenRect(fromView: self, rect: bounds)
    }

    /// A Tab stop, as web's buttons, whatever the Full Keyboard Access
    /// setting; a click still leaves focus where it was (the composer).
    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func keyDown(with event: NSEvent) {
        guard event.keyCode == 48 else { super.keyDown(with: event); return }
        KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self)
    }

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
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let focused = window?.firstResponder === self
        let radius = isPrimary ? min(bounds.height / 2, 18) : (layer?.cornerRadius ?? 8)
        if isPrimary { layer?.cornerRadius = radius }
        hoverWash.cornerRadius = radius
        hoverWash.frame = bounds
        if isPrimary {
            layer?.backgroundColor = (hovered && isEnabled ? NativeStyle.actionHover : NativeStyle.action).cgColor
            hoverWash.backgroundColor = NSColor.clear.cgColor
        } else {
            hoverWash.backgroundColor = (hovered && showsDefaultHover ? NativeStyle.hoverWash : .clear).cgColor
        }
        hoverWash.borderColor = (focused ? NativeStyle.accentBright : NativeStyle.border).cgColor
        hoverWash.borderWidth = focused ? 2 : (hovered && showsDefaultHover && !isPrimary && bounds.height >= 24 ? 0.5 : 0)
        hoverWash.isHidden = ((!hovered || !showsDefaultHover) && !focused) || !isEnabled
        CATransaction.commit()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let tracking = NSTrackingArea(rect: .zero, options: [.inVisibleRect, .mouseEnteredAndExited, .activeInKeyWindow], owner: self, userInfo: nil)
        self.tracking = tracking; addTrackingArea(tracking)
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; needsLayout = true; hoverChanged?(true) }
    override func mouseExited(with event: NSEvent) { hovered = false; needsLayout = true; hoverChanged?(false) }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { hovered = false; hoverChanged?(false) }
    }
    /// A right-click shows the button's `menu` as a Caper menu, not NSMenu's,
    /// and stops there rather than reaching the row or message behind it.
    override func rightMouseDown(with event: NSEvent) {
        guard let menu else { super.rightMouseDown(with: event); return }
        CaperMenu.show(menu, at: convert(event.locationInWindow, from: nil), in: self)
    }
    /// Every enabled button shows the hand, as web's buttons do.
    override func resetCursorRects() {
        super.resetCursorRects()
        if isEnabled { addCursorRect(bounds, cursor: .pointingHand) }
    }

    static func icon(_ label: String, symbol: String, action: @escaping () -> Void) -> NativeButton {
        let button = NativeButton(label, action: action)
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        button.imagePosition = .imageOnly
        button.layer?.backgroundColor = NSColor.clear.cgColor
        button.contentTintColor = NativeStyle.muted
        button.toolTip = label
        return button
    }
}

/// The host owns the preferred width. Pointer moves only preview it; releasing
/// the pointer, keyboard adjustments and AX adjustments commit a preference.
@MainActor final class NativePanelResizeHandle: NativeView {
    private let defaultWidth: CGFloat
    private var minimum: CGFloat
    private let widensToLeft: Bool
    private let resized: (CGFloat, Bool) -> Void
    private var width: CGFloat
    private var maximum: CGFloat = 440

    init(_ label: String, identifier: String, defaultWidth: CGFloat, minimum: CGFloat = 220, widensToLeft: Bool,
         resized: @escaping (CGFloat, Bool) -> Void) {
        self.defaultWidth = defaultWidth; self.width = defaultWidth; self.minimum = minimum
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
    /// Lit while dragging or keyboard-focused; a click's focus alone stays unlit, so releasing clears it.
    private var dragging = false { didSet { needsDisplay = true } }
    private var keyboardFocus = false { didSet { needsDisplay = true } }
    override var acceptsFirstResponder: Bool { true }
    override func becomeFirstResponder() -> Bool { keyboardFocus = NSApp.currentEvent?.type != .leftMouseDown; return true }
    override func resignFirstResponder() -> Bool { keyboardFocus = false; return true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
    override func draw(_ dirtyRect: NSRect) {
        guard dragging || keyboardFocus else { return }
        // Over the panel's 1-point divider: the left edge of a panel that widens
        // leftward (thread, members), else the sidebar's right edge.
        NativeStyle.resizeHighlight.setFill()
        NSRect(x: widensToLeft ? 0 : bounds.width - 1, y: 0, width: 1, height: bounds.height).fill()
    }

    func update(width: CGFloat, maximum: CGFloat, minimum: CGFloat? = nil) {
        self.minimum = max(0, minimum ?? self.minimum)
        self.maximum = max(self.minimum, maximum)
        let next = min(self.maximum, max(self.minimum, width))
        let changed = self.width != next
        self.width = next
        setAccessibilityMinValue(Double(self.minimum)); setAccessibilityMaxValue(Double(self.maximum))
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
        keyboardFocus = false; dragging = true
        defer { dragging = false }
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
        case 115: resize(minimum)
        case 119: resize(maximum)
        default: super.keyDown(with: event)
        }
    }
    override func accessibilityPerformIncrement() -> Bool { resize(width + 10); return true }
    override func accessibilityPerformDecrement() -> Bool { resize(width - 10); return true }
}

/// The shipping sidebar stores a fraction; keyboard steps are physical points,
/// while VoiceOver steps are five percentage points. Resizing never saves a clamp.
@MainActor final class NativeSidebarDivider: NativeView {
    private let resized: (Double, Bool) -> Void
    private var available: CGFloat = 0
    private var lower = 0.0
    private(set) var share = 0.5

    init(resized: @escaping (Double, Bool) -> Void) {
        self.resized = resized
        super.init()
        setAccessibilityElement(true); setAccessibilityRole(.slider)
        setAccessibilityOrientation(.vertical)
        setAccessibilityIdentifier("sidebar-sections-resize")
        setAccessibilityLabel("Channels and direct messages divider")
        setAccessibilityHelp("Drag to resize the lists. Up and Down adjust by 10 pixels; Home and End select the bounds. Double-click resets.")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    /// Lit while dragging or keyboard-focused, like NativePanelResizeHandle.
    private var dragging = false { didSet { needsDisplay = true } }
    private var keyboardFocus = false { didSet { needsDisplay = true } }
    override var acceptsFirstResponder: Bool { true }
    override func becomeFirstResponder() -> Bool { keyboardFocus = NSApp.currentEvent?.type != .leftMouseDown; return true }
    override func resignFirstResponder() -> Bool { keyboardFocus = false; return true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeUpDown) }
    override func draw(_ dirtyRect: NSRect) {
        (dragging || keyboardFocus ? NativeStyle.resizeHighlight : NativeStyle.border).setFill()
        NSRect(x: 0, y: floor(bounds.midY), width: bounds.width, height: 1).fill()
    }
    func update(available: CGFloat, share: Double) {
        self.available = max(0, available)
        lower = Double(min(96, self.available / 3) / max(1, self.available))
        let next = min(1 - lower, max(lower, share.isFinite ? share : 0.5))
        let changed = next != self.share
        self.share = next
        setAccessibilityMinValue(lower); setAccessibilityMaxValue(1 - lower)
        setAccessibilityValue("\(Int(next * 100)) percent channels")
        if changed { NSAccessibility.post(element: self, notification: .valueChanged) }
    }
    private func resize(_ share: Double, commit: Bool = true) {
        guard available > 0 else { return }
        update(available: available, share: share)
        resized(self.share, commit)
    }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        window.makeFirstResponder(self)
        if event.clickCount == 2 { resize(0.5); return }
        let start = event.locationInWindow.y, initial = share
        var dragged = false
        keyboardFocus = false; dragging = true
        defer { dragging = false }
        while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            if next.type == .leftMouseUp {
                if dragged { resize(share) }
                break
            }
            dragged = true
            resize(initial + Double((start - next.locationInWindow.y) / max(available, 1)), commit: false)
        }
    }
    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 126: resize(share - 10 / Double(max(available, 1)))
        case 125: resize(share + 10 / Double(max(available, 1)))
        case 115: resize(lower)
        case 119: resize(1 - lower)
        default: super.keyDown(with: event)
        }
    }
    override func accessibilityPerformIncrement() -> Bool { resize(share + 0.05); return available > 0 }
    override func accessibilityPerformDecrement() -> Bool { resize(share - 0.05); return available > 0 }
}
