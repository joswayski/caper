import CaperCore
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// SwiftUI's DrawerSwipe and ThreadBackSwipe: a one-finger swipe that can
/// start anywhere on its view and follows a mostly horizontal move in one
/// direction. It leaves sliders, switches, editors, selected text and
/// horizontal scrollers alone.
///
/// It decides from where the finger is, relative to where it went down, as the
/// web and Android drawers do, so a thumb's arc and a slow start still take
/// hold. It waits while the finger is within the slop, gives up once the move
/// is clearly vertical or heads the other way, and follows as soon as the
/// finger is past the slop and 1.5 times further across than up or down.
///
/// Once it follows, the touch is its own: buttons see their touch cancelled,
/// taps, holds and text gestures under the finger fail, scroll views stop, and
/// hosted SwiftUI controls skip the lift's tap (`SwipeTapGuard`).
final class DirectionalSwipe: UIGestureRecognizer, UIGestureRecognizerDelegate {
    /// How a followed swipe ended with the finger lifting.
    struct Lift {
        /// The last distance given to `moved`.
        let distance: CGFloat
        /// The finger's whole move from where it went down, in `direction`.
        let reach: CGFloat
        /// The finger's speed in `direction`, in points per second.
        let velocity: CGFloat
    }

    /// How far the finger moves before the swipe decides, as a pan's.
    static let slop: CGFloat = 10
    /// +1 for a swipe right, -1 for a swipe left.
    private let direction: CGFloat
    var allowed: () -> Bool = { true }
    /// The finger's distance in `direction` from where the swipe took hold:
    /// zero at first, and negative once the finger has gone back past it.
    var moved: (CGFloat) -> Void = { _ in }
    /// The finger lifted, or (nil) the system took the touch.
    var ended: (Lift?) -> Void = { _ in }

    private weak var finger: UITouch?
    /// Where the finger went down, in the window: the view moves under it.
    private var down: CGPoint = .zero
    /// The finger's move in `direction` when the swipe took hold.
    private var origin: CGFloat = 0
    private var following = false
    private var paused: [UIScrollView] = []
    /// Recent finger positions in `direction`, for the lift's speed.
    private var samples: [(time: TimeInterval, x: CGFloat)] = []

    init(direction: CGFloat) {
        self.direction = direction
        super.init(target: nil, action: nil)
        delegate = self
    }

    /// Stops following (the layout is changing under it); `ended` gets nil.
    func cancel() {
        guard isEnabled else { return }
        isEnabled = false
        isEnabled = true
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        // One finger; a second never joins or restarts it.
        guard finger == nil, touches.count == 1, let touch = touches.first, let view, !reserved(touch, in: view) else {
            if finger == nil { state = .failed } else { touches.forEach { ignore($0, for: event) } }
            return
        }
        finger = touch
        down = touch.location(in: nil)
        samples = [(touch.timestamp, 0)]
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let finger, touches.contains(finger) else { return }
        let point = finger.location(in: nil)
        let dx = (point.x - down.x) * direction, dy = point.y - down.y
        samples.append((finger.timestamp, dx))
        samples.removeAll { finger.timestamp - $0.time > 0.1 }
        if !following {
            // Clearly vertical (a scroll), or heading the other way.
            if abs(dy) > Self.slop && abs(dy) > abs(dx) || dx <= -Self.slop { state = .failed; return }
            // Still within the slop, or not yet clearly sideways: a thumb's arc flattens out.
            guard dx >= Self.slop, dx >= abs(dy) * 1.5 else { return }
            guard allowed() else { state = .failed; return }
            // Follow from here, so nothing jumps by the distance it took to decide.
            origin = dx
            following = true
            // As SwiftUI's scrollDisabled while a slide follows the finger: a
            // scroll that already took this touch stops; one that has not fails.
            paused = (finger.gestureRecognizers ?? []).compactMap { recognizer in
                guard let scroll = recognizer.view as? UIScrollView, recognizer === scroll.panGestureRecognizer,
                      scroll.isScrollEnabled else { return nil }
                return scroll
            }
            paused.forEach { $0.isScrollEnabled = false }
            state = .began
        } else {
            state = .changed
        }
        SwipeTapGuard.follow()
        moved(dx - origin)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let finger, touches.contains(finger) else { return }
        guard following else { state = .failed; return }
        let dx = (finger.location(in: nil).x - down.x) * direction
        // Only the last tenth of a second counts: a finger that stopped has no speed.
        let recent = samples.filter { finger.timestamp - $0.time <= 0.1 }
        let velocity = recent.first.map { first in
            finger.timestamp - first.time > 0.001 ? (dx - first.x) / CGFloat(finger.timestamp - first.time) : 0
        } ?? 0
        finish(Lift(distance: dx - origin, reach: dx, velocity: velocity))
        state = .ended
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let finger, touches.contains(finger) else { return }
        guard following else { state = .failed; return }
        finish(nil)
        state = .cancelled
    }

    /// Also after the system or `cancel()` ended it without a touch event.
    override func reset() {
        super.reset()
        if following { finish(nil) }
        finger = nil
        samples = []
    }

    private func finish(_ lift: Lift?) {
        following = false
        paused.forEach { $0.isScrollEnabled = true }
        paused = []
        SwipeTapGuard.release()
        ended(lift)
    }

    /// Nothing under the finger keeps a swipe from taking hold: a row's hold,
    /// a button's press and a scroll that started first all give way to it.
    override func canBePrevented(by preventingGestureRecognizer: UIGestureRecognizer) -> Bool { false }

    private func reserved(_ touch: UITouch, in view: UIView) -> Bool {
        var current = touch.view
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
}

extension UIViewPropertyAnimator {
    /// A released drawer swipe's finish, carrying on at the finger's speed
    /// (SwiftUI's DrawerSettle.Timing.release); nil settles one the system
    /// took back where it was.
    static func drawer(_ release: DrawerRelease?) -> UIViewPropertyAnimator {
        guard let release else {
            return UIViewPropertyAnimator(duration: 0.32, timingParameters: UICubicTimingParameters(
                controlPoint1: CGPoint(x: 0.32, y: 0.72), controlPoint2: CGPoint(x: 0, y: 1)))
        }
        return UIViewPropertyAnimator(duration: release.duration, timingParameters: UICubicTimingParameters(
            controlPoint1: CGPoint(x: 0.2, y: 0.2 * release.slope), controlPoint2: CGPoint(x: 0.35, y: 1)))
    }
}
