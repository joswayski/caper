import SwiftUI

/// Polling belongs to the open inbox, not the account or any of its reply
/// editors. AppModel retains pagination, optimistic reads and independent drafts.
@MainActor private func pollActivity(_ model: AppModel) -> Task<Void, Never> {
    Task {
        // Rows loaded ahead (or a load already on its way) show at once and
        // refresh in place, rather than dimming behind a reload.
        await model.loadActivity(reset: true, poll: model.activityLoading || model.activityLoaded || !model.activityItems.isEmpty)
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
            // Waiting on the retry, not caught up.
            model.activityError = nil; model.activityLoaded = false
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
        let groups = model.activityVisibleGroups
        status.stringValue = groups.isEmpty ? !model.activityLoaded ? "Loading notifications…"
            : model.activityError == nil ? "No notifications match these filters." : "" : ""
        status.isHidden = status.stringValue.isEmpty
        let ids = Set(groups.map(\.id))
        for id in cards.keys.filter({ !ids.contains($0) }) { cards.removeValue(forKey: id)?.removeFromSuperview() }
        for (index, group) in groups.enumerated() {
            let card: AppKitActivityRow
            if let previous = cards[group.id] { card = previous }
            else {
                card = AppKitActivityRow(model: model, group: group, close: close,
                    react: { [weak self] item, anchor in self?.react(item, from: anchor) })
                cards[group.id] = card; rows.insertArrangedSubview(card, at: index); AppKitKit.fillWidth(card, of: rows)
            }
            if rows.arrangedSubviews[index] !== card { rows.removeArrangedSubview(card); rows.insertArrangedSubview(card, at: index) }
            card.update(group)
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
        popover.animates = !CaperAccessibility.reduceMotion
        picker = popover; popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .maxY)
    }
}

@MainActor private final class AppKitActivityRow: NSStackView {
    private let model: AppModel
    private var group: ActivityGroup
    private var open: AppKitActivityOpenButton!
    private var read: AppKitKitButton!
    private var reply: AppKitKitButton!
    private var reactButton: AppKitKitButton!
    private let time = AppKitKit.label("", size: 11, color: AppKitKit.muted)
    private let replyBody = AppKitKit.stack([], spacing: 8)
    private var replyHost: AppKitQuickComposer?

