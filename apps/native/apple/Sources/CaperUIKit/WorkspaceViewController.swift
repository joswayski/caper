import CaperCore
import UIKit

/// The workspace, as SwiftUI's WorkspaceView. Phones: Browse underneath, the
/// conversation sliding fully over it; opening Browse slides the conversation
/// off to the right, and a swipe right does the same. Wider than 760 points
/// (iPad, a large phone in landscape, a wide split view): Browse beside the
/// conversation, its sidebar resizable, and the member list tiled when it fits.
final class WorkspaceViewController: UIViewController {
    private let model: AppModel
    let browse: BrowseViewController
    let conversation: ConversationViewController
    private var browsing = false
    /// The running slide, so a swipe can catch it where it is drawn.
    private var slideAnimator: UIViewPropertyAnimator?
    /// Where a following swipe found the drawer, as progress toward Browse.
    private var grab: CGFloat?
    /// The current geometry; nil until the first layout.
    private var layout: WideLayout?
    private var narrow: Bool { layout?.narrow ?? true }
    private var sidebarWidth = WideLayout.stored(WideLayout.sidebarKey, default: WideLayout.defaultSidebar)
    private var sidebarDrag: (start: CGFloat, width: CGFloat)?
    private var liveSidebarWidth: CGFloat { sidebarDrag?.width ?? sidebarWidth }
    private lazy var sidebarHandle: ResizeHandle = {
        let handle = ResizeHandle(label: "Channel sidebar width", identifier: "channel-sidebar-resize")
        handle.dragged = { [weak self] travel in
            guard let self, let layout = self.layout else { return }
            guard let travel else {
                if let drag = self.sidebarDrag { self.saveSidebar(drag.width) }
                self.sidebarDrag = nil
                return
            }
            let start = self.sidebarDrag?.start ?? min(self.sidebarWidth, layout.sidebarMaximum)
            self.sidebarDrag = (start, Self.clamp(start + travel, layout))
            self.view.setNeedsLayout()
        }
        handle.reset = { [weak self] in self?.saveSidebar(WideLayout.defaultSidebar) }
        handle.step = { [weak self] direction in
            guard let self, let layout = self.layout else { return }
            self.saveSidebar(min(self.sidebarWidth, layout.sidebarMaximum) + 10 * direction)
        }
        return handle
    }()
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
        // A main-queue block still runs before the first commit; a display
        // link first fires once that frame is on screen.
        CADisplayLink(target: FirstFrame { [weak self] in
            DispatchQueue.main.async { self?.loadBrowse() }
        }, selector: #selector(FirstFrame.tick)).add(to: .main, forMode: .common)
        conversation.view.layer.shadowColor = UIColor.black.cgColor
        conversation.view.layer.shadowRadius = 16
        conversation.view.layer.shadowOffset = CGSize(width: -12, height: 0)
        browse.opened = { [weak self] in if self?.narrow == true { self?.slide(browsing: false) } }
        conversation.browse = { [weak self] in if self?.narrow == true { self?.slide(browsing: true) } }
        // As SwiftUI's DrawerSwipe: a swipe right from anywhere on the
        // conversation brings Browse in under the finger; a swipe left on
        // Browse returns to the conversation.
        let toBrowse = DirectionalSwipe(direction: 1)
        toBrowse.allowed = { [weak self] in self.map { $0.narrow && !$0.browsing && $0.conversation.allowsBrowseSwipe } ?? false }
        toBrowse.moved = { [weak self] distance in
            guard let self else { return }
            let grab = self.grabSlide()
            self.update(progress: min(1, grab + distance / self.travel))
        }
        toBrowse.ended = { [weak self] complete, _ in self?.grab = nil; self?.slide(browsing: complete) }
        conversation.view.addGestureRecognizer(toBrowse)
        // No channel to show yet: start in Browse.
        // As SwiftUI's phone layout: Browse covers the conversation while the
        // model's navigationOpen says so. Opening a channel or DM (a row, a voice
        // card, a request, New message, a notification) clears it. Wide layouts
        // show both and leave navigationOpen alone. Phone until the first layout.
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
            guard self.narrow else { return }
            if !hasChannel && !self.browsing { self.slide(browsing: true, animated: false, record: false) }
            else if hasChannel && self.model.navigationOpen != self.browsing { self.slide(browsing: self.model.navigationOpen) }
        }
        update(progress: browsing ? 1 : 0)
        buildBanner()
        sidebarHandle.isHidden = true
        view.addSubview(sidebarHandle)
    }

    private static func clamp(_ width: CGFloat, _ layout: WideLayout) -> CGFloat {
        min(layout.sidebarMaximum, max(220, width.rounded()))
    }

    private func saveSidebar(_ width: CGFloat) {
        guard let layout else { return }
        sidebarWidth = Self.clamp(width, layout)
        UserDefaults.standard.set(Double(sidebarWidth), forKey: WideLayout.sidebarKey)
        view.setNeedsLayout()
    }

    /// Rotation, split view and Stage Manager resizes all arrive here.
    private func applyLayout() {
        let insets = view.safeAreaInsets
        let next = WideLayout(width: view.bounds.width - insets.left - insets.right, sidebar: liveSidebarWidth)
        let changed = layout?.narrow != next.narrow
        layout = next
        if changed {
            model.narrowLayout = next.narrow
            browse.wide = !next.narrow
            conversation.view.layer.shadowOpacity = next.narrow ? 0.45 : 0
            conversation.view.autoresizingMask = next.narrow ? [.flexibleWidth, .flexibleHeight] : []
            if next.narrow {
                // As SwiftUI's drawer rest: Browse shows if navigation is open or
                // nothing else can; a drag from the other layout never carries over.
                let shows = model.navigationOpen || !hasConversation
                if shows { loadBrowse() }
                browsing = shows
                // Browse may still wait for the first frame; loadBrowse sets it up then.
                if browse.isViewLoaded { browse.view.accessibilityElementsHidden = !shows }
                conversation.view.accessibilityElementsHidden = shows
            } else {
                loadBrowse()
                browsing = false
                browse.view.accessibilityElementsHidden = false
                conversation.view.accessibilityElementsHidden = false
            }
        }
        sidebarHandle.isHidden = next.narrow
        let sidebar = min(liveSidebarWidth, next.sidebarMaximum)
        sidebarHandle.accessibilityValue = "\(Int(sidebar)) points"
        if next.narrow {
            if browse.isViewLoaded {
                browse.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                browse.view.frame = view.bounds
            }
            // Coming from the wide layout; otherwise autoresizing keeps it full size.
            if changed { conversation.view.transform = .identity; conversation.view.frame = view.bounds }
            conversation.setLayout(narrow: true, membersTiled: false, membersMaximum: 0)
            update(progress: browsing ? 1 : 0)
        } else {
            let split = insets.left + next.navigationWidth
            conversation.view.transform = .identity
            browse.view.autoresizingMask = []
            browse.view.frame = CGRect(x: 0, y: 0, width: split, height: view.bounds.height)
            conversation.view.frame = CGRect(x: split, y: 0, width: view.bounds.width - split, height: view.bounds.height)
            conversation.setLayout(narrow: false, membersTiled: next.tilesMembers, membersMaximum: next.membersMaximum)
            sidebarHandle.frame = CGRect(x: split - 4, y: 0, width: 8, height: view.bounds.height)
            view.bringSubviewToFront(sidebarHandle)
            browse.view.isUserInteractionEnabled = true
        }
    }

    override func viewWillTransition(to size: CGSize, with coordinator: any UIViewControllerTransitionCoordinator) {
        super.viewWillTransition(to: size, with: coordinator)
        // A swipe in progress must not leave the conversation partly translated.
        conversation.view.endEditing(false)
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
        applyLayout()
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
            return self.narrow && self.browsing && self.hasConversation && self.browse.presentedViewController == nil
        }
        back.moved = { [weak self] distance in
            guard let self else { return }
            let grab = self.grabSlide()
            self.update(progress: max(0, grab - distance / self.travel))
        }
        back.ended = { [weak self] complete, _ in self?.grab = nil; self?.slide(browsing: !complete) }
        browse.view.addGestureRecognizer(back)
        browse.view.isUserInteractionEnabled = browsing || !narrow
        browse.view.accessibilityElementsHidden = !browsing && narrow
        view.setNeedsLayout()
    }

    /// `record: false` shows Browse only because there is nothing else to
    /// show yet, so the first channel to open (a new space's) slides away.
    func slide(browsing: Bool, animated: Bool = true, record: Bool = true) {
        guard narrow else { return }
        loadBrowse()
        self.browsing = browsing
        if record && model.navigationOpen != browsing { model.navigationOpen = browsing }
        if browsing { conversation.view.endEditing(true) }
        _ = catchSlide()
        let apply = { self.update(progress: browsing ? 1 : 0) }
        // Arrivals ease out; the system's reduce-motion setting makes it instant.
        if animated && !UIKitStyle.reduceMotion {
            let animator = UIViewPropertyAnimator(duration: 0.28, timingParameters: UIKitStyle.easeOut)
            animator.addAnimations(apply)
            animator.startAnimation()
            slideAnimator = animator
        } else { apply() }
        browse.view.accessibilityElementsHidden = !browsing
        conversation.view.accessibilityElementsHidden = browsing
        UIAccessibility.post(notification: .screenChanged, argument: nil)
    }

    /// How far the conversation slides to uncover Browse: a swipe moves it
    /// exactly as far as the finger.
    private var travel: CGFloat { max(1, view.bounds.width + 40) }

    /// Stops a slide still running where it is drawn and returns that
    /// position, as progress toward Browse.
    private func catchSlide() -> CGFloat {
        if let animator = slideAnimator, animator.state == .active {
            let drawn = conversation.view.layer.presentation()?.affineTransform() ?? conversation.view.transform
            animator.stopAnimation(true)
            conversation.view.transform = drawn
        }
        slideAnimator = nil
        return min(1, max(0, conversation.view.transform.tx / travel))
    }

    /// Where the current swipe took hold, so it follows from there rather
    /// than jumping. Its first move also loads Browse and puts the keyboard away.
    private func grabSlide() -> CGFloat {
        if let grab { return grab }
        loadBrowse()
        conversation.view.endEditing(true)
        let position = catchSlide()
        grab = position
        return position
    }

    private func update(progress: CGFloat) {
        guard narrow else { return }
        conversation.view.transform = CGAffineTransform(translationX: travel * progress, y: 0)
        if browse.isViewLoaded { browse.view.isUserInteractionEnabled = progress > 0.5 }
    }


    override func accessibilityPerformEscape() -> Bool {
        guard narrow, !browsing else { return false }
        slide(browsing: true)
        return true
    }
}
