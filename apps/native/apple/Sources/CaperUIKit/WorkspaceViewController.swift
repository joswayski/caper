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
    private var shownDirect: String?
    private let observer = ModelObserver()

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
        let pan = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(edgePan(_:)))
        pan.edges = .left
        conversation.view.addGestureRecognizer(pan)
        // No channel to show yet: start in Browse.
        observer.start { [weak self] in
            guard let self else { return }
            let direct = self.model.selectedDirectMessageID
            let hasChannel = self.model.selectedChannelID != nil || direct != nil || self.cached?.channelID != nil
            if !hasChannel && !self.browsing { self.slide(browsing: true, animated: false) }
            // A DM opened from Browse, a request, New message or a notification: show it.
            if let direct, direct != self.shownDirect, self.browsing { self.slide(browsing: false) }
            self.shownDirect = direct
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
        browse.view.isUserInteractionEnabled = browsing
        browse.view.accessibilityElementsHidden = !browsing
    }

    func slide(browsing: Bool, animated: Bool = true) {
        loadBrowse()
        self.browsing = browsing
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

    @objc private func edgePan(_ pan: UIScreenEdgePanGestureRecognizer) {
        let width = max(1, view.bounds.width)
        let progress = min(1, max(0, pan.translation(in: view).x / width))
        switch pan.state {
        case .began: loadBrowse()
        case .changed: update(progress: progress)
        case .ended, .cancelled:
            slide(browsing: progress > 0.35 || pan.velocity(in: view).x > 600)
        default: break
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        update(progress: browsing ? 1 : 0)
    }

    override func accessibilityPerformEscape() -> Bool {
        guard !browsing else { return false }
        slide(browsing: true)
        return true
    }
}
