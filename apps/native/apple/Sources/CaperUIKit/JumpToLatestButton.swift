import CaperCore
import UIKit

/// "Jump to latest" over a timeline's bottom trailing corner: a down arrow,
/// and "3 new messages" with a terracotta border once others have written.
/// It arrives with the short fade and rise and leaves at once.
final class JumpToLatestButton: UIButton {
    private(set) var count = 0

    init(identifier: String, action: @escaping () -> Void) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        var configuration = UIButton.Configuration.plain()
        configuration.image = UIImage(systemName: "arrow.down", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold))
        configuration.imagePadding = 6
        configuration.baseForegroundColor = UIKitStyle.text
        configuration.background.backgroundColor = UIKitStyle.surface
        configuration.background.cornerRadius = 8
        configuration.background.strokeWidth = 1
        configuration.background.strokeColor = UIKitStyle.border
        self.configuration = configuration
        accessibilityIdentifier = identifier
        isHidden = true
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            widthAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        addAction(UIAction { _ in action() }, for: .primaryActionTriggered)
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func update(shows: Bool, count: Int) {
        if count != self.count { self.count = count; render() }
        guard shows == isHidden else { return }
        isHidden = !shows
        layer.removeAnimation(forKey: "entrance")
        guard shows, !UIKitStyle.reduceMotion else { return }
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0; fade.toValue = 1
        let rise = CABasicAnimation(keyPath: "transform.translation.y")
        rise.fromValue = 6; rise.toValue = 0
        let entrance = CAAnimationGroup()
        entrance.animations = [fade, rise]
        entrance.duration = UIKitStyle.motionFast
        entrance.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
        layer.add(entrance, forKey: "entrance")
    }

    private func render() {
        let text = JumpToLatest.countText(count)
        var container = AttributeContainer()
        container.uiKit.font = UIKitStyle.font(13, .bold)
        configuration?.attributedTitle = text.map { AttributedString($0, attributes: container) }
        configuration?.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: text == nil ? 0 : 12, bottom: 0, trailing: text == nil ? 0 : 14)
        configuration?.background.strokeColor = text == nil ? UIKitStyle.border : UIKitStyle.terracotta
        accessibilityLabel = JumpToLatest.accessibilityLabel(count)
    }
}
