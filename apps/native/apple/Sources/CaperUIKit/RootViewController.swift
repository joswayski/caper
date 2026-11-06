import CaperCore
import UIKit

/// Switches between sign-in, onboarding and the workspace as the model's phase
/// changes. At launch it paints the last session's workspace from the startup
/// cache while the model restores the live one.
final class RootViewController: UIViewController {
    let model: AppModel
    private var cached: StartupSnapshot?
    private var current: UIViewController?
    private var currentKind: Kind?
    private let observer = ModelObserver()
    private let saver = ModelObserver()
    private let deliveries = ModelObserver()
    private var saveWork: DispatchWorkItem?
    /// Mounted once the live workspace is ready, after first paint.
    private var whatsNew: UIViewController?

    private enum Kind: Equatable { case loading, signIn, onboarding, firstSpace, spacesUnavailable, workspace }

    init(model: AppModel, cached: StartupSnapshot?) {
        self.model = model
        self.cached = cached
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        observer.start { [weak self] in self?.render() }
        saver.start { [weak self] in self?.scheduleSave() }
        deliveries.start { [weak self] in self?.deliver() }
    }

    /// As CaperView does: an opened invite link, or a tapped invitation
    /// notification, replaces whatever dialog is open with its own.
    private func deliver() {
        let item: WorkspaceSheet
        if model.deliverableInviteLink != nil, let link = model.takeInviteLink() { item = .joinInvite(link) }
        else if let invitation = model.notificationInvitation { model.notificationInvitation = nil; item = .invitation(invitation) }
        else if let invitation = model.notificationChannelInvitation { model.notificationChannelInvitation = nil; item = .channelInvitation(invitation) }
        else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let dialog = UIKitHosting.workspaceDialog(model: self.model, item)
            if self.presentedViewController != nil { self.dismiss(animated: false) { self.present(dialog, animated: false) } }
            else { self.present(dialog, animated: false) }
        }
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    private func render() {
        let kind: Kind
        // As CaperRootView: a DM, a visit or open navigation still shows the
        // workspace to an account in no spaces.
        let noSpaces = model.spaces.isEmpty && model.invitations.isEmpty && model.selectedDirectMessageID == nil
            && model.visit == nil && !model.navigationOpen
        switch model.phase {
        case .loading: kind = cached == nil ? .loading : .workspace
        case .signedOut: kind = .signIn
        case .onboarding: kind = .onboarding
        case .ready:
            if model.restoringWorkspace { kind = cached == nil ? .loading : .workspace }
            else if noSpaces && model.spacesLoaded { kind = .firstSpace }
            // Try again clears the error while it retries; the page stays.
            else if noSpaces && (model.spacesError != nil || (currentKind == .spacesUnavailable && !model.spacesLoaded)) {
                kind = .spacesUnavailable
            }
            else { kind = .workspace }
        }
        if model.phase == .signedOut || (model.phase == .ready && model.account?.id != cached?.accountID && cached != nil) {
            // Another account, or none: the last session's content never shows again.
            if model.phase == .signedOut { StartupCache.clear() }
            cached = nil
            (current as? WorkspaceViewController)?.cached = nil
        }
        if whatsNew == nil, model.phase == .ready, !model.restoringWorkspace, model.account != nil {
            let host = UIKitWhatsNew.presenter(model: model)
            addChild(host)
            host.view.frame = .zero
            view.addSubview(host.view)
            host.didMove(toParent: self)
            whatsNew = host
        }
        guard kind != currentKind else { return }
        currentKind = kind
        let next: UIViewController
        switch kind {
        case .loading: next = LoadingViewController()
        case .signIn: next = SignInViewController(model: model)
        case .onboarding: next = OnboardingViewController(model: model)
        case .firstSpace: next = UIKitHosting.firstSpace(model: model)
        case .spacesUnavailable: next = UIKitHosting.spacesUnavailable(model: model)
        case .workspace: next = WorkspaceViewController(model: model, cached: cached)
        }
        show(next)
    }

    private func show(_ next: UIViewController) {
        if let current {
            current.willMove(toParent: nil)
            current.view.removeFromSuperview()
            current.removeFromParent()
        }
        addChild(next)
        next.view.frame = view.bounds
        next.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.insertSubview(next.view, at: 0)
        next.didMove(toParent: self)
        current = next
        UIAccessibility.post(notification: .screenChanged, argument: nil)
    }

    // MARK: Startup cache

    /// Saves shortly after the open conversation settles, and on backgrounding.
    private func scheduleSave() {
        // Read what should trigger a save; the save itself reads the rest.
        _ = (model.phase, model.restoringWorkspace, model.selectedChannelID, model.chat.loading)
        saveWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveNow() }
        saveWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5, execute: work)
    }

    func saveNow() {
        guard model.phase == .ready, !model.restoringWorkspace, CaperUIKitLaunch.cacheEnabled else { return }
        StartupCache.save(model)
    }
}

final class LoadingViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        let wordmark = UIImageView(image: UIImage(named: "CaperWordmark"))
        wordmark.contentMode = .scaleAspectFit
        let spinner = UIActivityIndicatorView(style: .medium)
        spinner.color = UIKitStyle.muted
        spinner.startAnimating()
        let label = UIKitStyle.label("Loading your spaces…", size: 12, weight: .medium, color: UIKitStyle.muted)
        let stack = UIStackView(arrangedSubviews: [wordmark, spinner, label])
        stack.axis = .vertical
        stack.spacing = 14
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            wordmark.widthAnchor.constraint(equalToConstant: 132), wordmark.heightAnchor.constraint(equalToConstant: 35),
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
        wordmark.isAccessibilityElement = true
        wordmark.accessibilityLabel = "Caper"
    }
}
