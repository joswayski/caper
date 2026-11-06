import CaperCore
import UIKit

/// The phone workspace: Browse underneath, the conversation sliding fully over
/// it. Opening Browse slides the conversation off to the right; a pan from
/// the conversation's leading edge does the same, as on the SwiftUI app.
final class WorkspaceViewController: UIViewController {
    private let model: AppModel
    let browse: BrowseViewController
    let conversation: ConversationViewController
    private var browsing = false
    private var rosterShown = false
    private let observer = ModelObserver()
    /// SwiftUI's banner for a channel or DM that failed to open, above both
    /// panes; they inset their content below it.
    private let banner = UIStackView()
    private let bannerText = UIKitStyle.label(size: 12, color: UIKitStyle.error)
    private let bannerObserver = ModelObserver()
    private lazy var retryOpening = CaperButton("Retry opening", kind: .secondary, size: 12) { [weak self] in
        guard let model = self?.model else { return }
        Task { await model.retryNavigation() }
    }

    var cached: StartupSnapshot? {
        didSet {
            browse.cached = cached
            conversation.cached = cached
        }
    }

    init(model: AppModel, cached: StartupSnapshot?) {
        self.model = model
        browse = BrowseViewController(model: model)
        conversation = ConversationViewController(model: model)
        self.cached = cached
        super.init(nibName: nil, bundle: nil)
        browse.cached = cached
        conversation.cached = cached
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIKitStyle.blackout
        embed(conversation)
        // Browse sits under the conversation: build it after the first frame.
        DispatchQueue.main.async { [weak self] in self?.loadBrowse() }
        conversation.view.layer.shadowColor = UIColor.black.cgColor
        conversation.view.layer.shadowOpacity = 0.45
        conversation.view.layer.shadowRadius = 16
        conversation.view.layer.shadowOffset = CGSize(width: -12, height: 0)
        browse.opened = { [weak self] in self?.slide(browsing: false) }
        conversation.browse = { [weak self] in self?.slide(browsing: true) }
        // As SwiftUI's DrawerSwipe: a swipe right from anywhere on the
        // conversation brings Browse in under the finger; a swipe left on
        // Browse returns to the conversation.
        let toBrowse = DirectionalSwipe(direction: 1)
        toBrowse.allowed = { [weak self] in self.map { !$0.browsing && $0.conversation.allowsBrowseSwipe } ?? false }
        toBrowse.moved = { [weak self] distance in
            guard let self else { return }
            self.loadBrowse()
            self.conversation.view.endEditing(true)
            self.update(progress: min(1, distance / max(1, self.view.bounds.width)))
        }
        toBrowse.ended = { [weak self] complete, _ in self?.slide(browsing: complete) }
        conversation.view.addGestureRecognizer(toBrowse)
        // No channel to show yet: start in Browse.
        // As SwiftUI's phone layout: Browse covers the conversation while the
        // model's navigationOpen says so. Opening a channel or DM (a row, a voice
        // card, a request, New message, a notification) clears it.
        model.narrowLayout = true
        observer.start { [weak self] in
            guard let self else { return }
            // Parity fixtures' local call rosters, as SwiftUI shows them; nothing otherwise.
            if !self.rosterShown, self.model.detail != nil, self.model.selectedChannelID != nil {
                self.rosterShown = true
                let model = self.model
                DispatchQueue.main.async { CaperRuntime.showVoiceRosterPreview(model) }
            }
            let hasChannel = self.hasConversation
            if !hasChannel && !self.browsing { self.slide(browsing: true, animated: false, record: false) }
            else if hasChannel && self.model.navigationOpen != self.browsing { self.slide(browsing: self.model.navigationOpen) }
        }
        update(progress: browsing ? 1 : 0)
        buildBanner()
    }

    private var hasConversation: Bool {
        model.selectedChannelID != nil || model.selectedDirectMessageID != nil || model.visit != nil || cached?.channelID != nil
    }

