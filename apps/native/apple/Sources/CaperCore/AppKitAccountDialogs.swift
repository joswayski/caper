#if os(macOS)
import AppKit

/// One radio choice in a settings dialog, such as a DM privacy option: the
/// whole row selects, with a terracotta mark on the selected choice.
@MainActor final class AppKitKitChoiceRow: NSControl {
    private let handler: () -> Void
    private let mark = NSImageView()
    private let title: String

    init(title: String, detail: String? = nil, action: @escaping () -> Void) {
        self.title = title; handler = action
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        mark.translatesAutoresizingMaskIntoConstraints = false
        mark.setAccessibilityElement(false)
        var lines: [NSView] = [AppKitKit.label(title, size: 14)]
        if let detail { lines.append(AppKitKit.label(detail, size: 12, color: AppKitKit.muted)) }
        let text = AppKitKit.stack(lines, spacing: 3)
        lines.forEach { $0.setAccessibilityElement(false) }
        addSubview(mark); addSubview(text)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            mark.leadingAnchor.constraint(equalTo: leadingAnchor), mark.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            mark.widthAnchor.constraint(equalToConstant: 17), mark.heightAnchor.constraint(equalToConstant: 17),
            text.leadingAnchor.constraint(equalTo: mark.trailingAnchor, constant: 10),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 8), text.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.radioButton)
        setAccessibilityLabel(title)
        if let detail { setAccessibilityHelp(detail) }
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isSelected = false { didSet { if isSelected != oldValue { refresh() } } }
    override var isEnabled: Bool { didSet { refresh() } }
    override var acceptsFirstResponder: Bool { isEnabled }
    override var focusRingMaskBounds: NSRect { bounds }
    override func drawFocusRingMask() { NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill() }

    private func refresh() {
        let symbol = NSImage(systemSymbolName: isSelected ? "largecircle.fill.circle" : "circle", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 15, weight: .regular))
        mark.image = symbol
        mark.contentTintColor = isSelected ? AppKitKit.terracottaBright : AppKitKit.muted
        alphaValue = isEnabled ? 1 : 0.6
        setAccessibilityValue(isSelected ? 1 : 0)
        window?.invalidateCursorRects(for: self)
    }

    private func fire() { if isEnabled { handler() } }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { fire() }
    }
    override func keyDown(with event: NSEvent) {
        if event.charactersIgnoringModifiers == " " { fire() } else { super.keyDown(with: event) }
    }
    override func accessibilityPerformPress() -> Bool { fire(); return isEnabled }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
}

/// A settings dialog: the header, then a scrolling body as tall as its
/// content up to the dialog's limit.
@MainActor class AppKitSettingsDialog: NSView, AppKitDialogContent {
    let body = AppKitKit.stack([], spacing: 14)
    let observation = AppKitObservation()
    var dismissDisabled: Bool { false }

    init(title: String, detail: String, closeLabel: String, identifier: String, close: @escaping () -> Void) {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = AppKitKit.surface.cgColor
        setAccessibilityElement(true); setAccessibilityRole(.group)
        setAccessibilityIdentifier(identifier)
        let header = AppKitDialogHeader(title: title, detail: detail, closeLabel: closeLabel, close: close)
        let scroll = appKitKitScroll(body, insets: NSEdgeInsets(top: 22, left: 22, bottom: 22, right: 22))
        addSubview(header); addSubview(scroll)
        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor), header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.topAnchor.constraint(equalTo: topAnchor),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor), scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: header.bottomAnchor), scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            scroll.heightAnchor.constraint(equalTo: scroll.documentView!.heightAnchor).withPriority(.defaultHigh),
        ])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Replaces the body's rows, each filling the dialog's width.
    func setRows(_ rows: [NSView]) {
        body.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for row in rows { body.addArrangedSubview(row); AppKitKit.fillWidth(row, of: body) }
    }

    static func heading(_ text: String) -> NSTextField { AppKitKit.label(text, size: 13, weight: .bold) }
    static func note(_ text: String, error: Bool = false, identifier: String? = nil) -> NSTextField {
        let label = AppKitKit.label(text, size: 12, color: error ? AppKitKit.error : AppKitKit.muted)
        if let identifier { label.setAccessibilityIdentifier(identifier) }
        return label
    }
    /// A group of controls VoiceOver reads under one name, e.g. the DM choices.
    static func group(_ views: [NSView], label: String, spacing: CGFloat = 0) -> NSStackView {
        let stack = AppKitKit.stack(views, spacing: spacing)
        for view in views { AppKitKit.fillWidth(view, of: stack) }
        stack.setAccessibilityElement(true); stack.setAccessibilityRole(.group); stack.setAccessibilityLabel(label)
        return stack
    }
}

