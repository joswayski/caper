#if os(macOS)
import AppKit
import Observation
import SwiftUI

/// Building blocks for the AppKit app's native dialogs, which replace the
/// SwiftUI views it used to host. Colors come from CaperTheme's tokens.
@MainActor enum AppKitKit {
    static var surface: NSColor { CaperThemeStore.baseColor }
    static var raised: NSColor { CaperThemeStore.raisedColor }
    static var composer: NSColor { CaperThemeStore.composerColor }
    static var border: NSColor { CaperThemeStore.borderColor }
    static var text: NSColor { CaperThemeStore.textColor }
    static var muted: NSColor { CaperThemeStore.mutedColor }
    static var error: NSColor { CaperThemeStore.errorColor }
    static var terracotta: NSColor { CaperThemeStore.accentColor }
    static var terracottaBright: NSColor { CaperThemeStore.accentBrightColor }
    static var action: NSColor { CaperThemeStore.actionColor }
    static var actionHover: NSColor { CaperThemeStore.actionHoverColor }
    static var actionPress: NSColor { CaperThemeStore.actionPressColor }
    static var onAction: NSColor { CaperThemeStore.onActionColor }
    static var hoverWash: NSColor { CaperThemeStore.hoverWashColor }

    static func font(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        CaperFontLoader.register()
        let name = switch weight {
        case .bold, .heavy, .semibold: "Figtree-Bold"
        case .medium: "Figtree-Medium"
        case .black: "Figtree-Black"
        default: "Figtree-Regular"
        }
        return NSFont(name: name, size: size) ?? .systemFont(ofSize: size, weight: weight)
    }

    static func displayFont(_ size: CGFloat, weight: NSFont.Weight = .bold) -> NSFont {
        CaperFontLoader.register()
        let name = switch weight {
        case .black, .heavy: "Nunito-ExtraBold"
        default: "Nunito-Bold"
        }
        return NSFont(name: name, size: size) ?? font(size, weight: weight)
    }

    /// A wrapping label; `width` caps its preferred layout width.
    static func label(_ text: String, size: CGFloat = 14, weight: NSFont.Weight = .regular,
                      color: NSColor? = nil, wraps: Bool = true) -> NSTextField {
        let label = wraps ? NSTextField(wrappingLabelWithString: text) : NSTextField(labelWithString: text)
        label.font = font(size, weight: weight)
        label.textColor = color ?? AppKitKit.text
        label.isSelectable = false
        if !wraps { label.lineBreakMode = .byTruncatingTail }
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    static func divider() -> NSView {
        let line = NSBox()
        line.boxType = .custom; line.borderWidth = 0; line.fillColor = border
        line.translatesAutoresizingMaskIntoConstraints = false
        line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        return line
    }

    static func stack(_ views: [NSView], spacing: CGFloat, alignment: NSLayoutConstraint.Attribute = .leading) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .vertical; stack.alignment = alignment; stack.spacing = spacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        return stack
    }

    static func image(_ name: String, size: CGFloat) -> NSImageView {
        let view = NSImageView(image: NSImage(named: name) ?? caperResourceBundle.image(forResource: name) ?? NSImage())
        view.imageScaling = .scaleProportionallyUpOrDown
        view.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([view.widthAnchor.constraint(equalToConstant: size), view.heightAnchor.constraint(equalToConstant: size)])
        view.setAccessibilityElement(false)
        return view
    }

    /// Lets a stacked subview fill the stack's width.
    static func fillWidth(_ view: NSView, of stack: NSStackView) {
        view.translatesAutoresizingMaskIntoConstraints = false
        view.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -(stack.edgeInsets.left + stack.edgeInsets.right)).isActive = true
    }

    /// True when the view's window, or the parent window hosting an NSPopover, is key.
    static func windowActive(_ window: NSWindow?) -> Bool {
        guard let window else { return false }
        return window.isKeyWindow || window.parent?.isKeyWindow == true
    }
}

/// Re-runs `apply` with fresh values whenever observed model state it read
/// changes; several changes in one turn coalesce into one update.
@MainActor final class AppKitObservation {
    private var active = false
    private var scheduled = false
    private var run: (() -> Void)?

    func start<Value>(reading: @escaping () -> Value, apply: @escaping (Value) -> Void) {
        active = true
        run = { [weak self] in
            guard let self, self.active else { return }
            let value = withObservationTracking(reading) { [weak self] in
                Task { @MainActor in self?.schedule() }
            }
            apply(value)
        }
        run?()
    }

    private func schedule() {
        guard active, !scheduled else { return }
        scheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.scheduled = false
            self?.run?()
        }
    }

    func stop() { active = false; run = nil }
}

/// Whether a view's window shows anything: false while it is minimized, fully
/// covered, on another Space or not yet ordered in. Looping clips, frame
/// animations and clocks pause until `changed` reports it visible again.
@MainActor public final class AppKitWindowVisibility {
    private weak var window: NSWindow?
    private var token: NSObjectProtocol?
    private let changed: () -> Void
    public init(changed: @escaping () -> Void) { self.changed = changed }
    isolated deinit { token.map(NotificationCenter.default.removeObserver) }

    public var visible: Bool { window?.occlusionState.contains(.visible) == true }

    /// Call from `viewDidMoveToWindow`.
    public func track(_ window: NSWindow?) {
        guard window !== self.window else { return }
        token.map(NotificationCenter.default.removeObserver); token = nil
        self.window = window
        guard let window else { return }
        token = NotificationCenter.default.addObserver(forName: NSWindow.didChangeOcclusionStateNotification,
                                                       object: window, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.changed() }
        }
    }
}

/// Tab and Shift-Tab as web's document order: rows top to bottom and
/// side-by-side panes left to right (rail, sidebar, conversation, members).
/// An open in-window dialog, which is accessibility-modal, keeps Tab inside.
@MainActor public enum KeyboardTabOrder {
    public static func step(in window: NSWindow?, forward: Bool, from origin: NSView? = nil) {
        guard let window else { return }
        let current = origin ?? window.firstResponder as? NSView
        var scope = window.contentView
        var ancestor = current
        while let view = ancestor { if view.isAccessibilityModal() { scope = view; break }; ancestor = view.superview }
        guard let scope else { return }
        let stops = stops(in: scope)
        guard !stops.isEmpty else { return }
        // Stops list containers before their contents, so the last match is the
        // innermost: with Keyboard navigation on, the composer's scroll view is
        // a stop too, and matching it would "move" Tab back into the composer.
        let index = current.flatMap { current in stops.lastIndex { current === $0 || current.isDescendant(of: $0) } }
        let next = index.map { (forward ? $0 + 1 : $0 - 1 + stops.count) % stops.count } ?? (forward ? 0 : stops.count - 1)
        if window.makeFirstResponder(stops[next]) { stops[next].scrollToVisible(stops[next].bounds) }
    }

