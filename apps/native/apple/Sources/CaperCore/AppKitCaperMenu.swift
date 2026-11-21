#if os(macOS)
import AppKit

/// What an NSMenuItem means to a Caper menu beyond its title and state.
public enum CaperMenuRole { case destructive, note }

public extension NSMenuItem {
    /// Delete, Block and Leave draw error-coloured; notes draw muted.
    var caperRole: CaperMenuRole? {
        get { representedObject as? CaperMenuRole }
        set { representedObject = newValue }
    }
}

public extension CaperMenuItem {
    /// Reads a menu built with NSMenu: the AppKit builders keep NSMenuItem as
    /// their data, and CaperMenu draws it instead of the system menu.
    @MainActor static func items(from menu: NSMenu) -> [CaperMenuItem] {
        normalized(menu.items.compactMap { (item) -> CaperMenuItem? in
            if item.isSeparatorItem { return .separator }
            if item.isHidden { return nil }
            var result: CaperMenuItem
            if let submenu = item.submenu {
                result = .submenu(item.title, enabled: item.isEnabled, items(from: submenu))
            } else if item.caperRole == .note || item.action == nil {
                result = .note(item.title)
            } else {
                result = .action(item.title, enabled: item.isEnabled, selected: item.state == .on,
                                 destructive: item.caperRole == .destructive) { [item] in
                    guard let action = item.action else { return }
                    NSApp.sendAction(action, to: item.target, from: item)
                }
            }
            let identifier = item.accessibilityIdentifier()
            if !identifier.isEmpty { result.identifier = identifier }
            return result
        })
    }
}

/// Caper's own popup and context menu on the Mac, in place of NSMenu's
/// translucent system menu: the surface fill, border, 8-point corners and
/// rows of web's `.space-actions` menu. Submenus expand in place.
@MainActor public final class CaperMenu: NSObject {
    /// The open menu, if any. UI and view tests read and act on it.
    public private(set) static var current: CaperMenu?

    private let panel: CaperMenuPanel
    private let content: CaperMenuView
    private weak var parentWindow: NSWindow?
    private weak var anchor: NSView?
    /// Screen y of the edge the menu hangs from, and whether it grows upward.
    private var edgeY: CGFloat = 0
    private var growsUp = false
    private var monitors: [Any] = []
    private var closed = false

    // MARK: Opening

    /// Opens below `anchor`, left-aligned, or above it when there is no room.
    @discardableResult
    public static func show(_ menu: NSMenu, below anchor: NSView) -> CaperMenu? {
        show(CaperMenuItem.items(from: menu), below: anchor)
    }

    @discardableResult
    public static func show(_ items: [CaperMenuItem], below anchor: NSView) -> CaperMenu? {
        guard let window = anchor.window else { return nil }
        let rect = window.convertToScreen(anchor.convert(anchor.bounds, to: nil))
        return open(items, x: rect.minX, below: rect.minY - 4, above: rect.maxY + 4, window: window, anchor: anchor)
    }

    /// Opens with its top-left corner at `point` in `view` (screen coordinates
    /// when `view` is nil), as a context menu at the pointer.
    @discardableResult
    public static func show(_ menu: NSMenu, at point: NSPoint, in view: NSView?) -> CaperMenu? {
        show(CaperMenuItem.items(from: menu), at: point, in: view)
    }

    @discardableResult
    public static func show(_ items: [CaperMenuItem], at point: NSPoint, in view: NSView?) -> CaperMenu? {
        var screen = point
        if let view, let window = view.window { screen = window.convertPoint(toScreen: view.convert(point, to: nil)) }
        return open(items, x: screen.x, below: screen.y, above: screen.y,
                    window: view?.window ?? NSApp.keyWindow, anchor: nil)
    }

    private static func open(_ items: [CaperMenuItem], x: CGFloat, below: CGFloat, above: CGFloat,
                             window: NSWindow?, anchor: NSView?) -> CaperMenu? {
        current?.close()
        let items = CaperMenuItem.normalized(items)
        guard !items.isEmpty else { return nil }
        let menu = CaperMenu(items: items, window: window, anchor: anchor)
        current = menu
        menu.present(x: x, below: below, above: above)
        return menu
    }

