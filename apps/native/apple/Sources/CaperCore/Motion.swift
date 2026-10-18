import SwiftUI

/// The shared motion tokens (shared/design.css --motion-*, --ease-out).
enum CaperMotion {
    static let fast: Double = 0.14
    static let base: Double = 0.2
    static let slow: Double = 0.26
    static func easeOut(_ duration: Double) -> Animation { .timingCurve(0.16, 1, 0.3, 1, duration: duration) }
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
    /// A message sent or received at the live end settles into place
    /// (`MessageArrivals` decides which rows count).
    static func message(_ arriving: Bool) -> Entrance {
        Entrance(arriving, offset: CGSize(width: 0, height: 6), animation: CaperMotion.easeOut(CaperMotion.slow))
    }
}
