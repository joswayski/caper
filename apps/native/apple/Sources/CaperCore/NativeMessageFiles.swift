import AVFoundation
import CoreImage
import Observation
import QuartzCore
import SwiftUI

/// A realized file owns its decoded preview and player. Hidden spoilers display
/// only blurred stills and never start playback; viewers share the reveal set.
@MainActor private final class NativeFileMedia {
    let image = CALayer()
    let video = AVPlayerLayer()
    var changed: (() -> Void)?
    private(set) var attachment: ChatAttachment?
    private(set) var revealed = false
    private(set) var failed = false
    private(set) var playing = false
    private var chat: ChatModel?
    private var task: Task<Void, Never>?
    private var loop: LoopingPlayback?
    private var end: NSObjectProtocol?
    private var motion: NSObjectProtocol?
    private var frames: AttachmentOriginal?
    private var active = false
    private var motionCenter: NotificationCenter {
        #if os(macOS)
        NSWorkspace.shared.notificationCenter
        #else
        NotificationCenter.default
        #endif
    }
    init() {
        image.contentsGravity = .resizeAspectFill; image.masksToBounds = true
        video.videoGravity = .resizeAspectFill
        #if os(macOS)
        let name = NSWorkspace.accessibilityDisplayOptionsDidChangeNotification
        #else
        let name = UIAccessibility.reduceMotionStatusDidChangeNotification
        #endif
        motion = motionCenter.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.motionChanged() }
        }
    }
    isolated deinit { stop(); motion.map(motionCenter.removeObserver) }
    func update(_ attachment: ChatAttachment, revealed: Bool, chat: ChatModel?) {
        guard self.attachment != attachment || self.revealed != revealed else { return }
        reset(); self.attachment = attachment; self.revealed = revealed; self.chat = chat
        if active { load() }
    }
    func start() { guard !active else { return }; active = true; load() }
    func stop() { active = false; reset() }
    private func reset() {
        task?.cancel(); task = nil; loop?.stop(); loop = nil
        video.player?.pause(); video.player = nil; end.map(NotificationCenter.default.removeObserver); end = nil
        image.removeAllAnimations(); image.contents = nil; frames = nil; failed = false; playing = false
    }
    private func load() {
        guard let attachment, !attachment.unavailable, attachment.status != .failed else { return }
        let chat = chat
        task = Task { [weak self] in
            var preview = await loadAttachmentImage(attachment, preview: true, chat: chat).image
            guard !Task.isCancelled, let self, active else { return }
            if attachment.spoiler && !revealed {
                if preview == nil && attachment.kind == .image {
                    preview = await loadAttachmentImage(attachment, preview: false, chat: chat).image
                }
                guard !Task.isCancelled else { return }
                // Blur pixels before assigning them; no layer transition can
                // expose the decoded still while the spoiler is hidden.
                if let preview {
                    let input = CIImage(cgImage: preview)
                    image.contents = CIContext().createCGImage(input.clampedToExtent().applyingFilter("CIGaussianBlur",
                        parameters: [kCIInputRadiusKey: 32]), from: input.extent)
                }
                changed?(); return
            }
            image.contents = preview
            if attachment.status == .processing {
                if preview == nil, let local = chat?.localCopy(for: attachment.id), AttachmentKind.local(contentType: attachment.contentType) == .image {
                    let still = try? await AttachmentImageLoader.shared.load(key: "local:\(attachment.id)", url: local, maxPixelSize: 1200)
                    guard !Task.isCancelled else { return }; image.contents = still
                }
            } else if attachment.kind == .image {
                let result = await loadAttachmentOriginal(attachment, chat: chat, inline: true)
                guard !Task.isCancelled else { return }
                frames = result.frames; failed = result.failed; image.contents = frames?.images.first ?? preview
                if result.refusedURL { chat?.requestFreshAttachmentURLs(ids: [attachment.id]) }
                animate()
            } else if attachment.kind == .video, attachment.animated, !CaperAccessibility.reduceMotion {
                await playAnimation(attachment)
            }
            changed?()
        }
    }
    private func playAnimation(_ attachment: ChatAttachment) async {
        let url = await currentURL(attachment)
        guard !Task.isCancelled, active, let url else { return }
        let loop = LoopingPlayback(url: url); self.loop = loop; video.player = loop.player
        let loaded = await loop.loads()
        guard !Task.isCancelled, self.loop === loop else { return }
        failed = !loaded
        if loaded, !CaperAccessibility.reduceMotion { playing = true; loop.player.play() }
        else if !loaded { chat?.requestFreshAttachmentURLs(ids: [attachment.id]) }
    }
    func toggleAudio() {
        guard active, let attachment, attachment.kind == .audio else { return }
        if let player = video.player {
            playing.toggle(); if playing { player.play() } else { player.pause() }; changed?(); return
        }
        task?.cancel()
        task = Task { [weak self] in
            guard let self, let url = await currentURL(attachment), !Task.isCancelled, active else { return }
            let player = AVPlayer(url: url); video.player = player; playing = true; player.play(); changed?()
            end = NotificationCenter.default.addObserver(forName: AVPlayerItem.didPlayToEndTimeNotification, object: player.currentItem, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.playing = false; self?.video.player?.seek(to: .zero); self?.changed?() }
            }
        }
    }
    private func currentURL(_ attachment: ChatAttachment) async -> URL? {
        if let chat { return await chat.currentURL(for: attachment) }; return attachment.url.flatMap(URL.init(string:))
    }
    private func motionChanged() {
        guard active, let attachment, !attachment.spoiler || revealed else { return }; animate()
        if CaperAccessibility.reduceMotion { video.player?.pause(); playing = false }
        else if attachment.animated {
            if let loop { playing = true; loop.player.play() }
            else if attachment.kind == .video { task?.cancel(); task = Task { [weak self] in await self?.playAnimation(attachment); self?.changed?() } }
        }
        changed?()
    }
    private func animate() {
        image.removeAnimation(forKey: "frames")
        guard let frames, frames.images.count > 1, frames.durations.count == frames.images.count, !CaperAccessibility.reduceMotion else { return }
        let total = frames.durations.reduce(0, +); guard total > 0 else { return }
        let animation = CAKeyframeAnimation(keyPath: "contents"); animation.values = frames.images + [frames.images[0]]
        var elapsed = 0.0
        animation.keyTimes = frames.durations.map { duration in defer { elapsed += duration }; return NSNumber(value: elapsed / total) } + [1]
        animation.duration = total; animation.repeatCount = .infinity; animation.calculationMode = .discrete
        image.add(animation, forKey: "frames")
    }
}