    private init(items: [CaperMenuItem], window: NSWindow?, anchor: NSView?) {
        content = CaperMenuView(items: items)
        panel = CaperMenuPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        parentWindow = window
        self.anchor = anchor
        super.init()
        panel.isReleasedWhenClosed = false
        panel.level = .popUpMenu
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.acceptsMouseMovedEvents = true
        panel.collectionBehavior = [.transient, .ignoresCycle, .fullScreenAuxiliary]
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.contentView = content
        content.activate = { [weak self] perform in
            self?.close()
            perform()
        }
        content.cancel = { [weak self] in self?.close() }
        content.resized = { [weak self] in self?.place(animated: false) }
    }

    private func present(x: CGFloat, below: CGFloat, above: CGFloat) {
        let size = content.menuSize
        let visible = (parentWindow?.screen ?? NSScreen.main)?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        // Below the anchor when it fits, else above it, else whichever side has more room.
        growsUp = below - size.height < visible.minY + CaperMenuMetrics.edgeMargin
            && (above + size.height <= visible.maxY - CaperMenuMetrics.edgeMargin || visible.maxY - above > below - visible.minY)
        edgeY = growsUp ? above : below
        panel.setFrameOrigin(NSPoint(x: x, y: 0))
        place(animated: false)
        // The menu never takes key status: the window under it stays key, so
        // its first responder, hover state and any popover it sits in stay.
        panel.orderFront(nil)
        install()
        // NSMenu announces itself this way; VoiceOver and XCUITest wait for it.
        NSAccessibility.post(element: content, notification: NSAccessibility.Notification(rawValue: "AXMenuOpened"))
        content.arrive(from: growsUp ? 4 : -4)
    }

    /// Keeps the menu on its edge as submenus expand, inside the screen.
    private func place(animated: Bool) {
        let size = content.menuSize
        let visible = (parentWindow?.screen ?? panel.screen ?? NSScreen.main)?.visibleFrame ?? .zero
        let margin = CaperMenuMetrics.edgeMargin
        var frame = NSRect(x: panel.frame.minX, y: growsUp ? edgeY : edgeY - size.height, width: size.width, height: size.height)
        if !visible.isEmpty {
            frame.origin.x = min(max(frame.minX, visible.minX + margin), visible.maxX - margin - frame.width)
            frame.origin.y = min(max(frame.minY, visible.minY + margin), visible.maxY - margin - frame.height)
        }
        panel.setFrame(frame, display: true, animate: animated)
        panel.invalidateShadow()
    }

    private func install() {
        // A click outside closes the menu and still reaches what it hit, as
        // web's light-dismiss popovers; a click on the opener only closes it.
        let click = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] event in
            guard let self, event.window !== self.panel else { return event }
            let onAnchor = self.anchor.map { anchor in
                event.window === anchor.window && anchor.bounds.contains(anchor.convert(event.locationInWindow, from: nil))
            } ?? false
            self.close()
            return onAnchor ? nil : event
        }
        let scroll = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel]) { [weak self] event in
            if let self, event.window !== self.panel { self.close() }
            return event
        }
        let keys = NSEvent.addLocalMonitorForEvents(matching: [.keyDown]) { [weak self] event in
            guard let self else { return event }
            if event.modifierFlags.contains(.command) { self.close(); return event }
            self.content.keyDown(with: event)
            return nil
        }
        monitors = [click, scroll, keys].compactMap { $0 }
        if let parentWindow {
            NotificationCenter.default.addObserver(self, selector: #selector(parentChanged), name: NSWindow.willCloseNotification, object: parentWindow)
        }
        NotificationCenter.default.addObserver(self, selector: #selector(parentChanged), name: NSApplication.didResignActiveNotification, object: nil)
    }

    @objc private func parentChanged() { close() }

    // MARK: Closing

    /// Closing is instant on every motion setting.
    public func close() {
        guard !closed else { return }
        closed = true
        monitors.forEach(NSEvent.removeMonitor)
        monitors.removeAll()
        NotificationCenter.default.removeObserver(self)
        NSAccessibility.post(element: content, notification: NSAccessibility.Notification(rawValue: "AXMenuClosed"))
        panel.orderOut(nil)
        NSCursor.arrow.set()
        if Self.current === self { Self.current = nil }
    }

    // MARK: Tests

    /// The menu's frame on screen.
    public var frame: NSRect { panel.frame }

    /// The visible rows' titles, separators omitted.
    public var titles: [String] { content.rows.map(\.item.title) }

    /// Acts on the visible row titled `title`, as a click would.
    @discardableResult public func perform(_ title: String) -> Bool {
        guard let row = content.rows.first(where: { $0.item.title == title && $0.item.isInteractive }) else { return false }
        content.press(row)
        return true
    }
}

