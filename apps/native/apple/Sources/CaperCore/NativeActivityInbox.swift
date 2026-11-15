import SwiftUI

/// Polling belongs to the open inbox, not the account or any of its reply
/// editors. AppModel retains pagination, optimistic reads and independent drafts.
@MainActor private func pollActivity(_ model: AppModel) -> Task<Void, Never> {
    Task {
        await model.loadActivity(reset: true)
        while !Task.isCancelled {
            do { try await Task.sleep(for: .seconds(15)) } catch { return }
            if !model.activityLoading { await model.loadActivity(reset: true, poll: true) }
        }
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitActivityInbox: NSView, AppKitDialogContent {
    private let model: AppModel
    private let close: () -> Void
    private let observation = AppKitObservation()
    private var polling: Task<Void, Never>?
    private var picker: NSPopover?
    private let filters = NSStackView()
    private var filterButtons: [ActivityFilter: AppKitKitButton] = [:]
    private let unread = AppKitKitSwitch()
    private let unreadLabel = AppKitKit.label("", size: 13)
    private let error = AppKitKit.label("", size: 13, color: AppKitKit.error)
    private var retry: AppKitKitButton!
    private let rows = AppKitKit.stack([], spacing: 12)
    private var cards: [String: AppKitActivityRow] = [:]
    private let status = AppKitKit.label("", size: 13, color: AppKitKit.muted)
    private var more: AppKitKitButton!
    var dismissDisabled: Bool { false }

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; self.close = close
        super.init(frame: .zero)
        wantsLayer = true; layer?.backgroundColor = AppKitKit.surface.cgColor
        setAccessibilityElement(true); setAccessibilityRole(.group)
        setAccessibilityIdentifier("activity-inbox-dialog")
        let done = AppKitKitButton("Done", action: close)
        done.setAccessibilityIdentifier("activity-done"); done.keyEquivalent = "\u{1b}"
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let header = NSStackView(views: [AppKitKit.label("Notifications", size: 20, weight: .bold), spacer, done])
        header.orientation = .horizontal
        filters.orientation = .horizontal; filters.spacing = 6; filters.translatesAutoresizingMaskIntoConstraints = false
        for filter in ActivityFilter.allCases {
            let button = AppKitKitButton(filter.label) { [model] in
                model.activityFilter = filter
                Task { await model.loadActivity(reset: true) }
            }
            button.setAccessibilityIdentifier("activity-filter-\(filter.rawValue)")
            filters.addArrangedSubview(button); filterButtons[filter] = button
        }
        let filterScroll = NSScrollView(); filterScroll.drawsBackground = false
        filterScroll.documentView = filters
        filterScroll.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            filterScroll.heightAnchor.constraint(equalToConstant: 38),
            filters.leadingAnchor.constraint(equalTo: filterScroll.contentView.leadingAnchor),
            filters.topAnchor.constraint(equalTo: filterScroll.contentView.topAnchor),
            filters.heightAnchor.constraint(equalToConstant: 38),
        ])
        unread.target = self; unread.action = #selector(toggleUnread)
        unread.setAccessibilityLabel("Unread only"); unread.setAccessibilityIdentifier("activity-unread-only")
        let toggle = NSStackView(views: [unread, unreadLabel]); toggle.orientation = .horizontal; toggle.spacing = 8
        retry = AppKitKitButton("Retry") { [model] in
            model.activityError = nil
            Task { await model.loadActivity(reset: true, poll: true) }
        }
        let failures = NSStackView(views: [error, retry]); failures.orientation = .horizontal; failures.spacing = 12
        let controls = AppKitKit.stack([header, filterScroll, toggle, failures], spacing: 12)
        for view in [header, filterScroll, failures] as [NSView] { AppKitKit.fillWidth(view, of: controls) }
        more = AppKitKitButton("Load more") { [model] in Task { await model.loadActivity(reset: false) } }
        let content = AppKitKit.stack([status, rows, more], spacing: 12)
        AppKitKit.fillWidth(status, of: content); AppKitKit.fillWidth(rows, of: content)
        let scroll = appKitKitScroll(content, insets: NSEdgeInsets(top: 0, left: 20, bottom: 20, right: 20))
        addSubview(controls); addSubview(scroll)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 640).withPriority(.defaultHigh),
            controls.topAnchor.constraint(equalTo: topAnchor, constant: 20),
            controls.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            controls.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -20),
            scroll.topAnchor.constraint(equalTo: controls.bottomAnchor, constant: 12),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor), scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { polling?.cancel() }
    @objc private func toggleUnread() {
        model.activityUnreadOnly = unread.isOn
        Task { await model.loadActivity(reset: true) }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, polling == nil {
            polling = pollActivity(model)
            observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
        } else if window == nil {
            observation.stop(); polling?.cancel(); polling = nil
            picker?.close(); picker = nil
        }
    }

    private func render() {
        for (filter, button) in filterButtons {
            let selected = model.activityFilter == filter
            button.state = selected ? .on : .off
            button.showsSelection = selected
            button.setAccessibilityValue(selected ? "Selected" : "Not selected")
        }
        unread.isOn = model.activityUnreadOnly
        unreadLabel.stringValue = "Unread only (\(model.activityUnread))"
        error.stringValue = model.activityError ?? ""; error.isHidden = model.activityError == nil
        retry.isHidden = model.activityError == nil; retry.isEnabled = !model.activityLoading
        let items = model.activityVisibleItems
        status.stringValue = items.isEmpty ? model.activityLoading ? "Loading notifications…"
            : model.activityError == nil ? "No notifications match these filters." : "" : ""
        status.isHidden = status.stringValue.isEmpty
        let ids = Set(items.map(\.id))
        for id in cards.keys.filter({ !ids.contains($0) }) { cards.removeValue(forKey: id)?.removeFromSuperview() }
        for (index, item) in items.enumerated() {
            let card: AppKitActivityRow
            if let previous = cards[item.id] { card = previous }
            else {
                card = AppKitActivityRow(model: model, item: item, close: close,
                    react: { [weak self] item, anchor in self?.react(item, from: anchor) })
                cards[item.id] = card; rows.insertArrangedSubview(card, at: index); AppKitKit.fillWidth(card, of: rows)
            }
            if rows.arrangedSubviews[index] !== card { rows.removeArrangedSubview(card); rows.insertArrangedSubview(card, at: index) }
            card.update(item)
        }
        rows.alphaValue = model.activityStale ? 0.5 : 1
        more.isHidden = model.activityCursor == nil; more.isEnabled = !model.activityLoading
    }

    private func react(_ item: ActivityItem, from anchor: NSView) {
        guard !model.activityStale else { return }
        picker?.close()
        let popover = NSPopover(); popover.behavior = .transient
        let account = model.account?.id
        let close = { [weak popover] in popover?.animates = false; popover?.close() }
        popover.contentViewController = AppKitReactionPicker(close: close) { [model, weak popover] emoji in
            popover?.animates = false; popover?.close()
            Task { if model.account?.id == account { await model.reactToActivity(item, emoji: emoji) } }
        }
        popover.animates = !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        picker = popover; popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .maxY)
    }
}