    private static func stops(in view: NSView) -> [NSView] {
        let frames = Dictionary(uniqueKeysWithValues: view.subviews.map { (ObjectIdentifier($0), $0.convert($0.bounds, to: nil)) })
        let children = view.subviews.filter { !$0.isHidden && $0.alphaValue > 0 && !$0.frame.isEmpty }.sorted { a, b in
            let a = frames[ObjectIdentifier(a)]!, b = frames[ObjectIdentifier(b)]!
            // Window coordinates grow upwards: a wholly higher view comes first,
            // and views beside each other go left to right.
            if a.minY >= b.maxY - 1 { return true }
            if b.minY >= a.maxY - 1 { return false }
            return a.minX < b.minX
        }
        return children.flatMap { child -> [NSView] in
            // A field editor belongs to its text field, which is the stop.
            if (child as? NSTextView)?.isFieldEditor == true { return [] }
            let own = child.canBecomeKeyView ? [child] : []
            return own + (child is NSTextView || child is NSControl ? [] : stops(in: child))
        }
    }

    /// Buttons are Tab stops whatever Full Keyboard Access says, as on web;
    /// a click still leaves focus where it was.
    public static var keyboardFocusAllowed: Bool {
        NSApp.currentEvent.map { ![.leftMouseDown, .rightMouseDown, .otherMouseDown].contains($0.type) } ?? true
    }
}

/// "Version 2026.11.19.3", as on every client: a click, Return or Space copies
/// "Caper 2026.11.19.3 for macOS (5c00469)" and the label reads "Copied" for two
/// seconds, announced to VoiceOver. A development build is plain text.
@MainActor func appKitVersionControl(_ installed: InstalledVersion = .current) -> NSView {
    guard let copy = installed.copyText else { return AppKitKit.label(installed.label, size: 12, wraps: false) }
    final class Feedback { weak var button: AppKitKitButton?; var generation = 0 }
    let feedback = Feedback()
    let button = AppKitKitButton(installed.label, kind: .text) {
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(copy, forType: .string)
        guard let button = feedback.button else { return }
        feedback.generation += 1
        let generation = feedback.generation
        button.title = InstalledVersion.copied
        NSAccessibility.post(element: button, notification: .announcementRequested, userInfo: [
            .announcement: "Copied \(copy)", .priority: NSAccessibilityPriorityLevel.high.rawValue,
        ])
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(InstalledVersion.feedbackSeconds))
            guard feedback.generation == generation, let button = feedback.button else { return }
            button.title = installed.label
        }
    }
    feedback.button = button
    button.setAccessibilityLabel(installed.accessibilityLabel)
    button.setAccessibilityHelp(InstalledVersion.hint); button.toolTip = InstalledVersion.hint
    button.setAccessibilityIdentifier("copy-version")
    // Space presses a focused button already; Return and Enter copy too.
    button.keyPressed = { [weak button] event in
        guard event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              [36, 76].contains(event.keyCode), let button else { return false }
        button.performClick(nil); return true
    }
    return button
}

/// Caper's button: secondary (raised, bordered) or primary (eggshell). The
/// terracotta focus ring shows only on the control that has keyboard focus,
/// which Tab gives it and a click does not.
/// Draws a button's title and image inside a horizontal inset; the layer's
/// fill (hover, selection) still covers the whole bounds.
private final class AppKitKitButtonCell: NSButtonCell {
    var inset: CGFloat = 0
    override func drawInterior(withFrame cellFrame: NSRect, in controlView: NSView) {
        super.drawInterior(withFrame: cellFrame.insetBy(dx: min(inset, cellFrame.width / 2), dy: 0), in: controlView)
    }
}

