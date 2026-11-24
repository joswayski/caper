import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Posted when a sheet, card or dialog over an iPhone/iPad conversation
/// closes; the conversation returns focus to its composer (hardware keyboard only).
public enum CaperFocus {
    public static let presentationClosed = Notification.Name("CaperFocusPresentationClosed")
    public static func presentationDidClose() { NotificationCenter.default.post(name: presentationClosed, object: nil) }
}

/// Reduce Motion for the native AppKit and UIKit views. A Debug build under
/// the parity UI fixture can force it with CAPER_REDUCE_MOTION=1, so UI tests
/// render the still poses without changing the Mac's settings; Release builds
/// read only the system setting.
public enum CaperAccessibility {
    public static var reduceMotion: Bool {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        if environment["CAPER_TEST_MODE"] == "parity", environment["CAPER_REDUCE_MOTION"] == "1" { return true }
        #endif
        #if os(macOS)
        return NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        #else
        return UIAccessibility.isReduceMotionEnabled
        #endif
    }
}

/// View original's highlight on the message it jumped to: full strength for a
/// second, then a four-second quadratic ease-out back to the row's own look
/// (web's cubic-bezier(0.33, 0.67, 0.67, 1)). Reduce Motion holds it for the
/// five seconds and drops it without the fade.
public enum JumpHighlight {
    public static let duration: TimeInterval = 5
    public static func strength(since start: Date, now: Date = Date(), reduceMotion: Bool = CaperAccessibility.reduceMotion) -> CGFloat {
        let elapsed = now.timeIntervalSince(start)
        if elapsed >= duration { return 0 }
        if reduceMotion || elapsed <= 1 { return 1 }
        let remaining = 1 - (elapsed - 1) / (duration - 1)
        return CGFloat(remaining * remaining)
    }
}

/// The shared motion tokens (shared/design.css --motion-*, --ease-out).
enum CaperMotion {
    static let fast: Double = 0.14
    static let base: Double = 0.2
    static let slow: Double = 0.26
    static func easeOut(_ duration: Double) -> Animation { .timingCurve(0.16, 1, 0.3, 1, duration: duration) }
}

struct TimelineScrollActivity: ViewModifier {
    @Binding var active: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if #available(iOS 18, macOS 15, *) {
            content.onScrollPhaseChange { _, phase in active = phase != .idle }
        } else {
            content.simultaneousGesture(DragGesture(minimumDistance: 1)
                .onChanged { _ in active = true }
                .onEnded { _ in active = false })
        }
    }
}

/// Keep physical scroll pinning immediate; invert only the drawing displacement.
/// Measuring an unshifted wrapper avoids feeding the presentation offset back
/// into geometry. Retain the current painted offset when another arrival or
/// lazy measurement lands during the same entrance.
struct TimelineArrivalMotion: ViewModifier {
    var key: String
    var revision: Int
    var enabled: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var previous: Sample?
    @State private var compensation: CGFloat = 0
    @State private var started = Date.distantPast
    @State private var arrived = Date.distantPast

    private struct Sample: Equatable {
        var key: String
        var revision: Int
        var top: CGFloat
        var enabled: Bool
    }

    private func offset(at date: Date) -> CGFloat {
        let progress = min(1, max(0, date.timeIntervalSince(started) / CaperMotion.base))
        let curve = UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.16, y: 1), endControlPoint: UnitPoint(x: 0.3, y: 1))
        return compensation * (1 - curve.value(at: progress))
    }

    func body(content: Content) -> some View {
        TimelineView(.animation(paused: compensation == 0 || reduceMotion)) { timeline in
            VStack(spacing: 0) { content.offset(y: reduceMotion ? 0 : offset(at: timeline.date)) }
        }
            .onGeometryChange(for: Sample.self) {
                Sample(key: key, revision: revision,
                       top: $0.frame(in: .named(MentionCard.timelineSpace)).minY,
                       enabled: enabled && !reduceMotion)
            } action: { sample in
                let old = previous
                previous = sample
                guard let old, old.key == sample.key, old.enabled, sample.enabled else {
                    compensation = 0
                    arrived = .distantPast
                    return
                }
                let now = Date()
                if old.revision != sample.revision { arrived = now }
                guard now.timeIntervalSince(arrived) < CaperMotion.base, old.top != sample.top else { return }
                compensation = offset(at: now) + old.top - sample.top
                started = now
            }
            .task(id: started) {
                guard compensation != 0 else { return }
                do { try await Task.sleep(for: .seconds(CaperMotion.base)) }
                catch { return }
                compensation = 0
            }
    }
}

/// An entrance as a view appears (web motion.css): it fades in from `offset`
/// and `scale` without moving the layout around it. Leaving stays instant, so
/// a dismissed control never lingers, and Reduce Motion shows it in place.
struct Entrance: ViewModifier {
    private let offset: CGSize
    private let scale: CGFloat
    private let animation: Animation
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Read once, when the view first appears; later values are ignored.
    @State private var hidden: Bool

    init(_ enters: Bool = true, offset: CGSize = .zero, scale: CGFloat = 1, animation: Animation) {
        self.offset = offset
        self.scale = scale
        self.animation = animation
        _hidden = State(initialValue: enters)
    }

    func body(content: Content) -> some View {
        let hide = hidden && !reduceMotion
        content.opacity(hide ? 0 : 1).scaleEffect(hide ? scale : 1).offset(hide ? offset : .zero)
            .onAppear {
                guard hidden else { return }
                withAnimation(reduceMotion ? nil : animation) { hidden = false }
            }
    }
}

extension Entrance {
    /// Menus, cards and tooltips anchored to a control rise 4 points.
    static var anchored: Entrance { Entrance(offset: CGSize(width: 0, height: 4), animation: CaperMotion.easeOut(CaperMotion.fast)) }
    /// `anchored` where `enters`; otherwise the view simply shows (as a live
    /// card's body arrives beneath a row that was already there).
    static func anchored(when enters: Bool) -> Entrance {
        Entrance(enters, offset: CGSize(width: 0, height: 4), animation: CaperMotion.easeOut(CaperMotion.fast))
    }
    /// Modal dialogs rise 8 points from 98%.
    static var dialog: Entrance { Entrance(offset: CGSize(width: 0, height: 8), scale: 0.98, animation: CaperMotion.easeOut(CaperMotion.base)) }
    /// A dialog's scrim fades in.
    static var scrim: Entrance { Entrance(animation: .easeOut(duration: CaperMotion.fast)) }
    /// The thread opens beside the conversation (or over it on narrow windows).
    static var panel: Entrance { Entrance(offset: CGSize(width: 12, height: 0), animation: CaperMotion.easeOut(CaperMotion.base)) }
    /// Attached files pop into the composer.
    static var chip: Entrance { Entrance(scale: 0.94, animation: CaperMotion.easeOut(CaperMotion.base)) }
    /// A link preview card that arrives under a row already on screen fades in
    /// briefly, without moving (web chat-embed-arrive); cards present when the
    /// row is built, including a send's carried composer card, show in place.
    static func embed(_ arriving: Bool) -> Entrance {
        Entrance(arriving, animation: CaperMotion.easeOut(CaperMotion.fast))
    }
    /// A message sent or received at the live end settles into place
    /// (`MessageArrivals` decides which rows count).
    static func message(_ arriving: Bool) -> Entrance {
        Entrance(arriving, offset: CGSize(width: 0, height: 10), animation: CaperMotion.easeOut(CaperMotion.base))
    }
}
