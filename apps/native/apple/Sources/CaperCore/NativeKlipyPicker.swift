import Foundation
import Observation
import QuartzCore
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
import WebKit
#endif

/// Loads only a realized tile. Its layer owns playback, not a SwiftUI timeline.
@MainActor final class NativeKlipyImage {
    let layer = CALayer()
    private var task: Task<Void, Never>?
    private var sources: [URL] = []
    private var still: URL?
    private var maximum = 360
    private var limit = 4 * 1024 * 1024
    private var motion: NSObjectProtocol?
    private var stopped = true
    var failed: ((Bool) -> Void)?

    init(fill: Bool = false) {
        layer.contentsGravity = fill ? .resizeAspectFill : .resizeAspect
        layer.masksToBounds = fill
        #if os(macOS)
        motion = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil, queue: .main) { [weak self] _ in MainActor.assumeIsolated { if self?.stopped == false { self?.load() } } }
        #else
        motion = NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification,
            object: nil, queue: .main) { [weak self] _ in MainActor.assumeIsolated { if self?.stopped == false { self?.load() } } }
        #endif
    }
    isolated deinit {
        task?.cancel()
        #if os(macOS)
        motion.map(NSWorkspace.shared.notificationCenter.removeObserver)
        #else
        motion.map(NotificationCenter.default.removeObserver)
        #endif
    }
    func set(sources: [URL], still: URL?, maximum: Int = 360, limit: Int = 4 * 1024 * 1024) {
        guard stopped || sources != self.sources || still != self.still || maximum != self.maximum || limit != self.limit else { return }
        self.sources = sources; self.still = still; self.maximum = maximum; self.limit = limit; load()
    }
    func stop() { stopped = true; task?.cancel(); task = nil; layer.removeAllAnimations(); layer.contents = nil }
    private func load() {
        stop(); stopped = false
        let reduced = CaperAccessibility.reduceMotion
        let urls = (reduced ? [still].compactMap { $0 } : []) + sources
        let maximum = maximum, limit = limit
        task = Task { [weak self] in
            for url in urls {
                let original: AttachmentOriginal?
                if reduced {
                    original = (try? await AttachmentImageLoader.shared.load(key: "klipy-still:\(url.absoluteString)", url: url,
                        maxPixelSize: maximum)).map { AttachmentOriginal(images: [$0], durations: []) }
                } else {
                    original = try? await AttachmentImageLoader.shared.loadFrames(key: "klipy:\(url.absoluteString)", url: url,
                        maxPixelSize: maximum, animationByteLimit: limit)
                }
                guard !Task.isCancelled else { return }
                guard let original, let first = original.images.first else { continue }
                guard let self else { return }
                self.layer.contents = first; self.failed?(false)
                if !reduced, original.images.count > 1, original.durations.count == original.images.count {
                    let total = original.durations.reduce(0, +)
                    if total > 0 {
                        let animation = CAKeyframeAnimation(keyPath: "contents")
                        animation.values = original.images + [first]
                        var time = 0.0
                        animation.keyTimes = original.durations.map { duration in
                            defer { time += duration }; return NSNumber(value: time / total)
                        } + [1]
                        animation.duration = total; animation.calculationMode = .discrete; animation.repeatCount = .infinity
                        self.layer.add(animation, forKey: "frames")
                    }
                }
                return
            }
            if !Task.isCancelled { self?.failed?(true) }
        }
    }
}