@MainActor final class AppKitKitButton: NSButton {
    override class var cellClass: AnyClass? { get { AppKitKitButtonCell.self } set {} }
    /// A left-aligned button is a list row (a destination, a device, an emoji):
    /// its title and image sit 10 points inside the row's hover and selected
    /// fill, so its text starts where the search field above starts its text.
    override var alignment: NSTextAlignment {
        didSet {
            (cell as? AppKitKitButtonCell)?.inset = alignment == .left ? 10 : 0
            invalidateIntrinsicContentSize(); needsDisplay = true
        }
    }
    /// `text`: no fill, in the normal text colour, such as the copyable version.
    enum Kind { case primary, secondary, plain, destructive, text }
    private let handler: () -> Void
    private let kind: Kind
    private var hovered = false
    private var pressed = false
    private var tracking: NSTrackingArea?
    var minimumHeight: CGFloat = 0 { didSet { invalidateIntrinsicContentSize() } }
    var fillsWidth = false
    var showsSelection = false { didSet { if oldValue != showsSelection { refresh() } } }
    /// Keys for a button that takes keyboard focus whatever the system's
    /// keyboard-navigation setting, such as a confirmation's Cancel.
    var keyPressed: ((NSEvent) -> Bool)?
    override var acceptsFirstResponder: Bool {
        keyPressed != nil || super.acceptsFirstResponder || (isEnabled && KeyboardTabOrder.keyboardFocusAllowed)
    }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    /// A right-click shows the button's `menu` as a Caper menu, not NSMenu's,
    /// and stops there rather than reaching the row or message behind it.
    override func rightMouseDown(with event: NSEvent) {
        guard let menu else { super.rightMouseDown(with: event); return }
        CaperMenu.show(menu, at: convert(event.locationInWindow, from: nil), in: self)
    }
    override func keyDown(with event: NSEvent) {
        if keyPressed?(event) == true { return }
        if event.keyCode == 48 { KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self); return }
        super.keyDown(with: event)
    }

    init(_ title: String, kind: Kind = .secondary, action: @escaping () -> Void) {
        handler = action; self.kind = kind
        super.init(frame: .zero)
        self.title = title
        target = self; self.action = #selector(fire)
        isBordered = false; bezelStyle = .regularSquare
        wantsLayer = true
        layer?.cornerRadius = 8
        focusRingType = .none
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityLabel(title)
        if kind == .primary { minimumHeight = 42 }
        NotificationCenter.default.addObserver(self, selector: #selector(handleThemeChange),
                                               name: CaperThemeStore.didChangeNotification, object: nil)
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func handleThemeChange() { refresh() }

    override var title: String {
        // A custom accessibility label (e.g. "Close What’s new") stays; one that
        // mirrors the title follows it, e.g. "Leave channel" → "Leaving…".
        didSet {
            if accessibilityLabel() == oldValue { setAccessibilityLabel(title) }
            refresh()
            invalidateIntrinsicContentSize()
        }
    }
    override var isEnabled: Bool { didSet { refresh() } }

    @objc private func fire() { handler() }

    /// Custom-drawn borderless controls have no system bezel; without zero
    /// alignment insets, Auto Layout inflates a 32pt heightAnchor (e.g. Dictate)
    /// past its neighbors inside the composer.
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsetsZero }

    override var intrinsicContentSize: NSSize {
        let text = attributedTitle.size()
        let bare = kind == .plain || kind == .text
        let horizontal: CGFloat = bare ? 4 : 24
        let vertical: CGFloat = bare ? 4 : 16
        let inset = 2 * ((cell as? AppKitKitButtonCell)?.inset ?? 0)
        return NSSize(width: ceil(text.width) + horizontal + inset, height: max(ceil(text.height) + vertical, minimumHeight))
    }

    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    func refresh() {
        let isLight = CaperThemeStore.shared.active.isLight
        var color: NSColor
        var fill: NSColor
        var stroke: NSColor = .clear
        switch kind {
        case .primary:
            color = isEnabled ? AppKitKit.onAction : AppKitKit.muted
            fill = !isEnabled ? AppKitKit.raised : pressed ? AppKitKit.actionPress : hovered ? AppKitKit.actionHover : AppKitKit.action
            if !isEnabled { stroke = AppKitKit.border }
        case .secondary, .destructive:
            color = kind == .destructive ? AppKitKit.terracottaBright : AppKitKit.text
            fill = pressed ? AppKitKit.composer : AppKitKit.raised
            stroke = AppKitKit.border
        case .plain:
            color = hovered ? AppKitKit.text : AppKitKit.muted
            fill = hovered ? AppKitKit.hoverWash : .clear
        case .text:
            color = AppKitKit.text
            fill = hovered ? AppKitKit.hoverWash : .clear
        }
        if showsSelection {
            color = AppKitKit.terracottaBright
            fill = AppKitKit.terracotta.withAlphaComponent(0.14)
            stroke = AppKitKit.terracotta
        }
        let size: CGFloat = kind == .primary ? 13 : 12
        let weight: NSFont.Weight = kind == .primary ? .bold : .medium
        attributedTitle = NSAttributedString(string: title, attributes: [
            .font: AppKitKit.font(size, weight: weight), .foregroundColor: color,
        ])
        if image != nil {
            contentTintColor = color
        }
        let resolvedFill: NSColor
        if (kind == .secondary || kind == .destructive) && hovered && isEnabled {
            resolvedFill = fill.blended(withFraction: isLight ? 0.06 : 0.12, of: isLight ? .black : .white) ?? fill
        } else {
            resolvedFill = fill
        }
        layer?.backgroundColor = resolvedFill.cgColor
        if kind == .primary {
            layer?.cornerRadius = max(18, bounds.height / 2)
        }
        if focused && isEnabled {
            layer?.borderColor = (kind == .primary ? AppKitKit.terracottaBright : AppKitKit.terracotta).cgColor
            layer?.borderWidth = 2
        } else if kind == .primary && isEnabled {
            layer?.borderColor = (isLight ? NSColor.black.withAlphaComponent(0.28) : NSColor.white.withAlphaComponent(0.62)).cgColor
            layer?.borderWidth = 1
        } else if (kind == .secondary || kind == .destructive) && isEnabled {
            let rim = isLight
                ? NSColor.black.withAlphaComponent(hovered ? 0.22 : 0.12)
                : NSColor.white.withAlphaComponent(hovered ? 0.24 : 0.14)
            layer?.borderColor = rim.cgColor
            layer?.borderWidth = 1
        } else if kind == .plain && hovered && isEnabled && !showsSelection && bounds.height >= 24 {
            layer?.borderColor = AppKitKit.border.cgColor
            layer?.borderWidth = 0.5
        } else {
            layer?.borderColor = stroke.cgColor
            layer?.borderWidth = stroke == .clear ? 0 : 1
        }
        if (kind == .primary || kind == .secondary || kind == .destructive) && isEnabled {
            layer?.shadowColor = NSColor.black.cgColor
            layer?.shadowOpacity = pressed ? (isLight ? 0.08 : 0.18) : hovered ? (isLight ? 0.18 : 0.42) : (isLight ? 0.11 : 0.32)
            layer?.shadowOffset = NSSize(width: 0, height: pressed ? -1 : -3)
            layer?.shadowRadius = hovered ? 8 : 5
            layer?.masksToBounds = false
        } else {
            layer?.shadowOpacity = 0
        }
        alphaValue = (kind == .secondary || kind == .plain) && !isEnabled ? 0.45 : 1
    }

    override func layout() {
        super.layout()
        if kind == .primary { layer?.cornerRadius = bounds.height / 2 }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
        syncHover()
    }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); syncHover() }
    override func mouseEntered(with event: NSEvent) {
        guard AppKitKit.windowActive(window) else { return }
        setHovered(true)
    }
    override func mouseExited(with event: NSEvent) { setHovered(false) }
    /// Hover from where the pointer is now. Content scrolling under a still
    /// pointer, or a recycled grid cell, can skip mouseExited and leave
    /// several buttons lit at once.
    func syncHover() {
        guard let window, AppKitKit.windowActive(window), !isHiddenOrHasHiddenAncestor else { setHovered(false); return }
        setHovered(visibleRect.contains(convert(window.mouseLocationOutsideOfEventStream, from: nil)))
    }
    private func setHovered(_ value: Bool) {
        guard hovered != value else { return }
        hovered = value; refresh()
    }
    override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        pressed = true; refresh()
        super.mouseDown(with: event)
        pressed = false; refresh()
    }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    override func becomeFirstResponder() -> Bool { defer { refresh() }; return super.becomeFirstResponder() }
    override func resignFirstResponder() -> Bool { defer { refresh() }; return super.resignFirstResponder() }
    override func accessibilityFrame() -> NSRect {
        guard window != nil else { return super.accessibilityFrame() }
        return NSAccessibility.screenRect(fromView: self, rect: bounds)
    }
}

