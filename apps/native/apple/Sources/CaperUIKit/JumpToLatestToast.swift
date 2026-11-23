import CaperCore
import UIKit

/// "You’re viewing older messages" (or "3 new messages", with a terracotta
/// border once others have written) and a Jump to latest button, floating over
/// a timeline just above the composer, as in Discord. A narrow width wraps the
/// label onto two lines rather than cutting it off. It arrives with the short
/// fade and rise and leaves at once.
final class JumpToLatestToast: UIView {
    private let label = UIKitStyle.label(JumpToLatest.viewingOlder, size: 13, weight: .medium)
    let button = UIButton(type: .custom)
    private(set) var count = 0

    init(identifier: String, action: @escaping () -> Void) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        backgroundColor = UIKitStyle.raised
        layer.cornerRadius = 13
        layer.cornerCurve = .continuous
        layer.borderWidth = 1
        // Web's lift off the history: 0 8px 24px at 35% black.
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.35
        layer.shadowRadius = 12
        layer.shadowOffset = CGSize(width: 0, height: 8)

        label.numberOfLines = 2
        label.lineBreakMode = .byWordWrapping
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        var configuration = UIButton.Configuration.filled()
        configuration.cornerStyle = .fixed
        configuration.background.cornerRadius = 10
        configuration.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)
        configuration.baseBackgroundColor = UIKitStyle.action
        configuration.baseForegroundColor = UIKitStyle.onAction
        var title = AttributeContainer()
        title.uiKit.font = UIKitStyle.font(13, .bold)
        configuration.attributedTitle = AttributedString(JumpToLatest.button, attributes: title)
        button.configuration = configuration
        button.configurationUpdateHandler = { button in
            button.configuration?.baseBackgroundColor = button.isHighlighted ? UIKitStyle.actionPress : UIKitStyle.action
            button.configuration?.baseForegroundColor = UIKitStyle.onAction
        }
        button.setNeedsUpdateConfiguration()
        button.accessibilityIdentifier = identifier
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        button.addAction(UIAction { _ in action() }, for: .primaryActionTriggered)

        let stack = UIStackView(arrangedSubviews: [label, button])
        stack.axis = .horizontal
        stack.alignment = .center
        stack.spacing = 14
        stack.isLayoutMarginsRelativeArrangement = true
        stack.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 6, leading: 14, bottom: 6, trailing: 6)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            // A touch target, taller than web's 30 points.
            button.heightAnchor.constraint(greaterThanOrEqualToConstant: 40),
        ])
        isHidden = true
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
        label.text = JumpToLatest.label(count)
        layer.borderColor = (count > 0 ? UIKitStyle.terracotta : UIKitStyle.border).cgColor
        button.accessibilityLabel = JumpToLatest.accessibilityLabel(count)
    }
}
