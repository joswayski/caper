#if os(iOS)
import UIKit

/// Settings choices retain their native controls while server state updates,
/// so VoiceOver and keyboard focus stay on the option the person changed.
@MainActor final class UIKitSettingsChoice: UIButton {
    init(title: String, detail: String? = nil, action: @escaping () -> Void) {
        super.init(frame: .zero)
        var config = UIButton.Configuration.plain(); config.title = title; config.subtitle = detail
        config.titleAlignment = .leading; config.imagePlacement = .leading; config.imagePadding = 10
        config.contentInsets = NSDirectionalEdgeInsets(top: 8, leading: 0, bottom: 8, trailing: 0)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
            var value = value; value.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Satoshi-Regular", size: 14) ?? .systemFont(ofSize: 14)); return value
        }
        config.subtitleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { value in
            var value = value; value.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Satoshi-Regular", size: 12) ?? .systemFont(ofSize: 12)); return value
        }
        configuration = config; contentHorizontalAlignment = .fill
        titleLabel?.numberOfLines = 0
        heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        addAction(UIAction { _ in action() }, for: .primaryActionTriggered)
        configurationUpdateHandler = { button in
            var config = button.configuration
            config?.image = UIImage(systemName: button.isSelected ? "largecircle.fill.circle" : "circle", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15))
            config?.baseForegroundColor = UIColor(button.isSelected ? CaperTheme.terracottaBright : CaperTheme.text)
            button.configuration = config
        }
        accessibilityLabel = title; accessibilityHint = detail
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}

@MainActor final class UIKitPrivacyDialog: UIKitMessageDialog {
    private let model: AppModel
    private let accounts = UIStackView()
    private let error = UIKitMessageDialog.label("", error: true)
    private let unblockError = UIKitMessageDialog.label("", error: true)
    private let loading = UIKitMessageDialog.label("Loading…")
    private var choices: [DirectMessagePrivacy: UIKitSettingsChoice] = [:]
    private var unblocking: Set<String> = []
    private var shownAccounts: [BlockedAccount] = []
    private var shownPending: Set<String> = []
    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; super.init(title: "Privacy", closeLabel: "Close privacy settings", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.accessibilityIdentifier = "privacy-settings"
        body.addArrangedSubview(Self.label("Message requests and blocked accounts."))
        body.addArrangedSubview(Self.label("Who can start a DM with you", size: 13, bold: true))
        for option in DirectMessagePrivacy.allCases {
            let choice = UIKitSettingsChoice(title: option.title, detail: option.detail) { [model] in Task { await model.setDirectMessagePrivacy(option) } }
            choice.accessibilityIdentifier = "dm-privacy-\(option.rawValue)"; choices[option] = choice; body.addArrangedSubview(choice)
        }
        error.accessibilityIdentifier = "dm-privacy-error"
        [loading, error, Self.label("Blocked accounts", size: 13, bold: true)].forEach(body.addArrangedSubview)
        accounts.axis = .vertical; accounts.spacing = 8; body.addArrangedSubview(accounts); body.addArrangedSubview(unblockError)
        let delete = Self.button("Delete account") { UIApplication.shared.open(PrivacySettings.deleteAccountURL) }
        delete.accessibilityIdentifier = "delete-account-link"; body.addArrangedSubview(delete)
        observe(); Task { [model] in await model.loadPrivacy(); await model.refreshBlocks() }
    }
    override func render() {
        for option in DirectMessagePrivacy.allCases { choices[option]?.isSelected = model.directMessagePrivacy == option; choices[option]?.isEnabled = model.directMessagePrivacy != nil }
        loading.isHidden = model.directMessagePrivacy != nil || model.privacyError != nil
        error.text = model.privacyError; error.isHidden = model.privacyError == nil; unblockError.isHidden = unblockError.text?.isEmpty != false
        if model.blockedAccounts.isEmpty {
            accounts.arrangedSubviews.forEach { accounts.removeArrangedSubview($0); $0.removeFromSuperview() }
            let empty = Self.label(model.blocksLoaded ? "You haven't blocked anyone." : model.blocksError ?? "Loading…")
            empty.accessibilityIdentifier = "blocked-accounts-empty"; accounts.addArrangedSubview(empty)
            shownAccounts = []; shownPending = unblocking
        } else if shownAccounts != model.blockedAccounts || shownPending != unblocking {
            shownAccounts = model.blockedAccounts; shownPending = unblocking
            accounts.arrangedSubviews.forEach { accounts.removeArrangedSubview($0); $0.removeFromSuperview() }
            for account in model.blockedAccounts {
                let avatar = UIKitAvatar(side: 30); avatar.show(name: account.displayName, avatarID: account.avatarId, picture: account.avatarPicture)
                let names = UIStackView(arrangedSubviews: [Self.label(account.displayName, size: 13, bold: true), Self.label("@\(account.username)", size: 11)])
                names.axis = .vertical; names.spacing = 1
                let unblock = Self.button(unblocking.contains(account.id) ? "Unblocking…" : "Unblock") { [weak self] in self?.unblock(account.id) }
                unblock.isEnabled = !unblocking.contains(account.id); unblock.accessibilityLabel = "Unblock \(account.displayName)"
                unblock.accessibilityIdentifier = "unblock-\(account.id)"; unblock.setContentHuggingPriority(.required, for: .horizontal)
                let row = UIStackView(arrangedSubviews: [avatar, names, unblock]); row.spacing = 10; row.alignment = .center
                row.accessibilityIdentifier = "blocked-account-\(account.id)"; accounts.addArrangedSubview(row)
            }
        }
    }
    private func unblock(_ id: String) {
        guard !unblocking.contains(id) else { return }; unblocking.insert(id); unblockError.text = nil; render()
        Task { [weak self, model] in
            do { try await model.unblock(accountID: id) } catch { self?.unblockError.text = FriendlyError.message(for: error) }
            self?.unblocking.remove(id); self?.render()
        }
    }
}

