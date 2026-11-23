import AppKit
import CaperCore

/// "You’re viewing older messages" (or "3 new messages", with an accent
/// border) and a Jump to latest button, floating over a history just above
/// the composer, as in Discord. It arrives with the short fade and rise and
/// leaves at once. The host sizes it with `size(maxWidth:)`; a narrow pane
/// wraps the label onto two lines rather than cutting it off.
@MainActor final class JumpToLatestToast: NativeView {
    let button: NativeButton
    private let label = NativeStyle.label(JumpToLatest.viewingOlder, size: 13, bold: true)
    private(set) var count = 0
    private static let buttonHeight: CGFloat = 30

    init(action: @escaping () -> Void) {
        button = NativeButton(JumpToLatest.button, primary: true, action: action)
        super.init(NativeStyle.raised)
        layer?.cornerRadius = 13
        layer?.borderWidth = 1
        label.lineBreakMode = .byWordWrapping
        label.maximumNumberOfLines = 2
        label.cell?.wraps = true
        button.font = NativeStyle.font(13, bold: true)
        button.toolTip = JumpToLatest.button
        // Web's lift off the history: 0 8px 24px at 35% black. NSShadow
        // offsets are in window coordinates, so -8 is downward.
        let lift = NSShadow()
        lift.shadowColor = NSColor.black.withAlphaComponent(0.35)
        lift.shadowBlurRadius = 24
        lift.shadowOffset = NSSize(width: 0, height: -8)
        shadow = lift
        [label, button].forEach { addSubview($0) }
        isHidden = true
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func applyTheme() {
        super.applyTheme()
        render()
    }

    /// Shows, hides or recounts it.
    func update(shows: Bool, count: Int) {
        if count != self.count { self.count = count; render() }
        guard shows == isHidden else { return }
        isHidden = !shows
        layer?.removeAnimation(forKey: "entrance")
        guard shows, !CaperAccessibility.reduceMotion, let layer = self.layer else { return }
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0; fade.toValue = 1
        let rise = CABasicAnimation(keyPath: "transform.translation.y")
        rise.fromValue = 6; rise.toValue = 0  // The stage is flipped: from just below.
        let entrance = CAAnimationGroup()
        entrance.animations = [fade, rise]; entrance.duration = 0.14
        entrance.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
        layer.add(entrance, forKey: "entrance")
    }

    private func render() {
        label.stringValue = JumpToLatest.label(count)
        label.textColor = NativeStyle.text
        layer?.borderColor = (count > 0 ? NativeStyle.accent : NativeStyle.border).cgColor
        button.setAccessibilityLabel(JumpToLatest.accessibilityLabel(count))
        needsLayout = true
    }

    private var buttonWidth: CGFloat {
        ceil((JumpToLatest.button as NSString).size(withAttributes: [.font: NativeStyle.font(13, bold: true)]).width) + 24
    }

    private func labelHeight(_ width: CGFloat) -> CGFloat {
        let font = label.font ?? NativeStyle.font(13, bold: true)
        return ceil((label.stringValue as NSString).boundingRect(with: NSSize(width: max(1, width - 4), height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: [.font: font]).height) + 2
    }

    /// 6 points around the button, 14 before the label and between them.
    func size(maxWidth: CGFloat) -> NSSize {
        let font = label.font ?? NativeStyle.font(13, bold: true)
        let natural = ceil((label.stringValue as NSString).size(withAttributes: [.font: font]).width) + 4
        let labelWidth = min(natural, max(40, maxWidth - 14 - 14 - buttonWidth - 6))
        return NSSize(width: 14 + labelWidth + 14 + buttonWidth + 6, height: max(Self.buttonHeight, labelHeight(labelWidth)) + 12)
    }

    override func layout() {
        super.layout()
        let side = buttonWidth
        let textWidth = max(0, bounds.width - 14 - 14 - side - 6)
        let textHeight = min(max(0, bounds.height - 12), labelHeight(textWidth))
        label.frame = NSRect(x: 14, y: (bounds.height - textHeight) / 2, width: textWidth, height: textHeight)
        button.frame = NSRect(x: bounds.width - 6 - side, y: (bounds.height - Self.buttonHeight) / 2,
                              width: side, height: Self.buttonHeight)
    }
}