/// NSSwitch has no tint, and SwiftUI's settings switches were terracotta, so
/// this draws the same 38×22 switch and reports itself as one to accessibility.
@MainActor final class AppKitKitSwitch: NSButton {
    override init(frame: NSRect) {
        super.init(frame: frame)
        setButtonType(.switch)
        title = ""; isBordered = false; imagePosition = .noImage
        focusRingType = .none
        setAccessibilitySubrole(.switch)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isOn: Bool {
        get { state == .on }
        set { state = newValue ? .on : .off; needsDisplay = true }
    }
    override var state: NSControl.StateValue { didSet { needsDisplay = true } }
    override var isEnabled: Bool { didSet { needsDisplay = true } }
    override var intrinsicContentSize: NSSize { NSSize(width: 38, height: 22) }
    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    override func draw(_ dirtyRect: NSRect) {
        let track = bounds.insetBy(dx: 2, dy: 2)
        let radius = track.height / 2
        let isLight = CaperThemeStore.shared.active.isLight
        // The row dims itself when disabled.
        let offFill = isLight
            ? (AppKitKit.border.blended(withFraction: 0.12, of: .black) ?? AppKitKit.border)
            : (AppKitKit.raised.blended(withFraction: 0.14, of: .white) ?? AppKitKit.raised)
        (isOn ? AppKitKit.terracotta : offFill).setFill()
        NSBezierPath(roundedRect: track, xRadius: radius, yRadius: radius).fill()
        if !isOn {
            AppKitKit.border.setStroke()
            let outline = NSBezierPath(roundedRect: track.insetBy(dx: 0.5, dy: 0.5), xRadius: radius - 0.5, yRadius: radius - 0.5)
            outline.lineWidth = 1; outline.stroke()
        }
        let side = track.height - 4
        NSColor.white.setFill()
        let knob = NSBezierPath(ovalIn: NSRect(x: isOn ? track.maxX - side - 2 : track.minX + 2, y: track.minY + 2, width: side, height: side))
        knob.fill()
        if isLight {
            NSColor.black.withAlphaComponent(0.14).setStroke()
            knob.lineWidth = 0.75; knob.stroke()
        }
        if focused && isEnabled {
            AppKitKit.terracottaBright.setStroke()
            let ring = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: bounds.height / 2 - 1, yRadius: bounds.height / 2 - 1)
            ring.lineWidth = 2; ring.stroke()
        }
    }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func keyDown(with event: NSEvent) {
        guard event.keyCode == 48 else { super.keyDown(with: event); return }
        KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self)
    }
    override func becomeFirstResponder() -> Bool { needsDisplay = true; return super.becomeFirstResponder() }
    override func resignFirstResponder() -> Bool { needsDisplay = true; return super.resignFirstResponder() }
}

/// A titled switch with a muted description, like the SwiftUI settings rows.
/// The switch carries the accessibility identifier and the row's label.
@MainActor final class AppKitKitSwitchRow: NSView {
    let toggle = AppKitKitSwitch()
    private let changed: (Bool) -> Void

    init(title: String, detail: String?, identifier: String, changed: @escaping (Bool) -> Void) {
        self.changed = changed
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let text = AppKitKit.stack([AppKitKit.label(title, size: 14)] + (detail.map { [AppKitKit.label($0, size: 12, color: AppKitKit.muted)] } ?? []), spacing: 6)
        toggle.target = self; toggle.action = #selector(toggled)
        toggle.translatesAutoresizingMaskIntoConstraints = false
        toggle.setAccessibilityIdentifier(identifier)
        toggle.setAccessibilityLabel(title)
        if let detail { toggle.setAccessibilityHelp(detail) }
        addSubview(text); addSubview(toggle)
        NSLayoutConstraint.activate([
            text.leadingAnchor.constraint(equalTo: leadingAnchor), text.topAnchor.constraint(equalTo: topAnchor),
            text.bottomAnchor.constraint(equalTo: bottomAnchor),
            toggle.leadingAnchor.constraint(greaterThanOrEqualTo: text.trailingAnchor, constant: 16),
            toggle.trailingAnchor.constraint(equalTo: trailingAnchor), toggle.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isOn: Bool {
        get { toggle.isOn }
        set { toggle.isOn = newValue }
    }
    var isEnabled: Bool {
        get { toggle.isEnabled }
        set { toggle.isEnabled = newValue; alphaValue = newValue ? 1 : 0.6 }
    }
    @objc private func toggled() { changed(toggle.isOn) }
}

/// Caper's single-line text field, as web's inputs: composer fill, a 1-point
/// border and 8-point corners, and a terracotta border while it is edited,
/// in place of the system bezel and blue focus ring.
@MainActor public final class AppKitKitField: NSTextField {
    private let height: CGFloat
    private let customFill: NSColor?
    private var themeObserver: NSObjectProtocol?
    private var tracking: NSTrackingArea?
    private var hovered = false

    /// `height`: 42 in dialogs, as on web; pickers use a shorter search field.
    /// `fill` replaces the composer fill, e.g. surface on the sign-in page.
    public init(height: CGFloat = 42, size: CGFloat = 14, fill: NSColor? = nil) {
        self.height = height
        self.customFill = fill
        super.init(frame: .zero)
        isBezeled = false; isBordered = false; drawsBackground = false
        focusRingType = .none
        font = AppKitKit.font(size, weight: .medium); textColor = AppKitKit.text
        usesSingleLineMode = true; cell?.wraps = false; cell?.isScrollable = true
        wantsLayer = true
        layer?.cornerRadius = 8; layer?.borderWidth = 1
        applyTheme()
        themeObserver = NotificationCenter.default.addObserver(
            forName: CaperThemeStore.didChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyTheme() }
        }
    }
    public required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit {
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
    }

    private func updateBorder() {
        let color: NSColor
        if currentEditor() != nil {
            color = AppKitKit.terracotta
        } else if hovered && isEnabled {
            let isLight = CaperThemeStore.shared.active.isLight
            color = isLight ? NSColor.black.withAlphaComponent(0.24) : NSColor.white.withAlphaComponent(0.24)
        } else {
            color = AppKitKit.border
        }
        layer?.borderColor = color.cgColor
    }

    private func applyTheme() {
        textColor = AppKitKit.text
        layer?.backgroundColor = (customFill ?? AppKitKit.composer).cgColor
        updateBorder()
        needsDisplay = true
    }

    public override class var cellClass: AnyClass? {
        get { AppKitKitFieldCell.self }
        set {}
    }
    public override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: height) }

    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    public override func mouseEntered(with event: NSEvent) {
        guard AppKitKit.windowActive(window) else { return }
        hovered = true; updateBorder()
    }
    public override func mouseExited(with event: NSEvent) { hovered = false; updateBorder() }
    public override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); if window == nil { hovered = false; updateBorder() } }

    public override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        syncFocus()
        return became
    }
    public override func textDidEndEditing(_ notification: Notification) {
        super.textDidEndEditing(notification)
        syncFocus()
    }
    /// Read once the field editor has settled: Return ends editing and starts it again.
    private func syncFocus() {
        DispatchQueue.main.async { [weak self] in
            self?.updateBorder()
        }
    }
}