/// Who can start a DM, blocked accounts and account deletion.
@MainActor final class AppKitPrivacyDialog: AppKitSettingsDialog {
    private let model: AppModel
    private var unblocking: Set<String> = []
    private var unblockError: String?
    private lazy var choices: [DirectMessagePrivacy: AppKitKitChoiceRow] = Dictionary(uniqueKeysWithValues: DirectMessagePrivacy.allCases.map { option in
        let row = AppKitKitChoiceRow(title: option.title, detail: option.detail) { [weak self] in
            guard let self else { return }
            Task { await self.model.setDirectMessagePrivacy(option) }
        }
        row.setAccessibilityIdentifier("dm-privacy-\(option.rawValue)")
        return (option, row)
    })
    private lazy var deleteLink: AppKitKitLink = {
        let link = AppKitKitLink("Delete account", url: PrivacySettings.deleteAccountURL, color: AppKitKit.error, help: nil)
        link.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        link.setAccessibilityIdentifier("delete-account-link")
        return link
    }()

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model
        super.init(title: "Privacy", detail: "Message requests and blocked accounts.", closeLabel: "Close privacy settings",
                   identifier: "privacy-settings", close: close)
        observation.start(reading: { [model] in
            (model.directMessagePrivacy, model.privacyError, model.blockedAccounts, model.blocksLoaded, model.blocksError)
        }) { [weak self] _ in self?.render() }
        Task { [model] in
            await model.loadPrivacy()
            await model.refreshBlocks()
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func render() {
        var rows: [NSView] = [Self.heading("Who can start a DM with you")]
        for option in DirectMessagePrivacy.allCases {
            choices[option]?.isSelected = model.directMessagePrivacy == option
            choices[option]?.isEnabled = model.directMessagePrivacy != nil
        }
        rows.append(Self.group(DirectMessagePrivacy.allCases.compactMap { choices[$0] }, label: "Who can start a DM with you"))
        if model.directMessagePrivacy == nil && model.privacyError == nil { rows.append(Self.note("Loading…")) }
        if let error = model.privacyError { rows.append(Self.note(error, error: true, identifier: "dm-privacy-error")) }
        rows += [AppKitKit.divider(), Self.heading("Blocked accounts")]
        if model.blockedAccounts.isEmpty {
            rows.append(Self.note(model.blocksLoaded ? "You haven't blocked anyone." : model.blocksError ?? "Loading…",
                                  identifier: "blocked-accounts-empty"))
        }
        rows += model.blockedAccounts.map(row)
        if let unblockError { rows.append(Self.note(unblockError, error: true)) }
        rows += [AppKitKit.divider(), deleteLink]
        setRows(rows)
    }

    private func row(_ account: BlockedAccount) -> NSView {
        let avatar = AppKitAvatar()
        avatar.update(name: account.displayName, avatarId: account.avatarId, picture: account.avatarPicture)
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatar.widthAnchor.constraint(equalToConstant: 30).isActive = true
        avatar.heightAnchor.constraint(equalToConstant: 30).isActive = true
        var names: [NSView] = [AppKitKit.label(account.displayName, size: 13, weight: .medium, wraps: false)]
        if !account.username.isEmpty { names.append(AppKitKit.label("@\(account.username)", size: 11, color: AppKitKit.muted, wraps: false)) }
        let pending = unblocking.contains(account.id)
        let unblock = AppKitKitButton(pending ? "Unblocking…" : "Unblock") { [weak self] in self?.unblock(account.id) }
        unblock.isEnabled = !pending
        unblock.setAccessibilityLabel("Unblock \(account.displayName)")
        unblock.setAccessibilityIdentifier("unblock-\(account.id)")
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let row = NSStackView(views: [avatar, AppKitKit.stack(names, spacing: 1), spacer, unblock])
        row.orientation = .horizontal; row.spacing = 10
        row.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        row.setAccessibilityElement(true); row.setAccessibilityRole(.group)
        row.setAccessibilityIdentifier("blocked-account-\(account.id)")
        return row
    }

    private func unblock(_ id: String) {
        unblocking.insert(id); unblockError = nil; render()
        Task { [weak self, model] in
            var failure: String?
            do { try await model.unblock(accountID: id) } catch { failure = FriendlyError.message(for: error) }
            guard let self else { return }
            self.unblockError = failure
            self.unblocking.remove(id); self.render()
        }
    }
}

/// What notifies you, and on an iPhone with push available, that phone's
/// delivery and its on/off switch.
@MainActor final class AppKitNotificationsDialog: AppKitSettingsDialog {
    private let model: AppModel
    private lazy var levels: [NotificationLevel: AppKitKitChoiceRow] = Dictionary(uniqueKeysWithValues: NotificationLevel.allCases.map { level in
        let row = AppKitKitChoiceRow(title: level.accountTitle) { [weak self] in
            guard let self else { return }
            Task { await self.model.setAccountNotificationLevel(level) }
        }
        row.setAccessibilityIdentifier("notify-level-\(level.rawValue)")
        return (level, row)
    })
    private lazy var voiceStarted = AppKitKitSwitchRow(title: "When voice starts in a space", detail: nil,
                                                       identifier: "notify-voice-started") { [weak self] enabled in
        guard let self else { return }
        Task { await self.model.setVoiceStartedNotifications(enabled) }
    }

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model
        super.init(title: NotificationLabels.notifications, detail: "What Caper notifies you about.",
                   closeLabel: "Close notification settings", identifier: "notification-settings", close: close)
        observation.start(reading: { [model] in
            (model.notificationSettings, model.accountNotificationsSaving, model.notificationError, model.notificationsLoadError)
        }) { [weak self] _ in self?.render() }
        Task { [model] in await model.loadNotificationSettings() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    private func render() {
        let settings = model.notificationSettings, saving = model.accountNotificationsSaving
        for level in NotificationLevel.allCases {
            levels[level]?.isSelected = settings?.level == level
            levels[level]?.isEnabled = settings != nil && !saving.contains("level")
        }
        voiceStarted.isOn = settings?.voiceStarted ?? true
        voiceStarted.isEnabled = settings != nil && !saving.contains("voiceStarted")
        // Push delivery is per phone; the Mac has no phone section.
        var rows: [NSView] = [Self.group([Self.heading(NotificationLabels.notifyMeAbout)] + NotificationLevel.allCases.compactMap { levels[$0] },
                                         label: NotificationLabels.notifyMeAbout),
                              voiceStarted]
        if let error = model.notificationError ?? model.notificationsLoadError {
            rows.append(Self.note(error, error: true, identifier: "notification-settings-error"))
        } else if settings == nil { rows.append(Self.note("Loading…")) }
        setRows(rows)
    }
}
#endif