@MainActor private final class AppKitActivityRow: NSStackView {
    private let model: AppModel
    private var item: ActivityItem
    private var open: AppKitActivityOpenButton!
    private var read: AppKitKitButton!
    private var reply: AppKitKitButton!
    private var reactButton: AppKitKitButton!
    private let time = AppKitKit.label("", size: 11, color: AppKitKit.muted)
    private let replyBody = AppKitKit.stack([], spacing: 8)
    private var replyHost: NSHostingView<AnyView>?
    private var replyStale: Bool?

    init(model: AppModel, item: ActivityItem, close: @escaping () -> Void,
         react: @escaping (ActivityItem, NSView) -> Void) {
        self.model = model; self.item = item
        super.init(frame: .zero)
        orientation = .vertical; alignment = .leading; spacing = 6
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityElement(true); setAccessibilityRole(.group); setAccessibilityIdentifier("activity-row-\(item.id)")
        open = AppKitActivityOpenButton { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { if await model.openActivity(self.item) { close() } }
        }
        open.setAccessibilityIdentifier("activity-open-\(item.id)")
        read = AppKitKitButton("✓", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { _ = await model.readActivity(self.item) }
        }
        read.setAccessibilityIdentifier("activity-read-\(item.id)")
        reply = AppKitKitButton("Reply", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            model.activityExpanded[self.item.id] = model.activityExpanded[self.item.id] == nil ? self.item : nil
        }
        reply.setAccessibilityIdentifier("activity-reply-\(item.id)")
        reactButton = AppKitKitButton("React", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            react(self.item, self.reactButton)
        }
        reactButton.setAccessibilityIdentifier("activity-react-\(item.id)")
        let actions = NSStackView(views: [reply, reactButton, time, read]); actions.orientation = .horizontal; actions.spacing = 12
        for view in [open, actions, replyBody, AppKitKit.divider()] as [NSView] { addArrangedSubview(view) }
        for view in [open, replyBody] as [NSView] { AppKitKit.fillWidth(view, of: self) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(_ item: ActivityItem) {
        self.item = item
        open.update(item)
        open.isEnabled = !model.activityStale && model.activityOpening != item.id
        read.isHidden = !item.unread; read.isEnabled = !model.activityStale
        read.setAccessibilityLabel("Mark notification from \(item.actorName) read")
        time.stringValue = FeedbackInbox.relativeTime(item.createdAt)
        let canReply = model.canReplyToActivity(item)
        reply.isHidden = !canReply; reactButton.isHidden = !canReply
        reply.isEnabled = !model.activityStale; reactButton.isEnabled = !model.activityStale
        let expanded = model.activityExpanded[item.id] != nil
        reply.title = expanded ? "Close reply" : "Reply"
        // The composer observes the model itself. Rebuild its root only when
        // the inbox goes stale or fresh: replacing it on every render took
        // keyboard focus away from the reply field as soon as it was clicked.
        if expanded, replyHost == nil || replyStale != model.activityStale {
            replyStale = model.activityStale
            let content = AnyView(NativeQuickComposerSheet(model: model, activity: item)
                .modifier(Entrance.anchored)
                .disabled(model.activityStale).preferredColorScheme(.dark).tint(CaperTheme.terracottaBright))
            if let replyHost { replyHost.rootView = content }
            else {
                let host = NSHostingView(rootView: content); host.sizingOptions = [.intrinsicContentSize]
                let label = AppKitKit.label(item.replyLabel, size: 13, color: AppKitKit.muted)
                replyBody.addArrangedSubview(label); replyBody.addArrangedSubview(host)
                AppKitKit.fillWidth(label, of: replyBody); AppKitKit.fillWidth(host, of: replyBody)
                replyHost = host
            }
        } else if !expanded, replyHost != nil {
            replyBody.arrangedSubviews.forEach { $0.removeFromSuperview() }; replyHost = nil; replyStale = nil
        }
        replyBody.isHidden = !expanded
    }
}

/// Width-constrained labels measure the row; NSButtonCell's single-line
/// intrinsic size cannot measure a notification at a narrow window width.
@MainActor private final class AppKitActivityOpenButton: NSButton {
    private let actor = AppKitKit.label("", size: 13)
    private let summary = AppKitKit.label("", size: 13, color: AppKitKit.muted)
    private let preview = AppKitKit.label("", size: 13)
    private let location = AppKitKit.label("", size: 13, color: AppKitKit.muted)
    private let handler: () -> Void

    init(action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        title = ""; isBordered = false; focusRingType = .default
        target = self; self.action = #selector(open)
        preview.maximumNumberOfLines = 3; preview.lineBreakMode = .byTruncatingTail
        let content = AppKitKit.stack([actor, summary, preview, location], spacing: 5)
        [actor, summary, preview, location].forEach { AppKitKit.fillWidth($0, of: content) }
        addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: topAnchor, constant: 4), content.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            content.leadingAnchor.constraint(equalTo: leadingAnchor), content.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric) }
    override func hitTest(_ point: NSPoint) -> NSView? { super.hitTest(point) == nil ? nil : self }
    // An untitled, imageless NSButton is otherwise ignored by accessibility,
    // so VoiceOver and UI tests could not reach a row's Open target.
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityChildren() -> [Any]? { [] }
    // The labels inside are not children, so hit tests must stop here too.
    override func accessibilityHitTest(_ point: NSPoint) -> Any? { self }
    override func accessibilityFrame() -> NSRect {
        guard window != nil else { return super.accessibilityFrame() }
        return NSAccessibility.screenRect(fromView: self, rect: bounds)
    }
    override var isEnabled: Bool { didSet { alphaValue = isEnabled ? 1 : 0.5 } }
    @objc private func open() { handler() }
    func update(_ item: ActivityItem) {
        actor.stringValue = "\(item.unread ? "● " : "")\(item.actorName)"
        actor.font = AppKitKit.font(13, weight: item.unread ? .bold : .medium)
        summary.stringValue = item.description ?? ""; summary.isHidden = item.description == nil
        preview.stringValue = item.body; location.stringValue = item.location
        location.isHidden = item.location.isEmpty
        setAccessibilityLabel([actor.stringValue, item.description, item.body, item.location].compactMap { $0 }.joined(separator: "\n"))
    }
}
#elseif os(iOS)
import UIKit
import Observation

