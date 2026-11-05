import CaperCore
import Observation
import UIKit

/// Re-runs `update` whenever a model property it read changes. Each region
/// observes only what it reads, so composer edits never invalidate the
/// timeline or navigation views.
@MainActor final class ModelObserver {
    private var track: (() -> Void)?
    private var scheduled = false

    func start(_ update: @escaping () -> Void) {
        let owner = WeakObserver(self)
        track = {
            withObservationTracking(update) {
                Task { @MainActor in owner.value?.fire() }
            }
        }
        track?()
    }

    /// Several changes in one turn coalesce into one update.
    private func fire() {
        guard !scheduled else { return }
        scheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.scheduled = false
            self?.track?()
        }
    }

    func stop() { track = nil }
}

private final class WeakObserver: @unchecked Sendable {
    weak var value: ModelObserver?
    init(_ value: ModelObserver) { self.value = value }
}

/// Caper's tokens (shared/design.css, via CaperTheme) for UIKit.
enum UIKitStyle {
    static let blackout = UIColor(CaperTheme.blackout)
    static let surface = UIColor(CaperTheme.surface)
    static let raised = UIColor(CaperTheme.raised)
    static let sidebar = UIColor(CaperTheme.sidebar)
    static let conversation = UIColor(CaperTheme.conversation)
    static let composer = UIColor(CaperTheme.composer)
    static let border = UIColor(CaperTheme.border)
    static let text = UIColor(CaperTheme.text)
    /// Message body text, a step under headings (SwiftUI's MessageRow).
    static let body = UIColor(red: 222 / 255, green: 223 / 255, blue: 224 / 255, alpha: 1)
    static let muted = UIColor(CaperTheme.muted)
    static let link = UIColor(CaperTheme.link)
    static let terracotta = UIColor(CaperTheme.terracotta)
    static let terracottaBright = UIColor(CaperTheme.terracottaBright)
    static let action = UIColor(CaperTheme.action)
    static let actionPress = UIColor(CaperTheme.actionPress)
    static let error = UIColor(CaperTheme.error)
    static let green = UIColor(CaperTheme.green)

    enum Weight { case regular, medium, bold, black }

    /// Satoshi at a fixed size, scaled with the reader's text size.
    static func font(_ size: CGFloat, _ weight: Weight = .regular, scaled: Bool = true) -> UIFont {
        let name: String
        switch weight {
        case .regular: name = "Satoshi-Regular"
        case .medium: name = "Satoshi-Medium"
        case .bold: name = "Satoshi-Bold"
        case .black: name = "Satoshi-Black"
        }
        let font = UIFont(name: name, size: size) ?? .systemFont(ofSize: size)
        return scaled ? UIFontMetrics.default.scaledFont(for: font) : font
    }

    static func label(_ text: String = "", size: CGFloat = 14, weight: Weight = .regular, color: UIColor = text) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = font(size, weight)
        label.textColor = color
        label.adjustsFontForContentSizeCategory = true
        label.translatesAutoresizingMaskIntoConstraints = false
        return label
    }

    /// A Lucide icon from the app's asset catalog, tinted like text.
    static func icon(_ name: String) -> UIImage? {
        UIImage(named: "caper-\(name)")?.withRenderingMode(.alwaysTemplate)
    }

    static var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }
    /// shared/design.css --motion-fast and --ease-out.
    static let motionFast: TimeInterval = 0.16
    static let easeOut = UICubicTimingParameters(controlPoint1: CGPoint(x: 0.2, y: 0.8), controlPoint2: CGPoint(x: 0.2, y: 1))
}

/// Web's primary action: eggshell with a blackout label; disabled, it steps
/// back to neutral chrome. Secondary buttons are bordered neutral chrome.
final class CaperButton: UIButton {
    enum Kind { case primary, secondary, plain }
    private let kind: Kind
    private let handler: () -> Void

