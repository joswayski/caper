#if os(iOS)
import UIKit
#endif

/// Caper's small set of haptics on iPhone, kept light so they confirm an action
/// rather than announce it. They do nothing on a Mac or a device without a
/// haptic engine, and iOS mutes them when the user turns System Haptics off.
@MainActor public enum CaperHaptics {
    #if os(iOS)
    private static let light = UIImpactFeedbackGenerator(style: .light)
    private static let soft = UIImpactFeedbackGenerator(style: .soft)
    private static var lastArrival: TimeInterval = 0
    #endif

    /// A send, a reaction, play or pause, a toggle, a drawer settling.
    public static func tap() {
        #if os(iOS)
        light.impactOccurred(intensity: 0.7)
        #endif
    }

    /// Someone else's message arriving while the reader watches the
    /// conversation: fainter than a tap, and at most once every 1.5 seconds
    /// so a busy channel never buzzes continuously.
    public static func arrival() {
        #if os(iOS)
        let now = ProcessInfo.processInfo.systemUptime
        guard now - lastArrival >= 1.5, UIApplication.shared.applicationState == .active else { return }
        lastArrival = now
        soft.impactOccurred(intensity: 0.5)
        #endif
    }
}