/// Centres the field's one line vertically and insets it 11 points, as web's inputs.
private final class AppKitKitFieldCell: NSTextFieldCell {
    private static let metrics = NSLayoutManager()
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        // The cell already draws its text 2 points in.
        var area = super.drawingRect(forBounds: rect.insetBy(dx: 9, dy: 0))
        let line = ceil(Self.metrics.defaultLineHeight(for: font ?? .systemFont(ofSize: NSFont.systemFontSize)))
        if area.height > line { area.origin.y += floor((area.height - line) / 2); area.size.height = line }
        return area
    }
    // NSTextField starts editing with the field's whole bounds, and the cell
    // seats its field editor in exactly that rect: the text and caret would sit
    // in the top-left corner. Resizes re-seat the editor through drawingRect,
    // so starting it there keeps the edited line where the unedited one draws.
    // Bounds smaller than the inset (a field focused before it has a size) keep
    // the plain rect: a negative width seats the editor at a NaN origin.
    private func editorFrame(_ rect: NSRect) -> NSRect {
        let area = drawingRect(forBounds: rect)
        return area.width > 0 && area.height > 0 ? area : rect
    }
    override func edit(withFrame rect: NSRect, in controlView: NSView, editor textObj: NSText, delegate: Any?, event: NSEvent?) {
        super.edit(withFrame: editorFrame(rect), in: controlView, editor: textObj, delegate: delegate, event: event)
    }
    override func select(withFrame rect: NSRect, in controlView: NSView, editor textObj: NSText, delegate: Any?,
                         start selStart: Int, length selLength: Int) {
        super.select(withFrame: editorFrame(rect), in: controlView, editor: textObj, delegate: delegate,
                     start: selStart, length: selLength)
    }
}

/// A flipped document view, so scroll content starts at the top.
final class AppKitKitFlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// A vertical scroll view whose document is `content`, pinned to its width.
@MainActor func appKitKitScroll(_ content: NSView, insets: NSEdgeInsets) -> NSScrollView {
    let scroll = NSScrollView()
    scroll.drawsBackground = false
    scroll.hasVerticalScroller = true
    scroll.autohidesScrollers = true
    scroll.translatesAutoresizingMaskIntoConstraints = false
    let document = AppKitKitFlippedView()
    document.translatesAutoresizingMaskIntoConstraints = false
    document.addSubview(content)
    content.translatesAutoresizingMaskIntoConstraints = false
    scroll.documentView = document
    NSLayoutConstraint.activate([
        document.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
        document.trailingAnchor.constraint(equalTo: scroll.contentView.trailingAnchor),
        document.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
        content.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: insets.left),
        content.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -insets.right),
        content.topAnchor.constraint(equalTo: document.topAnchor, constant: insets.top),
        content.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -insets.bottom),
    ])
    return scroll
}

/// A version label that opens its commit, muted at rest and underlined on hover.
/// A plain link: no fill, the shared external-link icon after its text in the
/// text's colour, and one underline under both.
@MainActor final class AppKitKitLink: NSButton {
    private let url: URL
    /// The label as given; `title` reads back the styled title, so restyling
    /// from it would append another space and icon on every hover.
    private let label: String
    private var tracking: NSTrackingArea?
    private let size: CGFloat
    private let color: NSColor?
    private var hovered = false

    /// `color` replaces the muted rest colour, e.g. red for Delete account.
    init(_ title: String, url: URL, size: CGFloat = 13, color: NSColor? = nil, help: String? = "View this version on GitHub") {
        self.url = url; self.label = title; self.size = size; self.color = color
        super.init(frame: .zero)
        self.title = title
        isBordered = false; bezelStyle = .regularSquare
        imagePosition = .imageTrailing; imageHugsTitle = true
        target = self; action = #selector(open)
        toolTip = help
        if let help { setAccessibilityHelp(help) }
        setAccessibilityLabel(title)
        translatesAutoresizingMaskIntoConstraints = false
        style()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private var tint: NSColor { color ?? (hovered ? AppKitKit.actionHover : AppKitKit.muted.withAlphaComponent(0.72)) }

    private func style() {
        let tint = tint
        attributedTitle = NSAttributedString(string: label, attributes: [.font: AppKitKit.font(size), .foregroundColor: tint])
        image = Self.icon(tint)
        needsDisplay = true
    }

    /// The text starts 2 pt in, where an NSTextField label's text starts, so it
    /// lines up with the labels around it; NSButtonCell pads its title further.
    private static let inset: CGFloat = 2
    private static let iconGap: CGFloat = 4
    override var intrinsicContentSize: NSSize {
        let text = attributedTitle.size()
        return NSSize(width: Self.inset + ceil(text.width) + Self.iconGap + 12 + Self.inset, height: ceil(max(text.height, 12)))
    }

    /// The shared external-link icon (12 pt) in `tint`, its alpha included.
    private static func icon(_ tint: NSColor) -> NSImage? {
        guard let source = NSImage(named: "caper-external-link") ?? caperResourceBundle.image(forResource: "caper-external-link") else { return nil }
        let rect = NSRect(x: 0, y: 0, width: 12, height: 12)
        let solid = NSImage(size: rect.size, flipped: false) { r in
            source.draw(in: r)
            tint.withAlphaComponent(1).set()
            r.fill(using: .sourceAtop)
            return true
        }
        return NSImage(size: rect.size, flipped: false) { r in
            solid.draw(in: r, from: .zero, operation: .sourceOver, fraction: tint.alphaComponent)
            return true
        }
    }

    /// One 1 pt underline under the text and its icon while hovered.
    override func draw(_ dirtyRect: NSRect) {
        let text = attributedTitle.size()
        attributedTitle.draw(at: NSPoint(x: bounds.minX + Self.inset, y: bounds.midY - text.height / 2))
        image?.draw(in: NSRect(x: bounds.minX + Self.inset + ceil(text.width) + Self.iconGap, y: bounds.midY - 6, width: 12, height: 12),
                    from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        guard hovered else { return }
        tint.withAlphaComponent(tint.alphaComponent * 0.7).setFill()
        let y = isFlipped ? bounds.maxY - 1 : bounds.minY
        NSRect(x: bounds.minX + Self.inset, y: y, width: bounds.width - 2 * Self.inset, height: 1).fill()
    }
    @objc private func open() { NSWorkspace.shared.open(url) }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; fade() }
    override func mouseExited(with event: NSEvent) { hovered = false; fade() }
    /// A plain link's 200 ms ease-in-out cross-fade to the new colour and
    /// underline; at once with Reduce motion.
    private func fade() {
        if !CaperAccessibility.reduceMotion {
            wantsLayer = true
            let transition = CATransition()
            transition.type = .fade
            transition.duration = CaperMotion.base
            transition.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            layer?.add(transition, forKey: "hover")
        }
        style()
    }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .pointingHand) }
}