#if os(macOS)
@MainActor final class AppKitKlipyPicker: NSViewController, NSTextFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
    private let model: KlipyPickerModel
    private let close: () -> Void
    private let select: (ChatGif, String) -> Void
    private let search = AppKitKitField(height: 36)
    private let table = NSTableView()
    private let scroll = NSScrollView()
    private let status = AppKitKit.label("")
    private let attribution = AppKitKit.label("Powered by KLIPY", size: 11, color: AppKitKit.muted, wraps: false)
    private var retry: AppKitKitButton!
    private var tabBar: AppKitKlipyTabBar!
    private var rows: [KlipyGridRow] = []
    private var results = -1
    private var active = false
    private var themeObserver: NSObjectProtocol?

    init(configuration: KlipyConfiguration, close: @escaping () -> Void, select: @escaping (ChatGif, String) -> Void) {
        model = KlipyPickerModel(client: KlipyClient(configuration: configuration)); self.close = close; self.select = select
        super.init(nibName: nil, bundle: nil); preferredContentSize = NSSize(width: 400, height: 480)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit {
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
    }
    override func loadView() {
        view = NSView(); view.wantsLayer = true; view.layer?.backgroundColor = AppKitKit.surface.cgColor
        view.setAccessibilityIdentifier("gif-picker")
        let cancel = AppKitKitButton("Cancel", action: close); cancel.keyEquivalent = "\u{1b}"
        let heading = NSStackView(views: [AppKitKit.label("GIFs and stickers", size: 14, weight: .bold, wraps: false), NSView(), cancel])
        heading.spacing = 8
        tabBar = AppKitKlipyTabBar(selected: model.kind) { [weak self] kind in
            self?.model.kind = kind
        }
        search.placeholderString = "Search KLIPY"; search.setAccessibilityLabel("Search KLIPY")
        search.setAccessibilityIdentifier("gif-picker-search"); search.delegate = self
        let column = NSTableColumn(identifier: .init("results")); table.addTableColumn(column)
        table.headerView = nil; table.dataSource = self; table.delegate = self
        table.rowHeight = 108; table.intercellSpacing = NSSize(width: 0, height: 8)
        table.backgroundColor = .clear; table.selectionHighlightStyle = .none
        scroll.documentView = table; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        scroll.setAccessibilityIdentifier("gif-picker-grid")
        // Only the tile under the pointer stays lit while the grid scrolls beneath it.
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(gridScrolled),
                                               name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        retry = AppKitKitButton("Try again") { [model] in model.retry() }; retry.setAccessibilityIdentifier("gif-picker-retry")
        attribution.alignment = .right
        attribution.setAccessibilityIdentifier("gif-picker-attribution")
        let stack = AppKitKit.stack([heading, tabBar, search, status, retry, scroll, attribution], spacing: 10)
        view.addSubview(stack)
        for row in stack.arrangedSubviews { AppKitKit.fillWidth(row, of: stack) }
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12), stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 12), stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -12),
        ])
        themeObserver = NotificationCenter.default.addObserver(
            forName: CaperThemeStore.didChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyTheme() }
        }
    }
    override func viewDidAppear() { super.viewDidAppear(); active = true; observe(); model.start(); view.window?.makeFirstResponder(search) }
    override func viewDidDisappear() { super.viewDidDisappear(); active = false; model.stop() }
    func controlTextDidChange(_ notification: Notification) { model.query = search.stringValue }
    private func applyTheme() {
        view.layer?.backgroundColor = AppKitKit.surface.cgColor
        attribution.textColor = AppKitKit.muted
        tabBar.applyTheme()
        table.enumerateAvailableRowViews { rowView, _ in
            guard let stack = rowView.view(atColumn: 0) as? NSStackView else { return }
            for view in stack.arrangedSubviews { (view as? AppKitKlipyTile)?.applyTheme() }
        }
    }
    @objc private func gridScrolled() { syncTileHover() }
    private func syncTileHover() {
        table.enumerateAvailableRowViews { rowView, _ in
            guard let stack = rowView.view(atColumn: 0) as? NSStackView else { return }
            for view in stack.arrangedSubviews { (view as? AppKitKlipyTile)?.syncHover() }
        }
    }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor [weak self] in self?.observe() } }
    }
    private func render() {
        tabBar.selectedKind = model.kind
        let next = KlipyGridRow.rows(model.items, columns: 3)
        status.stringValue = model.error ?? (model.loading ? "Loading…" : model.items.isEmpty ? "No results" : "")
        status.setAccessibilityIdentifier(model.error == nil && !model.loading && model.items.isEmpty ? "gif-picker-empty" : "gif-picker-status")
        status.isHidden = status.stringValue.isEmpty; retry.isHidden = model.error == nil
        if next != rows {
            rows = next; table.reloadData()
            DispatchQueue.main.async { [weak self] in self?.syncTileHover() }
        }
        if results != model.resultsID { results = model.resultsID; scroll.contentView.scroll(to: .zero) }
    }
    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }
    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let stack = NSStackView(); stack.distribution = .fillEqually; stack.spacing = 8
        if case .tiles(_, let items) = rows[row] {
            for item in items {
                let tile = AppKitKlipyTile(item: item) { [weak self] in
                    guard let self else { return }; self.select(item.gif, self.model.resultsQuery)
                }
                // A titleless borderless button has almost no intrinsic height; tiles fill the row.
                stack.addArrangedSubview(tile); tile.heightAnchor.constraint(equalTo: stack.heightAnchor).isActive = true
            }
            for _ in items.count..<3 { stack.addArrangedSubview(NSView()) }
        }
        if row == rows.count - 1 { Task { @MainActor [weak self] in self?.model.loadMore() } }
        return stack
    }
}

