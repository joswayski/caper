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
    private var saveWork: DispatchWorkItem?

    private enum Kind: Equatable { case loading, signIn, onboarding, empty, workspace }

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
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    private func render() {
        let kind: Kind
        switch model.phase {
        case .loading: kind = cached == nil ? .loading : .workspace
        case .signedOut: kind = .signIn
        case .onboarding: kind = .onboarding
        case .ready:
            if model.restoringWorkspace { kind = cached == nil ? .loading : .workspace }
            else if model.spaces.isEmpty && model.invitations.isEmpty && model.spacesLoaded { kind = .empty }
            else { kind = .workspace }
        }
        if model.phase == .signedOut || (model.phase == .ready && model.account?.id != cached?.accountID && cached != nil) {
            // Another account, or none: the last session's content never shows again.
            if model.phase == .signedOut { StartupCache.clear() }
            cached = nil
            (current as? WorkspaceViewController)?.cached = nil
        }
        guard kind != currentKind else { return }
        currentKind = kind
        let next: UIViewController
        switch kind {
        case .loading: next = LoadingViewController()
        case .signIn: next = SignInViewController(model: model)
        case .onboarding: next = OnboardingViewController(model: model)
        case .empty: next = EmptyWorkspaceViewController(model: model)
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
        view.addSubview(next.view)
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

/// An account in no spaces. Creating and joining spaces arrive with the
/// owner workflows; until then this says so and offers sign-out.
final class EmptyWorkspaceViewController: UIViewController {
    private let model: AppModel
    init(model: AppModel) {
        self.model = model
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        let title = UIKitStyle.label("You’re not in any spaces yet", size: 22, weight: .bold)
        title.numberOfLines = 0
        title.accessibilityTraits = .header
        let body = UIKitStyle.label("Spaces you create or join on caper.chat appear here.", size: 15, color: UIKitStyle.muted)
        body.numberOfLines = 0
        let signOut = CaperButton("Sign out", kind: .secondary, size: 14) { [weak self] in
            guard let model = self?.model else { return }
            Task { await model.logout() }
        }
        signOut.heightAnchor.constraint(equalToConstant: 44).isActive = true
        let stack = UIStackView(arrangedSubviews: [title, body, signOut])
        stack.axis = .vertical
        stack.spacing = 14
        stack.alignment = .leading
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }
}
