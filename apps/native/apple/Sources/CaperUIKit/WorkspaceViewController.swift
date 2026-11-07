import CaperCore
import UIKit

/// The phone workspace: Browse underneath, the conversation sliding fully over
/// it. Opening Browse slides the conversation off to the right; a pan from
/// the conversation's leading edge does the same, as on the SwiftUI app.
/// Over 760 points of safe width (iPad, large phones in landscape) it is
/// SwiftUI's wide layout instead: Browse beside the conversation, and Members
/// docked at the trailing edge once 540 points are left for the conversation.
final class WorkspaceViewController: UIViewController {
    private let model: AppModel
    let browse: BrowseViewController
    let conversation: ConversationViewController
    private var browsing = false
    /// SwiftUI's `narrow` breakpoint, inverted.
    private(set) var wide = false
    /// Wide: the reader's Members choice; until they make one, it follows the width.
    private var membersPreference: Bool?
    private var closingMembers = false
    private var bannerInset: CGFloat = 0
    private lazy var dockedMembers: MembersPanel = {
        let panel = MembersPanel(model: model, docked: true)
        panel.openProfile = { [weak self] member in self?.conversation.showProfile(member) }
        panel.closed = { [weak self] in
            guard let self, !self.closingMembers else { return }
            self.membersPreference = false
            self.view.setNeedsLayout()
        }
        return panel
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
        conversation.view.layer.shadowOpacity = 0.45
        conversation.view.layer.shadowRadius = 16
        conversation.view.layer.shadowOffset = CGSize(width: -12, height: 0)
        browse.opened = { [weak self] in self?.slide(browsing: false) }
        conversation.browse = { [weak self] in self?.slide(browsing: true) }
        conversation.toggleWideMembers = { [weak self] in
            guard let self else { return }
            self.membersPreference = !self.conversation.wideMembersShown
            self.view.setNeedsLayout()
        }
        // As SwiftUI's DrawerSwipe: a swipe right from anywhere on the
        // conversation brings Browse in under the finger; a swipe left on
        // Browse returns to the conversation.
        let toBrowse = DirectionalSwipe(direction: 1)
        toBrowse.allowed = { [weak self] in self.map { !$0.wide && !$0.browsing && $0.conversation.allowsBrowseSwipe } ?? false }
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
            if self.wide {
                // Members follow the conversation: none for a DM or a visit.
                _ = (self.model.selectedDirectMessageID, self.model.visit?.channelID, self.model.detail?.space.id)
                self.view.setNeedsLayout()
                return
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
        bannerInset = inset
        for child in [browse, conversation] where child.additionalSafeAreaInsets.top != inset {
            child.additionalSafeAreaInsets.top = inset
        }
        let safeWidth = view.bounds.width - view.safeAreaInsets.left - view.safeAreaInsets.right
        if (safeWidth > 760) != wide { setWide(safeWidth > 760) }
        if wide { layoutWide() } else { update(progress: browsing ? 1 : 0) }
    }

    /// Crossing the breakpoint (rotation, Split View, Stage Manager) keeps both
    /// controllers, and so the draft, scroll position and revealed messages.
    private func setWide(_ wide: Bool) {
        self.wide = wide
        model.narrowLayout = !wide
        loadBrowse()
        browse.wide = wide
        conversation.wide = wide
        conversation.view.transform = .identity
        conversation.view.layer.shadowOpacity = wide ? 0 : 0.45
        for child in [browse.view!, conversation.view!] {
            child.autoresizingMask = wide ? [] : [.flexibleWidth, .flexibleHeight]
            if !wide { child.frame = view.bounds }
        }
        if wide {
            if dockedMembers.superview == nil { view.insertSubview(dockedMembers, aboveSubview: conversation.view) }
            browse.view.isUserInteractionEnabled = true
            browse.view.accessibilityElementsHidden = false
            conversation.view.accessibilityElementsHidden = false
        } else {
            showDockedMembers(false)
            // Back to the phone pose: the conversation, unless nothing is open.
            browsing = !hasConversation
            if model.navigationOpen != browsing { model.navigationOpen = browsing }
            browse.view.accessibilityElementsHidden = !browsing
            conversation.view.accessibilityElementsHidden = browsing
        }
        UIAccessibility.post(notification: .layoutChanged, argument: nil)
    }

    /// SwiftUI's wide widths: the 60pt rail and the saved sidebar width
    /// (220...440, leaving the conversation 320), then the conversation, then
    /// Members at their saved width (220...440) when they tile.
    private func layoutWide() {
        let bounds = view.bounds, insets = view.safeAreaInsets
        let usable = Double(bounds.width - insets.left - insets.right)
        let defaults = UserDefaults.standard
        let savedSidebar = defaults.object(forKey: "caper.channelSidebarWidth") as? Double ?? 280
        let sidebar = min(max(220, min(440, usable - 60 - 320)), max(220, savedSidebar.rounded()))
        let navigation = 60 + sidebar
        let tiles = usable - navigation >= 540
        let membersMaximum = min(440, max(220, usable - navigation - (tiles ? 320 : 64)))
        let savedMembers = defaults.object(forKey: "caper.membersWidth") as? Double ?? 220
        let membersWidth = CGFloat(min(membersMaximum, max(220, savedMembers)))
        let hasMembers = model.selectedDirectMessageID == nil && model.visit == nil && model.detail != nil
        let showsMembers = hasMembers && (membersPreference ?? tiles)
        let lead = insets.left + CGFloat(navigation)
        browse.view.frame = CGRect(x: 0, y: 0, width: lead, height: bounds.height)
        let tiled = showsMembers && tiles ? membersWidth + insets.right : 0
        conversation.view.frame = CGRect(x: lead, y: 0, width: bounds.width - lead - tiled, height: bounds.height)
        // Tiled: a full-height column. Otherwise over the conversation, below its header.
        let top = insets.top + bannerInset + (tiles ? 0 : 50)
        let width = tiles ? tiled : membersWidth + insets.right
        dockedMembers.frame = CGRect(x: bounds.width - width, y: top, width: width, height: bounds.height - top - insets.bottom)
        showDockedMembers(showsMembers)
        conversation.wideMembersShown = showsMembers
    }

    private func showDockedMembers(_ shows: Bool) {
        guard shows != dockedMembers.isOpen else { return }
        if shows { dockedMembers.open() } else {
            closingMembers = true
            dockedMembers.close()
            closingMembers = false
        }
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
            return !self.wide && self.browsing && self.hasConversation && self.browse.presentedViewController == nil
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
        // Browse is always beside a wide conversation: nothing slides.
        if wide {
            self.browsing = false
            if record && model.navigationOpen { model.navigationOpen = false }
            return
        }
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
        guard !wide else { return }
        let travel = view.bounds.width + 40
        conversation.view.transform = CGAffineTransform(translationX: travel * progress, y: 0)
        if browse.isViewLoaded { browse.view.isUserInteractionEnabled = progress > 0.5 }
    }


    override func accessibilityPerformEscape() -> Bool {
        guard !browsing, !wide else { return false }
        slide(browsing: true)
        return true
    }
}