@MainActor final class UIKitActivityInbox: UIViewController, UIAdaptivePresentationControllerDelegate {
    private let model: AppModel
    private let filters = UIStackView()
    private var filterButtons: [ActivityFilter: UIButton] = [:]
    private let unread = UISwitch()
    private let unreadLabel = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private var retry: UIButton!
    private let rows = UIStackView()
    private var cards: [String: UIKitActivityRow] = [:]
    private let status = UIKitMessageDialog.label("")
    private var more: UIButton!
    private var polling: Task<Void, Never>?
    private var active = false

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { polling?.cancel() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(CaperTheme.surface); overrideUserInterfaceStyle = .dark
        view.accessibilityIdentifier = "activity-inbox-dialog"
        let done = UIKitMessageDialog.button("Done") { [weak self] in self?.finish() }
        done.accessibilityIdentifier = "activity-done"
        let title = UIKitMessageDialog.label("Notifications", size: 20, bold: true)
        title.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let header = UIStackView(arrangedSubviews: [title, done]); header.spacing = 12
        filters.spacing = 6
        for filter in ActivityFilter.allCases {
            let button = UIKitMessageDialog.button(filter.label) { [model] in
                model.activityFilter = filter
                Task { await model.loadActivity(reset: true) }
            }
            button.accessibilityIdentifier = "activity-filter-\(filter.rawValue)"
            button.configurationUpdateHandler = { button in
                var config = button.configuration
                config?.baseBackgroundColor = UIColor(button.isSelected ? CaperTheme.terracotta.opacity(0.12) : CaperTheme.raised)
                config?.baseForegroundColor = UIColor(button.isSelected ? CaperTheme.terracottaBright : CaperTheme.muted)
                config?.background.strokeColor = UIColor(button.isSelected ? CaperTheme.terracotta : CaperTheme.border)
                config?.background.strokeWidth = 1
                button.configuration = config
            }
            filters.addArrangedSubview(button); filterButtons[filter] = button
        }
        let filterScroll = UIScrollView(); filterScroll.showsHorizontalScrollIndicator = false
        filters.translatesAutoresizingMaskIntoConstraints = false; filterScroll.addSubview(filters)
        NSLayoutConstraint.activate([
            filterScroll.heightAnchor.constraint(equalToConstant: 44),
            filters.leadingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.leadingAnchor),
            filters.trailingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.trailingAnchor),
            filters.topAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.topAnchor),
            filters.bottomAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.bottomAnchor),
            filters.heightAnchor.constraint(equalTo: filterScroll.frameLayoutGuide.heightAnchor),
        ])
        unread.onTintColor = UIColor(CaperTheme.terracotta)
        unread.accessibilityLabel = "Unread only"; unread.accessibilityIdentifier = "activity-unread-only"
        unread.addTarget(self, action: #selector(toggleUnread), for: .valueChanged)
        let toggle = UIStackView(arrangedSubviews: [unread, unreadLabel, UIView()]); toggle.spacing = 8; toggle.alignment = .center
        retry = UIKitMessageDialog.button("Retry") { [model] in
            model.activityError = nil
            Task { await model.loadActivity(reset: true, poll: true) }
        }
        let failures = UIStackView(arrangedSubviews: [error, retry]); failures.spacing = 12
        let controls = UIStackView(arrangedSubviews: [header, filterScroll, toggle, failures]); controls.axis = .vertical; controls.spacing = 12
        more = UIKitMessageDialog.button("Load more") { [model] in Task { await model.loadActivity(reset: false) } }
        rows.axis = .vertical; rows.spacing = 12
        let content = UIStackView(arrangedSubviews: [status, rows, more]); content.axis = .vertical; content.spacing = 12
        let scroll = UIScrollView(); scroll.keyboardDismissMode = .interactive
        for child in [controls, scroll] as [UIView] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        content.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(content)
        NSLayoutConstraint.activate([
            controls.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            controls.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            controls.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            scroll.topAnchor.constraint(equalTo: controls.bottomAnchor, constant: 12),
            scroll.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            content.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
            content.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            content.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -20),
            content.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        presentationController?.delegate = self
        if polling == nil { active = true; model.activityShown = true; polling = pollActivity(model); observe() }
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || presentingViewController == nil { stop() }
    }
    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { stop() }
    private func stop() { active = false; polling?.cancel(); polling = nil; model.activityShown = false }
    private func finish() { stop(); dismiss(animated: true) }
    @objc private func toggleUnread() {
        model.activityUnreadOnly = unread.isOn
        Task { await model.loadActivity(reset: true) }
    }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    private func render() {
        guard model.account != nil else { finish(); return }
        for (filter, button) in filterButtons {
            let selected = model.activityFilter == filter
            button.isSelected = selected
            button.accessibilityTraits = selected ? [.button, .selected] : .button
        }
        unread.isOn = model.activityUnreadOnly; unreadLabel.text = "Unread only (\(model.activityUnread))"
        error.text = model.activityError; error.isHidden = model.activityError == nil
        retry.isHidden = model.activityError == nil; retry.isEnabled = !model.activityLoading
        let items = model.activityVisibleItems
        status.text = items.isEmpty ? model.activityLoading ? "Loading notifications…"
            : model.activityError == nil ? "No notifications match these filters." : "" : ""
        status.isHidden = status.text?.isEmpty != false
        let ids = Set(items.map(\.id))
        for id in cards.keys.filter({ !ids.contains($0) }) {
            guard let card = cards.removeValue(forKey: id) else { continue }
            card.removeReply(); card.removeFromSuperview()
        }
        for (index, item) in items.enumerated() {
            let card: UIKitActivityRow
            if let previous = cards[item.id] { card = previous }
            else {
                card = UIKitActivityRow(model: model, item: item, parent: self,
                    close: { [weak self] in self?.finish() }, react: { [weak self] item in self?.react(item) })
                cards[item.id] = card; rows.insertArrangedSubview(card, at: index)
            }
            if rows.arrangedSubviews[index] !== card { rows.removeArrangedSubview(card); rows.insertArrangedSubview(card, at: index) }
            card.update(item)
        }
        rows.alpha = model.activityStale ? 0.5 : 1; rows.isUserInteractionEnabled = !model.activityStale
        more.isHidden = model.activityCursor == nil; more.isEnabled = !model.activityLoading
    }
    private func react(_ item: ActivityItem) {
        guard !model.activityStale else { return }
        let account = model.account?.id
        present(UIKitHosting.reactionPicker { [model] emoji in
            Task { if model.account?.id == account { await model.reactToActivity(item, emoji: emoji) } }
        }, animated: true)
    }
}

