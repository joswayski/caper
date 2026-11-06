import CaperCore
import UIKit

/// A message's reactions (SwiftUI's ReactionRow): chips of emoji and count
/// that wrap like words, then Add reaction. Your own reactions are terracotta.
/// Tap toggles yours; hold shows who reacted. Laid out by hand for the row's
/// known width, so the timeline's self-sizing measures it without a pass.
final class ReactionChipsView: UIView {
    struct Chip: Equatable {
        let emoji: String
        let count: Int
        let own: Bool
    }

    static let spacing: CGFloat = 6
    /// Each chip is drawn 28 points tall in a 44-point touch row, as on web.
    static let rowHeight: CGFloat = 44

    private var buttons: [ChipButton] = []
    private lazy var add: UIButton = {
        let button = UIButton(type: .system)
        button.setImage(UIKitStyle.icon("smile-plus")?.resized(to: 18), for: .normal)
        button.tintColor = UIKitStyle.muted
        button.accessibilityLabel = "Add reaction"
        button.addAction(UIAction { [weak self] _ in self?.addReaction?() }, for: .primaryActionTriggered)
        addSubview(button)
        return button
    }()
    private var heightConstraint: NSLayoutConstraint!
    var toggle: ((String) -> Void)?
    var showReactors: ((String) -> Void)?
    var addReaction: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightConstraint = heightAnchor.constraint(equalToConstant: 0)
        heightConstraint.isActive = true
        accessibilityContainerType = .semanticGroup
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ chips: [Chip], messageID: String, canInteract: Bool, width: CGFloat) {
        accessibilityIdentifier = "reaction-row-\(messageID)"
        while buttons.count < chips.count {
            let button = ChipButton()
            button.addAction(UIAction { [weak self, weak button] _ in
                guard let emoji = button?.emoji else { return }
                self?.toggle?(emoji)
            }, for: .touchUpInside)
            button.held = { [weak self] emoji in self?.showReactors?(emoji) }
            addSubview(button)
            buttons.append(button)
        }
        for (index, button) in buttons.enumerated() {
            button.isHidden = index >= chips.count
            guard index < chips.count else { continue }
            button.configure(chips[index], messageID: messageID, enabled: canInteract)
            button.showReactors = { [weak self] in self?.showReactors?(chips[index].emoji) }
        }
        add.isHidden = chips.isEmpty || !canInteract
        add.accessibilityIdentifier = "reaction-row-add-\(messageID)"

        // Flow: chips left to right, wrapping to a new 44-point row.
        var x: CGFloat = 0, y: CGFloat = 0
        let views: [UIView] = buttons.prefix(chips.count).map { $0 } + (add.isHidden ? [] : [add])
        for view in views {
            let size = view === add ? CGSize(width: 44, height: Self.rowHeight) : (view as! ChipButton).fittingSize
            if x > 0, x + size.width > width { x = 0; y += Self.rowHeight + Self.spacing }
            view.frame = CGRect(x: x, y: y, width: size.width, height: Self.rowHeight)
            x += size.width + Self.spacing
        }
        heightConstraint.constant = views.isEmpty ? 0 : y + Self.rowHeight
        isHidden = chips.isEmpty
    }

    override var accessibilityElements: [Any]? {
        get { subviews.filter { !$0.isHidden } }
        set {}
    }
}

private final class ChipButton: UIControl {
    private let pill = UIView()
    private let artwork = UIImageView()
    private let glyph = UILabel()
    private let count = UIKitStyle.label(size: 11, weight: .bold)
    private(set) var emoji = ""
    private var suppressTap = false
    var held: ((String) -> Void)?
    var showReactors: (() -> Void)?
    private(set) var fittingSize: CGSize = .zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        pill.layer.cornerRadius = 8
        pill.layer.cornerCurve = .continuous
        pill.layer.borderWidth = 1
        pill.isUserInteractionEnabled = false
        addSubview(pill)
        artwork.contentMode = .scaleAspectFit
        glyph.font = .systemFont(ofSize: 15)
        glyph.textAlignment = .center
        count.translatesAutoresizingMaskIntoConstraints = true
        [artwork, glyph, count].forEach(pill.addSubview)
        isAccessibilityElement = true
        accessibilityTraits = .button
        // Just under the row's hold, so the chip's sheet wins over message actions.
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(longPress(_:)))
        hold.minimumPressDuration = 0.45
        addGestureRecognizer(hold)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func configure(_ chip: ReactionChipsView.Chip, messageID: String, enabled: Bool) {
        emoji = chip.emoji
        if let image = UIKitReactions.artwork(chip.emoji) {
            artwork.image = UIImage(cgImage: image)
            artwork.isHidden = false
            glyph.isHidden = true
        } else {
            glyph.text = chip.emoji
            glyph.isHidden = false
            artwork.isHidden = true
        }
        count.text = "\(chip.count)"
        count.sizeToFit()
        pill.backgroundColor = chip.own ? UIKitStyle.terracotta.withAlphaComponent(0.24) : UIKitStyle.surface
        pill.layer.borderColor = (chip.own ? UIKitStyle.terracottaBright : UIKitStyle.border).cgColor
        // 7 + 18 + 4 + count + 7.
        let width = 7 + 18 + 4 + ceil(count.bounds.width) + 7
        fittingSize = CGSize(width: width, height: ReactionChipsView.rowHeight)
        isEnabled = enabled
        accessibilityIdentifier = "reaction-chip-\(messageID)-\(chip.emoji)"
        accessibilityLabel = "\(chip.emoji) reaction, \(chip.count), \(chip.own ? "selected by you" : "not selected by you")"
        accessibilityTraits = chip.own ? [.button, .selected] : .button
        if !enabled { accessibilityTraits.insert(.notEnabled) }
        // Read-only previews can still see who reacted.
        accessibilityCustomActions = [UIAccessibilityCustomAction(name: "Show who reacted") { [weak self] _ in
            self?.showReactors?(); return true
        }]
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        pill.frame = CGRect(x: 0, y: (bounds.height - 28) / 2, width: bounds.width, height: 28)
        artwork.frame = CGRect(x: 7, y: 5, width: 18, height: 18)
        glyph.frame = artwork.frame
        count.frame.origin = CGPoint(x: 29, y: (28 - count.bounds.height) / 2)
    }

    override var isHighlighted: Bool { didSet { pill.alpha = isHighlighted ? 0.7 : 1 } }

    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        suppressTap = false
        return super.beginTracking(touch, with: event)
    }

    // The release that ends a hold must not also toggle the reaction.
    override func sendAction(_ action: UIAction) {
        guard !suppressTap else { return }
        super.sendAction(action)
    }

    override func sendAction(_ action: Selector, to target: Any?, for event: UIEvent?) {
        guard !suppressTap else { return }
        super.sendAction(action, to: target, for: event)
    }

    @objc private func longPress(_ gesture: UILongPressGestureRecognizer) {
        guard gesture.state == .began else { return }
        suppressTap = true
        cancelTracking(with: nil)
        held?(emoji)
    }

    override func accessibilityActivate() -> Bool {
        guard isEnabled else { return false }
        sendActions(for: .touchUpInside)
        return true
    }
}
