import CaperCore
import UIKit

/// "You’re viewing older messages" (or "3 new messages", with a terracotta
/// border once others have written) and a Jump to latest button, floating over
/// a timeline just above the composer, as in Discord. A narrow width wraps the
/// label onto two lines rather than cutting it off. On a compact width (an
/// iPhone) it folds into Discord's arrow at the right edge, with the count as
/// a badge, so it never covers much of a short conversation. It arrives with
/// the short fade and rise and leaves at once.
final class JumpToLatestToast: UIView {
    private let label = UIKitStyle.label(JumpToLatest.viewingOlder, size: 13, weight: .medium)
    let button = UIButton(type: .custom)
    private(set) var count = 0
    private let badge = PaddedLabel()
    private var wide: [NSLayoutConstraint] = []
    private var compactPlacement: [NSLayoutConstraint] = []
    private var compactSize: [NSLayoutConstraint] = []
    private var compact: Bool? = nil

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
        button.configurationUpdateHandler = { [weak self] button in
            let arrow = self?.compact == true
            button.configuration?.baseBackgroundColor = arrow ? (button.isHighlighted ? UIColor.white.withAlphaComponent(0.08) : .clear)
                : button.isHighlighted ? UIKitStyle.actionPress : UIKitStyle.action
            button.configuration?.baseForegroundColor = arrow ? UIKitStyle.text : UIKitStyle.onAction
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
        badge.padding = 10
        badge.font = UIKitStyle.font(10, .bold)
        badge.textColor = UIKitStyle.text
        badge.textAlignment = .center
        badge.backgroundColor = UIKitStyle.terracotta
        badge.layer.cornerRadius = 9
        badge.layer.masksToBounds = true
        badge.isHidden = true
        badge.isAccessibilityElement = false
        badge.translatesAutoresizingMaskIntoConstraints = false
        addSubview(badge)
        compactSize = [widthAnchor.constraint(equalToConstant: 44), heightAnchor.constraint(equalToConstant: 44)]
        NSLayoutConstraint.activate([
            badge.heightAnchor.constraint(equalToConstant: 18),
            badge.widthAnchor.constraint(greaterThanOrEqualToConstant: 18),
            badge.topAnchor.constraint(equalTo: topAnchor, constant: -7),
            badge.trailingAnchor.constraint(equalTo: trailingAnchor, constant: 5),
        ])
        isHidden = true
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    /// Floats 12 points above `timeline`'s bottom edge, just above the
    /// composer: centered and at most its width less 24, or on a compact
    /// width an arrow 12 points from its right edge.
    func attach(over timeline: UIView, in container: UIView) {
        container.addSubview(self)
        let bottom = bottomAnchor.constraint(equalTo: timeline.bottomAnchor, constant: -12)
        wide = [centerXAnchor.constraint(equalTo: timeline.centerXAnchor),
                widthAnchor.constraint(lessThanOrEqualTo: timeline.widthAnchor, constant: -24)]
        compactPlacement = [trailingAnchor.constraint(equalTo: timeline.trailingAnchor, constant: -12)]
        bottom.isActive = true
        applyWidth()
        registerForTraitChanges([UITraitHorizontalSizeClass.self]) { (toast: JumpToLatestToast, _) in toast.applyWidth() }
    }

    private func applyWidth() {
        let next = traitCollection.horizontalSizeClass != .regular
        guard next != compact, superview != nil else { return }
        compact = next
        NSLayoutConstraint.deactivate(next ? wide : compactPlacement + compactSize)
        NSLayoutConstraint.activate(next ? compactPlacement + compactSize : wide)
        label.isHidden = next
        layer.cornerRadius = next ? 10 : 13
        var configuration = button.configuration
        configuration?.image = next ? UIKitStyle.icon("arrow-down")?.resized(to: 20) : nil
        configuration?.contentInsets = next ? .zero : NSDirectionalEdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12)
        if next { configuration?.attributedTitle = nil } else {
            var title = AttributeContainer()
            title.uiKit.font = UIKitStyle.font(13, .bold)
            configuration?.attributedTitle = AttributedString(JumpToLatest.button, attributes: title)
        }
        button.configuration = configuration
        (button.superview as? UIStackView)?.directionalLayoutMargins = next ? .zero
            : NSDirectionalEdgeInsets(top: 6, leading: 14, bottom: 6, trailing: 6)
        button.setNeedsUpdateConfiguration()
        render()
    }

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
        badge.text = count > 99 ? "99+" : String(count)
        badge.isHidden = compact != true || count == 0
        layer.borderColor = (count > 0 ? UIKitStyle.terracotta : UIKitStyle.border).cgColor
        button.accessibilityLabel = JumpToLatest.accessibilityLabel(count)
    }
}

extension UICollectionView {
    /// Cells that start below the visible area (above the composer's inset),
    /// for the toast's ten-message rule.
    var itemsBelowViewport: Int {
        let fold = contentOffset.y + bounds.height - adjustedContentInset.bottom
        let below = CGRect(x: 0, y: fold, width: max(1, bounds.width), height: max(0, contentSize.height - fold) + 1)
        return collectionViewLayout.layoutAttributesForElements(in: below)?
            .filter { $0.representedElementCategory == .cell && $0.frame.minY >= fold }.count ?? 0
    }
}