@MainActor private final class UIKitActivityRow: UIStackView {
    private let model: AppModel
    private var item: ActivityItem
    private weak var parent: UIViewController?
    private var open: UIKitActivityOpenButton!
    private var read: UIButton!
    private var reply: UIButton!
    private var reactButton: UIButton!
    private let time = UIKitMessageDialog.label("", size: 11)
    private let replyBody = UIStackView()
    private var replyHost: UIHostingController<AnyView>?

    init(model: AppModel, item: ActivityItem, parent: UIViewController, close: @escaping () -> Void,
         react: @escaping (ActivityItem) -> Void) {
        self.model = model; self.item = item; self.parent = parent
        super.init(frame: .zero)
        axis = .vertical; spacing = 6; accessibilityIdentifier = "activity-row-\(item.id)"
        open = UIKitActivityOpenButton { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { if await model.openActivity(self.item) { close() } }
        }
        open.accessibilityIdentifier = "activity-open-\(item.id)"
        read = UIKitMessageDialog.button("✓") { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { _ = await model.readActivity(self.item) }
        }
        read.accessibilityIdentifier = "activity-read-\(item.id)"
        reply = UIKitMessageDialog.button("Reply") { [weak self] in
            guard let self, !self.model.activityStale else { return }
            model.activityExpanded[self.item.id] = model.activityExpanded[self.item.id] == nil ? self.item : nil
        }
        reply.accessibilityIdentifier = "activity-reply-\(item.id)"
        reactButton = UIKitMessageDialog.button("React") { [weak self] in
            guard let self, !self.model.activityStale else { return }; react(self.item)
        }
        reactButton.accessibilityIdentifier = "activity-react-\(item.id)"
        let actions = UIStackView(arrangedSubviews: [reply, reactButton, UIView(), time, read]); actions.spacing = 8; actions.alignment = .center
        replyBody.axis = .vertical; replyBody.spacing = 8
        let line = UIView(); line.backgroundColor = UIColor(CaperTheme.border); line.heightAnchor.constraint(equalToConstant: 1).isActive = true
        let parts: [UIView] = [open, actions, replyBody, line]
        parts.forEach { addArrangedSubview($0) }
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(_ item: ActivityItem) {
        self.item = item
        open.update(item)
        open.isEnabled = !model.activityStale && model.activityOpening != item.id
        read.isHidden = !item.unread; read.isEnabled = !model.activityStale
        read.accessibilityLabel = "Mark notification from \(item.actorName) read"
        time.text = FeedbackInbox.relativeTime(item.createdAt)
        let canReply = model.canReplyToActivity(item)
        reply.isHidden = !canReply; reactButton.isHidden = !canReply
        reply.isEnabled = !model.activityStale; reactButton.isEnabled = !model.activityStale
        let expanded = model.activityExpanded[item.id] != nil
        reply.configuration?.title = expanded ? "Close reply" : "Reply"
        if expanded {
            let content = AnyView(NativeQuickComposerSheet(model: model, activity: item)
                .modifier(Entrance.anchored)
                .disabled(model.activityStale).preferredColorScheme(.dark).tint(CaperTheme.terracottaBright))
            if let replyHost { replyHost.rootView = content }
            else if let parent {
                let host = UIHostingController(rootView: content); host.sizingOptions = [.intrinsicContentSize]
                host.view.backgroundColor = .clear; parent.addChild(host)
                replyBody.addArrangedSubview(UIKitMessageDialog.label(item.replyLabel))
                replyBody.addArrangedSubview(host.view); host.didMove(toParent: parent); replyHost = host
            }
        } else { removeReply() }
        replyBody.isHidden = !expanded
    }
    func removeReply() {
        guard let host = replyHost else { return }
        host.willMove(toParent: nil); replyBody.arrangedSubviews.forEach { $0.removeFromSuperview() }
        host.removeFromParent(); replyHost = nil
    }
}

@MainActor private final class UIKitActivityOpenButton: UIButton {
    private let actor = UIKitMessageDialog.label("", size: 13, bold: true)
    private let summary = UIKitMessageDialog.label("", size: 13)
    private let preview = UIKitMessageDialog.label("", size: 13)
    private let location = UIKitMessageDialog.label("", size: 13)