@MainActor final class UIKitNotificationsDialog: UIKitMessageDialog {
    private let model: AppModel
    private var levels: [NotificationLevel: UIKitSettingsChoice] = [:]
    private var policies: [MobilePushPolicy: UIKitSettingsChoice] = [:]
    private let voice = UISwitch(), push = UISwitch()
    private let phone = UIStackView()
    private let status = UIKitMessageDialog.label("")
    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; super.init(title: NotificationLabels.notifications, closeLabel: "Close notification settings", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.accessibilityIdentifier = "notification-settings"
        body.addArrangedSubview(Self.label("What Caper notifies you about."))
        body.addArrangedSubview(Self.label(NotificationLabels.notifyMeAbout, size: 13, bold: true))
        for level in NotificationLevel.allCases {
            let choice = UIKitSettingsChoice(title: level.accountTitle) { [model] in Task { await model.setAccountNotificationLevel(level) } }
            choice.accessibilityIdentifier = "notify-level-\(level.rawValue)"; levels[level] = choice; body.addArrangedSubview(choice)
        }
        body.addArrangedSubview(toggle(voice, label: "When voice starts in a space", id: "notify-voice-started") { [model] in await model.setVoiceStartedNotifications($0) })
        phone.axis = .vertical; phone.spacing = 6; phone.addArrangedSubview(Self.label(NotificationLabels.sendToThisPhone, size: 13, bold: true))
        for policy in MobilePushPolicy.allCases {
            let choice = UIKitSettingsChoice(title: policy.title) { [model] in Task { await model.setMobilePushPolicy(policy) } }
            choice.accessibilityIdentifier = "notify-mobile-\(policy.rawValue)"; policies[policy] = choice; phone.addArrangedSubview(choice)
        }
        phone.addArrangedSubview(toggle(push, label: "Notifications on this phone", id: "push-opt-in") { [model] in await model.changePushEnabled($0) })
        body.addArrangedSubview(phone); body.addArrangedSubview(status); observe()
        Task { [model] in await model.loadNotificationSettings() }
    }
    override func render() {
        let settings = model.notificationSettings, saving = model.accountNotificationsSaving
        for level in NotificationLevel.allCases { levels[level]?.isSelected = settings?.level == level; levels[level]?.isEnabled = settings != nil && !saving.contains("level") }
        for policy in MobilePushPolicy.allCases { policies[policy]?.isSelected = settings?.mobile == policy; policies[policy]?.isEnabled = settings != nil && !saving.contains("mobile") }
        voice.isOn = settings?.voiceStarted ?? true; voice.isEnabled = settings != nil && !saving.contains("voiceStarted")
        phone.isHidden = !model.pushAvailable; push.isOn = model.pushEnabled
        let error = model.notificationError ?? model.notificationsLoadError
        status.text = error ?? (settings == nil ? "Loading…" : nil); status.isHidden = status.text == nil
        status.textColor = UIColor(error == nil ? CaperTheme.muted : CaperTheme.error)
        status.accessibilityIdentifier = error == nil ? nil : "notification-settings-error"
    }
    private func toggle(_ control: UISwitch, label: String, id: String, change: @escaping (Bool) async -> Void) -> UIView {
        control.accessibilityLabel = label; control.accessibilityIdentifier = id; control.onTintColor = UIColor(CaperTheme.terracottaBright)
        control.addAction(UIAction { [weak control] _ in guard let value = control?.isOn else { return }; Task { await change(value) } }, for: .valueChanged)
        let row = UIStackView(arrangedSubviews: [Self.label(label, size: 14), control]); row.spacing = 10; row.alignment = .center
        row.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true; return row
    }
}

