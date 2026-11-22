#if os(macOS)
import AppKit
import Observation
import SwiftUI

/// Building blocks for the AppKit app's native dialogs, which replace the
/// SwiftUI views it used to host. Colors come from CaperTheme's tokens.
@MainActor enum AppKitKit {
    static var surface: NSColor { NSColor(CaperTheme.surface) }
    static var raised: NSColor { NSColor(CaperTheme.raised) }
    static var composer: NSColor { NSColor(CaperTheme.composer) }
    static var border: NSColor { NSColor(CaperTheme.border) }
    static var text: NSColor { NSColor(CaperTheme.text) }
    static var muted: NSColor { NSColor(CaperTheme.muted) }
    static var error: NSColor { NSColor(CaperTheme.error) }
    static var terracotta: NSColor { NSColor(CaperTheme.terracotta) }
    static var terracottaBright: NSColor { NSColor(CaperTheme.terracottaBright) }
    static var action: NSColor { NSColor(CaperTheme.action) }
    static var actionHover: NSColor { NSColor(CaperTheme.actionHover) }
    static var actionPress: NSColor { NSColor(CaperTheme.actionPress) }
    static var onAction: NSColor { NSColor(CaperTheme.onAction) }

    static func font(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        CaperFontLoader.register()
        let name = switch weight {
        case .bold, .heavy, .semibold: "Inrond40-Bold"
        case .medium: "Inrond40-Medium"
        case .black: "Inrond40-Black"
        default: "Inrond40-Regular"
        }
        return NSFont(name: name, size: size) ?? .systemFont(ofSize: size, weight: weight)
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
@MainActor final class AppKitKitButton: NSButton {
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
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var title: String {
        // A custom accessibility label (e.g. "Close What’s new") stays; one that
        // mirrors the title follows it, e.g. "Leave channel" → "Leaving…".
        didSet { if accessibilityLabel() == oldValue { setAccessibilityLabel(title) }; refresh() }
    }
    override var isEnabled: Bool { didSet { refresh() } }

    @objc private func fire() { handler() }

    override var intrinsicContentSize: NSSize {
        let text = attributedTitle.size()
        let bare = kind == .plain || kind == .text
        let horizontal: CGFloat = bare ? 4 : 24
        let vertical: CGFloat = bare ? 4 : 16
        return NSSize(width: ceil(text.width) + horizontal, height: max(ceil(text.height) + vertical, minimumHeight))
    }

    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    func refresh() {
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
            fill = .clear
        case .text:
            color = AppKitKit.text
            fill = hovered ? NSColor.white.withAlphaComponent(0.06) : .clear
        }
        if showsSelection {
            color = AppKitKit.terracottaBright
            fill = AppKitKit.terracotta.withAlphaComponent(0.12)
            stroke = AppKitKit.terracotta
        }
        let size: CGFloat = kind == .primary ? 13 : 12
        let weight: NSFont.Weight = kind == .primary ? .bold : .medium
        attributedTitle = NSAttributedString(string: title, attributes: [
            .font: AppKitKit.font(size, weight: weight), .foregroundColor: color,
        ])
        layer?.backgroundColor = fill.cgColor
        if focused && isEnabled {
            layer?.borderColor = (kind == .primary ? AppKitKit.terracottaBright : AppKitKit.terracotta).cgColor
            layer?.borderWidth = 2
        } else {
            layer?.borderColor = stroke.cgColor
            layer?.borderWidth = stroke == .clear ? 0 : 1
        }
        alphaValue = kind == .secondary && !isEnabled ? 0.45 : 1
        if kind == .secondary || kind == .destructive, hovered, isEnabled {
            layer?.backgroundColor = fill.blended(withFraction: 0.06, of: .white)?.cgColor
        }
        invalidateIntrinsicContentSize()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; refresh() }
    override func mouseExited(with event: NSEvent) { hovered = false; refresh() }
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
        // The row dims itself when disabled.
        (isOn ? AppKitKit.terracotta : AppKitKit.raised.blended(withFraction: 0.12, of: .white) ?? AppKitKit.raised).setFill()
        NSBezierPath(roundedRect: track, xRadius: radius, yRadius: radius).fill()
        if !isOn {
            AppKitKit.border.setStroke()
            let outline = NSBezierPath(roundedRect: track.insetBy(dx: 0.5, dy: 0.5), xRadius: radius - 0.5, yRadius: radius - 0.5)
            outline.lineWidth = 1; outline.stroke()
        }
        let side = track.height - 4
        NSColor.white.setFill()
        NSBezierPath(ovalIn: NSRect(x: isOn ? track.maxX - side - 2 : track.minX + 2, y: track.minY + 2, width: side, height: side)).fill()
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

    /// `height`: 42 in dialogs, as on web; pickers use a shorter search field.
    /// `fill` replaces the composer fill, e.g. surface on the sign-in page.
    public init(height: CGFloat = 42, size: CGFloat = 14, fill: NSColor? = nil) {
        self.height = height
        super.init(frame: .zero)
        isBezeled = false; isBordered = false; drawsBackground = false
        focusRingType = .none
        font = AppKitKit.font(size, weight: .medium); textColor = AppKitKit.text
        usesSingleLineMode = true; cell?.wraps = false; cell?.isScrollable = true
        wantsLayer = true
        layer?.cornerRadius = 8; layer?.borderWidth = 1
        layer?.backgroundColor = (fill ?? AppKitKit.composer).cgColor
        layer?.borderColor = AppKitKit.border.cgColor
    }
    public required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public override class var cellClass: AnyClass? {
        get { AppKitKitFieldCell.self }
        set {}
    }
    public override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: height) }

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
            guard let self else { return }
            self.layer?.borderColor = (self.currentEditor() != nil ? AppKitKit.terracotta : AppKitKit.border).cgColor
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
#endif