private enum NativeSentFiles {
    static func frames(_ files: [ChatAttachment], width: CGFloat, height: (String, CGFloat) -> CGFloat) -> [CGRect] {
        let width = max(1, width), visuals = files.filter { $0.kind == .image || $0.kind == .video }
        var byID: [String: CGRect] = [:], y: CGFloat = 0
        if visuals.count > 1 {
            let frames = SentGalleryGeometry.frames(count: visuals.count, width: width)
            for (file, frame) in zip(visuals, frames) { byID[file.id] = frame }; y = (frames.map(\.maxY).max() ?? 0) + 6
        } else if let file = visuals.first {
            let size = AttachmentPolicy.displaySize(width: file.width, height: file.height) ?? (width: 240.0, height: 180.0)
            let w = min(width, size.width), h = w * size.height / max(1, size.width)
            byID[file.id] = CGRect(x: 0, y: 0, width: w, height: h); y = h + 6
        }
        for file in files where file.voice && file.kind != .image && file.kind != .video {
            let w = min(width, 320), h = height(file.id, w)
            byID[file.id] = CGRect(x: 0, y: y, width: w, height: h); y += h + 6
        }
        let columns = max(1, Int((width + 6) / 226))
        let cardWidth = min(360, max(1, (width - CGFloat(columns - 1) * 6) / CGFloat(columns)))
        var rowHeight: CGFloat = 0
        for (index, file) in files.filter({ !$0.voice && $0.kind != .image && $0.kind != .video }).enumerated() {
            if index > 0 && index % columns == 0 { y += rowHeight + 6; rowHeight = 0 }
            let h = height(file.id, cardWidth)
            byID[file.id] = CGRect(x: CGFloat(index % columns) * (cardWidth + 6), y: y, width: cardWidth, height: h)
            rowHeight = max(rowHeight, h)
        }
        return files.map { byID[$0.id] ?? .zero }
    }
    @MainActor static func label(_ file: ChatAttachment, revealed: Bool, failed: Bool, chat: ChatModel?) -> String {
        if file.spoiler && !revealed { return "Spoiler" }
        if file.unavailable { return "File removed" }
        if file.status == .failed { return "Processing failed" }
        if file.status == .processing { return chat?.attachmentProgress[file.id].map { "Processing… \($0)%" } ?? "Processing…" }
        if file.url == nil { return "File unavailable" }
        if failed { return file.kind == .image ? "Image unavailable" : "Video unavailable" }
        if file.kind == .image { return "" }
        if file.kind == .video { return file.animated && !CaperAccessibility.reduceMotion ? "" : "▶" }
        return "\(file.name)\n\(AttachmentPolicy.formatBytes(file.size))"
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitMessageFiles: NSView {
    var heightChanged: (() -> Void)?
    private let chat: ChatModel?
    private let available: (() -> Bool)?
    private let show: (MediaViewerRequest) -> Void
    private var files: [ChatAttachment] = []
    private var tiles: [String: AppKitSentFile] = [:]
    private var revealed: Set<String> = []
    private var lastHeight: CGFloat = 0
    override var isFlipped: Bool { true }
    init(chat: ChatModel?, available: (() -> Bool)? = nil, show: @escaping (MediaViewerRequest) -> Void) {
        self.chat = chat; self.available = available; self.show = show; super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ files: [ChatAttachment]) {
        self.files = files; let ids = Set(files.map(\.id)); revealed.formIntersection(ids)
        for id in tiles.keys.filter({ !ids.contains($0) }) { tiles.removeValue(forKey: id)?.removeFromSuperview() }
        for file in files {
            let tile = tiles[file.id] ?? AppKitSentFile(chat: chat, activate: { [weak self] in self?.open(file.id) }, hide: { [weak self] in self?.hide(file.id) })
            if tiles[file.id] == nil {
                tiles[file.id] = tile; addSubview(tile)
                tile.available = available
                tile.heightChanged = { [weak self] in self?.needsLayout = true; self?.invalidateIntrinsicContentSize(); self?.heightChanged?() }
            }
            tile.update(file, revealed: revealed.contains(file.id))
        }
        needsLayout = true; invalidateIntrinsicContentSize()
    }
    private func hide(_ id: String) { revealed.remove(id); update(files) }
    private func open(_ id: String) {
        guard available?() != false, let file = files.first(where: { $0.id == id }) else { return }
        if file.spoiler && !revealed.contains(id) { revealed.insert(id); update(files); return }
        if file.kind == .audio { tiles[id]?.toggleAudio(); return }
        let items = MediaViewerPolicy.items(files, revealedSpoilerIDs: revealed)
        if let start = items.firstIndex(where: { $0.id == id }) { show(MediaViewerRequest(items: items, start: start, chat: chat, hideSpoiler: { [weak self] in self?.hide($0) }, isAvailable: available)) }
        else { Task { [chat] in
            let url: URL?; if let chat { url = await chat.currentURL(for: file) } else { url = file.url.flatMap(URL.init(string:)) }
            if let url, available?() != false { NSWorkspace.shared.open(url) }
        } }
    }
    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: frames.map(\.maxY).max() ?? 0)
    }
    private var frames: [CGRect] { NativeSentFiles.frames(files, width: bounds.width) { tiles[$0]?.height(for: $1) ?? 64 } }
    override func layout() {
        super.layout(); for (file, frame) in zip(files, frames) { tiles[file.id]?.frame = frame }
        let height = intrinsicContentSize.height
        if height != lastHeight { lastHeight = height; invalidateIntrinsicContentSize(); DispatchQueue.main.async { [weak self] in self?.heightChanged?() } }
    }
}

