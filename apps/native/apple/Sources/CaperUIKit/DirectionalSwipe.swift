import UIKit

/// SwiftUI's DrawerSwipe and ThreadBackSwipe: a pan that can start anywhere
/// but claims only a mostly horizontal move in one direction. It starts over
/// buttons, rows and message text as SwiftUI's simultaneous drag did, and
/// once it follows the finger it cancels their presses and stops vertical
/// scrolling. It leaves sliders, switches, editors, selected text and
/// horizontal scrollers alone.
final class DirectionalSwipe: UIPanGestureRecognizer, UIGestureRecognizerDelegate {
    /// +1 for a swipe right, -1 for a swipe left.
    private let direction: CGFloat
    var allowed: () -> Bool = { true }
    /// The distance travelled in `direction`, never negative.
    var moved: (CGFloat) -> Void = { _ in }
    /// Lifted (true when it should complete) or cancelled (false).
    var ended: (_ complete: Bool, _ distance: CGFloat) -> Void = { _, _ in }
    private var paused: [UIScrollView] = []
    /// Where the finger went down, in the view's coordinates.
    private var start: CGPoint = .zero

    init(direction: CGFloat) {
        self.direction = direction
        super.init(target: nil, action: nil)
        addTarget(self, action: #selector(changed))
        delegate = self
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard allowed(), let view else { return false }
        let velocity = velocity(in: view)
        let translation = translation(in: view)
        let dx = (translation.x != 0 ? translation.x : velocity.x) * direction
        let dy = translation.y != 0 ? translation.y : velocity.y
        guard dx > 0, dx >= abs(dy) * 1.5 else { return false }
        let location = location(in: view)
        start = CGPoint(x: location.x - translation.x, y: location.y - translation.y)
        return !reserved(at: start, in: view)
    }

    /// Beside everything under the finger: a row's press wash and hold, a
    /// SwiftUI button's press and the timeline's scrolling must not keep a
    /// swipe from starting. Once it follows, `claim` cancels them.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }

    private func reserved(at point: CGPoint, in view: UIView) -> Bool {
        var current = view.hitTest(point, with: nil)
        while let candidate = current, candidate !== view.superview {
            if candidate is UISlider || candidate is UISwitch { return true }
            if let field = candidate as? UITextField, field.isFirstResponder { return true }
            if let text = candidate as? UITextView, text.isEditable || text.selectedRange.length > 0 { return true }
            if let scroll = candidate as? UIScrollView, scroll.isScrollEnabled,
               scroll.contentSize.width > scroll.bounds.width + 1 { return true }
            current = candidate.superview
        }
        return false
    }

    /// The swipe owns the touch now: presses, holds, taps and text gestures
    /// under the finger stop (a lift must not open a row or a menu), and
    /// buttons see their touch cancelled. Scroll views pause instead.
    private func claim(in view: UIView) {
        var current = view.hitTest(start, with: nil)
        while let candidate = current, candidate !== view.superview {
            for other in candidate.gestureRecognizers ?? [] where other !== self && other.isEnabled {
                if other is DirectionalSwipe || (other.view is UIScrollView && other is UIPanGestureRecognizer) { continue }
                other.isEnabled = false
                other.isEnabled = true
            }
            current = candidate.superview
        }
    }

    @objc private func changed() {
        guard let view else { return }
        let distance = max(0, translation(in: view).x * direction)
        switch state {
        case .began:
            // As SwiftUI's scrollDisabled while a slide follows the finger.
            paused = Self.scrollViews(under: view).filter(\.isScrollEnabled)
            paused.forEach { $0.isScrollEnabled = false }
            claim(in: view)
            moved(distance)
        case .changed: moved(distance)
        case .ended:
            resume()
            let velocity = velocity(in: view).x * direction
            ended(distance >= 72 || velocity > 600, distance)
        case .cancelled, .failed:
            resume()
            ended(false, distance)
        default: break
        }
    }

    private func resume() {
        paused.forEach { $0.isScrollEnabled = true }
        paused = []
    }

    private static func scrollViews(under view: UIView) -> [UIScrollView] {
        view.subviews.flatMap { subview -> [UIScrollView] in
            (subview as? UIScrollView).map { [$0] } ?? scrollViews(under: subview)
        }
    }
}