    init(_ title: String, kind: Kind = .primary, icon: String? = nil, size: CGFloat = 16, handler: @escaping () -> Void) {
        self.kind = kind
        self.handler = handler
        super.init(frame: .zero)
        var configuration = UIButton.Configuration.plain()
        configuration.title = title
        configuration.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attributes in
            var attributes = attributes
            attributes.font = UIKitStyle.font(size, kind == .plain ? .regular : .bold)
            return attributes
        }
        if let icon {
            configuration.image = UIKitStyle.icon(icon)?.resized(to: 14)
            configuration.imagePlacement = .trailing
            configuration.imagePadding = 12
        }
        configuration.contentInsets = kind == .plain ? .zero : NSDirectionalEdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 20)
        self.configuration = configuration
        layer.cornerRadius = 8
        layer.cornerCurve = .continuous
        clipsToBounds = true
        translatesAutoresizingMaskIntoConstraints = false
        addAction(UIAction { [weak self] _ in self?.handler() }, for: .primaryActionTriggered)
        configurationUpdateHandler = { [weak self] button in self?.restyle(button) }
        accessibilityLabel = title
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func setTitle(_ title: String) {
        configuration?.title = title
        accessibilityLabel = title
    }

    private func restyle(_ button: UIButton) {
        let enabled = button.isEnabled
        switch kind {
        case .primary:
            backgroundColor = !enabled ? UIKitStyle.raised : button.isHighlighted ? UIKitStyle.actionPress : UIKitStyle.action
            layer.borderWidth = enabled ? 0 : 1
            layer.borderColor = UIKitStyle.border.cgColor
            button.configuration?.baseForegroundColor = enabled ? UIKitStyle.blackout : UIKitStyle.muted
        case .secondary:
            backgroundColor = button.isHighlighted ? UIKitStyle.border.withAlphaComponent(0.55) : UIKitStyle.surface
            layer.borderWidth = 1
            layer.borderColor = UIKitStyle.border.cgColor
            button.configuration?.baseForegroundColor = enabled ? UIKitStyle.text : UIKitStyle.muted
        case .plain:
            backgroundColor = .clear
            button.configuration?.baseForegroundColor = enabled ? UIKitStyle.muted : UIKitStyle.muted.withAlphaComponent(0.5)
        }
    }
}

/// A square icon control with at least a 44pt target.
final class IconButton: UIButton {
    private let handler: () -> Void
    init(icon: String, label: String, size: CGFloat = 20, side: CGFloat = 44, handler: @escaping () -> Void) {
        self.handler = handler
        super.init(frame: .zero)
        setImage(UIKitStyle.icon(icon)?.resized(to: size), for: .normal)
        tintColor = UIKitStyle.text
        accessibilityLabel = label
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: side), heightAnchor.constraint(equalToConstant: side)])
        addAction(UIAction { [weak self] _ in self?.handler() }, for: .primaryActionTriggered)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var isEnabled: Bool { didSet { alpha = isEnabled ? 1 : 0.45 } }
}

extension UIImage {
    func resized(to side: CGFloat) -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            draw(in: CGRect(x: 0, y: 0, width: side, height: side))
        }.withRenderingMode(renderingMode)
    }
}

/// The saved mascot for an account, or its initial (SwiftUI's Avatar). Custom
/// pictures arrive in a later milestone; the mascot keeps its place meanwhile.
enum AvatarImages {
    private static let bundle = Bundle(for: AppModel.self)
    private static var cache: [Int: UIImage] = [:]

    static func image(avatarID: Int?) -> UIImage? {
        guard let index = CaperAvatar.index(for: avatarID) else { return nil }
        if let cached = cache[index] { return cached }
        let image = UIImage(named: "caper-avatar-\(index)", in: bundle, compatibleWith: nil)
        cache[index] = image
        return image
    }
}

final class AvatarView: UIView {
    private let imageView = UIImageView()
    private let initial = UILabel()
    private let side: CGFloat

    init(side: CGFloat) {
        self.side = side
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        layer.cornerRadius = side / 2
        clipsToBounds = true
        backgroundColor = UIKitStyle.raised
        imageView.contentMode = .scaleAspectFit
        imageView.frame = CGRect(x: 0, y: 0, width: side, height: side)
        initial.font = UIKitStyle.font(side * 0.36, .black, scaled: false)
        initial.textColor = UIKitStyle.text
        initial.textAlignment = .center
        initial.frame = imageView.frame
        addSubview(imageView)
        addSubview(initial)
        isAccessibilityElement = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: side), heightAnchor.constraint(equalToConstant: side)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func show(name: String, avatarID: Int?) {
        let image = AvatarImages.image(avatarID: avatarID)
        imageView.image = image
        backgroundColor = image == nil ? UIKitStyle.raised : .clear
        initial.text = image == nil ? String(name.prefix(1)).uppercased() : nil
    }
}

extension NSLayoutConstraint {
    func withPriority(_ priority: UILayoutPriority) -> NSLayoutConstraint {
        self.priority = priority
        return self
    }
}

/// Runs work on the display refresh after the current frame is committed, so
/// it never delays what the reader is about to see.
@MainActor final class AfterFrame: NSObject {
    private var work: (() -> Void)?
    private var link: CADisplayLink?

    static func run(_ work: @escaping () -> Void) {
        let runner = AfterFrame()
        runner.work = work
        let link = CADisplayLink(target: runner, selector: #selector(tick))
        runner.link = link
        link.add(to: .main, forMode: .common)
    }

    @objc private func tick() {
        link?.invalidate()
        link = nil
        work?()
        work = nil
    }
}