/// Bespoke Caper tactile slider replacing stock macOS NSSlider: grooved track,
/// botanical green active range, optional default-value tick + double-click reset,
/// and a beveled circular chalk thumb with hover elevation.
@MainActor public final class AppKitCaperSlider: NSControl {
    public var minValue: Double = 0 { didSet { needsDisplay = true } }
    public var maxValue: Double = 100 { didSet { needsDisplay = true } }
    public var defaultValue: Double? = nil { didSet { needsDisplay = true } }
    public var snapTolerance: Double = 4
    public var onChange: ((Double) -> Void)?
    private var _value: Double = 0
    private var hovered = false
    private var dragging = false
    private var tracking: NSTrackingArea?

    public var value: Double {
        get { _value }
        set {
            let clamped = min(maxValue, max(minValue, newValue))
            guard clamped != _value else { return }
            _value = clamped
            setAccessibilityValue("\(Int(clamped.rounded()))%")
            needsDisplay = true
        }
    }

    public init(value: Double = 100, minValue: Double = 0, maxValue: Double = 200, defaultValue: Double? = 100, onChange: ((Double) -> Void)? = nil) {
        self.minValue = minValue
        self.maxValue = maxValue
        self.defaultValue = defaultValue
        self._value = min(maxValue, max(minValue, value))
        self.onChange = onChange
        super.init(frame: .zero)
        focusRingType = .none
        setAccessibilityRole(.slider)
        setAccessibilityValue("\(Int(self._value.rounded()))%")
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 26) }
    public override var isEnabled: Bool { didSet { needsDisplay = true } }
    public override var acceptsFirstResponder: Bool { isEnabled }
    public override var isFlipped: Bool { true }
    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    private var trackRect: NSRect {
        let inset: CGFloat = 10
        let width = max(1, bounds.width - inset * 2)
        return NSRect(x: inset, y: bounds.midY - 3, width: width, height: 6)
    }

    private func fraction(for val: Double) -> CGFloat {
        guard maxValue > minValue else { return 0 }
        return CGFloat(min(1, max(0, (val - minValue) / (maxValue - minValue))))
    }

    public override func draw(_ dirtyRect: NSRect) {
        let track = trackRect
        let r = track.height / 2
        let opacity: CGFloat = isEnabled ? 1 : 0.45

        // Recessed track background with hairline rim.
        let trackPath = NSBezierPath(roundedRect: track, xRadius: r, yRadius: r)
        AppKitKit.composer.withAlphaComponent(opacity).setFill()
        trackPath.fill()
        AppKitKit.border.withAlphaComponent(opacity).setStroke()
        trackPath.lineWidth = 1
        trackPath.stroke()

        // Default-value notch under track.
        if let def = defaultValue, def >= minValue && def <= maxValue {
            let x = track.minX + track.width * fraction(for: def)
            AppKitKit.muted.withAlphaComponent(0.55 * opacity).setFill()
            NSRect(x: x - 1, y: track.maxY + 3, width: 2, height: 4).fill()
        }

        // Botanical green active range fill.
        let frac = fraction(for: _value)
        let fillW = max(track.height, track.width * frac)
        if frac > 0 {
            let fillRect = NSRect(x: track.minX, y: track.minY, width: min(track.width, fillW), height: track.height)
            let fillPath = NSBezierPath(roundedRect: fillRect, xRadius: r, yRadius: r)
            AppKitKit.terracottaBright.withAlphaComponent(opacity).setFill()
            fillPath.fill()
        }

        // Beveled circular thumb with drop shadow and specular top rim.
        let thumbD: CGFloat = (dragging || hovered) && isEnabled ? 18 : 16
        let cx = track.minX + track.width * frac
        let thumbRect = NSRect(x: cx - thumbD / 2, y: bounds.midY - thumbD / 2, width: thumbD, height: thumbD)

        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.45 * opacity)
        shadow.shadowOffset = NSSize(width: 0, height: dragging ? -1 : -2)
        shadow.shadowBlurRadius = dragging ? 4 : 6
        shadow.set()
        let thumbPath = NSBezierPath(ovalIn: thumbRect)
        AppKitKit.action.withAlphaComponent(opacity).setFill()
        thumbPath.fill()
        NSGraphicsContext.restoreGraphicsState()

        if let grad = NSGradient(starting: NSColor.white.withAlphaComponent(0.35 * opacity), ending: NSColor.black.withAlphaComponent(0.14 * opacity)) {
            grad.draw(in: thumbPath, angle: 90)
        }
        (focused && isEnabled ? AppKitKit.terracottaBright : AppKitKit.terracotta.withAlphaComponent(0.75 * opacity)).setStroke()
        thumbPath.lineWidth = focused && isEnabled ? 2 : 1.5
        thumbPath.stroke()
    }

    private func valueForPoint(_ point: NSPoint, snap: Bool) -> Double {
        let track = trackRect
        let frac = Double(min(1, max(0, (point.x - track.minX) / max(1, track.width))))
        var next = minValue + frac * (maxValue - minValue)
        if snap, let def = defaultValue, abs(next - def) <= snapTolerance {
            next = def
        }
        return next.rounded()
    }

    public override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        window?.makeFirstResponder(self)
        if event.clickCount == 2, let def = defaultValue {
            commit(def)
            return
        }
        dragging = true
        let pt = convert(event.locationInWindow, from: nil)
        commit(valueForPoint(pt, snap: true))
        while let next = window?.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            let p = convert(next.locationInWindow, from: nil)
            if next.type == .leftMouseUp {
                dragging = false
                needsDisplay = true
                break
            }
            commit(valueForPoint(p, snap: true))
        }
    }

    public override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123, 125: _ = step(-1)
        case 124, 126: _ = step(1)
        default: super.keyDown(with: event)
        }
    }

    public override func accessibilityPerformIncrement() -> Bool { step(1) }
    public override func accessibilityPerformDecrement() -> Bool { step(-1) }

    private func step(_ delta: Double) -> Bool {
        guard isEnabled else { return false }
        commit(min(maxValue, max(minValue, _value + delta)))
        return true
    }

    private func commit(_ next: Double) {
        let clamped = min(maxValue, max(minValue, next))
        _value = clamped
        setAccessibilityValue("\(Int(clamped.rounded()))%")
        needsDisplay = true
        onChange?(clamped)
    }

    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    public override func mouseEntered(with event: NSEvent) { hovered = true; needsDisplay = true }
    public override func mouseExited(with event: NSEvent) { hovered = false; needsDisplay = true }
    public override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    public override func becomeFirstResponder() -> Bool { needsDisplay = true; return super.becomeFirstResponder() }
    public override func resignFirstResponder() -> Bool { needsDisplay = true; return super.resignFirstResponder() }
}

