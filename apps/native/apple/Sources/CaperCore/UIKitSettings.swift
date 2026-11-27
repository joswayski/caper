#if os(iOS)
import UIKit

/// iPhone Settings, opened from the account gear: one full-screen page with
/// every client's sections in order (Profile, Appearance, Notifications,
/// Voice & audio, Privacy & blocked accounts, About, Log out). Notifications
/// and Privacy push onto its navigation stack, where Back returns here; the
/// profile editor and audio dialogs open over it and close back to it.
@MainActor final class UIKitSettings: UIKitMessageDialog {
    private let model: AppModel
    private let avatar = UIKitAvatar(side: 48)
    private let name = UIKitMessageDialog.label("", size: 15, bold: true)
    private let username = UIKitMessageDialog.label("", size: 12)
    private let pauseStatus = UIKitMessageDialog.label("", size: 12)
    private let sendOnStop = UISwitch()
    private var audioTest: UIButton!
    private var connection: UIButton!
    private var diagnostics: UIButton!
    private let version = UIKitVersionControl()
    private var leaving = false

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model
        super.init(title: "Settings", closeLabel: "Close settings", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad(); view.accessibilityIdentifier = "settings"
        body.spacing = 12

        // Profile
        username.textColor = UIColor(CaperTheme.muted)
        let names = UIStackView(arrangedSubviews: [name, username]); names.axis = .vertical; names.spacing = 2
        let identity = UIStackView(arrangedSubviews: [avatar, names]); identity.spacing = 12; identity.alignment = .center
        identity.accessibilityIdentifier = "settings-profile"
        let edit = Self.button("Edit profile") { [weak self] in self?.open(.profile) }
        edit.accessibilityIdentifier = "settings-edit-profile"
        section("Profile", [identity, edit])

        // Appearance: web's and macOS's two presets. Switching rebuilds the
        // window's views in the new theme.
        let active = CaperThemeStore.shared.active.id
        let themes = [("obsidian", "Obsidian", "Dark"), ("paper", "Paper", "Light")].map { id, title, detail in
            let choice = UIKitSettingsChoice(title: title, detail: detail) {
                guard CaperThemeStore.shared.active.id != id else { return }
                CaperThemeStore.shared.selectPreset(id)
            }
            choice.isSelected = active == id; choice.accessibilityIdentifier = "appearance-\(id)"
            return choice
        }
        section("Appearance", themes)

        // Notifications
        pauseStatus.textColor = UIColor(CaperTheme.muted); pauseStatus.accessibilityIdentifier = "settings-do-not-disturb-status"
        let notifications = Self.button(NotificationLabels.notifications) { [weak self] in
            guard let self else { return }
            self.push(UIKitNotificationsDialog(model: self.model, close: { [weak self] in self?.popToSettings() }))
        }
        notifications.accessibilityIdentifier = "open-notifications"
        section(NotificationLabels.notifications, [pauseStatus, notifications])

        // Voice & audio
        sendOnStop.accessibilityLabel = VoiceMessage.sendOnStopTitle; sendOnStop.accessibilityIdentifier = "voice-messages-send-on-stop"
        sendOnStop.onTintColor = UIColor(CaperTheme.terracottaBright)
        sendOnStop.addAction(UIAction { [weak self] _ in
            guard let value = self?.sendOnStop.isOn else { return }; VoiceMessageSettings.shared.sendOnStop = value
        }, for: .valueChanged)
        let sendLabel = Self.label(VoiceMessage.sendOnStopTitle, size: 14)
        // The title wraps; the switch keeps its full size inside the page.
        sendLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        sendOnStop.setContentHuggingPriority(.required, for: .horizontal)
        sendOnStop.setContentCompressionResistancePriority(.required, for: .horizontal)
        let sendRow = UIStackView(arrangedSubviews: [sendLabel, sendOnStop]); sendRow.spacing = 10; sendRow.alignment = .center
        sendRow.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        let sendDetail = Self.label(VoiceMessage.sendOnStopDetail); sendDetail.textColor = UIColor(CaperTheme.muted)
        audioTest = Self.button("Audio test") { [weak self] in self?.open(.audio) }
        connection = Self.button("Connection details") { [weak self] in self?.open(.connection) }
        diagnostics = Self.button("Audio diagnostics") { [weak self] in self?.open(.diagnostics) }
        section("Voice & audio", [sendRow, sendDetail, audioTest, connection, diagnostics])

        // Privacy & blocked accounts
        let privacy = Self.button("Privacy and blocked accounts") { [weak self] in
            guard let self else { return }
            self.push(UIKitPrivacyDialog(model: self.model, close: { [weak self] in self?.popToSettings() }))
        }
        privacy.accessibilityIdentifier = "open-privacy"
        section("Privacy & blocked accounts", [privacy])

        // About
        let whatsNew = Self.button("What’s new") { [weak self] in self?.model.whatsNew.open() }
        whatsNew.accessibilityIdentifier = "open-whats-new"
        section("About", [version, whatsNew])

        // Log out: Settings closes first, so it never stays over sign-in.
        let logOut = Self.button("Log out") { [weak self] in self?.logOut() }
        logOut.configurationUpdateHandler = { button in
            var config = button.configuration
            config?.baseBackgroundColor = UIColor(CaperTheme.surface)
            config?.baseForegroundColor = UIColor(CaperTheme.error)
            button.configuration = config
        }
        logOut.accessibilityIdentifier = "log-out"
        body.addArrangedSubview(logOut)

        observe()
        Task { [model] in await model.loadNotificationSettings() }
    }

