#if os(iOS)
import UIKit

/// Shared by the UIKit timeline and native profile cards.
@MainActor public final class UIKitAvatar: UIView {
    private let imageView = UIImageView()
    private let initial = UILabel()
    private let side: CGFloat
    private var pictureKey: String?
    /// Held while shown, so the loader finds it again for other rows.
    private var frames: ProfilePictureFrames?
    private var loading: Task<Void, Never>?

    public init(side: CGFloat, corner: CGFloat? = nil) {
        self.side = side
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        layer.cornerRadius = corner ?? side / 2; layer.cornerCurve = .continuous
        clipsToBounds = true; backgroundColor = UIColor(CaperTheme.raised)
        imageView.contentMode = .scaleAspectFit
        imageView.frame = CGRect(x: 0, y: 0, width: side, height: side)
        initial.font = UIFont(name: "Figtree-Black", size: side * 0.36) ?? .systemFont(ofSize: side * 0.36)
        initial.textColor = UIColor(CaperTheme.text); initial.textAlignment = .center
        initial.frame = imageView.frame
        addSubview(imageView); addSubview(initial)
        isAccessibilityElement = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: side), heightAnchor.constraint(equalToConstant: side)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    public func show(name: String, avatarID: Int?, picture: CaperPicture? = nil) {
        let scale = window?.screen.scale ?? UIScreen.main.scale
        let pixelSize = ProfilePicturePolicy.displayPixelSize(points: side, scale: scale)
        let key = picture.map { ProfilePictureLoader.key(asset: $0.asset, pixelSize: pixelSize) }
        if let key, key == pictureKey, imageView.contentMode == .scaleAspectFill { return }
        loading?.cancel(); loading = nil; pictureKey = key
        if let key, let frames = ProfilePictureLoader.shared.cached(key), let image = Self.image(frames, scale: scale) {
            showPicture(image, frames: frames); return
        }
        frames = nil
        let mascot = CaperAvatar.image(for: avatarID)
        imageView.contentMode = .scaleAspectFit; imageView.image = mascot
        backgroundColor = mascot == nil ? UIColor(CaperTheme.raised) : .clear
        initial.text = mascot == nil ? String(name.prefix(1)).uppercased() : nil
        guard let picture, let key else { return }
        loading = Task { @MainActor [weak self] in
            guard let frames = await ProfilePictureLoader.shared.load(asset: picture.asset, url: picture.url, pixelSize: pixelSize),
                  !Task.isCancelled, let self, self.pictureKey == key, let image = Self.image(frames, scale: scale) else { return }
            self.showPicture(image, frames: frames)
        }
    }

    private func showPicture(_ image: UIImage, frames: ProfilePictureFrames) {
        self.frames = frames
        imageView.contentMode = .scaleAspectFill; imageView.image = image
        backgroundColor = UIColor(CaperTheme.raised); initial.text = nil
    }
    private static func image(_ frames: ProfilePictureFrames, scale: CGFloat) -> UIImage? {
        guard let first = frames.images.first else { return nil }
        guard frames.animated, !CaperAccessibility.reduceMotion else { return UIImage(cgImage: first, scale: scale, orientation: .up) }
        return UIImage.animatedImage(with: frames.images.map { UIImage(cgImage: $0, scale: scale, orientation: .up) },
                                     duration: frames.durations.reduce(0, +))
    }
}
#endif