    private func buildBanner() {
        bannerText.numberOfLines = 0
        bannerText.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        retryOpening.setContentCompressionResistancePriority(.required, for: .horizontal)
        retryOpening.setContentHuggingPriority(.required, for: .horizontal)
        [bannerText, retryOpening].forEach(banner.addArrangedSubview)
        banner.spacing = 12
        banner.alignment = .center
        banner.isLayoutMarginsRelativeArrangement = true
        banner.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12)
        banner.backgroundColor = UIKitStyle.surface
        banner.accessibilityIdentifier = "navigation-error"
        banner.isHidden = true
        banner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(banner)
        NSLayoutConstraint.activate([
            banner.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            banner.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            banner.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        bannerObserver.start { [weak self] in
            guard let self else { return }
            let error = self.model.navigationError
            self.bannerText.text = error
            self.retryOpening.isEnabled = self.model.openingSpaceID == nil
            if self.banner.isHidden != (error == nil) {
                self.banner.isHidden = error == nil
                self.view.setNeedsLayout()
                if let error { UIAccessibility.post(notification: .announcement, argument: error) }
            }
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let inset = banner.isHidden ? 0 : banner.systemLayoutSizeFitting(
            CGSize(width: view.bounds.width, height: 0), withHorizontalFittingPriority: .required,
            verticalFittingPriority: .fittingSizeLevel).height
        for child in [browse, conversation] where child.additionalSafeAreaInsets.top != inset {
            child.additionalSafeAreaInsets.top = inset
        }
        update(progress: browsing ? 1 : 0)
    }

    private func embed(_ child: UIViewController, below: UIView? = nil) {
        addChild(child)
        child.view.frame = view.bounds
        child.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        if let below { view.insertSubview(child.view, belowSubview: below) } else { view.addSubview(child.view) }
        child.didMove(toParent: self)
    }

    private func loadBrowse() {
        guard browse.parent == nil else { return }
        embed(browse, below: conversation.view)
        let back = DirectionalSwipe(direction: -1)
        back.allowed = { [weak self] in
            guard let self else { return false }
            return self.browsing && self.hasConversation && self.browse.presentedViewController == nil
        }
        back.moved = { [weak self] distance in
            guard let self else { return }
            self.update(progress: max(0, 1 - distance / max(1, self.view.bounds.width)))
        }
        back.ended = { [weak self] complete, _ in self?.slide(browsing: !complete) }
        browse.view.addGestureRecognizer(back)
        browse.view.isUserInteractionEnabled = browsing
        browse.view.accessibilityElementsHidden = !browsing
    }

    /// `record: false` shows Browse only because there is nothing else to
    /// show yet, so the first channel to open (a new space's) slides away.
    func slide(browsing: Bool, animated: Bool = true, record: Bool = true) {
        loadBrowse()
        self.browsing = browsing
        if record && model.navigationOpen != browsing { model.navigationOpen = browsing }
        if browsing { conversation.view.endEditing(true) }
        let apply = { self.update(progress: browsing ? 1 : 0) }
        // Arrivals ease out; the system's reduce-motion setting makes it instant.
        if animated && !UIKitStyle.reduceMotion {
            let animator = UIViewPropertyAnimator(duration: 0.28, timingParameters: UIKitStyle.easeOut)
            animator.addAnimations(apply)
            animator.startAnimation()
        } else { apply() }
        browse.view.accessibilityElementsHidden = !browsing
        conversation.view.accessibilityElementsHidden = browsing
        UIAccessibility.post(notification: .screenChanged, argument: nil)
    }

    private func update(progress: CGFloat) {
        let travel = view.bounds.width + 40
        conversation.view.transform = CGAffineTransform(translationX: travel * progress, y: 0)
        if browse.isViewLoaded { browse.view.isUserInteractionEnabled = progress > 0.5 }
    }


    override func accessibilityPerformEscape() -> Bool {
        guard !browsing else { return false }
        slide(browsing: true)
        return true
    }
}
