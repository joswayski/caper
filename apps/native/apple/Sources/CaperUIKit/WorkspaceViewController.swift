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
    /// One thread controller across both layouts: full-width on the phone,
    /// beside the conversation when wide. Resizing never recreates its state.
    private var threadPane: ThreadViewController?
    /// Wide: a drag's width before release saves it, for each resize handle.
    private var sidebarDrag: CGFloat?
    private var membersDrag: CGFloat?
    private var threadDrag: CGFloat?
    private var sidebarRange: ClosedRange<CGFloat> = 220...440
    private var membersRange: ClosedRange<CGFloat> = 220...440
    private var threadRange: ClosedRange<CGFloat> = 300...600
    private var sidebarWidth: CGFloat = 280
    private var membersWidth: CGFloat = 220
    private var threadWidth: CGFloat = 340
    private lazy var sidebarHandle = PanelResizeHandle(label: "Channel sidebar width", identifier: "channel-sidebar-resize",
        hint: "Drag to resize. Double-tap resets.", widensToRight: true, reset: 280,
        current: { [weak self] in self?.sidebarWidth ?? 280 }, range: { [weak self] in self?.sidebarRange ?? 220...440 }) { [weak self] width, commit in
        self?.resized(width, commit: commit, key: "caper.channelSidebarWidth", drag: \.sidebarDrag)
    }
    private lazy var membersHandle = PanelResizeHandle(label: "Member list width", identifier: "members-panel-resize",
        hint: "Drag to resize. Double-tap resets.", widensToRight: false, reset: 220,
        current: { [weak self] in self?.membersWidth ?? 220 }, range: { [weak self] in self?.membersRange ?? 220...440 }) { [weak self] width, commit in
        self?.resized(width, commit: commit, key: "caper.membersWidth", drag: \.membersDrag)
    }
    private lazy var threadHandle = PanelResizeHandle(label: "Thread width", identifier: "thread-panel-resize",
        hint: "Drag to resize. Double-tap resets.", widensToRight: false, reset: 340,
        current: { [weak self] in self?.threadWidth ?? 340 }, range: { [weak self] in self?.threadRange ?? 300...600 }) { [weak self] width, commit in
        self?.resized(width, commit: commit, key: "caper.threadPanelWidth", drag: \.threadDrag)
    }
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
            // Both reads are unconditional: rotating a loaded workspace must
            // resubscribe before the next thread or navigation change.
            _ = (self.model.narrowLayout, self.model.chat.threadRootID)
            self.view.setNeedsLayout()
            // Parity fixtures' local call rosters, as SwiftUI shows them; nothing otherwise.
            if !self.rosterShown, self.model.detail != nil, self.model.selectedChannelID != nil {
                self.rosterShown = true
                let model = self.model
                DispatchQueue.main.async { CaperRuntime.showVoiceRosterPreview(model) }
            }
            if self.wide {
                // Members follow the conversation: none for a DM or a visit.
                _ = (self.model.selectedDirectMessageID, self.model.visit?.channelID, self.model.detail?.space.id)
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
        showThreadPane(model.chat.threadRootID != nil)
        if wide { layoutWide() } else {
            update(progress: browsing ? 1 : 0)
            threadPane?.layout(frame: view.bounds, paned: false, topInset: inset)
            if let threadPane { view.bringSubviewToFront(threadPane.view) }
        }
        let covered = !wide && threadPane != nil
        browse.view.accessibilityElementsHidden = covered || (!wide && !browsing)
        conversation.view.accessibilityElementsHidden = covered || (!wide && browsing)
        browse.view.isUserInteractionEnabled = !covered && (wide || browsing)
        conversation.view.isUserInteractionEnabled = !covered
        view.bringSubviewToFront(banner)
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
            for handle in [sidebarHandle, membersHandle, threadHandle] where handle.superview == nil { view.addSubview(handle) }
            browse.view.isUserInteractionEnabled = true
            browse.view.accessibilityElementsHidden = false
            conversation.view.accessibilityElementsHidden = false
        } else {
            for handle in [sidebarHandle, membersHandle, threadHandle] {
                handle.resignFirstResponder()
                handle.isHidden = true
            }
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
        let sidebarMaximum = max(220, min(440, usable - 60 - 320))
        let sidebar = min(sidebarMaximum, max(220, sidebarDrag.map { Double($0) } ?? savedSidebar.rounded()))
        sidebarRange = 220...CGFloat(sidebarMaximum); sidebarWidth = CGFloat(sidebar)
        let navigation = 60 + sidebar
        let tiles = usable - navigation >= 540
        let membersMaximum = min(440, max(220, usable - navigation - (tiles ? 320 : 64)))
        let savedMembers = defaults.object(forKey: "caper.membersWidth") as? Double ?? 220
        let membersWidth = CGFloat(min(membersMaximum, max(220, membersDrag.map { Double($0) } ?? savedMembers)))
        membersRange = 220...CGFloat(membersMaximum); self.membersWidth = membersWidth
        let hasMembers = model.selectedDirectMessageID == nil && model.visit == nil && model.detail != nil
        let showsMembers = hasMembers && (membersPreference ?? tiles)
        let lead = insets.left + CGFloat(navigation)
        browse.view.frame = CGRect(x: 0, y: 0, width: lead, height: bounds.height)
        let tiled = showsMembers && tiles ? membersWidth + insets.right : 0
        // SwiftUI's thread panel shares the conversation's stage: 300...600
        // points, leaving the channel at least half of it (up to 300).
        let stage = Double(bounds.width - lead - tiled)
        let threadMaximum = min(600, stage - min(300, stage / 2)), threadMinimum = min(300, threadMaximum)
        let savedThread = defaults.object(forKey: "caper.threadPanelWidth") as? Double ?? 340
        threadWidth = CGFloat(min(threadMaximum, max(threadMinimum, threadDrag.map { Double($0) } ?? savedThread)))
        threadRange = CGFloat(threadMinimum)...CGFloat(max(threadMinimum, threadMaximum))
        let threadOpen = model.chat.threadRootID != nil
        let pane = threadOpen ? threadWidth : 0
        conversation.view.frame = CGRect(x: lead, y: 0, width: CGFloat(stage) - pane, height: bounds.height)
        threadPane?.layout(frame: CGRect(x: conversation.view.frame.maxX, y: 0, width: pane, height: bounds.height),
                           paned: true, topInset: bannerInset)
        sidebarHandle.frame = CGRect(x: lead - 8, y: 0, width: 8, height: bounds.height)
        threadHandle.frame = CGRect(x: conversation.view.frame.maxX, y: 0, width: 8, height: bounds.height)
        threadHandle.isHidden = !threadOpen
        sidebarHandle.isHidden = false
        // Tiled: a full-height column. Otherwise over the conversation, below its header.
        let top = insets.top + bannerInset + (tiles ? 0 : 50)
        let width = tiles ? tiled : membersWidth + insets.right
        dockedMembers.frame = CGRect(x: bounds.width - width, y: top, width: width, height: bounds.height - top - insets.bottom)
        showDockedMembers(showsMembers)
        membersHandle.frame = CGRect(x: dockedMembers.frame.minX, y: top, width: 8, height: dockedMembers.frame.height)
        membersHandle.isHidden = !showsMembers
        for handle in [sidebarHandle, membersHandle, threadHandle] {
            if handle.isHidden { handle.resignFirstResponder() }
            view.bringSubviewToFront(handle)
        }
        conversation.wideMembersShown = showsMembers
    }

    /// A pointer or finger previews the width; releasing, a double tap or
    /// VoiceOver saves it where SwiftUI keeps the same preference.
    private func resized(_ width: CGFloat, commit: Bool, key: String, drag: ReferenceWritableKeyPath<WorkspaceViewController, CGFloat?>) {
        if commit {
            UserDefaults.standard.set(Double(width.rounded()), forKey: key)
            self[keyPath: drag] = nil
        } else {
            self[keyPath: drag] = width
        }
        view.setNeedsLayout()
        view.layoutIfNeeded()
    }

    private func showThreadPane(_ shows: Bool) {
        if shows, threadPane == nil {
            view.endEditing(true)
            if conversation.presentedViewController != nil { conversation.dismiss(animated: false) }
            let controller = ThreadViewController(model: model)
            controller.closed = { [weak self, weak controller] in
                guard let self, let controller, self.threadPane === controller else { return }
                self.showThreadPane(false)
                self.view.setNeedsLayout()
            }
            addChild(controller)
            view.insertSubview(controller.view, aboveSubview: conversation.view)
            controller.didMove(toParent: self)
            threadPane = controller
            if !UIKitStyle.reduceMotion {
                controller.view.alpha = 0
                DispatchQueue.main.async { [weak self, weak controller] in
                    guard let controller, self?.threadPane === controller else { return }
                    controller.view.transform = CGAffineTransform(translationX: 12, y: 0)
                    let animator = UIViewPropertyAnimator(duration: UIKitStyle.motionFast, timingParameters: UIKitStyle.easeOut)
                    animator.addAnimations { controller.view.alpha = 1; controller.view.transform = .identity }
                    animator.startAnimation()
                }
            }
        } else if !shows, let controller = threadPane {
            threadPane = nil
            controller.willMove(toParent: nil)
            controller.view.removeFromSuperview()
            controller.removeFromParent()
        }
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
        browse.view.accessibilityElementsHidden = threadPane != nil || !browsing
        conversation.view.accessibilityElementsHidden = threadPane != nil || browsing
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

/// SwiftUI's 8-point resize handles in the wide layout. A drag previews the
/// width and saves it on release; a double tap resets it; VoiceOver adjusts
/// it as a slider in 10-point steps.
final class PanelResizeHandle: UIView {
    private let widensToRight: Bool
    private let reset: CGFloat
    private let current: () -> CGFloat
    private let range: () -> ClosedRange<CGFloat>
    private let changed: (CGFloat, Bool) -> Void
    private var start: CGFloat?

    init(label: String, identifier: String, hint: String, widensToRight: Bool, reset: CGFloat,
         current: @escaping () -> CGFloat, range: @escaping () -> ClosedRange<CGFloat>,
         changed: @escaping (CGFloat, Bool) -> Void) {
        self.widensToRight = widensToRight; self.reset = reset
        self.current = current; self.range = range; self.changed = changed
        super.init(frame: .zero)
        isAccessibilityElement = true
        accessibilityTraits = .adjustable
        accessibilityLabel = label
        accessibilityIdentifier = identifier
        accessibilityHint = hint + (widensToRight
            ? " Left narrows and Right widens by 10 pixels; Home and End select the bounds."
            : " Left widens and Right narrows by 10 pixels; Home and End select the bounds.")
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        addGestureRecognizer(pan)
        let double = UITapGestureRecognizer(target: self, action: #selector(doubleTapped))
        double.numberOfTapsRequired = 2
        addGestureRecognizer(double)
        let tap = UITapGestureRecognizer(target: self, action: #selector(focusHandle))
        tap.require(toFail: double)
        addGestureRecognizer(tap)
        addInteraction(UIPointerInteraction(delegate: self))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var canBecomeFocused: Bool { true }
    override var canBecomeFirstResponder: Bool { true }
    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        setNeedsDisplay()
        return accepted
    }
    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        setNeedsDisplay()
        return accepted
    }
    override func didUpdateFocus(in context: UIFocusUpdateContext, with coordinator: UIFocusAnimationCoordinator) {
        super.didUpdateFocus(in: context, with: coordinator)
        if isFocused { becomeFirstResponder() }
        else if isFirstResponder { resignFirstResponder() }
        setNeedsDisplay()
    }
    override func draw(_ rect: CGRect) {
        guard isFirstResponder || isFocused else { return }
        UIKitStyle.terracotta.setFill()
        UIRectFill(CGRect(x: bounds.midX - 1, y: 0, width: 2, height: bounds.height))
    }

    @objc private func focusHandle() { becomeFirstResponder() }

    override var keyCommands: [UIKeyCommand]? {
        [UIKeyCommand.inputLeftArrow, UIKeyCommand.inputRightArrow, UIKeyCommand.inputHome, UIKeyCommand.inputEnd].map {
            let command = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(resizeKey(_:)))
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
    }

    @objc private func resizeKey(_ command: UIKeyCommand) {
        let width: CGFloat
        switch command.input {
        case UIKeyCommand.inputLeftArrow: width = current() + (widensToRight ? -10 : 10)
        case UIKeyCommand.inputRightArrow: width = current() + (widensToRight ? 10 : -10)
        case UIKeyCommand.inputHome: width = range().lowerBound
        case UIKeyCommand.inputEnd: width = range().upperBound
        default: return
        }
        changed(clamped(width), true)
    }

    private func clamped(_ width: CGFloat) -> CGFloat {
        let range = range()
        return min(range.upperBound, max(range.lowerBound, width))
    }

    @objc private func panned(_ pan: UIPanGestureRecognizer) {
        let offset = pan.translation(in: superview).x * (widensToRight ? 1 : -1)
        switch pan.state {
        case .began: focusHandle(); start = current()
        case .changed: changed(clamped((start ?? current()) + offset), false)
        case .ended, .cancelled:
            changed(clamped((start ?? current()) + offset), true)
            start = nil
        default: break
        }
    }

    @objc private func doubleTapped() { focusHandle(); changed(clamped(reset), true) }

    override var accessibilityValue: String? {
        get { "\(Int(current())) pixels" }
        set {}
    }
    override func accessibilityIncrement() { changed(clamped(current() + 10), true) }
    override func accessibilityDecrement() { changed(clamped(current() - 10), true) }
}

extension PanelResizeHandle: UIPointerInteractionDelegate {
    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        UIPointerStyle(shape: .verticalBeam(length: 24), constrainedAxis: .vertical)
    }
}