    /// Identifiers stay keyed by the newest item: the inbox keys rows by it.
    init(model: AppModel, group: ActivityGroup, close: @escaping () -> Void,
         react: @escaping (ActivityItem, NSView) -> Void) {
        self.model = model; self.group = group
        super.init(frame: .zero)
        orientation = .vertical; alignment = .leading; spacing = 6
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityElement(true); setAccessibilityRole(.group); setAccessibilityIdentifier("activity-row-\(group.id)")
        open = AppKitActivityOpenButton { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { if await model.openActivity(self.group) { close() } }
        }
        open.setAccessibilityIdentifier("activity-open-\(group.id)")
        read = AppKitKitButton("✓", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { _ = await model.readActivity(self.group) }
        }
        read.setAccessibilityIdentifier("activity-read-\(group.id)")
        reply = AppKitKitButton("Reply", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            let target = model.activityReplyTarget(self.group)
            model.activityExpanded[target.id] = model.activityExpanded[target.id] == nil ? target : nil
        }
        reply.setAccessibilityIdentifier("activity-reply-\(group.id)")
        reactButton = AppKitKitButton("React", kind: .plain) { [weak self] in
            guard let self, !self.model.activityStale else { return }
            react(model.activityReplyTarget(self.group), self.reactButton)
        }
        reactButton.setAccessibilityIdentifier("activity-react-\(group.id)")
        let actions = NSStackView(views: [reply, reactButton, time, read]); actions.orientation = .horizontal; actions.spacing = 12
        for view in [open, actions, replyBody, AppKitKit.divider()] as [NSView] { addArrangedSubview(view) }
        for view in [open, replyBody] as [NSView] { AppKitKit.fillWidth(view, of: self) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(_ group: ActivityGroup) {
        self.group = group
        let item = group.newest, target = model.activityReplyTarget(group)
        open.update(group)
        open.isEnabled = !model.activityStale && model.activityOpening != group.id
        read.isHidden = !group.unread; read.isEnabled = !model.activityStale
        read.setAccessibilityLabel(group.readLabel)
        time.stringValue = FeedbackInbox.relativeTime(item.createdAt)
        let canReply = model.canReplyToActivity(target)
        reply.isHidden = !canReply; reactButton.isHidden = !canReply
        reply.isEnabled = !model.activityStale; reactButton.isEnabled = !model.activityStale
        let expanded = model.activityExpanded[target.id] != nil
        reply.title = expanded ? "Close reply" : "Reply"
        // Retain the editor even across stale/fresh changes; only its permission
        // changes, never its identity or first responder.
        if expanded {
            if replyHost == nil {
                let person = MentionCardPerson(id: target.actorId, username: target.actorName, displayName: nil, avatarId: target.avatarId, isSelf: false)
                let host = AppKitQuickComposer(state: NativeQuickDraft(model: model, person: person, activity: target, close: {}))
                let label = AppKitKit.label(target.replyLabel, size: 13, color: AppKitKit.muted)
                replyBody.addArrangedSubview(label); replyBody.addArrangedSubview(host)
                AppKitKit.fillWidth(label, of: replyBody); AppKitKit.fillWidth(host, of: replyBody)
                replyHost = host
            }
            replyHost?.state.enabled = !model.activityStale
        } else if !expanded, replyHost != nil {
            replyHost?.stop(); replyBody.arrangedSubviews.forEach { $0.removeFromSuperview() }; replyHost = nil
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
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
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
    func update(_ group: ActivityGroup) {
        let item = group.newest, description = group.summary
        actor.stringValue = "\(group.unread ? "● " : "")\(group.names)"
        actor.font = AppKitKit.font(13, weight: group.unread ? .bold : .medium)
        summary.stringValue = description ?? ""; summary.isHidden = description == nil
        preview.stringValue = item.body; location.stringValue = item.location
        location.isHidden = item.location.isEmpty
        setAccessibilityLabel([actor.stringValue, description, item.body, item.location].compactMap { $0 }.joined(separator: "\n"))
    }
}
#elseif os(iOS)
import UIKit
import Observation

/// The inbox's own small controls: filter chips, quiet row actions.
@MainActor private enum UIKitInboxStyle {
    nonisolated static func font(_ name: String, _ size: CGFloat) -> UIFont {
        UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: name, size: size) ?? .systemFont(ofSize: size))
    }
    /// A 44-point target drawing a 32-point chip: selected is terracotta, the rest neutral.
    static func chip(_ title: String, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.title = title; config.cornerStyle = .fixed
        config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)
        config.background.cornerRadius = 8; config.background.strokeWidth = 1
        config.background.backgroundInsets = NSDirectionalEdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming; attributes.font = font("Figtree-Medium", 13); return attributes
        }
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        button.configurationUpdateHandler = { button in
            var config = button.configuration
            config?.background.backgroundColor = button.isSelected ? UIColor(CaperTheme.terracotta).withAlphaComponent(0.14) : .clear
            config?.background.strokeColor = UIColor(button.isSelected ? CaperTheme.terracotta : CaperTheme.border)
            config?.baseForegroundColor = UIColor(button.isSelected ? CaperTheme.terracottaBright : CaperTheme.muted)
            button.configuration = config
        }
        return button
    }
    /// An icon and a short word, muted, with a full touch target.
    static func action(_ title: String, symbol: String, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.title = title; config.imagePadding = 5
        config.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold))
        config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0)
        config.baseForegroundColor = UIColor(CaperTheme.muted)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming; attributes.font = font("Figtree-Medium", 12); return attributes
        }
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        return button
    }
}