private final class CaperMenuPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// The menu's body: its rows, highlight, keyboard and expansion.
private final class CaperMenuView: NSView {
    var activate: ((@escaping () -> Void) -> Void)?
    var cancel: (() -> Void)?
    var resized: (() -> Void)?
    private let items: [CaperMenuItem]
    private var expanded: Set<[Int]> = []
    private(set) var rows: [CaperMenuRow] = []
    private var highlighted: CaperMenuRow? {
        didSet {
            guard oldValue !== highlighted else { return }
            oldValue?.highlighted = false
            highlighted?.highlighted = true
            if let highlighted { NSAccessibility.post(element: highlighted, notification: .focusedUIElementChanged) }
        }
    }
    private var pressed: CaperMenuRow?
    private let width: CGFloat
    /// The rows' size. The panel sizes itself from this, never from the
    /// view's frame, which the panel resets when it takes the view.
    private(set) var menuSize: NSSize = .zero

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    init(items: [CaperMenuItem]) {
        self.items = items
        width = CaperMenuView.width(for: items)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = AppKitKit.surface.cgColor
        layer?.borderColor = AppKitKit.border.cgColor
        layer?.borderWidth = 1
        layer?.cornerRadius = CaperMenuMetrics.cornerRadius
        layer?.masksToBounds = true
        setAccessibilityElement(true)
        setAccessibilityRole(.menu)
        setAccessibilityIdentifier("caper-menu")
        rebuild()
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self, userInfo: nil))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Wide enough for every row, expanded or not, so expanding never reflows.
    private static func width(for items: [CaperMenuItem], depth: Int = 0) -> CGFloat {
        let checks = items.contains(where: \.isSelected) ? CaperMenuMetrics.checkWidth + CaperMenuMetrics.gap : 0
        var widest: CGFloat = 0
        for item in items {
            let title = (item.title as NSString).size(withAttributes: [.font: CaperMenuRow.font(for: item)]).width
            var row = 2 * CaperMenuMetrics.itemInset + checks + CGFloat(depth) * CaperMenuMetrics.childIndent + ceil(title)
            if let children = item.children {
                row += CaperMenuMetrics.gap + 12
                widest = max(widest, width(for: children, depth: depth + 1) - 2 * CaperMenuMetrics.padding)
            }
            widest = max(widest, row)
        }
        return min(max(widest, 196), 340) + 2 * CaperMenuMetrics.padding
    }

    private func rebuild() {
        let keep = highlighted.map(\.path)
        subviews.forEach { $0.removeFromSuperview() }
        rows.removeAll()
        var y = CaperMenuMetrics.padding
        func add(_ items: [CaperMenuItem], depth: Int, path: [Int]) {
            let checks = items.contains(where: \.isSelected)
            for (index, item) in items.enumerated() {
                let itemPath = path + [index]
                if item.isSeparator {
                    let line = NSView(frame: NSRect(x: CaperMenuMetrics.padding, y: y + 5, width: width - 2 * CaperMenuMetrics.padding, height: 1))
                    line.wantsLayer = true
                    line.layer?.backgroundColor = AppKitKit.border.cgColor
                    addSubview(line)
                    y += CaperMenuMetrics.separatorHeight
                    continue
                }
                let open = expanded.contains(itemPath)
                let row = CaperMenuRow(item: item, path: itemPath, depth: depth, checks: checks, expanded: open)
                row.frame = NSRect(x: CaperMenuMetrics.padding, y: y, width: width - 2 * CaperMenuMetrics.padding, height: row.height)
                addSubview(row)
                rows.append(row)
                y += row.height
                if open, let children = item.children { add(children, depth: depth + 1, path: itemPath) }
            }
        }
        add(items, depth: 0, path: [])
        y += CaperMenuMetrics.padding
        menuSize = NSSize(width: width, height: y)
        setFrameSize(menuSize)
        setAccessibilityChildren(rows)
        highlighted = keep.flatMap { path in rows.first { $0.path == path } }
    }

    /// Entrances rise or drop 4 points into place and fade in (--motion-fast).
    func arrive(from offset: CGFloat) {
        guard !CaperAccessibility.reduceMotion, let layer else { return }
        let fade = CABasicAnimation(keyPath: "opacity"); fade.fromValue = 0; fade.toValue = 1
        let slide = CABasicAnimation(keyPath: "transform.translation.y"); slide.fromValue = offset; slide.toValue = 0
        let group = CAAnimationGroup()
        group.animations = [fade, slide]
        group.duration = 0.14
        group.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
        layer.add(group, forKey: "arrive")
    }

    private func rowAt(_ event: NSEvent) -> CaperMenuRow? {
        let point = convert(event.locationInWindow, from: nil)
        return rows.first { $0.frame.contains(point) }
    }

    func press(_ row: CaperMenuRow) {
        guard row.item.isInteractive else { return }
        switch row.item.kind {
        case let .action(perform): activate?(perform)
        case .submenu: toggle(row)
        case .note, .separator: break
        }
    }

    private func toggle(_ row: CaperMenuRow, open: Bool? = nil) {
        let isOpen = expanded.contains(row.path)
        guard open != isOpen else { return }
        if isOpen { expanded = expanded.filter { !$0.starts(with: row.path) } } else { expanded.insert(row.path) }
        highlighted = row
        rebuild()
        resized?()
    }

    /// The menu window is never key, so it sets the cursor itself: the hand
    /// over every enabled row, as web's menu buttons.
    override func mouseMoved(with event: NSEvent) {
        let row = rowAt(event)
        highlighted = row?.item.isInteractive == true ? row : nil
        (highlighted == nil ? NSCursor.arrow : NSCursor.pointingHand).set()
    }
    override func mouseExited(with event: NSEvent) { highlighted = nil; NSCursor.arrow.set() }
    override func mouseDown(with event: NSEvent) { pressed = rowAt(event) }
    override func mouseUp(with event: NSEvent) {
        defer { pressed = nil }
        guard let row = rowAt(event), row === pressed else { return }
        press(row)
    }
    override func rightMouseDown(with event: NSEvent) { pressed = rowAt(event) }
    override func rightMouseUp(with event: NSEvent) { mouseUp(with: event) }

    override func keyDown(with event: NSEvent) {
        let interactive = rows.filter(\.item.isInteractive)
        let index = highlighted.flatMap { current in interactive.firstIndex { $0 === current } }
        switch event.keyCode {
        case 125, 48 where !event.modifierFlags.contains(.shift): // down, tab
            guard !interactive.isEmpty else { return }
            highlighted = interactive[index.map { ($0 + 1) % interactive.count } ?? 0]
        case 126, 48: // up, shift-tab
            guard !interactive.isEmpty else { return }
            highlighted = interactive[index.map { ($0 + interactive.count - 1) % interactive.count } ?? interactive.count - 1]
        case 36, 76, 49: // return, enter, space
            if let highlighted { press(highlighted) }
        case 124: // right expands
            if let highlighted, highlighted.item.children != nil { toggle(highlighted, open: true) }
        case 123: // left collapses, or returns to the parent row
            guard let highlighted else { return }
            if highlighted.item.children != nil, expanded.contains(highlighted.path) { toggle(highlighted, open: false) }
            else if let parent = rows.first(where: { $0.path == Array(highlighted.path.dropLast()) }) { toggle(parent, open: false) }
        case 53: cancel?() // escape
        default:
            // Type to jump to the next row starting with that letter.
            guard let letter = event.charactersIgnoringModifiers?.lowercased().first, letter.isLetter || letter.isNumber,
                  !interactive.isEmpty else { return }
            let start = index.map { $0 + 1 } ?? 0
            let order = interactive[start...] + interactive[..<start]
            if let match = order.first(where: { $0.item.title.lowercased().first == letter }) { highlighted = match }
        }
    }
}

