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
        case .bold, .heavy, .semibold: "Satoshi-Bold"
        case .medium: "Satoshi-Medium"
        case .black: "Satoshi-Black"
        default: "Satoshi-Regular"
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

/// Caper's button: secondary (raised, bordered) or primary (eggshell). The
/// terracotta focus ring shows only on the control that has keyboard focus,
/// which AppKit gives buttons only during keyboard navigation.
@MainActor final class AppKitKitButton: NSButton {
    enum Kind { case primary, secondary, plain, destructive }
    private let handler: () -> Void
    private let kind: Kind
    private var hovered = false
    private var pressed = false
    private var tracking: NSTrackingArea?
    var minimumHeight: CGFloat = 0 { didSet { invalidateIntrinsicContentSize() } }
    var fillsWidth = false

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

    override var title: String { didSet { refresh() } }
    override var isEnabled: Bool { didSet { refresh() } }

    @objc private func fire() { handler() }

    override var intrinsicContentSize: NSSize {
        let text = attributedTitle.size()
        let horizontal: CGFloat = kind == .plain ? 4 : 24
        let vertical: CGFloat = kind == .plain ? 4 : 16
        return NSSize(width: ceil(text.width) + horizontal, height: max(ceil(text.height) + vertical, minimumHeight))
    }

    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    func refresh() {
        let color: NSColor
        let fill: NSColor
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

/// A titled switch with a muted description, like the SwiftUI settings rows.
/// The switch carries the accessibility identifier and the row's label.
@MainActor final class AppKitKitSwitchRow: NSView {
    let toggle = NSSwitch()
    private let changed: (Bool) -> Void

    init(title: String, detail: String, identifier: String, changed: @escaping (Bool) -> Void) {
        self.changed = changed
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let text = AppKitKit.stack([AppKitKit.label(title, size: 14), AppKitKit.label(detail, size: 12, color: AppKitKit.muted)], spacing: 6)
        toggle.target = self; toggle.action = #selector(toggled)
        toggle.translatesAutoresizingMaskIntoConstraints = false
        toggle.setAccessibilityIdentifier(identifier)
        toggle.setAccessibilityLabel(title)
        toggle.setAccessibilityHelp(detail)
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
        get { toggle.state == .on }
        set { toggle.state = newValue ? .on : .off }
    }
    var isEnabled: Bool {
        get { toggle.isEnabled }
        set { toggle.isEnabled = newValue; alphaValue = newValue ? 1 : 0.6 }
    }
    @objc private func toggled() { changed(toggle.state == .on) }
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
@MainActor final class AppKitKitLink: NSButton {
    private let url: URL
    private var tracking: NSTrackingArea?
    private let size: CGFloat
    private var hovered = false

    init(_ title: String, url: URL, size: CGFloat = 13) {
        self.url = url; self.size = size
        super.init(frame: .zero)
        self.title = title
        isBordered = false; bezelStyle = .regularSquare
        target = self; action = #selector(open)
        toolTip = "View this version on GitHub"
        setAccessibilityHelp("View this version on GitHub")
        setAccessibilityLabel(title)
        translatesAutoresizingMaskIntoConstraints = false
        style()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func style() {
        var attributes: [NSAttributedString.Key: Any] = [
            .font: AppKitKit.font(size),
            .foregroundColor: hovered ? AppKitKit.actionHover : AppKitKit.muted.withAlphaComponent(0.72),
        ]
        if hovered { attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        attributedTitle = NSAttributedString(string: title + " ↗", attributes: attributes)
    }
    @objc private func open() { NSWorkspace.shared.open(url) }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; style() }
    override func mouseExited(with event: NSEvent) { hovered = false; style() }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .pointingHand) }
}
#endif