/// Confirmation and invitation answers keep the dialog open until the API
/// confirms them. Failures preserve the action and allow retrying it.
@MainActor final class UIKitConfirmationDialog: UIKitMessageDialog {
    private let detail: String
    private let action: String
    private let pendingLabel: String
    private let perform: () async throws -> Void
    private let decline: (() async throws -> Void)?
    private let invitationArt: Bool
    private let status = UIKitMessageDialog.label("", error: true)
    private var confirm: UIButton!, cancel: UIButton!
    private var pending: String?
    override var dismissDisabled: Bool { pending != nil }
    init(title: String, detail: String, action: String, pendingLabel: String, invitationArt: Bool = false,
         close: @escaping () -> Void, decline: (() async throws -> Void)? = nil, perform: @escaping () async throws -> Void) {
        self.detail = detail; self.action = action; self.pendingLabel = pendingLabel; self.perform = perform
        self.decline = decline; self.invitationArt = invitationArt; super.init(title: title, close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        if invitationArt {
            let image = UIImageView(image: UIImage(named: "IncomingEnvelope", in: caperResourceBundle, compatibleWith: nil)); image.contentMode = .scaleAspectFit
            image.heightAnchor.constraint(equalToConstant: 32).isActive = true; image.isAccessibilityElement = false; body.addArrangedSubview(image)
        }
        body.addArrangedSubview(Self.label(detail)); body.addArrangedSubview(status)
        confirm = Self.button(action, primary: true) { [weak self] in self?.run(declining: false) }
        confirm.accessibilityIdentifier = "confirm-destructive-action"
        cancel = Self.button(decline == nil ? "Cancel" : "Decline") { [weak self] in
            guard let self, self.pending == nil else { return }
            if self.decline == nil { self.close() } else { self.run(declining: true) }
        }
        let actions = UIStackView(arrangedSubviews: [cancel, confirm]); actions.spacing = 8; body.addArrangedSubview(actions); render()
    }
    override func render() {
        confirm.configuration?.title = pending == "confirm" ? pendingLabel : action
        cancel.configuration?.title = pending == "decline" ? "Declining…" : decline == nil ? "Cancel" : "Decline"
        confirm.isEnabled = pending == nil; cancel.isEnabled = pending == nil; status.isHidden = status.text?.isEmpty != false
        updateDismissGuard(); parent?.view.setNeedsLayout()
    }
    private func run(declining: Bool) {
        guard pending == nil else { return }; pending = declining ? "decline" : "confirm"; status.text = nil; render()
        let operation = declining ? decline : perform
        Task { [weak self] in
            do { try await operation?(); self?.pending = nil; self?.close() }
            catch { self?.pending = nil; self?.status.text = FriendlyError.message(for: error); self?.render() }
        }
    }
}
#endif
