import AppKit
import CaperCore

/// Only realized rows retain decoded frames. Core Animation plays GIFs without
/// a per-frame main-thread timer; Reduce Motion keeps their first frame.
@MainActor final class NativeAvatar: NativeView {
    private let saved = NSImageView()
    private let initial = NativeStyle.label("", size: 11, bold: true)
    private let pictureLayer = CALayer()
    private var member: Member?
    private var requestID: String?
    private var frames: ProfilePictureFrames?
    private var loading: Task<Void, Never>?

    init() {
        super.init(NSColor(CaperTheme.raised))
        layer?.masksToBounds = true
        saved.imageScaling = .scaleProportionallyUpOrDown
        initial.alignment = .center
        addSubview(saved); addSubview(initial)
        pictureLayer.contentsGravity = .resizeAspectFill
        layer?.addSublayer(pictureLayer)
        setAccessibilityElement(false)
        saved.setAccessibilityElement(false); initial.setAccessibilityElement(false)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(motionChanged),
            name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { loading?.cancel(); NSWorkspace.shared.notificationCenter.removeObserver(self) }

    func update(_ member: Member) {
        guard self.member != member else { return }
        self.member = member
        saved.image = CaperAvatar.image(for: member.avatarId)
        initial.stringValue = String(member.displayName.prefix(1)).uppercased()
        initial.isHidden = saved.image != nil
        refreshPicture()
    }

    override func layout() {
        super.layout()
        saved.frame = bounds
        initial.frame = NSRect(x: 0, y: (bounds.height - 17) / 2, width: bounds.width, height: 17)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        pictureLayer.frame = bounds
        layer?.cornerRadius = frames == nil ? bounds.width / 2 : ProfilePicturePolicy.cornerRadius(side: bounds.width)
        CATransaction.commit()
        refreshPicture()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            loading?.cancel(); loading = nil; requestID = nil; frames = nil
            showPicture()
        } else { refreshPicture() }
    }
    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        refreshPicture()
    }

    private func refreshPicture() {
        guard let picture = member?.avatarPicture, let window, bounds.width > 0 else {
            loading?.cancel(); loading = nil; requestID = nil; frames = nil
            showPicture()
            return
        }
        let pixels = ProfilePicturePolicy.displayPixelSize(points: bounds.width, scale: window.backingScaleFactor)
        let key = ProfilePictureLoader.key(asset: picture.asset, pixelSize: pixels)
        let request = "\(key):\(picture.url.absoluteString)"
        guard requestID != request else { return }
        loading?.cancel(); requestID = request
        frames = ProfilePictureLoader.shared.cached(key)
        showPicture()
        guard frames == nil else { return }
        loading = Task { [weak self] in
            let frames = await ProfilePictureLoader.shared.load(asset: picture.asset, url: picture.url, pixelSize: pixels)
            guard !Task.isCancelled, let self, self.requestID == request else { return }
            self.frames = frames
            self.showPicture()
        }
    }

    @objc private func motionChanged() { showPicture() }
    private func showPicture() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        pictureLayer.removeAnimation(forKey: "frames")
        pictureLayer.contents = frames?.images.first
        pictureLayer.isHidden = frames?.images.first == nil
        saved.isHidden = !pictureLayer.isHidden
        initial.isHidden = !pictureLayer.isHidden || saved.image != nil
        layer?.cornerRadius = pictureLayer.isHidden ? bounds.width / 2 : ProfilePicturePolicy.cornerRadius(side: bounds.width)
        if let frames, frames.animated, window != nil, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            let total = frames.durations.reduce(0, +)
            if total > 0 {
                var elapsed = 0.0
                let animation = CAKeyframeAnimation(keyPath: "contents")
                animation.values = frames.images
                animation.keyTimes = frames.durations.map { duration in
                    defer { elapsed += duration }
                    return NSNumber(value: elapsed / total)
                } + [1] // Discrete segments need an end time as well as each frame's start.
                animation.calculationMode = .discrete
                animation.duration = total
                animation.repeatCount = .infinity
                pictureLayer.add(animation, forKey: "frames")
            }
        }
        CATransaction.commit()
    }
}