    init(action: @escaping () -> Void) {
        super.init(frame: .zero)
        summary.textColor = UIColor(CaperTheme.muted); location.textColor = UIColor(CaperTheme.muted)
        preview.numberOfLines = 3
        let content = UIStackView(arrangedSubviews: [actor, summary, preview, location])
        content.axis = .vertical; content.spacing = 5; content.isUserInteractionEnabled = false
        content.translatesAutoresizingMaskIntoConstraints = false; addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: topAnchor, constant: 4), content.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            content.leadingAnchor.constraint(equalTo: leadingAnchor), content.trailingAnchor.constraint(equalTo: trailingAnchor),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        isAccessibilityElement = true
        addAction(UIAction { _ in action() }, for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric) }
    override var isHighlighted: Bool { didSet { alpha = isHighlighted ? 0.7 : isEnabled ? 1 : 0.5 } }
    override var isEnabled: Bool { didSet { alpha = isEnabled ? 1 : 0.5 } }
    func update(_ item: ActivityItem) {
        actor.text = "\(item.unread ? "● " : "")\(item.actorName)"
        summary.text = item.description; summary.isHidden = item.description == nil
        preview.text = item.body; location.text = item.location; location.isHidden = item.location.isEmpty
        accessibilityLabel = [actor.text, item.description, item.body, item.location].compactMap { $0 }.joined(separator: "\n")
    }
}
#endif
