#if os(macOS)
import AppKit
#else
import UIKit
import UIKit.UIGestureRecognizerSubclass
#endif

/// Any touch (iPhone) or click, scroll or key (Mac) in this app's windows counts as using Caper
/// here, so this device (not another one signed in to the account) alerts.
/// Reports at most every few seconds; heartbeats carry the age to the gateway.
@MainActor
final class InputActivity {
    private let report: () -> Void
    private var observers: [Any] = []
    private var last: ContinuousClock.Instant?

    init(report: @escaping () -> Void) { self.report = report }

    func start() {
        guard observers.isEmpty else { return }
        #if os(macOS)
        let events: NSEvent.EventTypeMask = [.keyDown, .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel, .mouseMoved]
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: events, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.noteInput() }
            return event
        }) { observers.append(monitor) }
        #else
        for case let scene as UIWindowScene in UIApplication.shared.connectedScenes {
            scene.windows.forEach(attach)
        }
        observers.append(NotificationCenter.default.addObserver(forName: UIWindow.didBecomeKeyNotification, object: nil, queue: .main) { [weak self] note in
            MainActor.assumeIsolated { if let window = note.object as? UIWindow { self?.attach(window) } }
        })
        #endif
    }

    private func noteInput() {
        let now = ContinuousClock.now
        if let last, last.duration(to: now) < .seconds(5) { return }
        last = now
        report()
    }

    #if !os(macOS)
    private func attach(_ window: UIWindow) {
        guard !(window.gestureRecognizers ?? []).contains(where: { $0 is TouchObserver }) else { return }
        window.addGestureRecognizer(TouchObserver { [weak self] in self?.noteInput() })
    }
    #endif
}

#if !os(macOS)
/// Sees every touch in its window without recognizing, delaying or cancelling anything.
private final class TouchObserver: UIGestureRecognizer, UIGestureRecognizerDelegate {
    private let touched: @MainActor () -> Void

    init(touched: @escaping @MainActor () -> Void) {
        self.touched = touched
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        touched()
        state = .failed
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
}
#endif