/// Caper segmented tabs for GIFs / Stickers with clear selected contrast,
/// pointer hover states, hand cursor and keyboard focus.
@MainActor private final class AppKitKlipyTabBar: NSStackView {
    private let gifs: AppKitKlipyTabButton
    private let stickers: AppKitKlipyTabButton
    var selectedKind: ChatGif.Kind {
        didSet {
            guard oldValue != selectedKind else { return }
            syncSelection()
        }
    }

    init(selected: ChatGif.Kind, onChange: @escaping (ChatGif.Kind) -> Void) {
        selectedKind = selected
        gifs = AppKitKlipyTabButton("GIFs") { onChange(.gif) }
        stickers = AppKitKlipyTabButton("Stickers") { onChange(.sticker) }
        super.init(frame: .zero)
        orientation = .horizontal; distribution = .fillEqually; spacing = 4
        edgeInsets = NSEdgeInsets(top: 3, left: 3, bottom: 3, right: 3)
        wantsLayer = true; layer?.cornerRadius = 10; layer?.borderWidth = 1
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(equalToConstant: 38).isActive = true
        setAccessibilityElement(true); setAccessibilityRole(.group)
        setAccessibilityLabel("Kind"); setAccessibilityIdentifier("gif-picker-kind")
        for button in [gifs, stickers] {
            addArrangedSubview(button)
            button.heightAnchor.constraint(equalTo: heightAnchor, constant: -6).isActive = true
        }
        applyTheme()
        syncSelection()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func applyTheme() {
        layer?.backgroundColor = AppKitKit.composer.cgColor
        layer?.borderColor = AppKitKit.border.cgColor
        gifs.refresh(); stickers.refresh()
    }

    private func syncSelection() {
        gifs.isSelectedTab = selectedKind == .gif
        stickers.isSelectedTab = selectedKind == .sticker
    }
}

@MainActor private final class AppKitKlipyTabButton: NSButton {
    private let onSelect: () -> Void
    private let labelText: String
    private let indicator = CALayer()
    private var hovered = false
    private var pressed = false
    private var tracking: NSTrackingArea?
    var isSelectedTab = false {
        didSet {
            state = isSelectedTab ? .on : .off
            setAccessibilityValue(isSelectedTab ? "Selected" : "Not selected")
            refresh()
        }
    }

    init(_ title: String, onSelect: @escaping () -> Void) {
        self.labelText = title; self.onSelect = onSelect
        super.init(frame: .zero)
        self.title = title; isBordered = false; bezelStyle = .regularSquare; focusRingType = .none
        wantsLayer = true; layer?.cornerRadius = 7; layer?.masksToBounds = true
        indicator.cornerRadius = 1
        layer?.addSublayer(indicator)
        target = self; action = #selector(clicked)
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityLabel(title)
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 48 { KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self); return }
        super.keyDown(with: event)
    }

    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    func refresh() {
        let isLight = CaperThemeStore.shared.active.isLight
        let fg: NSColor = (isSelectedTab || hovered) ? AppKitKit.text : AppKitKit.muted
        attributedTitle = NSAttributedString(string: labelText, attributes: [
            .font: AppKitKit.font(13, weight: isSelectedTab ? .bold : .medium),
            .foregroundColor: fg,
        ])
        let bg: NSColor
        if isSelectedTab {
            let base = isLight ? AppKitKit.surface : AppKitKit.raised
            bg = hovered ? (base.blended(withFraction: isLight ? 0.04 : 0.08, of: isLight ? .black : .white) ?? base) : base
        } else if pressed {
            bg = AppKitKit.raised
        } else if hovered {
            bg = AppKitKit.hoverWash
        } else {
            bg = .clear
        }
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = bg.cgColor
        if focused {
            layer?.borderColor = AppKitKit.terracotta.cgColor
            layer?.borderWidth = 2
        } else if isSelectedTab {
            layer?.borderColor = (isLight ? NSColor.black.withAlphaComponent(0.12) : NSColor.white.withAlphaComponent(0.16)).cgColor
            layer?.borderWidth = 1
        } else if hovered {
            layer?.borderColor = AppKitKit.border.cgColor
            layer?.borderWidth = 0.5
        } else {
            layer?.borderColor = NSColor.clear.cgColor
            layer?.borderWidth = 0
        }
        indicator.backgroundColor = AppKitKit.terracottaBright.cgColor
        indicator.isHidden = !isSelectedTab
        CATransaction.commit()
    }