/// Bespoke tactile Caper dropdown replacing stock macOS NSPopUpButton + NSMenu.
/// Opens a custom borderless floating card with beveled option rows, hover
/// highlights, and a botanical green checkmark on the selected item.
@MainActor public final class AppKitCaperSelect: NSControl {
    public var items: [String] = [] {
        didSet {
            if selectedIndex >= items.count { selectedIndex = max(0, items.count - 1) }
            setAccessibilityValue(currentTitle)
            needsDisplay = true
        }
    }
    public var selectedIndex: Int = 0 {
        didSet {
            let clamped = items.isEmpty ? 0 : min(items.count - 1, max(0, selectedIndex))
            if selectedIndex != clamped { selectedIndex = clamped; return }
            setAccessibilityValue(currentTitle)
            needsDisplay = true
        }
    }
    public var onSelect: ((Int) -> Void)?
    public var isOpen: Bool { popup != nil }
    private var hovered = false
    private var tracking: NSTrackingArea?
    private var popup: AppKitSelectPopup?
    private var currentTitle: String { items.indices.contains(selectedIndex) ? items[selectedIndex] : "" }

    public init(items: [String] = [], selectedIndex: Int = 0, onSelect: ((Int) -> Void)? = nil) {
        self.items = items
        self.selectedIndex = selectedIndex
        self.onSelect = onSelect
        super.init(frame: .zero)
        focusRingType = .none
        setAccessibilityRole(.popUpButton)
        setAccessibilityValue(currentTitle)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { popup?.closePopup() }

    public override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 34) }
    public override var isEnabled: Bool { didSet { needsDisplay = true } }
    public override var acceptsFirstResponder: Bool { isEnabled }
    public override var isFlipped: Bool { true }
    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    public override func draw(_ dirtyRect: NSRect) {
        let opacity: CGFloat = isEnabled ? 1 : 0.45
        let rect = bounds.insetBy(dx: 0.5, dy: 0.5)
        let path = NSBezierPath(roundedRect: rect, xRadius: 10, yRadius: 10)

        let base = hovered || isOpen
            ? (AppKitKit.raised.blended(withFraction: 0.1, of: .white) ?? AppKitKit.raised)
            : AppKitKit.raised
        base.withAlphaComponent(opacity).setFill()
        path.fill()

        // Top specular rim highlight for tactile depth.
        if isEnabled {
            let rim = NSBezierPath()
            rim.move(to: NSPoint(x: rect.minX + 10, y: rect.minY + 1))
            rim.line(to: NSPoint(x: rect.maxX - 10, y: rect.minY + 1))
            NSColor.white.withAlphaComponent(hovered || isOpen ? 0.22 : 0.12).setStroke()
            rim.lineWidth = 1
            rim.stroke()
        }

        let borderColor = focused && isEnabled
            ? AppKitKit.terracottaBright
            : isOpen ? AppKitKit.terracotta : NSColor.white.withAlphaComponent(hovered ? 0.24 : 0.12)
        borderColor.withAlphaComponent(opacity).setStroke()
        path.lineWidth = focused && isEnabled ? 2 : 1
        path.stroke()

        // Selected label.
        let style = NSMutableParagraphStyle(); style.lineBreakMode = .byTruncatingTail
        let attrs: [NSAttributedString.Key: Any] = [
            .font: AppKitKit.font(12.5, weight: .medium),
            .foregroundColor: AppKitKit.text.withAlphaComponent(opacity),
            .paragraphStyle: style,
        ]
        let label = NSAttributedString(string: currentTitle, attributes: attrs)
        let labelH = label.size().height
        label.draw(in: NSRect(x: 12, y: bounds.midY - labelH / 2, width: max(0, bounds.width - 36), height: labelH + 2))

        // Crisp custom chevron.
        let cx = bounds.maxX - 16, cy = bounds.midY
        let chevron = NSBezierPath()
        chevron.lineCapStyle = .round; chevron.lineJoinStyle = .round; chevron.lineWidth = 1.6
        if isOpen {
            chevron.move(to: NSPoint(x: cx - 4, y: cy + 2))
            chevron.line(to: NSPoint(x: cx, y: cy - 2))
            chevron.line(to: NSPoint(x: cx + 4, y: cy + 2))
        } else {
            chevron.move(to: NSPoint(x: cx - 4, y: cy - 2))
            chevron.line(to: NSPoint(x: cx, y: cy + 2))
            chevron.line(to: NSPoint(x: cx + 4, y: cy - 2))
        }
        (hovered || isOpen ? AppKitKit.text : AppKitKit.muted).withAlphaComponent(opacity).setStroke()
        chevron.stroke()
    }

    public override func mouseDown(with event: NSEvent) {
        guard isEnabled, !items.isEmpty else { return }
        window?.makeFirstResponder(self)
        if isOpen { closePopup() } else { openPopup() }
    }

    public override func keyDown(with event: NSEvent) {
        guard isEnabled, !items.isEmpty else { super.keyDown(with: event); return }
        switch event.keyCode {
        case 49, 36, 76, 125: openPopup()
        default: super.keyDown(with: event)
        }
    }

    public func closePopup() {
        popup?.closePopup()
        popup = nil
        needsDisplay = true
    }

    private func openPopup() {
        guard let parentWindow = window, !items.isEmpty else { return }
        closePopup()
        let panel = AppKitSelectPopup(
            anchor: self,
            parentWindow: parentWindow,
            items: items,
            selectedIndex: selectedIndex,
            onChoose: { [weak self] index in
                guard let self else { return }
                self.selectedIndex = index
                self.closePopup()
                self.onSelect?(index)
            },
            onDismiss: { [weak self] in
                self?.popup = nil
                self?.needsDisplay = true
            }
        )
        popup = panel
        needsDisplay = true
        panel.show()
    }

    public override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow == nil { closePopup() }
        super.viewWillMove(toWindow: newWindow)
    }

    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    public override func mouseEntered(with event: NSEvent) { hovered = true; needsDisplay = true }
    public override func mouseExited(with event: NSEvent) { hovered = false; needsDisplay = true }
    public override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    public override func becomeFirstResponder() -> Bool { needsDisplay = true; return super.becomeFirstResponder() }
    public override func resignFirstResponder() -> Bool { needsDisplay = true; return super.resignFirstResponder() }
}