@MainActor final class UIKitActivityInbox: UIViewController, UIAdaptivePresentationControllerDelegate {
    private let model: AppModel
    private let filters = UIStackView()
    private var filterButtons: [ActivityFilter: UIButton] = [:]
    private let count = UIKitMessageDialog.label("", size: 12, bold: true)
    private let countBadge = UIView()
    private let unread = UISwitch()
    private let unreadLabel = UIKitMessageDialog.label("Unread only", size: 13)
    private let error = UIKitMessageDialog.label("", size: 13, error: true)
    private var retry: UIButton!
    private var failures: UIStackView!
    private let rows = UIStackView()
    private var cards: [String: UIKitActivityRow] = [:]
    private let empty = UIStackView()
    private let emptyTitle = UIKitMessageDialog.label("", size: 15, bold: true)
    private let emptyDetail = UIKitMessageDialog.label("", size: 13)
    private var more: UIButton!
    private var polling: Task<Void, Never>?
    private var active = false

    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { polling?.cancel() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(CaperTheme.surface); overrideUserInterfaceStyle = CaperTheme.interfaceStyle
        view.accessibilityIdentifier = "activity-inbox-dialog"

        // Title, its unread count, and Done.
        let title = UIKitMessageDialog.label("Notifications", size: 22, bold: true)
        title.numberOfLines = 1; title.adjustsFontSizeToFitWidth = true; title.minimumScaleFactor = 0.8
        count.textColor = UIColor(CaperTheme.text); count.textAlignment = .center; count.numberOfLines = 1
        countBadge.backgroundColor = UIColor(CaperTheme.terracotta); countBadge.layer.cornerRadius = 10
        count.translatesAutoresizingMaskIntoConstraints = false; countBadge.addSubview(count)
        NSLayoutConstraint.activate([
            count.leadingAnchor.constraint(equalTo: countBadge.leadingAnchor, constant: 7),
            count.trailingAnchor.constraint(equalTo: countBadge.trailingAnchor, constant: -7),
            count.topAnchor.constraint(equalTo: countBadge.topAnchor, constant: 2),
            count.bottomAnchor.constraint(equalTo: countBadge.bottomAnchor, constant: -2),
            countBadge.widthAnchor.constraint(greaterThanOrEqualTo: countBadge.heightAnchor),
        ])
        countBadge.isAccessibilityElement = true
        let heading = UIStackView(arrangedSubviews: [title, countBadge]); heading.spacing = 10; heading.alignment = .center
        var doneConfig = UIButton.Configuration.plain()
        doneConfig.title = "Done"; doneConfig.baseForegroundColor = UIColor(CaperTheme.text)
        doneConfig.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 0)
        doneConfig.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming; attributes.font = UIKitInboxStyle.font("Figtree-Bold", 15); return attributes
        }
        let done = UIButton(configuration: doneConfig, primaryAction: UIAction { [weak self] _ in self?.finish() })
        done.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        done.accessibilityIdentifier = "activity-done"
        done.setContentCompressionResistancePriority(.required, for: .horizontal)
        let header = UIStackView(arrangedSubviews: [heading, UIView(), done]); header.alignment = .center

        // Filters scroll sideways; Unread only sits on its own quiet line.
        filters.spacing = 8
        for filter in ActivityFilter.allCases {
            let button = UIKitInboxStyle.chip(filter.label) { [model] in
                guard model.activityFilter != filter else { return }
                model.activityFilter = filter
                Task { await model.loadActivity(reset: true) }
            }
            button.accessibilityIdentifier = "activity-filter-\(filter.rawValue)"
            filters.addArrangedSubview(button); filterButtons[filter] = button
        }
        let filterScroll = UIScrollView(); filterScroll.showsHorizontalScrollIndicator = false; filterScroll.alwaysBounceHorizontal = true
        filters.translatesAutoresizingMaskIntoConstraints = false; filterScroll.addSubview(filters)
        NSLayoutConstraint.activate([
            // Grows with the chips at large text sizes.
            filterScroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            filters.leadingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.leadingAnchor, constant: 16),
            filters.trailingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.trailingAnchor, constant: -16),
            filters.topAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.topAnchor),
            filters.bottomAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.bottomAnchor),
            filters.heightAnchor.constraint(equalTo: filterScroll.frameLayoutGuide.heightAnchor),
        ])
        unreadLabel.textColor = UIColor(CaperTheme.muted)
        unread.onTintColor = UIColor(CaperTheme.terracotta)
        unread.accessibilityLabel = "Unread only"; unread.accessibilityIdentifier = "activity-unread-only"
        unread.addTarget(self, action: #selector(toggleUnread), for: .valueChanged)
        let toggle = UIStackView(arrangedSubviews: [unreadLabel, UIView(), unread]); toggle.spacing = 8; toggle.alignment = .center
        unreadLabel.isAccessibilityElement = false
        retry = UIKitInboxStyle.action("Retry", symbol: "arrow.clockwise") { [model] in
            // Waiting on the retry, not caught up.
            model.activityError = nil; model.activityLoaded = false
            Task { await model.loadActivity(reset: true, poll: true) }
        }
        failures = UIStackView(arrangedSubviews: [error, retry]); failures.spacing = 12; failures.alignment = .center
        let controls = UIStackView(arrangedSubviews: [header, toggle, failures]); controls.axis = .vertical; controls.spacing = 4
        controls.setCustomSpacing(8, after: header)
        let divider = UIView(); divider.backgroundColor = UIColor(CaperTheme.border)

        // Empty: a calm note in place of the list.
        let bell = UIImageView(image: UIImage(systemName: "bell", withConfiguration: UIImage.SymbolConfiguration(pointSize: 26, weight: .regular)))
        bell.tintColor = UIColor(CaperTheme.muted); bell.contentMode = .scaleAspectFit
        emptyTitle.textAlignment = .center; emptyDetail.textAlignment = .center; emptyDetail.textColor = UIColor(CaperTheme.muted)
        [bell, emptyTitle, emptyDetail].forEach(empty.addArrangedSubview)
        empty.axis = .vertical; empty.alignment = .center; empty.spacing = 8; empty.setCustomSpacing(14, after: bell)
        empty.isLayoutMarginsRelativeArrangement = true
        empty.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 56, leading: 24, bottom: 24, trailing: 24)

        more = UIKitInboxStyle.action("Load more", symbol: "chevron.down") { [model] in Task { await model.loadActivity(reset: false) } }
        rows.axis = .vertical; rows.spacing = 0
        let content = UIStackView(arrangedSubviews: [empty, rows, more]); content.axis = .vertical; content.spacing = 8; content.alignment = .fill
        let scroll = UIScrollView(); scroll.keyboardDismissMode = .interactive; scroll.alwaysBounceVertical = true
        for child in [controls, filterScroll, divider, scroll] as [UIView] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        content.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(content)
        let safe = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            controls.topAnchor.constraint(equalTo: safe.topAnchor, constant: 12),
            controls.leadingAnchor.constraint(equalTo: safe.leadingAnchor, constant: 16),
            controls.trailingAnchor.constraint(equalTo: safe.trailingAnchor, constant: -16),
            filterScroll.topAnchor.constraint(equalTo: controls.bottomAnchor, constant: 4),
            filterScroll.leadingAnchor.constraint(equalTo: safe.leadingAnchor),
            filterScroll.trailingAnchor.constraint(equalTo: safe.trailingAnchor),
            divider.topAnchor.constraint(equalTo: filterScroll.bottomAnchor, constant: 8),
            divider.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1 / UIScreen.main.scale),
            scroll.topAnchor.constraint(equalTo: divider.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: safe.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: safe.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            content.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
            content.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            content.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -20),
            content.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
        ])
    }
    // Rendered before the sheet slides up, so it arrives showing its rows
    // and never its unconfigured controls.
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        if polling == nil { active = true; model.activityShown = true; polling = pollActivity(model); observe() }
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        presentationController?.delegate = self
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
        count.text = model.activityUnread > 99 ? "99+" : "\(model.activityUnread)"
        countBadge.isHidden = model.activityUnread == 0
        countBadge.accessibilityLabel = "\(model.activityUnread) unread"
        unread.isOn = model.activityUnreadOnly
        error.text = model.activityError; failures.isHidden = model.activityError == nil
        retry.isEnabled = !model.activityLoading
        let groups = model.activityVisibleGroups
        // A first load with nothing to show yet stays blank instead of spinning;
        // rows loaded ahead are already here. Background refreshes leave the
        // empty note alone: it waits only for the first answer.
        empty.isHidden = !groups.isEmpty || !model.activityLoaded || model.activityError != nil
        emptyTitle.text = model.activityUnreadOnly && model.activityFilter == .all ? "You’re all caught up" : "Nothing here"
        emptyDetail.text = "No notifications match these filters."
        let ids = Set(groups.map(\.id))
        for id in cards.keys.filter({ !ids.contains($0) }) {
            guard let card = cards.removeValue(forKey: id) else { continue }
            card.removeReply(); card.removeFromSuperview()
        }
        for (index, group) in groups.enumerated() {
            let card: UIKitActivityRow
            if let previous = cards[group.id] { card = previous }
            else {
                card = UIKitActivityRow(model: model, group: group, parent: self,
                    close: { [weak self] in self?.finish() }, react: { [weak self] item in self?.react(item) })
                cards[group.id] = card; rows.insertArrangedSubview(card, at: index)
            }
            if rows.arrangedSubviews[index] !== card { rows.removeArrangedSubview(card); rows.insertArrangedSubview(card, at: index) }
            card.update(group, last: index == groups.count - 1)
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

/// One notification (or a run of related ones): who, what, where and when,
/// then its quiet actions.
@MainActor private final class UIKitActivityRow: UIStackView {
    private let model: AppModel
    private var group: ActivityGroup
    private weak var parent: UIViewController?
    private var open: UIKitActivityOpenButton!
    private var read: UIButton!
    private var reply: UIButton!
    private var reactButton: UIButton!
    private var actions: UIStackView!
    private let replyBody = UIStackView()
    private var replyHost: UIKitQuickComposer?
    private let line = UIView()
    /// Avatar width plus its gap: actions and replies line up with the text.
    static let indent: CGFloat = 48

    /// Identifiers stay keyed by the newest item: the inbox keys rows by it.
    init(model: AppModel, group: ActivityGroup, parent: UIViewController, close: @escaping () -> Void,
         react: @escaping (ActivityItem) -> Void) {
        self.model = model; self.group = group; self.parent = parent
        super.init(frame: .zero)
        axis = .vertical; spacing = 0; accessibilityIdentifier = "activity-row-\(group.id)"
        open = UIKitActivityOpenButton { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { if await model.openActivity(self.group) { close() } }
        }
        open.accessibilityIdentifier = "activity-open-\(group.id)"
        read = UIKitInboxStyle.action("Mark read", symbol: "checkmark") { [weak self] in
            guard let self, !self.model.activityStale else { return }
            Task { _ = await model.readActivity(self.group) }
        }
        read.accessibilityIdentifier = "activity-read-\(group.id)"
        reply = UIKitInboxStyle.action("Reply", symbol: "arrowshape.turn.up.left") { [weak self] in
            guard let self, !self.model.activityStale else { return }
            let target = model.activityReplyTarget(self.group)
            model.activityExpanded[target.id] = model.activityExpanded[target.id] == nil ? target : nil
        }
        reply.accessibilityIdentifier = "activity-reply-\(group.id)"
        reactButton = UIKitInboxStyle.action("React", symbol: "face.smiling") { [weak self] in
            guard let self, !self.model.activityStale else { return }; react(model.activityReplyTarget(self.group))
        }
        reactButton.accessibilityIdentifier = "activity-react-\(group.id)"
        actions = UIStackView(arrangedSubviews: [reply, reactButton, UIView(), read]); actions.spacing = 20; actions.alignment = .center
        // At accessibility text sizes the actions stack so each keeps whole words.
        func fit(_ actions: UIStackView) {
            let stacked = actions.traitCollection.preferredContentSizeCategory.isAccessibilityCategory
            actions.axis = stacked ? .vertical : .horizontal; actions.alignment = stacked ? .leading : .center
            actions.spacing = stacked ? 0 : 20
        }
        fit(actions)
        actions.registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (actions: UIStackView, _) in fit(actions) }
        for stack in [actions!, replyBody] {
            stack.isLayoutMarginsRelativeArrangement = true
            stack.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Self.indent, bottom: 0, trailing: 0)
        }
        replyBody.axis = .vertical; replyBody.spacing = 8; replyBody.directionalLayoutMargins.bottom = 12
        line.backgroundColor = UIColor(CaperTheme.border)
        line.heightAnchor.constraint(equalToConstant: 1 / UIScreen.main.scale).isActive = true
        [open, actions, replyBody, line].forEach { addArrangedSubview($0) }
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(_ group: ActivityGroup, last: Bool) {
        self.group = group
        let item = model.activityReplyTarget(group)
        open.update(group)
        open.isEnabled = !model.activityStale && model.activityOpening != group.id
        read.isHidden = !group.unread; read.isEnabled = !model.activityStale
        read.accessibilityLabel = group.readLabel
        let canReply = model.canReplyToActivity(item)
        reply.isHidden = !canReply; reactButton.isHidden = !canReply
        reply.isEnabled = !model.activityStale; reactButton.isEnabled = !model.activityStale
        actions.isHidden = !canReply && !group.unread
        let expanded = model.activityExpanded[item.id] != nil
        reply.configuration?.title = expanded ? "Close reply" : "Reply"
        reply.configuration?.baseForegroundColor = UIColor(expanded ? CaperTheme.terracottaBright : CaperTheme.muted)
        if expanded {
            if replyHost == nil, let parent {
                let person = MentionCardPerson(id: item.actorId, username: item.actorName, displayName: nil, avatarId: item.avatarId, isSelf: false)
                let host = UIKitQuickComposer(state: NativeQuickDraft(model: model, person: person, activity: item, close: {}))
                host.view.backgroundColor = .clear; parent.addChild(host)
                let label = UIKitMessageDialog.label(item.replyLabel); label.textColor = UIColor(CaperTheme.muted)
                replyBody.addArrangedSubview(label)
                replyBody.addArrangedSubview(host.view); host.didMove(toParent: parent); replyHost = host
            }
            replyHost?.state.enabled = !model.activityStale
        } else { removeReply() }
        replyBody.isHidden = !expanded
        line.isHidden = last
    }
    func removeReply() {
        guard let host = replyHost else { return }
        host.stop(); host.willMove(toParent: nil); replyBody.arrangedSubviews.forEach { $0.removeFromSuperview() }
        host.removeFromParent(); replyHost = nil
    }
}

