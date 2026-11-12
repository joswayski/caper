import AppKit
import CaperCore

/// The "Update available" decoration, as SwiftUI's UpdateButtonShine draws it:
/// a still prismatic edge in five colours and a short light sweep every eight
/// seconds. Only sublayers move; the button's label, frame and hit target
/// never animate. While the button has keyboard focus the decoration steps
/// aside for the normal focus ring. Reduce Motion, an inactive window or a
/// disabled button keeps the edge still (or hides it, when disabled).
@MainActor final class AppKitUpdateShine {
    private weak var button: NSButton?
    private let decoration = CALayer()
    private let glow = CAGradientLayer()
    private let edge = CAGradientLayer()
    private let glowMask = CAShapeLayer()
    private let edgeMask = CAShapeLayer()
    private let sweepClip = CALayer()
    private let sweep = CAGradientLayer()
    private var timer: Timer?
    private var observers: [NSObjectProtocol] = []
    private var focused = false
    /// CaperMotion.slow (0.26 s, --motion-slow) × 6, as in SwiftUI.
    private static let duration = 0.26 * 6
    private static let colors: [NSColor] = [
        NSColor(srgbRed: 167/255, green: 128/255, blue: 242/255, alpha: 1),
        NSColor(srgbRed: 104/255, green: 175/255, blue: 236/255, alpha: 1),
        NSColor(srgbRed: 99/255, green: 210/255, blue: 193/255, alpha: 1),
        NSColor(srgbRed: 215/255, green: 205/255, blue: 130/255, alpha: 1),
        NSColor(srgbRed: 229/255, green: 139/255, blue: 183/255, alpha: 1),
    ]

    init(button: NSButton) {
        self.button = button
        button.wantsLayer = true
        for (layer, mask, opacity, width) in [(glow, glowMask, Float(0.12), CGFloat(4)), (edge, edgeMask, Float(0.75), CGFloat(1))] {
            layer.colors = Self.colors.map(\.cgColor)
            layer.startPoint = CGPoint(x: 0, y: 0.5); layer.endPoint = CGPoint(x: 1, y: 0.5)
            layer.opacity = opacity
            mask.fillColor = nil; mask.strokeColor = NSColor.black.cgColor; mask.lineWidth = width
            layer.mask = mask
            decoration.addSublayer(layer)
        }
        sweep.colors = [NSColor.clear.cgColor, NSColor.white.withAlphaComponent(0.16).cgColor, NSColor.clear.cgColor]
        sweep.startPoint = CGPoint(x: 0, y: 0.5); sweep.endPoint = CGPoint(x: 1, y: 0.5)
        sweep.anchorPoint = .zero
        sweep.setAffineTransform(CGAffineTransform(rotationAngle: 18 * .pi / 180))
        sweep.isHidden = true
        sweepClip.masksToBounds = true; sweepClip.cornerRadius = 8
        sweepClip.addSublayer(sweep)
        decoration.addSublayer(sweepClip)
        // Layers take no clicks and are invisible to accessibility.
        button.layer?.addSublayer(decoration)

        let center = NotificationCenter.default
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.refresh() }
            })
        }
        observers.append(NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        })
    }

    deinit {
        timer?.invalidate()
        observers.forEach { NotificationCenter.default.removeObserver($0); NSWorkspace.shared.notificationCenter.removeObserver($0) }
    }

    func focusChanged(_ focused: Bool) { self.focused = focused; refresh() }

    /// After the button's frame, visibility or enabled state changes.
    func layout() {
        guard let button else { return }
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let bounds = button.bounds
        decoration.frame = bounds
        for (layer, mask, inset) in [(glow, glowMask, CGFloat(2)), (edge, edgeMask, CGFloat(0.5))] {
            layer.frame = bounds
            mask.frame = bounds
            mask.path = CGPath(roundedRect: bounds.insetBy(dx: inset, dy: inset), cornerWidth: 8 - inset, cornerHeight: 8 - inset, transform: nil)
        }
        sweepClip.frame = bounds
        sweep.bounds = CGRect(x: 0, y: 0, width: 56, height: bounds.height * 3)
        sweep.position = CGPoint(x: -80, y: -bounds.height)
        CATransaction.commit()
        refresh()
    }

    private var running: Bool {
        guard let button, button.isEnabled, !button.isHiddenOrHasHiddenAncestor, !focused,
              !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return false }
        return button.window?.isKeyWindow == true
    }

    private func refresh() {
        guard let button else { return }
        decoration.isHidden = !button.isEnabled || focused
        if running {
            guard timer == nil else { return }
            shine()
            timer = Timer.scheduledTimer(withTimeInterval: 8, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.shine() }
            }
        } else {
            timer?.invalidate(); timer = nil
            sweep.removeAllAnimations(); sweep.isHidden = true
        }
    }

    private func shine() {
        guard let button, running else { return }
        let width = button.bounds.width
        let move = CABasicAnimation(keyPath: "position.x")
        move.fromValue = -80; move.toValue = width + 80
        move.duration = Self.duration
        move.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
        move.isRemovedOnCompletion = true
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in self?.sweep.isHidden = true }
        sweep.isHidden = false
        sweep.add(move, forKey: "sweep")
        CATransaction.commit()
    }
}