@MainActor private final class AppKitSelectPopup: NSPanel {
    private weak var anchor: NSView?
    private weak var ownerWindow: NSWindow?
    private let onDismiss: () -> Void
    private var monitor: Any?
    private var closed = false

    init(anchor: NSView, parentWindow: NSWindow, items: [String], selectedIndex: Int,
         onChoose: @escaping (Int) -> Void, onDismiss: @escaping () -> Void) {
        self.anchor = anchor
        self.ownerWindow = parentWindow
        self.onDismiss = onDismiss

        let rowH: CGFloat = 32
        let pad: CGFloat = 6
        let width = max(anchor.bounds.width, 220)
        let maxH: CGFloat = 280
        let contentH = CGFloat(items.count) * rowH + pad * 2
        let height = min(maxH, contentH)

        let anchorRect = anchor.convert(anchor.bounds, to: nil)
        let screenRect = parentWindow.convertToScreen(anchorRect)
        let screenFrame = parentWindow.screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        var originY = screenRect.minY - height - 4
        if originY < screenFrame.minY + 8 { originY = screenRect.maxY + 4 }
        let originX = min(max(screenFrame.minX + 8, screenRect.minX), screenFrame.maxX - width - 8)

        super.init(contentRect: NSRect(x: originX, y: originY, width: width, height: height),
                   styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        level = .popUpMenu
        collectionBehavior = [.transient, .ignoresCycle]
        appearance = parentWindow.appearance

        let card = NSView(frame: NSRect(x: 0, y: 0, width: width, height: height))
        card.wantsLayer = true
        card.layer?.backgroundColor = AppKitKit.surface.cgColor
        card.layer?.cornerRadius = 13
        card.layer?.borderWidth = 1
        card.layer?.borderColor = AppKitKit.border.cgColor
        card.layer?.masksToBounds = true

        let list = AppKitSelectListView(items: items, selectedIndex: selectedIndex, width: width - pad * 2, rowH: rowH, onChoose: onChoose)
        let scroll = NSScrollView(frame: NSRect(x: pad, y: pad, width: width - pad * 2, height: height - pad * 2))
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = contentH > maxH
        scroll.scrollerStyle = .overlay
        scroll.documentView = list
        card.addSubview(scroll)
        contentView = card
    }

    func show() {
        ownerWindow?.addChildWindow(self, ordered: .above)
        orderFront(nil)
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .keyDown]) { [weak self] event in
            guard let self else { return event }
            if event.type == .keyDown && event.keyCode == 53 {
                self.closePopup()
                return nil
            }
            if event.type == .leftMouseDown || event.type == .rightMouseDown {
                if event.window !== self {
                    let ptInAnchor = self.anchor?.convert(event.locationInWindow, from: nil)
                    let clickedAnchor = event.window === self.ownerWindow && (self.anchor?.bounds.contains(ptInAnchor ?? .zero) == true)
                    self.closePopup()
                    if clickedAnchor { return nil }
                }
            }
            return event
        }
    }

    func closePopup() {
        guard !closed else { return }
        closed = true
        if let monitor { NSEvent.removeMonitor(monitor); self.monitor = nil }
        ownerWindow?.removeChildWindow(self)
        orderOut(nil)
        onDismiss()
    }
}

@MainActor private final class AppKitSelectListView: NSView {
    private let items: [String]
    private let selectedIndex: Int
    private let rowH: CGFloat
    private let onChoose: (Int) -> Void
    private var hoveredIndex: Int? = nil
    private var tracking: NSTrackingArea?

    init(items: [String], selectedIndex: Int, width: CGFloat, rowH: CGFloat, onChoose: @escaping (Int) -> Void) {
        self.items = items
        self.selectedIndex = selectedIndex
        self.rowH = rowH
        self.onChoose = onChoose
        super.init(frame: NSRect(x: 0, y: 0, width: width, height: CGFloat(items.count) * rowH))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        for (i, title) in items.enumerated() {
            let row = NSRect(x: 0, y: CGFloat(i) * rowH, width: bounds.width, height: rowH)
            let isSelected = i == selectedIndex
            let isHovered = i == hoveredIndex
            let pill = row.insetBy(dx: 0, dy: 1)
            let path = NSBezierPath(roundedRect: pill, xRadius: 8, yRadius: 8)
            if isHovered {
                AppKitKit.hoverWash.setFill()
                path.fill()
                AppKitKit.border.setStroke()
                path.lineWidth = 1
                path.stroke()
            } else if isSelected {
                AppKitKit.terracotta.withAlphaComponent(0.22).setFill()
                path.fill()
                AppKitKit.terracottaBright.withAlphaComponent(0.35).setStroke()
                path.lineWidth = 1
                path.stroke()
            }

            if isSelected {
                let check = NSBezierPath()
                check.lineCapStyle = .round; check.lineJoinStyle = .round; check.lineWidth = 1.8
                let x = pill.minX + 10, y = pill.midY
                check.move(to: NSPoint(x: x, y: y))
                check.line(to: NSPoint(x: x + 3.2, y: y + 3.2))
                check.line(to: NSPoint(x: x + 9, y: y - 3.2))
                AppKitKit.terracottaBright.setStroke()
                check.stroke()
            }

            let style = NSMutableParagraphStyle(); style.lineBreakMode = .byTruncatingTail
            let attrs: [NSAttributedString.Key: Any] = [
                .font: AppKitKit.font(12.5, weight: isSelected ? .bold : .regular),
                .foregroundColor: isSelected || isHovered ? AppKitKit.text : AppKitKit.text.withAlphaComponent(0.88),
                .paragraphStyle: style,
            ]
            let attr = NSAttributedString(string: title, attributes: attrs)
            let h = attr.size().height
            attr.draw(in: NSRect(x: 28, y: pill.midY - h / 2, width: max(0, pill.width - 38), height: h + 2))
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let idx = Int(p.y / rowH)
        let next = items.indices.contains(idx) ? idx : nil
        if hoveredIndex != next { hoveredIndex = next; needsDisplay = true }
    }

    override func mouseExited(with event: NSEvent) {
        if hoveredIndex != nil { hoveredIndex = nil; needsDisplay = true }
    }

    override func mouseUp(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let idx = Int(p.y / rowH)
        if items.indices.contains(idx) { onChoose(idx) }
    }

    override func resetCursorRects() { addCursorRect(bounds, cursor: .pointingHand) }
}
#endif
