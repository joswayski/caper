import AVFoundation
import Combine
import Observation
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// The viewer follows deletion/signature updates without changing its original
/// page order. Only the visible page retains decoded frames or a running player.
@MainActor @Observable final class NativeMediaViewing {
    let request: MediaViewerRequest
    private let close: () -> Void
    private(set) var position: Int
    private(set) var item: ChatAttachment?
    private(set) var image: CGImage?
    private(set) var player: AVPlayer?
    private(set) var failed = false
    private(set) var loading = false
    private(set) var animated = false
    private(set) var playing = false
    private(set) var busy = false
    var status: String?
    var scale: CGFloat = 1
    var offset: CGSize = .zero
    private var active = false
    private var retry = 0
    private var frames: AttachmentOriginal?
    private var loop: LoopingPlayback?
    private var loadTask: Task<Void, Never>?
    private var animationTask: Task<Void, Never>?
    private var actionTask: Task<Void, Never>?
    @ObservationIgnored private var motionObserver: NSObjectProtocol?
    private var motionCenter: NotificationCenter {
        #if os(macOS)
        NSWorkspace.shared.notificationCenter
        #else
        NotificationCenter.default
        #endif
    }

    init(request: MediaViewerRequest, close: @escaping () -> Void) {
        self.request = request; self.close = close
        position = min(max(0, request.start), max(0, request.items.count - 1))
    }
    var items: [ChatAttachment] {
        guard let chat = request.chat else { return request.items }
        return MediaViewerPolicy.latest(request.items, in: chat.messages + chat.pinnedMessages)
    }
    var available: Bool { item.map(MediaViewerPolicy.isViewable) ?? false }
    /// Loading: a video spins over its poster until it can play; an image
    /// only until something shows, since its preview stands in meanwhile.
    var spinning: Bool { available && loading && !failed && (item?.kind == .video || image == nil) }
    func start() {
        active = true
        if motionObserver == nil {
            #if os(macOS)
            let name = NSWorkspace.accessibilityDisplayOptionsDidChangeNotification
            #else
            let name = UIAccessibility.reduceMotionStatusDidChangeNotification
            #endif
            motionObserver = motionCenter.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in
                    guard let self, self.active, self.animated, CaperAccessibility.reduceMotion else { return }
                    self.playing = false; self.player?.pause(); self.animate()
                }
            }
        }
        synchronize()
    }
    func synchronize() {
        guard active else { return }
        guard request.isAvailable?() ?? true else { finish(); return }
        let items = items
        guard items.indices.contains(position) else { finish(); return }
        let next = items[position], old = item
        if next != old { item = next }
        if next.id != old?.id {
            reset(); item = next; load()
        } else if !MediaViewerPolicy.isViewable(next) {
            if image != nil || player != nil || loading || busy || status != nil || failed { reset(); item = next }
        } else if failed, retry == 0, old?.url != next.url {
            retry = 1; load()
        }
    }
    func move(_ delta: Int) {
        let next = MediaViewerPolicy.step(position, by: delta, count: items.count)
        guard next != position else { return }
        position = next; synchronize()
    }
    func finish() { stop(); close() }
    func hideSpoiler() {
        guard let item else { return }
        stop(); request.hideSpoiler(item.id); close()
    }
    func stop() {
        active = false; reset(); item = nil
        if let motionObserver { motionCenter.removeObserver(motionObserver) }; motionObserver = nil
    }
    private func reset() {
        loadTask?.cancel(); animationTask?.cancel(); loadTask = nil; animationTask = nil
        actionTask?.cancel(); actionTask = nil; busy = false; status = nil
        player?.pause(); player = nil; loop?.stop(); loop = nil; frames = nil; image = nil
        loading = false; failed = false; animated = false; playing = false; retry = 0
        scale = 1; offset = .zero
    }
    private func load() {
        guard let item, available else { return }
        loadTask?.cancel(); loading = true; failed = false
        loadTask = Task { [weak self] in
            guard let self else { return }
            let preview = await loadAttachmentImage(item, preview: true, chat: request.chat).image
            guard active, !Task.isCancelled, self.item?.id == item.id, available else { return }
            image = preview
            if item.kind == .image {
                let result = await loadAttachmentOriginal(item, chat: request.chat)
                guard active, !Task.isCancelled, self.item?.id == item.id, available else { return }
                loading = false; failed = result.failed
                if let frames = result.frames {
                    self.frames = frames; image = frames.images.first
                    animated = frames.images.count > 1; playing = animated && !CaperAccessibility.reduceMotion
                    animate()
                } else if result.refusedURL && retry == 0 { request.chat?.requestFreshAttachmentURLs(ids: [item.id]) }
            } else {
                let url = await currentURL(item)
                guard active, !Task.isCancelled, self.item?.id == item.id, available else { return }
                guard let url else { failed = true; loading = false; return }
                player?.pause(); loop?.stop()
                if item.animated {
                    let loop = LoopingPlayback(url: url); self.loop = loop; player = loop.player
                    animated = true; playing = !CaperAccessibility.reduceMotion
                    if playing { player?.play() }
                    let loaded = await loop.loads()
                    guard active, !Task.isCancelled, self.loop === loop else { return }
                    loading = false; failed = !loaded
                } else {
                    let created = AVPlayer(url: url); player = created; created.play()
                    if let current = created.currentItem {
                        // Filtered upstream: an `.unknown` reaching the loop would
                        // use up the sequence's demand and drop `.readyToPlay`.
                        for await status in current.publisher(for: \.status).filter({ $0 != .unknown }).values {
                            guard active, !Task.isCancelled, player === created else { return }
                            loading = false; failed = status == .failed; break
                        }
                    }
                }
                if failed, retry == 0 { request.chat?.requestFreshAttachmentURLs(ids: [item.id]) }
            }
        }
    }
    func toggleAnimation() {
        guard animated else { return }
        playing.toggle()
        if playing { player?.play() } else { player?.pause() }
        animate()
    }
    private func animate() {
        animationTask?.cancel(); animationTask = nil
        guard active, playing, let frames, frames.images.count > 1 else { return }
        animationTask = Task { [weak self] in
            var index = 0
            while !Task.isCancelled {
                guard let self, self.active, self.playing else { return }
                self.image = frames.images[index]
                do { try await Task.sleep(for: .seconds(frames.durations[index])) } catch { return }
                index = (index + 1) % frames.images.count
            }
        }
    }
    private func currentURL(_ item: ChatAttachment) async -> URL? {
        if let chat = request.chat { return await chat.currentURL(for: item) }
        return item.url.flatMap(URL.init(string:))
    }
    func perform(_ action: @escaping (ChatAttachment, URL) async throws -> String?) {
        guard !busy, available, let item else { return }
        busy = true; status = nil
        actionTask = Task { [weak self] in
            guard let self else { return }
            defer { if !Task.isCancelled { self.busy = false } }
            guard let url = await currentURL(item), !Task.isCancelled, active, available, self.item?.id == item.id else { return }
            do {
                let result = try await action(item, url)
                if active, !Task.isCancelled { status = result }
            } catch { if active, !Task.isCancelled { status = "Couldn’t open or save this file. Please try again." } }
        }
    }
    func copyImage() {
        perform { _, url in
            let image = try await AttachmentImageLoader.shared.clipboardImage(url)
            try Task.checkCancellation()
            #if os(macOS)
            NSPasteboard.general.clearContents()
            return NSPasteboard.general.writeObjects([NSImage(cgImage: image, size: .zero)]) ? "Image copied" : "Couldn’t copy image. Try saving it instead."
            #else
            UIPasteboard.general.image = UIImage(cgImage: image)
            return "Image copied"
            #endif
        }
    }
}
