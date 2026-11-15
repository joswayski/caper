#if os(iOS)
import UIKit

/// A native workspace overlay, sized like the former dialog: content height
/// up to 680 points, with its scroll area shrinking above the keyboard.
@MainActor final class UIKitWorkspaceFrame: UIViewController, UIGestureRecognizerDelegate {
    private var content: UIViewController
    private let maximumWidth: CGFloat
    private let card = UIView()
    private var closing = false
    private var appeared = false
    init(content: UIViewController, width: CGFloat) {
        self.content = content; maximumWidth = width; super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .overFullScreen; overrideUserInterfaceStyle = .dark
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.backgroundColor = UIColor.black.withAlphaComponent(0.55); view.accessibilityViewIsModal = true
        card.backgroundColor = UIColor(CaperTheme.surface); card.layer.cornerRadius = 8
        card.layer.borderWidth = 1; card.layer.borderColor = UIColor(CaperTheme.border).cgColor; card.clipsToBounds = true
        view.addSubview(card); addChild(content); card.addSubview(content.view); content.didMove(toParent: self)
        let backdrop = UITapGestureRecognizer(target: self, action: #selector(requestClose)); backdrop.delegate = self; view.addGestureRecognizer(backdrop)
        for name in [UIResponder.keyboardDidShowNotification, UIResponder.keyboardDidChangeFrameNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(revealFocus), name: name, object: nil)
        }
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let bounds = view.bounds
        let top = view.safeAreaInsets.top + max(16, (bounds.height - view.safeAreaInsets.top) * 0.1)
        let width = max(1, min(maximumWidth, bounds.width - 32))
        let bottom = min(bounds.maxY - view.safeAreaInsets.bottom, view.keyboardLayoutGuide.layoutFrame.minY)
        let ideal: CGFloat
        if let form = content as? UIKitMessageDialog { ideal = form.fittingHeight(width: width) }
        else if let profile = content as? UIKitProfilePresentation { ideal = profile.fittingHeight(width: width) }
        else { ideal = 560 }
        let height = max(1, min(ideal, 680, bottom - top - 16))
        card.frame = CGRect(x: (bounds.width - width) / 2, y: top, width: width, height: height)
        content.view.frame = card.bounds
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !appeared else { return }; appeared = true
        guard !CaperAccessibility.reduceMotion else { return }
        card.alpha = 0; card.transform = CGAffineTransform(translationX: 0, y: 8)
        UIView.animate(withDuration: CaperMotion.fast, delay: 0, options: [.curveEaseOut]) { self.card.alpha = 1; self.card.transform = .identity }
    }
    override var keyCommands: [UIKeyCommand]? { [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(requestClose))] }
    override func accessibilityPerformEscape() -> Bool { requestClose(); return true }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        !card.frame.contains(touch.location(in: view))
    }
    @objc private func requestClose() {
        if let profile = content as? UIKitProfilePresentation { profile.requestClose() }
        else if (content as? UIKitMessageDialog)?.dismissDisabled != true { close() }
    }
    func close() {
        guard !closing else { return }; closing = true
        (content as? UIKitProfilePresentation)?.stop(); (content as? UIKitMessageDialog)?.stop()
        // From the presenter, so a confirmation shown over the editor closes with the frame.
        (presentingViewController ?? self).dismiss(animated: false)
    }
    func replace(with next: UIViewController) {
        (content as? UIKitMessageDialog)?.stop()
        content.willMove(toParent: nil); content.view.removeFromSuperview(); content.removeFromParent()
        content = next; addChild(next); card.addSubview(next.view); next.didMove(toParent: self)
        view.setNeedsLayout()
    }
    @objc private func revealFocus() {
        view.setNeedsLayout(); view.layoutIfNeeded()
        guard let field = firstResponder(in: content.view) else { return }
        var candidate = field.superview
        while let next = candidate, !(next is UIScrollView) { candidate = next.superview }
        if let scroll = candidate as? UIScrollView {
            scroll.scrollRectToVisible(field.convert(field.bounds, to: scroll).insetBy(dx: 0, dy: -12), animated: !CaperAccessibility.reduceMotion)
        }
    }
    private func firstResponder(in view: UIView) -> UIView? {
        if view.isFirstResponder { return view }
        for child in view.subviews { if let found = firstResponder(in: child) { return found } }; return nil
    }
}
#endif
