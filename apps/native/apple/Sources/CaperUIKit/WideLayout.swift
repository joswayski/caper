import UIKit

/// SwiftUI WorkspaceView's geometry: phones (760 points or less inside the
/// safe area) slide the conversation over Browse; wider windows (iPad, a
/// large phone in landscape, a wide split view) show Browse beside the
/// conversation, and tile the member list once 540 points are left for it.
struct WideLayout: Equatable {
    static let sidebarKey = "caper.channelSidebarWidth"
    static let membersKey = "caper.membersWidth"
    static let defaultSidebar: CGFloat = 280
    static let defaultMembers: CGFloat = 220

    /// The width inside the safe area, as SwiftUI's GeometryReader measures it.
    let width: CGFloat
    let narrow: Bool
    /// The space rail plus the channel sidebar.
    let navigationWidth: CGFloat
    let sidebarMaximum: CGFloat
    /// The member list sits beside the conversation instead of over it.
    let tilesMembers: Bool
    let membersMaximum: CGFloat

    init(width: CGFloat, sidebar: CGFloat) {
        self.width = width
        narrow = width <= 760
        sidebarMaximum = Self.sidebarMaximum(for: width)
        navigationWidth = 60 + min(sidebar, sidebarMaximum)
        tilesMembers = !narrow && width - navigationWidth >= 540
        membersMaximum = min(440, max(220, width - navigationWidth - (tilesMembers ? 320 : 64)))
    }

    static func sidebarMaximum(for width: CGFloat) -> CGFloat { max(220, min(440, width - 60 - 320)) }

    func membersWidth(_ preferred: CGFloat) -> CGFloat { min(membersMaximum, max(220, preferred)) }

    static func stored(_ key: String, default value: CGFloat) -> CGFloat {
        UserDefaults.standard.object(forKey: key) == nil ? value : CGFloat(UserDefaults.standard.double(forKey: key))
    }
}

/// SwiftUI's sidebar and member list resize handles: an 8-point strip that
/// follows a drag, resets on a double tap, and is a VoiceOver adjustable that
/// steps by 10 points.
final class ResizeHandle: UIView {
    /// The drag's horizontal travel since it began; nil when it ends.
    var dragged: ((CGFloat?) -> Void)?
    var reset: (() -> Void)?
    /// +1 widens, -1 narrows.
    var step: ((CGFloat) -> Void)?

    init(label: String, identifier: String) {
        super.init(frame: .zero)
        backgroundColor = .clear
        addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(pan(_:))))
        let double = UITapGestureRecognizer(target: self, action: #selector(doubleTap))
        double.numberOfTapsRequired = 2
        addGestureRecognizer(double)
        isAccessibilityElement = true
        accessibilityTraits = .adjustable
        accessibilityLabel = label
        accessibilityHint = "Swipe up or down to resize by 10 points."
        accessibilityIdentifier = identifier
        addInteraction(UIPointerInteraction(delegate: self))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func pan(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began, .changed: dragged?(pan.translation(in: superview).x)
        default: dragged?(nil)
        }
    }

    @objc private func doubleTap() { reset?() }
    override func accessibilityIncrement() { step?(1) }
    override func accessibilityDecrement() { step?(-1) }
}

extension ResizeHandle: UIPointerInteractionDelegate {
    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        UIPointerStyle(shape: .verticalBeam(length: 24), constrainedAxes: .vertical)
    }
}