@MainActor private final class UIKitActivityOpenButton: UIButton {
    private let avatar = UIKitAvatar(side: 36)
    private let dot = UIView()
    private let actor = UIKitMessageDialog.label("", size: 14, bold: true)
    private let time = UIKitMessageDialog.label("", size: 12)
    private let summary = UIKitMessageDialog.label("", size: 13)
    private let preview = UIKitMessageDialog.label("", size: 14)
    private let location = UIKitMessageDialog.label("", size: 12)

    init(action: @escaping () -> Void) {
        super.init(frame: .zero)
        actor.numberOfLines = 1; actor.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        time.numberOfLines = 1; time.textColor = UIColor(CaperTheme.muted)
        time.setContentCompressionResistancePriority(.required, for: .horizontal)
        time.setContentHuggingPriority(.required, for: .horizontal)
        summary.textColor = UIColor(CaperTheme.muted); location.textColor = UIColor(CaperTheme.muted)
        preview.numberOfLines = 3; location.numberOfLines = 1
        dot.backgroundColor = UIColor(CaperTheme.terracottaBright); dot.layer.cornerRadius = 4
        dot.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([dot.widthAnchor.constraint(equalToConstant: 8), dot.heightAnchor.constraint(equalToConstant: 8)])
        let heading = UIStackView(arrangedSubviews: [actor, time, dot]); heading.spacing = 8; heading.alignment = .center
        let text = UIStackView(arrangedSubviews: [heading, summary, preview, location]); text.axis = .vertical; text.spacing = 3
        text.setCustomSpacing(6, after: preview)
        let row = UIStackView(arrangedSubviews: [avatar, text]); row.spacing = UIKitActivityRow.indent - 36; row.alignment = .top
        row.isUserInteractionEnabled = false; row.translatesAutoresizingMaskIntoConstraints = false; addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: topAnchor, constant: 14), row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            row.leadingAnchor.constraint(equalTo: leadingAnchor), row.trailingAnchor.constraint(equalTo: trailingAnchor),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        isAccessibilityElement = true
        addAction(UIAction { _ in action() }, for: .touchUpInside)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric) }
    override var isHighlighted: Bool { didSet { alpha = isHighlighted ? 0.7 : isEnabled ? 1 : 0.5 } }
    override var isEnabled: Bool { didSet { alpha = isEnabled ? 1 : 0.5 } }
    func update(_ group: ActivityGroup) {
        let item = group.newest, description = group.summary
        avatar.show(name: item.actorName, avatarID: item.avatarId)
        actor.text = group.names
        actor.textColor = UIColor(group.unread ? CaperTheme.text : CaperTheme.muted)
        dot.isHidden = !group.unread
        time.text = FeedbackInbox.relativeTime(item.createdAt)
        summary.text = description; summary.isHidden = description == nil
        preview.text = item.body; preview.isHidden = item.body.isEmpty
        preview.textColor = UIColor(group.unread ? CaperTheme.text : CaperTheme.muted)
        location.text = item.location; location.isHidden = item.location.isEmpty
        accessibilityLabel = [group.unread ? "Unread" : nil, group.names, description, item.body, item.location, time.text]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n")
    }
}
#endif