/// One row: an optional check, the title, and a chevron for a submenu.
private final class CaperMenuRow: NSView {
    let item: CaperMenuItem
    let path: [Int]
    private let depth: Int
    private let checks: Bool
    private let expanded: Bool
    var highlighted = false { didSet { if highlighted != oldValue { needsDisplay = true } } }

    var height: CGFloat { if case .note = item.kind { 26 } else { 32 } }
    override var isFlipped: Bool { true }

    static func font(for item: CaperMenuItem) -> NSFont {
        if case .note = item.kind { return AppKitKit.font(12) }
        return AppKitKit.font(13, weight: .medium)
    }

    init(item: CaperMenuItem, path: [Int], depth: Int, checks: Bool, expanded: Bool) {
        self.item = item; self.path = path; self.depth = depth; self.checks = checks; self.expanded = expanded
        super.init(frame: .zero)
        setAccessibilityElement(true)
        setAccessibilityRole(.menuItem)
        setAccessibilityLabel(item.title)
        setAccessibilityIdentifier(item.identifier ?? item.title)
        setAccessibilityEnabled(item.isInteractive)
        if item.children != nil { setAccessibilityExpanded(expanded) }
        if item.isSelected { setAccessibilityValue("Selected") }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func accessibilityPerformPress() -> Bool {
        guard let menu = superview as? CaperMenuView, item.isInteractive else { return false }
        menu.press(self)
        return true
    }

    override func draw(_ dirtyRect: NSRect) {
        if highlighted, item.isInteractive {
            NSColor.white.withAlphaComponent(0.06).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: CaperMenuMetrics.itemRadius, yRadius: CaperMenuMetrics.itemRadius).fill()
        }
        let color: NSColor
        switch item.kind {
        case .note: color = AppKitKit.muted
        default: color = item.isDestructive ? AppKitKit.error : AppKitKit.text
        }
        let alpha: CGFloat = item.isEnabled || isNote ? 1 : 0.45
        var x = CaperMenuMetrics.itemInset + CGFloat(depth) * CaperMenuMetrics.childIndent
        if checks {
            if item.isSelected { symbol("checkmark", at: x, tint: AppKitKit.terracottaBright.withAlphaComponent(alpha)) }
            x += CaperMenuMetrics.checkWidth + CaperMenuMetrics.gap
        }
        let chevron: CGFloat = item.children == nil ? 0 : 12 + CaperMenuMetrics.gap
        let font = Self.font(for: item)
        let title = NSAttributedString(string: item.title, attributes: [
            .font: font, .foregroundColor: color.withAlphaComponent(alpha),
            .paragraphStyle: { let style = NSMutableParagraphStyle(); style.lineBreakMode = .byTruncatingTail; return style }(),
        ])
        let lineHeight = ceil(font.ascender - font.descender)
        title.draw(with: NSRect(x: x, y: (bounds.height - lineHeight) / 2, width: max(0, bounds.width - x - CaperMenuMetrics.itemInset - chevron),
                                height: lineHeight), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        if item.children != nil {
            symbol(expanded ? "chevron.down" : "chevron.right", at: bounds.width - CaperMenuMetrics.itemInset - 12,
                   tint: AppKitKit.muted.withAlphaComponent(alpha), size: 10)
        }
    }

    private var isNote: Bool { if case .note = item.kind { true } else { false } }

    private func symbol(_ name: String, at x: CGFloat, tint: NSColor, size: CGFloat = 11) {
        let configuration = NSImage.SymbolConfiguration(pointSize: size, weight: .bold).applying(.init(paletteColors: [tint]))
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?.withSymbolConfiguration(configuration) else { return }
        let width = name == "checkmark" ? CaperMenuMetrics.checkWidth : 12
        let rect = NSRect(x: x + (width - image.size.width) / 2, y: (bounds.height - image.size.height) / 2,
                          width: image.size.width, height: image.size.height)
        image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }
}
#endif
