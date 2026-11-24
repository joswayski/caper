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

/// Caper's tokens (shared/themes, via CaperTheme) for UIKit: Obsidian, or Paper
/// while it is the active theme.
enum UIKitStyle {
    static let blackout = CaperTheme.UI.blackout
    static let surface = CaperTheme.UI.surface
    static let raised = CaperTheme.UI.raised
    static let sidebar = CaperTheme.UI.sidebar
    static let conversation = CaperTheme.UI.conversation
    static let composer = CaperTheme.UI.composer
    static let border = CaperTheme.UI.border
    static let text = CaperTheme.UI.text
    /// Message body text, a step under headings (SwiftUI's MessageRow).
    static let body = CaperTheme.themed(0xDEDFE0, 0x24262F)
    static let muted = CaperTheme.UI.muted
    static let link = CaperTheme.UI.link
    static let terracotta = CaperTheme.UI.terracotta
    static let terracottaBright = CaperTheme.UI.terracottaBright
    static let action = CaperTheme.UI.action
    static let actionPress = CaperTheme.UI.actionPress
    /// Ink on a primary (chalk) fill.
    static let onAction = CaperTheme.UI.onAction
    static let error = CaperTheme.UI.error
    static let green = CaperTheme.UI.green

    enum Weight { case regular, medium, bold, black }

    /// Inrond at a fixed size, scaled with the reader's text size.
    static func font(_ size: CGFloat, _ weight: Weight = .regular, scaled: Bool = true) -> UIFont {
        let name: String
        switch weight {
        case .regular: name = "Figtree-Regular"
        case .medium: name = "Figtree-Medium"
        case .bold: name = "Figtree-Bold"
        case .black: name = "Figtree-Black"
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

    /// A hovered or pressed row (macOS's hoverWash).
    static let hoverWash = CaperTheme.hoverWash
    static let pinGold = CaperTheme.UI.pinGold
    static let sidebarRaised = CaperTheme.UI.sidebarRaised

    private static let fractionalDates: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    private static let wholeDates = ISO8601DateFormatter()
    /// A server timestamp, with or without fractional seconds.
    static func date(_ value: String) -> Date? { fractionalDates.date(from: value) ?? wholeDates.date(from: value) }

    static var reduceMotion: Bool { CaperAccessibility.reduceMotion }
    /// shared/design.css --motion-fast and --ease-out.
    static let motionFast: TimeInterval = 0.16
    static let easeOut = UICubicTimingParameters(controlPoint1: CGPoint(x: 0.2, y: 0.8), controlPoint2: CGPoint(x: 0.2, y: 1))
}

/// Web's primary action: eggshell with a blackout label; disabled, it steps
/// back to neutral chrome. Secondary buttons are bordered neutral chrome;
/// compact ones are SwiftUI's CaperSecondaryButton, as on message rows.
final class CaperButton: UIButton {
    enum Kind { case primary, secondary, compact, plain }
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
            attributes.font = UIKitStyle.font(size, kind == .plain ? .regular : kind == .compact ? .medium : .bold)
            return attributes
        }
        if let icon {
            configuration.image = UIKitStyle.icon(icon)?.resized(to: 14)
            configuration.imagePlacement = .trailing
            configuration.imagePadding = 12
        }
        configuration.contentInsets = switch kind {
        case .plain: .zero
        case .compact: NSDirectionalEdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12)
        default: NSDirectionalEdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 20)
        }
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
            button.configuration?.baseForegroundColor = enabled ? UIKitStyle.onAction : UIKitStyle.muted
        case .secondary:
            backgroundColor = button.isHighlighted ? UIKitStyle.border.withAlphaComponent(0.55) : UIKitStyle.surface
            layer.borderWidth = 1
            layer.borderColor = UIKitStyle.border.cgColor
            button.configuration?.baseForegroundColor = enabled ? UIKitStyle.text : UIKitStyle.muted
        case .compact:
            backgroundColor = button.isHighlighted ? UIKitStyle.composer : UIKitStyle.raised
            layer.borderWidth = 1
            layer.borderColor = UIKitStyle.border.cgColor
            button.configuration?.baseForegroundColor = UIKitStyle.text
            alpha = enabled ? 1 : 0.45
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
    /// Hardware-keyboard Tab stops, as web's buttons, whatever Full Keyboard Access says.
    override var canBecomeFocused: Bool { isEnabled && !isHidden }
    /// Smaller buttons still take touches across 44 × 44pt around their centre.
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        bounds.insetBy(dx: min(0, (bounds.width - 44) / 2), dy: min(0, (bounds.height - 44) / 2)).contains(point)
    }
}

extension UIImage {
    func resized(to side: CGFloat) -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            draw(in: CGRect(x: 0, y: 0, width: side, height: side))
        }.withRenderingMode(renderingMode)
    }
}

/// The saved mascot for an account, or its initial (SwiftUI's Avatar).
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

    /// Decoded picture frames as a UIImage: GIFs play unless Reduce Motion is on.
    static func image(_ frames: ProfilePictureFrames, scale: CGFloat) -> UIImage? {
        guard let first = frames.images.first else { return nil }
        guard frames.animated, !UIKitStyle.reduceMotion else { return UIImage(cgImage: first, scale: scale, orientation: .up) }
        let images = frames.images.map { UIImage(cgImage: $0, scale: scale, orientation: .up) }
        return UIImage.animatedImage(with: images, duration: frames.durations.reduce(0, +))
    }
}

/// The same avatar now serves both timeline rows and native profile cards.
typealias AvatarView = UIKitAvatar

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