    override func layout() {
        super.layout()
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let w = min(max(24, bounds.width * 0.34), 64)
        let y = isFlipped ? bounds.height - 2 : 0
        indicator.frame = NSRect(x: (bounds.width - w) / 2, y: y, width: w, height: 2)
        CATransaction.commit()
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
    @objc private func clicked() { onSelect() }
}

@MainActor private final class AppKitKlipyTile: NSButton {
    private let isSticker: Bool
    private let picture: NativeKlipyImage
    private let washLayer = CALayer()
    private let borderLayer = CALayer()
    private let pick: () -> Void
    private var hovered = false
    private var pressed = false
    private var tracking: NSTrackingArea?

    init(item: KlipyResult, pick: @escaping () -> Void) {
        self.isSticker = item.gif.kind == .sticker
        self.picture = NativeKlipyImage(fill: !isSticker)
        self.pick = pick
        super.init(frame: .zero)
        title = ""; isBordered = false; focusRingType = .none
        wantsLayer = true; layer?.cornerRadius = 8; layer?.masksToBounds = true
        washLayer.cornerRadius = 8; washLayer.zPosition = 1
        borderLayer.cornerRadius = 8; borderLayer.zPosition = 2
        layer?.addSublayer(picture.layer)
        layer?.addSublayer(washLayer)
        layer?.addSublayer(borderLayer)
        target = self; action = #selector(chosen)
        toolTip = item.title.isEmpty ? item.gif.accessibilityLabel : item.title
        setAccessibilityLabel(item.gif.accessibilityLabel); setAccessibilityHelp("Sends it now")
        setAccessibilityIdentifier("gif-picker-item-\(item.slug)")
        picture.set(sources: URL(string: item.tile.url).map { [$0] } ?? [], still: item.tile.still.flatMap(URL.init(string:)))
        picture.failed = { [weak self] failed in self?.title = failed ? "GIF unavailable" : "" }
        applyTheme()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 48 { KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self); return }
        super.keyDown(with: event)
    }

    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }

    func applyTheme() {
        let isLight = CaperThemeStore.shared.active.isLight
        let highlighted = (hovered || focused) && isEnabled
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer?.backgroundColor = (isSticker ? (highlighted ? AppKitKit.hoverWash : .clear) : AppKitKit.raised).cgColor
        if pressed && isEnabled {
            washLayer.backgroundColor = NSColor.black.withAlphaComponent(isLight ? 0.12 : 0.22).cgColor
        } else if highlighted && !isSticker {
            washLayer.backgroundColor = (isLight ? NSColor.black.withAlphaComponent(0.06) : NSColor.white.withAlphaComponent(0.08)).cgColor
        } else {
            washLayer.backgroundColor = NSColor.clear.cgColor
        }
        if highlighted {
            borderLayer.borderColor = AppKitKit.terracottaBright.cgColor
            borderLayer.borderWidth = 2
        } else if !isSticker {
            borderLayer.borderColor = AppKitKit.border.cgColor
            borderLayer.borderWidth = 1
        } else {
            borderLayer.borderColor = NSColor.clear.cgColor
            borderLayer.borderWidth = 0
        }
        CATransaction.commit()
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
    func syncHover() {
        guard let window, AppKitKit.windowActive(window), !isHiddenOrHasHiddenAncestor else { setHovered(false); return }
        setHovered(visibleRect.contains(convert(window.mouseLocationOutsideOfEventStream, from: nil)))
    }
    private func setHovered(_ value: Bool) {
        guard hovered != value else { return }
        hovered = value; applyTheme()
    }
    override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        pressed = true; applyTheme()
        super.mouseDown(with: event)
        pressed = false; applyTheme()
    }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
    override func becomeFirstResponder() -> Bool { defer { applyTheme() }; return super.becomeFirstResponder() }
    override func resignFirstResponder() -> Bool { defer { applyTheme() }; return super.resignFirstResponder() }
    override func layout() {
        super.layout()
        CATransaction.begin(); CATransaction.setDisableActions(true)
        picture.layer.frame = isSticker ? bounds.insetBy(dx: 4, dy: 4) : bounds
        washLayer.frame = bounds
        borderLayer.frame = bounds
        CATransaction.commit()
    }
    @objc private func chosen() { pick() }
}
#elseif os(iOS)
@MainActor final class UIKitKlipyPicker: UIViewController, UITableViewDataSource, UITableViewDelegate {
    private let model: KlipyPickerModel
    private let close: () -> Void
    private let select: (ChatGif, String) -> Void
    private let embedded: Bool
    private let search = UITextField()
    private let table = UITableView(frame: .zero, style: .plain)
    private let status = UIKitMessageDialog.label("")
    private var retry: UIButton!
    private var rows: [KlipyGridRow] = []
    private var results = -1
    private var active = false
    init(configuration: KlipyConfiguration, kind: ChatGif.Kind? = nil, close: @escaping () -> Void, select: @escaping (ChatGif, String) -> Void) {
        model = KlipyPickerModel(client: KlipyClient(configuration: configuration), kind: kind ?? .gif)
        embedded = kind != nil; self.close = close; self.select = select; super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func setKind(_ kind: ChatGif.Kind) { model.kind = kind }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = CaperTheme.interfaceStyle; view.backgroundColor = UIColor(CaperTheme.surface)
        view.accessibilityIdentifier = "gif-picker"
        let header = UIKitMessageDialog.header(UIKitMessageDialog.label("GIFs", size: 20, bold: true), UIKitMessageDialog.button("Cancel", action: close))
        header.isHidden = embedded
        let tabs = UISegmentedControl(items: ["GIFs", "Stickers"]); tabs.selectedSegmentIndex = 0
        tabs.isHidden = embedded; tabs.accessibilityIdentifier = "gif-picker-kind"
        tabs.addAction(UIAction { [weak self, weak tabs] _ in self?.model.kind = tabs?.selectedSegmentIndex == 1 ? .sticker : .gif }, for: .valueChanged)
        search.placeholder = "Search KLIPY"; search.accessibilityLabel = search.placeholder
        search.accessibilityIdentifier = "gif-picker-search"; search.borderStyle = .roundedRect
        search.autocorrectionType = .no; search.autocapitalizationType = .none
        search.addTarget(self, action: #selector(queryChanged), for: .editingChanged)
        search.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        retry = UIKitMessageDialog.button("Try again") { [model] in model.retry() }; retry.accessibilityIdentifier = "gif-picker-retry"
        table.backgroundColor = .clear; table.separatorStyle = .none; table.dataSource = self; table.delegate = self
        table.keyboardDismissMode = .interactive; table.accessibilityIdentifier = "gif-picker-grid"
        let attribution = UIKitMessageDialog.label("Powered by KLIPY", size: 11); attribution.textAlignment = .center
        attribution.accessibilityIdentifier = "gif-picker-attribution"
        let stack = UIStackView(arrangedSubviews: [header, tabs, search, status, retry, table, attribution])
        stack.axis = .vertical; stack.spacing = 10; stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12), stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12), stack.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -12),
        ])
    }
    override func viewDidLayoutSubviews() { super.viewDidLayoutSubviews(); model.gridWidth = table.bounds.width }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); active = true; observe(); model.start() }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); active = false; model.stop() }
    @objc private func queryChanged() { model.query = search.text ?? "" }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor [weak self] in self?.observe() } }
    }
    private func render() {
        let next = KlipyGridRow.rows(model.items, columns: 2)
        status.text = model.error ?? (model.loading ? "Loading…" : model.items.isEmpty ? "No results" : "")
        status.accessibilityIdentifier = model.error == nil && !model.loading && model.items.isEmpty ? "gif-picker-empty" : "gif-picker-status"
        status.isHidden = status.text?.isEmpty != false; retry.isHidden = model.error == nil
        if next != rows { rows = next; table.reloadData() }
        if results != model.resultsID { results = model.resultsID; table.setContentOffset(.zero, animated: false) }
    }
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows.count }
    func tableView(_ tableView: UITableView, heightForRowAt indexPath: IndexPath) -> CGFloat {
        if case .ad(_, let ad) = rows[indexPath.row] { return CGFloat(ad.height ?? 100) + 28 }
        return max(80, (tableView.bounds.width - 8) / 2) + 8
    }
    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(); cell.backgroundColor = .clear; cell.selectionStyle = .none
        let content: UIView
        switch rows[indexPath.row] {
        case .tiles(_, let items):
            let stack = UIStackView(); stack.spacing = 8; stack.distribution = .fillEqually
            for item in items { stack.addArrangedSubview(UIKitKlipyTile(item: item) { [weak self] in
                guard let self else { return }; self.select(item.gif, self.model.resultsQuery)
            }) }
            if items.count < 2 { stack.addArrangedSubview(UIView()) }
            content = stack
        case .ad(_, let ad):
            let stack = UIStackView(arrangedSubviews: [UIKitMessageDialog.label("Sponsored", size: 10, bold: true), NativeKlipyAd(html: ad.html)])
            stack.axis = .vertical; stack.spacing = 4; stack.accessibilityIdentifier = "gif-picker-ad"; content = stack
        }
        content.translatesAutoresizingMaskIntoConstraints = false; cell.contentView.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: cell.contentView.leadingAnchor), content.trailingAnchor.constraint(equalTo: cell.contentView.trailingAnchor),
            content.topAnchor.constraint(equalTo: cell.contentView.topAnchor), content.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -8),
        ])
        return cell
    }
    func tableView(_ tableView: UITableView, willDisplay cell: UITableViewCell, forRowAt indexPath: IndexPath) {
        if indexPath.row == rows.count - 1 { model.loadMore() }
    }
}

