#if os(iOS)
import UIKit
import Observation

/// Account setup and first-space pages retain their fields across model
/// updates. They use the same validation and account operations as desktop.
@MainActor final class UIKitOnboarding: UIViewController, UITextFieldDelegate {
    enum Page { case profile, firstSpace, unavailable }
    private let model: AppModel
    private let page: Page
    private let body = UIStackView()
    private let username = UITextField(), name = UITextField()
    private let error = UIKitMessageDialog.label("", error: true)
    private let hint = UIKitMessageDialog.label("")
    private let limit = UIKitMessageDialog.label("You have reached your space limit.")
    private var action: UIButton!
    private var activity: UIButton?
    private var pending = false
    private var createError: String?
    private var generation = 0
    private var invite: UIViewController?
    init(model: AppModel, page: Page) { self.model = model; self.page = page; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.backgroundColor = UIColor(CaperTheme.blackout); overrideUserInterfaceStyle = .dark
        let scroll = UIScrollView(); scroll.translatesAutoresizingMaskIntoConstraints = false; scroll.keyboardDismissMode = .interactive
        view.addSubview(scroll); body.axis = .vertical; body.spacing = 18; body.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(body)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor), scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            body.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 28),
            body.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -28),
            body.centerXAnchor.constraint(equalTo: scroll.frameLayoutGuide.centerXAnchor),
            body.widthAnchor.constraint(lessThanOrEqualToConstant: 384),
            body.widthAnchor.constraint(lessThanOrEqualTo: scroll.frameLayoutGuide.widthAnchor, constant: -56),
        ])
        let width = body.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -56); width.priority = .defaultHigh; width.isActive = true
        let mark = UIImageView(image: UIImage(named: "CaperWordmark")); mark.contentMode = .scaleAspectFit
        mark.heightAnchor.constraint(equalToConstant: 35).isActive = true; mark.widthAnchor.constraint(equalToConstant: 132).isActive = true
        mark.isAccessibilityElement = true; mark.accessibilityLabel = "Caper"
        body.addArrangedSubview(UIStackView(arrangedSubviews: [mark, UIView()]))
        switch page {
        case .profile:
            body.addArrangedSubview(UIKitMessageDialog.label("ONE LAST THING", bold: true))
            heading("Choose how you show up.")
            field(username, label: "Username"); username.returnKeyType = .next
            body.addArrangedSubview(UIKitMessageDialog.label("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores."))
            field(name, label: "Display name"); body.addArrangedSubview(hint); body.addArrangedSubview(error)
            action = UIKitMessageDialog.button("Finish setup", primary: true) { [weak self] in self?.submit() }
            action.accessibilityIdentifier = "profile-continue"; body.addArrangedSubview(action)
        case .firstSpace:
            let inbox = UIKitMessageDialog.button("Notifications") { [weak self] in
                guard let self, self.presentedViewController == nil else { return }
                self.present(UIKitActivity.inbox(model: self.model), animated: !CaperAccessibility.reduceMotion)
            }; inbox.accessibilityIdentifier = "activity-inbox-button"; activity = inbox; body.addArrangedSubview(inbox)
            heading("Name your space")
            body.addArrangedSubview(UIKitMessageDialog.label("Choose something you will recognize easily. You can always change it later!", size: 14))
            field(name, label: "Space name", placeholder: "Gaming nights")
            action = UIKitMessageDialog.button("Create space", primary: true) { [weak self] in self?.submit() }
            action.accessibilityIdentifier = "first-space-create"
            [action!, limit, error].forEach(body.addArrangedSubview)
        case .unavailable:
            heading("Spaces are unavailable."); body.addArrangedSubview(error)
            action = UIKitMessageDialog.button("Try again") { [model] in Task { await model.loadSpaces() } }
            body.addArrangedSubview(action)
            body.addArrangedSubview(UIKitMessageDialog.button("Direct messages") { [model] in model.navigationOpen = true })
        }
        body.addArrangedSubview(UIKitMessageDialog.button("Log out") { [model] in Task { await model.logout() } })
    }
    override func didMove(toParent parent: UIViewController?) {
        super.didMove(toParent: parent); generation += 1
        if parent != nil { loadViewIfNeeded(); observe(generation) }
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); presentInvite() }
    private func observe(_ token: Int) {
        guard token == generation, parent != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func heading(_ text: String) {
        let label = UIKitMessageDialog.label(text, size: 28, bold: true); label.accessibilityTraits.insert(.header); body.addArrangedSubview(label)
    }
    private func field(_ field: UITextField, label: String, placeholder: String? = nil) {
        body.addArrangedSubview(UIKitMessageDialog.label(label, bold: true))
        field.accessibilityLabel = label; field.placeholder = placeholder; field.borderStyle = .roundedRect
        field.font = UIFont(name: "Satoshi-Regular", size: 14); field.autocapitalizationType = .none; field.autocorrectionType = .no
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true; field.returnKeyType = .done; field.delegate = self
        field.addAction(UIAction { [weak self] _ in self?.render() }, for: .editingChanged); body.addArrangedSubview(field)
    }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if page == .profile, textField === username { name.becomeFirstResponder() } else { submit() }; return false
    }
    private func render() {
        switch page {
        case .profile:
            let invalid = ProfileValidation.hiddenCharacterError(displayName: name.text ?? "", saved: model.account?.displayName)
            hint.text = invalid ?? "This is what people see in conversations."; hint.textColor = UIColor(invalid == nil ? CaperTheme.muted : CaperTheme.error)
            error.text = model.error; action.configuration?.title = model.busy ? "Saving…" : "Finish setup"
            action.isEnabled = !model.busy && ProfileValidation.error(username: username.text ?? "", displayName: name.text ?? "", saved: model.account?.displayName) == nil
        case .firstSpace:
            error.text = createError; action.configuration?.title = pending ? "Creating…" : "Create space"
            action.isEnabled = !pending && model.canCreateSpace && !(name.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            limit.isHidden = model.canCreateSpace || model.limits == nil
            activity?.accessibilityValue = "\(model.activityUnread) unread"
            // Read the delivery key even before the page has a window.
            _ = model.deliverableInviteLink?.id; presentInvite()
        case .unavailable:
            error.text = model.spacesError; action.configuration?.title = model.busy ? "Trying…" : "Try again"; action.isEnabled = !model.busy
        }
        error.isHidden = error.text == nil
    }
    private func submit() {
        guard action.isEnabled else { return }
        if page == .profile {
            let username = username.text ?? "", displayName = name.text ?? ""
            Task { [model] in await model.saveProfile(username: username, displayName: displayName) }
        } else if page == .firstSpace {
            let name = name.text ?? ""; pending = true; createError = nil; render()
            Task { [weak self, model] in
                do { try await model.createSpace(name: name) } catch { self?.createError = FriendlyError.message(for: error) }
                self?.pending = false; self?.render()
            }
        }
    }
    private func presentInvite() {
        guard page == .firstSpace, view.window != nil, model.deliverableInviteLink != nil else { return }
        if let invite, invite.presentingViewController != nil {
            invite.dismiss(animated: false) { [weak self] in self?.invite = nil; self?.presentInvite() }; return
        }
        guard presentedViewController == nil, let link = model.takeInviteLink() else { return }
        let controller = UIKitJoinInvite(model: model, opened: link) { [weak self] in
            self?.invite?.dismiss(animated: false); self?.invite = nil
        }
        invite = controller; UIKitHosting.sheet(controller, background: CaperTheme.surface)
        present(controller, animated: !CaperAccessibility.reduceMotion)
    }
}
#endif