    override func render() {
        // Signed out elsewhere (an expired session): leave with the account.
        guard model.account != nil, model.phase == .ready else { leave(); return }
        let account = model.account
        let display = account?.displayName ?? account?.username ?? ""
        avatar.show(name: display, avatarID: account?.avatarId, picture: account?.avatarPicture)
        name.text = display
        username.text = account?.username.map { "@\($0)" }; username.isHidden = account?.username == nil
        let pause = model.doNotDisturb
        pauseStatus.text = "\(DoNotDisturbLabels.title): \(DoNotDisturbLabels.status(pause))"
        sendOnStop.isOn = VoiceMessageSettings.shared.sendOnStop
        audioTest.isEnabled = model.voice.phase != .leaving
        connection.isHidden = model.voice.phase != .connected
        // In a call; the Audio test has its own, while it captures.
        diagnostics.isHidden = account?.debugEnabled != true || model.voice.phase != .connected
    }

    private func section(_ title: String, _ rows: [UIView]) {
        let heading = Self.label(title, size: 12, bold: true)
        heading.textColor = UIColor(CaperTheme.muted); heading.accessibilityTraits = .header
        let stack = UIStackView(arrangedSubviews: [heading] + rows); stack.axis = .vertical; stack.spacing = 8
        body.addArrangedSubview(stack)
        body.setCustomSpacing(22, after: stack)
    }

    private func push(_ page: UIKitMessageDialog) {
        guard let navigation = navigationController, navigation.topViewController === self,
              navigation.presentedViewController == nil else { return }
        page.embedded = true
        navigation.pushViewController(page, animated: !CaperAccessibility.reduceMotion)
    }

    private func popToSettings() {
        navigationController?.popToViewController(self, animated: !CaperAccessibility.reduceMotion)
    }

    /// The existing workspace dialog, over Settings; closing it returns here.
    private func open(_ item: WorkspaceSheet) {
        guard let navigation = navigationController, navigation.presentedViewController == nil else { return }
        navigation.present(UIKitHosting.workspaceDialog(model: model, item), animated: false)
    }

    private func logOut() {
        let model = self.model
        leave()
        Task { await model.logout() }
    }

    private func leave() {
        guard !leaving else { return }
        leaving = true; stop(); close()
    }
}

/// Settings' navigation stack: the bar shows only on sub-pages, where its
/// Back returns to Settings.
@MainActor final class UIKitSettingsNavigation: UINavigationController, UINavigationControllerDelegate {
    init(settings: UIKitSettings) {
        super.init(rootViewController: settings)
        modalPresentationStyle = .fullScreen; overrideUserInterfaceStyle = .dark
        delegate = self
        setNavigationBarHidden(true, animated: false)
        let appearance = UINavigationBarAppearance()
        appearance.configureWithOpaqueBackground()
        appearance.backgroundColor = UIColor(CaperTheme.raised); appearance.shadowColor = .clear
        appearance.titleTextAttributes = [.foregroundColor: UIColor(CaperTheme.text)]
        navigationBar.standardAppearance = appearance; navigationBar.scrollEdgeAppearance = appearance
        navigationBar.tintColor = UIColor(CaperTheme.text)
        view.backgroundColor = UIColor(CaperTheme.raised)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func navigationController(_ navigationController: UINavigationController, willShow viewController: UIViewController, animated: Bool) {
        navigationController.setNavigationBarHidden(viewController === navigationController.viewControllers.first, animated: animated)
    }
}
#endif