@MainActor private final class UIKitKlipyTile: UIButton {
    private let isSticker: Bool
    private let picture: NativeKlipyImage
    private let borderLayer = CALayer()
    private var pointerHovered = false

    init(item: KlipyResult, pick: @escaping () -> Void) {
        self.isSticker = item.gif.kind == .sticker
        self.picture = NativeKlipyImage(fill: !isSticker)
        super.init(frame: .zero)
        layer.cornerRadius = 8; layer.cornerCurve = .continuous; layer.masksToBounds = true
        borderLayer.cornerRadius = 8; borderLayer.cornerCurve = .continuous; borderLayer.zPosition = 2
        layer.addSublayer(picture.layer)
        layer.addSublayer(borderLayer)
        isPointerInteractionEnabled = true
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(handleHover(_:))))
        accessibilityLabel = item.gif.accessibilityLabel; accessibilityHint = "Sends it now"
        accessibilityIdentifier = "gif-picker-item-\(item.slug)"
        addAction(UIAction { _ in pick() }, for: .touchUpInside)
        picture.set(sources: URL(string: item.tile.url).map { [$0] } ?? [], still: item.tile.still.flatMap(URL.init(string:)))
        picture.failed = { [weak self] failed in self?.setTitle(failed ? "GIF unavailable" : nil, for: .normal) }
        applyStyle()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func handleHover(_ gesture: UIHoverGestureRecognizer) {
        let next = gesture.state == .began || gesture.state == .changed
        guard pointerHovered != next else { return }
        pointerHovered = next; applyStyle()
    }
    private func applyStyle() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        backgroundColor = isSticker ? (pointerHovered ? UIColor(CaperTheme.raised).withAlphaComponent(0.55) : .clear) : UIColor(CaperTheme.raised)
        if pointerHovered {
            borderLayer.borderColor = UIColor(CaperTheme.terracottaBright).cgColor
            borderLayer.borderWidth = 2
        } else if !isSticker {
            borderLayer.borderColor = UIColor(CaperTheme.border).cgColor
            borderLayer.borderWidth = 1
        } else {
            borderLayer.borderColor = UIColor.clear.cgColor
            borderLayer.borderWidth = 0
        }
        CATransaction.commit()
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        CATransaction.begin(); CATransaction.setDisableActions(true)
        picture.layer.frame = isSticker ? bounds.insetBy(dx: 4, dy: 4) : bounds
        borderLayer.frame = bounds
        CATransaction.commit()
    }
}