@MainActor private final class AppKitSentFile: NSView {
    var heightChanged: (() -> Void)?
    var available: (() -> Bool)?
    private var voice: AppKitVoiceMessage?
    private let media = NativeFileMedia()
    private let chat: ChatModel?
    private let text = AppKitKit.label("", size: 12)
    private let open = AppKitMediaHitTarget()
    private let hide: AppKitKitButton
    private var file: ChatAttachment?
    private var revealed = false
    private let observation = AppKitObservation()
    override var isFlipped: Bool { true }
    init(chat: ChatModel?, activate: @escaping () -> Void, hide: @escaping () -> Void) {
        self.chat = chat; self.hide = AppKitKitButton("Hide spoiler", kind: .plain, action: hide)
        super.init(frame: .zero); wantsLayer = true; layer?.cornerRadius = 8; layer?.masksToBounds = true
        open.handler = activate
        layer?.backgroundColor = AppKitKit.surface.cgColor; layer?.borderWidth = 1; layer?.borderColor = AppKitKit.border.cgColor
        layer?.addSublayer(media.image); layer?.addSublayer(media.video); text.alignment = .center
        addSubview(text); addSubview(open); addSubview(self.hide)
        open.translatesAutoresizingMaskIntoConstraints = true; self.hide.translatesAutoresizingMaskIntoConstraints = true
        media.changed = { [weak self] in self?.render() }; setAccessibilityElement(false)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ file: ChatAttachment, revealed: Bool) {
        self.file = file; self.revealed = revealed
        if file.voice && file.kind == .audio && file.status == .ready && !file.unavailable && file.url != nil && (!file.spoiler || revealed) {
            if voice == nil {
                let voice = AppKitVoiceMessage(file: file, chat: chat, available: available); self.voice = voice
                addSubview(voice, positioned: .below, relativeTo: hide)
                voice.heightChanged = { [weak self] in self?.heightChanged?() }
            }
            voice?.update(file); media.stop()
        } else { voice?.removeFromSuperview(); voice = nil; media.update(file, revealed: revealed, chat: chat); if window != nil { media.start() } }
        render()
    }
    func height(for width: CGFloat) -> CGFloat {
        if let voice { return voice.height(for: width) }
        if file?.spoiler == true && !revealed { return 80 }
        text.preferredMaxLayoutWidth = max(1, width - 20)
        return max(64, text.fittingSize.height + 24)
    }
    func toggleAudio() { media.toggleAudio() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { if voice == nil { media.start() }; observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { media.stop(); observation.stop() }
    }
    private func render() {
        guard let file else { return }
        text.stringValue = NativeSentFiles.label(file, revealed: revealed, failed: media.failed, chat: chat)
        text.isHidden = voice != nil; open.isHidden = voice != nil
        layer?.backgroundColor = voice == nil ? AppKitKit.surface.cgColor : nil; layer?.borderWidth = voice == nil ? 1 : 0
        hide.isHidden = !file.spoiler || !revealed
        open.isEnabled = file.spoiler && !revealed || (!file.unavailable && file.status == .ready && file.url != nil)
        open.setAccessibilityLabel(file.spoiler && !revealed ? "Reveal spoiler attachment" : file.kind == .audio ? "\(media.playing ? "Pause" : "Play") \(file.name)" : "Open \(file.name)")
        open.setAccessibilityValue(text.stringValue)
        open.setAccessibilityIdentifier("attachment-\(file.id)"); text.setAccessibilityElement(false)
        needsLayout = true
    }
    override func layout() {
        super.layout(); CATransaction.begin(); CATransaction.setDisableActions(true)
        media.image.frame = bounds; media.video.frame = bounds; CATransaction.commit()
        voice?.frame = bounds
        open.frame = bounds; text.preferredMaxLayoutWidth = max(1, bounds.width - 20)
        text.frame = NSRect(x: 10, y: (bounds.height - text.fittingSize.height) / 2, width: max(1, bounds.width - 20), height: text.fittingSize.height)
        hide.frame = NSRect(x: max(0, bounds.width - 110), y: 4, width: min(106, bounds.width), height: 30)
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitMessageFiles: UIView {
    var heightChanged: (() -> Void)?
    private let chat: ChatModel?
    private let available: (() -> Bool)?
    private let show: (MediaViewerRequest) -> Void
    private var files: [ChatAttachment] = []
    private var tiles: [String: UIKitSentFile] = [:]
    private var revealed: Set<String> = []
    private var lastHeight: CGFloat = 0
    init(chat: ChatModel?, available: (() -> Bool)? = nil, show: @escaping (MediaViewerRequest) -> Void) {
        self.chat = chat; self.available = available; self.show = show; super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ files: [ChatAttachment]) {
        self.files = files; let ids = Set(files.map(\.id)); revealed.formIntersection(ids)
        for id in tiles.keys.filter({ !ids.contains($0) }) { tiles.removeValue(forKey: id)?.removeFromSuperview() }
        for file in files {
            let tile = tiles[file.id] ?? UIKitSentFile(chat: chat, activate: { [weak self] in self?.open(file.id) }, hide: { [weak self] in self?.hide(file.id) })
            if tiles[file.id] == nil {
                tiles[file.id] = tile; addSubview(tile)
                tile.available = available
                tile.heightChanged = { [weak self] in self?.setNeedsLayout(); self?.invalidateIntrinsicContentSize(); self?.heightChanged?() }
            }
            tile.update(file, revealed: revealed.contains(file.id))
        }
        setNeedsLayout(); invalidateIntrinsicContentSize()
    }
    private func hide(_ id: String) { revealed.remove(id); update(files) }
    private func open(_ id: String) {
        guard !SwipeTapGuard.swiping, available?() != false, let file = files.first(where: { $0.id == id }) else { return }
        if file.spoiler && !revealed.contains(id) { revealed.insert(id); update(files); return }
        if file.kind == .audio { tiles[id]?.toggleAudio(); return }
        let items = MediaViewerPolicy.items(files, revealedSpoilerIDs: revealed)
        if let start = items.firstIndex(where: { $0.id == id }) { show(MediaViewerRequest(items: items, start: start, chat: chat, hideSpoiler: { [weak self] in self?.hide($0) }, isAvailable: available)) }
        else { Task { [chat] in
            let url: URL?; if let chat { url = await chat.currentURL(for: file) } else { url = file.url.flatMap(URL.init(string:)) }
            if let url, available?() != false { UIApplication.shared.open(url) }
        } }
    }
    override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: frames.map(\.maxY).max() ?? 0) }
    private var frames: [CGRect] { NativeSentFiles.frames(files, width: bounds.width) { tiles[$0]?.height(for: $1) ?? 64 } }
    override func layoutSubviews() {
        super.layoutSubviews(); for (file, frame) in zip(files, frames) { tiles[file.id]?.frame = frame }
        let height = intrinsicContentSize.height
        if height != lastHeight { lastHeight = height; invalidateIntrinsicContentSize(); heightChanged?() }
    }
}

@MainActor private final class UIKitSentFile: UIView {
    var heightChanged: (() -> Void)?
    var available: (() -> Bool)?
    private var voice: UIKitVoiceMessage?
    private let media = NativeFileMedia()
    private let chat: ChatModel?
    private let text = UIKitMessageDialog.label("")
    private let open = UIButton(type: .custom), hide: UIButton
    private var file: ChatAttachment?
    private var revealed = false
    private var generation = 0
    init(chat: ChatModel?, activate: @escaping () -> Void, hide: @escaping () -> Void) {
        self.chat = chat; self.hide = UIKitMessageDialog.button("Hide spoiler", action: hide)
        super.init(frame: .zero); layer.cornerRadius = 8; clipsToBounds = true
        backgroundColor = UIColor(CaperTheme.surface); layer.borderWidth = 1; layer.borderColor = UIColor(CaperTheme.border).cgColor
        layer.addSublayer(media.image); layer.addSublayer(media.video); text.textAlignment = .center
        addSubview(text); addSubview(open); addSubview(self.hide)
        open.addAction(UIAction { _ in activate() }, for: .touchUpInside); media.changed = { [weak self] in self?.render() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(_ file: ChatAttachment, revealed: Bool) {
        self.file = file; self.revealed = revealed
        if file.voice && file.kind == .audio && file.status == .ready && !file.unavailable && file.url != nil && (!file.spoiler || revealed) {
            if voice == nil {
                let voice = UIKitVoiceMessage(file: file, chat: chat, available: available); self.voice = voice
                insertSubview(voice, belowSubview: hide)
                voice.heightChanged = { [weak self] in self?.heightChanged?() }
            }
            voice?.update(file); media.stop()
        } else { voice?.removeFromSuperview(); voice = nil; media.update(file, revealed: revealed, chat: chat); if window != nil { media.start() } }
        render()
    }
    func height(for width: CGFloat) -> CGFloat {
        if let voice { return voice.height(for: width) }
        if file?.spoiler == true && !revealed { return 80 }
        return max(64, text.sizeThatFits(CGSize(width: max(1, width - 20), height: .greatestFiniteMagnitude)).height + 24)
    }
    func toggleAudio() { media.toggleAudio() }
    override func didMoveToWindow() { super.didMoveToWindow(); generation += 1; if window != nil { if voice == nil { media.start() }; observe(generation) } else { media.stop() } }
    private func observe(_ token: Int) { guard window != nil, token == generation else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } } }
    private func render() {
        guard let file else { return }
        text.text = NativeSentFiles.label(file, revealed: revealed, failed: media.failed, chat: chat); text.isAccessibilityElement = false
        text.isHidden = voice != nil; open.isHidden = voice != nil
        backgroundColor = voice == nil ? UIColor(CaperTheme.surface) : .clear; layer.borderWidth = voice == nil ? 1 : 0
        hide.isHidden = !file.spoiler || !revealed
        open.isEnabled = file.spoiler && !revealed || (!file.unavailable && file.status == .ready && file.url != nil)
        open.accessibilityLabel = file.spoiler && !revealed ? "Reveal spoiler attachment" : file.kind == .audio ? "\(media.playing ? "Pause" : "Play") \(file.name)" : "Open \(file.name)"
        open.accessibilityValue = text.text
        open.accessibilityIdentifier = "attachment-\(file.id)"; setNeedsLayout()
    }
    override func layoutSubviews() {
        super.layoutSubviews(); CATransaction.begin(); CATransaction.setDisableActions(true)
        media.image.frame = bounds; media.video.frame = bounds; CATransaction.commit()
        voice?.frame = bounds
        open.frame = bounds; let size = text.sizeThatFits(CGSize(width: max(1, bounds.width - 20), height: .greatestFiniteMagnitude))
        text.frame = CGRect(x: 10, y: (bounds.height - size.height) / 2, width: max(1, bounds.width - 20), height: size.height)
        hide.frame = CGRect(x: max(0, bounds.width - 130), y: 4, width: min(126, bounds.width), height: 44)
    }
}
#endif
