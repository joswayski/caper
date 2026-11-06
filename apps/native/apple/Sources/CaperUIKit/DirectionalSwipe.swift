import UIKit

/// SwiftUI's DrawerSwipe and ThreadBackSwipe: a pan that can start anywhere
/// but claims only a mostly horizontal move in one direction. It leaves
/// controls, editors, selected text and horizontal scrollers alone, and stops
/// vertical scrolling only while it follows the finger.
final class DirectionalSwipe: UIPanGestureRecognizer, UIGestureRecognizerDelegate {
    /// +1 for a swipe right, -1 for a swipe left.
    private let direction: CGFloat
    var allowed: () -> Bool = { true }
    /// The distance travelled in `direction`, never negative.
    var moved: (CGFloat) -> Void = { _ in }
    /// Lifted (true when it should complete) or cancelled (false).
    var ended: (_ complete: Bool, _ distance: CGFloat) -> Void = { _, _ in }
    private var paused: [UIScrollView] = []

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
        return !reserved(at: location(in: view), in: view)
    }

    /// Runs beside the timeline's scrolling, which pauses once this follows.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        other.view is UIScrollView && other is UIPanGestureRecognizer
    }

    private func reserved(at point: CGPoint, in view: UIView) -> Bool {
        var current = view.hitTest(point, with: nil)
        while let candidate = current, candidate !== view.superview {
            if candidate is UIControl { return true }
            if let text = candidate as? UITextView, text.isEditable || text.selectedRange.length > 0 { return true }
            if let scroll = candidate as? UIScrollView, scroll.contentSize.width > scroll.bounds.width + 1 { return true }
            current = candidate.superview
        }
        return false
    }

    @objc private func changed() {
        guard let view else { return }
        let distance = max(0, translation(in: view).x * direction)
        switch state {
        case .began:
            // As SwiftUI's scrollDisabled while a slide follows the finger.
            paused = Self.scrollViews(under: view).filter(\.isScrollEnabled)
            paused.forEach { $0.isScrollEnabled = false }
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