@MainActor final class UIKitExpressionPicker: UIViewController {
    private let configuration: KlipyConfiguration?
    private let mediaEnabled: Bool
    private let close: () -> Void
    private let select: (String) -> Void
    private let send: (ChatGif, String) -> Void
    private let tabs = UISegmentedControl(items: ["Emojis", "GIFs", "Stickers"])
    private let content = UIView()
    private var child: UIViewController?
    init(configuration: KlipyConfiguration?, mediaEnabled: Bool, close: @escaping () -> Void,
         select: @escaping (String) -> Void, send: @escaping (ChatGif, String) -> Void) {
        self.configuration = configuration; self.mediaEnabled = mediaEnabled; self.close = close; self.select = select; self.send = send
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = CaperTheme.interfaceStyle; view.backgroundColor = UIColor(CaperTheme.surface)
        let header = UIKitMessageDialog.header(UIKitMessageDialog.label("Emojis, GIFs & Stickers", size: 16, bold: true),
            UIKitMessageDialog.button("Cancel", action: close))
        tabs.selectedSegmentIndex = 0; tabs.accessibilityLabel = "Expression type"
        tabs.addTarget(self, action: #selector(changeTab), for: .valueChanged)
        let stack = UIStackView(arrangedSubviews: [header, tabs, content]); stack.axis = .vertical; stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12), stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12), stack.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        changeTab()
    }
    @objc private func changeTab() {
        child?.willMove(toParent: nil); child?.view.removeFromSuperview(); child?.removeFromParent()
        let next: UIViewController
        if tabs.selectedSegmentIndex == 0 {
            next = UIKitReactionPicker(title: "Insert emoji", embedded: true, close: close, select: select)
        } else if let configuration, mediaEnabled {
            next = UIKitKlipyPicker(configuration: configuration, kind: tabs.selectedSegmentIndex == 1 ? .gif : .sticker, close: close, select: send)
        } else {
            next = UIViewController()
            let label = UIKitMessageDialog.label(configuration == nil ? "KLIPY is not configured." : "Media cannot be sent right now.")
            label.textAlignment = .center; next.view = label
        }
        child = next; addChild(next); next.view.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(next.view)
        NSLayoutConstraint.activate([
            next.view.leadingAnchor.constraint(equalTo: content.leadingAnchor), next.view.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            next.view.topAnchor.constraint(equalTo: content.topAnchor), next.view.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])
        next.didMove(toParent: self)
    }
}

/// The provider's HTML still needs WebKit, with the same isolated data store and
/// tap-only external navigation policy. It does not require a SwiftUI host.
@MainActor private final class NativeKlipyAd: WKWebView {
    private let coordinator = KlipyAdWebView.Coordinator()
    init(html: String) {
        let configuration = WKWebViewConfiguration(); configuration.websiteDataStore = .nonPersistent()
        configuration.allowsInlineMediaPlayback = true; configuration.mediaTypesRequiringUserActionForPlayback = .all
        super.init(frame: .zero, configuration: configuration)
        navigationDelegate = coordinator; uiDelegate = coordinator
        isOpaque = false; backgroundColor = .clear; scrollView.backgroundColor = .clear
        scrollView.isScrollEnabled = false; scrollView.bounces = false
        let tap = UITapGestureRecognizer(target: coordinator, action: #selector(KlipyAdWebView.Coordinator.tapped))
        tap.cancelsTouchesInView = false; tap.delegate = coordinator; addGestureRecognizer(tap)
        loadHTMLString(KlipyAdWebView.document(html), baseURL: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}
#endif
