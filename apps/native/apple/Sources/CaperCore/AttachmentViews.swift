import AVKit
import Combine
import ImageIO
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

// MARK: Image loading

enum AttachmentLoadError: Error { case status(Int), undecodable }

private final class CachedAttachmentImage: NSObject {
    let image: CGImage
    init(_ image: CGImage) { self.image = image }
}

/// Decoded attachment images cached by attachment id and variant, never by
/// URL, so refreshed signatures reuse what is already on screen. Decoding is
/// ImageIO, which reads WebP (lossy and lossless), AVIF (since iOS 16/macOS
/// 13) and local HEIC originals; URLSession transparently removes any
/// `Content-Encoding: gzip`.
final class AttachmentImageLoader: @unchecked Sendable {
    static let shared = AttachmentImageLoader()
    private let cache = NSCache<NSString, CachedAttachmentImage>()
    private let session: URLSession

    init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieAcceptPolicy = .never
        configuration.httpShouldSetCookies = false
        session = URLSession(configuration: configuration)
        cache.totalCostLimit = 160 * 1024 * 1024
    }

    func cached(_ key: String) -> CGImage? { cache.object(forKey: key as NSString)?.image }

    private func fetch(_ url: URL) async throws -> Data {
        if url.isFileURL { return try Data(contentsOf: url) }
        let (body, response) = try await session.data(from: url)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) { throw AttachmentLoadError.status(http.statusCode) }
        return body
    }

    func load(key: String, url: URL, maxPixelSize: Int) async throws -> CGImage {
        if let hit = cached(key) { return hit }
        let data = try await fetch(url)
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { throw AttachmentLoadError.undecodable }
        cache.setObject(CachedAttachmentImage(image), forKey: key as NSString, cost: image.bytesPerRow * image.height)
        return image
    }

    /// Animations are decoded whole for the viewer within these limits;
    /// larger ones show their first frame.
    private static let maxAnimationFrames = 600
    private static let maxAnimationBytes = 192 * 1024 * 1024

    /// The media viewer's original: every frame of an animated GIF, WebP or
    /// PNG, or one still (cached like `load`).
    func loadFrames(key: String, url: URL, maxPixelSize: Int) async throws -> AttachmentOriginal {
        if let hit = cached(key) { return AttachmentOriginal(images: [hit], durations: []) }
        let data = try await fetch(url)
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { throw AttachmentLoadError.undecodable }
        let count = CGImageSourceGetCount(source)
        if count > 1, count <= Self.maxAnimationFrames, Self.frameDelay(source, at: 0) != nil {
            let frameOptions: [CFString: Any] = [kCGImageSourceShouldCacheImmediately: true]
            var images: [CGImage] = []
            var durations: [Double] = []
            var bytes = 0
            for index in 0..<count {
                try Task.checkCancellation()
                guard let image = CGImageSourceCreateImageAtIndex(source, index, frameOptions as CFDictionary) else { break }
                bytes += image.bytesPerRow * image.height
                guard bytes <= Self.maxAnimationBytes else { break }
                images.append(image)
                durations.append(MediaViewerPolicy.frameDuration(Self.frameDelay(source, at: index)))
            }
            if images.count == count { return AttachmentOriginal(images: images, durations: durations) }
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { throw AttachmentLoadError.undecodable }
        cache.setObject(CachedAttachmentImage(image), forKey: key as NSString, cost: image.bytesPerRow * image.height)
        return AttachmentOriginal(images: [image], durations: [])
    }

    /// A frame's GIF, WebP or APNG delay in seconds; nil when the image is
    /// not one of those animations (such as a multi-picture JPEG).
    private static func frameDelay(_ source: CGImageSource, at index: Int) -> Double? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [String: Any] else { return nil }
        let formats: [(dictionary: CFString, unclamped: CFString, clamped: CFString)] = [
            (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFUnclampedDelayTime, kCGImagePropertyGIFDelayTime),
            (kCGImagePropertyWebPDictionary, kCGImagePropertyWebPUnclampedDelayTime, kCGImagePropertyWebPDelayTime),
            (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGUnclampedDelayTime, kCGImagePropertyAPNGDelayTime),
        ]
        for format in formats {
            guard let values = properties[format.dictionary as String] as? [String: Any] else { continue }
            if let delay = (values[format.unclamped as String] as? Double) ?? (values[format.clamped as String] as? Double) { return delay }
        }
        return nil
    }

    /// Downloads a file under its name for the share sheet, into a folder
    /// that keeps only the latest share: each one clears the last, so
    /// shared originals don't pile up in temporary storage.
    func download(_ url: URL, name: String) async throws -> URL {
        let (location, response) = try await session.download(from: url)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            try? FileManager.default.removeItem(at: location)
            throw AttachmentLoadError.status(http.statusCode)
        }
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("caper-shared", isDirectory: true)
        try? FileManager.default.removeItem(at: folder)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let destination = folder.appendingPathComponent(AttachmentStaging.sanitized(name))
        try FileManager.default.moveItem(at: location, to: destination)
        return destination
    }
}

/// The media viewer's original image: one still, or each frame of an
/// animation with how long it shows (`durations` is empty for a still).
/// Decoded images are immutable, so it crosses from the loader as they do.
struct AttachmentOriginal: @unchecked Sendable {
    let images: [CGImage]
    let durations: [Double]
}

/// Loads one variant of an attachment image, asking the chat model for fresh
/// URLs when the signature is expiring or the CDN refused it.
@MainActor
private func loadAttachmentImage(_ attachment: ChatAttachment, preview: Bool, chat: ChatModel?, maxPixelSize: Int = 1200) async -> (image: CGImage?, failed: Bool) {
    let usePreview = preview && attachment.previewUrl != nil
    let key = "\(attachment.id):\(usePreview ? "preview" : "full")"
    if let cached = AttachmentImageLoader.shared.cached(key) { return (cached, false) }
    guard let raw = usePreview ? attachment.previewUrl : attachment.url, let url = URL(string: raw) else { return (nil, true) }
    if AttachmentURLPolicy.needsRefresh(raw) { chat?.requestFreshAttachmentURLs(ids: [attachment.id]) }
    do {
        let image = try await AttachmentImageLoader.shared.load(key: key, url: url, maxPixelSize: maxPixelSize)
        return (image, false)
    } catch AttachmentLoadError.status(let status) where AttachmentURLPolicy.refreshesAfterFailure(status: status) {
        // The view retries once when the refreshed URL arrives.
        chat?.requestFreshAttachmentURLs(ids: [attachment.id])
        return (nil, true)
    } catch {
        return (nil, !(error is CancellationError) && !Task.isCancelled)
    }
}

/// The media viewer's original (every frame of an animation), refreshed
/// first when its signature is expiring, like opening a file. `refusedURL`
/// means a fresh signature could fix the failure.
@MainActor
private func loadAttachmentOriginal(_ attachment: ChatAttachment, chat: ChatModel?) async -> (frames: AttachmentOriginal?, failed: Bool, refusedURL: Bool) {
    let url: URL?
    if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
    guard let url else { return (nil, true, false) }
    do {
        let frames = try await AttachmentImageLoader.shared.loadFrames(key: "\(attachment.id):original", url: url, maxPixelSize: 4096)
        return (frames, false, false)
    } catch AttachmentLoadError.status(let status) where AttachmentURLPolicy.refreshesAfterFailure(status: status) {
        return (nil, true, true)
    } catch {
        return (nil, !(error is CancellationError) && !Task.isCancelled, false)
    }
}

// MARK: Message attachments

/// Files under a message's text. `chat` is nil for the optimistic pending
/// row, whose attachments point at local copies, and for another
/// conversation's files. Below a `MediaViewerHost`, images and videos open
/// the in-app viewer; the pending row's local copies still open on their own.
struct MessageAttachmentsView: View {
    let attachments: [ChatAttachment]
    let chat: ChatModel?
    @Environment(\.mediaViewer) private var mediaViewer

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(attachments) { attachment in
                if attachment.unavailable {
                    AttachmentFileCard(attachment: attachment, action: nil)
                } else if attachment.status == .failed {
                    AttachmentFailedCard(attachment: attachment)
                } else if attachment.status == .processing {
                    AttachmentProcessingView(attachment: attachment, chat: chat)
                } else if attachment.url == nil {
                    AttachmentFileCard(attachment: attachment, action: nil)
                } else {
                    switch attachment.kind {
                    case .image: AttachmentImageView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
                    case .video:
                        if attachment.animated {
                            AttachmentAnimationView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
                        } else {
                            AttachmentVideoView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
                        }
                    case .audio: AttachmentAudioView(attachment: attachment, chat: chat)
                    case .file: AttachmentOpenCard(attachment: attachment, chat: chat)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Opens this message's images and videos at `attachment`; nil keeps the
    /// file's own behavior (no viewer here, or not a viewable file).
    private func viewer(_ attachment: ChatAttachment) -> (() -> Void)? {
        guard let mediaViewer else { return nil }
        let items = MediaViewerPolicy.items(attachments)
        guard let start = items.firstIndex(where: { $0.id == attachment.id }) else { return nil }
        return { mediaViewer(MediaViewerRequest(items: items, start: start, chat: chat)) }
    }
}

@MainActor
private func openAttachment(_ attachment: ChatAttachment, chat: ChatModel?, openURL: OpenURLAction) {
    Task { @MainActor in
        let url: URL?
        if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
        if let url { openURL(url) }
    }
}

private struct AttachmentFrame: ViewModifier {
    let attachment: ChatAttachment
    let fallback: (width: Double, height: Double)

    func body(content: Content) -> some View {
        let size = AttachmentPolicy.displaySize(width: attachment.width, height: attachment.height) ?? fallback
        // Reserve the final aspect ratio before anything loads.
        return Color.clear
            .aspectRatio(CGSize(width: size.width, height: size.height), contentMode: .fit)
            .overlay { content }
            .background(CaperTheme.surface)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .frame(maxWidth: size.width, alignment: .leading)
    }
}

private struct AttachmentImageView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    /// Opens the media viewer; nil opens the file's URL.
    let openViewer: (() -> Void)?
    @Environment(\.openURL) private var openURL
    @State private var image: CGImage?
    @State private var failed = false

    var body: some View {
        Button {
            if let openViewer { openViewer() } else { openAttachment(attachment, chat: chat, openURL: openURL) }
        } label: {
            ZStack {
                if let image {
                    Image(decorative: image, scale: 1).resizable().scaledToFill()
                } else if failed {
                    VStack(spacing: 4) {
                        Image(systemName: "photo").font(.system(size: 18))
                        Text("Image unavailable").font(CaperTheme.font(11))
                    }.foregroundStyle(CaperTheme.muted)
                } else {
                    ProgressView().controlSize(.small)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .modifier(AttachmentFrame(attachment: attachment, fallback: (240, 180)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Image \(attachment.name)")
        .accessibilityHint(openViewer == nil ? "Opens the full-size image" : "Opens the image viewer")
        .accessibilityIdentifier("attachment-\(attachment.id)")
        .task(id: attachment.previewUrl ?? attachment.url) {
            let result = await loadAttachmentImage(attachment, preview: true, chat: chat)
            if let loaded = result.image { image = loaded; failed = false } else if image == nil { failed = result.failed }
        }
    }
}

private struct AttachmentVideoView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    /// Opens the media viewer, which plays it; nil plays it inline.
    let openViewer: (() -> Void)?
    @Environment(\.openURL) private var openURL
    @State private var player: AVPlayer?
    @State private var poster: CGImage?
    @State private var starting = false

    var body: some View {
        ZStack {
            if let player {
                VideoPlayer(player: player)
            } else {
                Button { if let openViewer { openViewer() } else { start() } } label: {
                    ZStack {
                        if let poster { Image(decorative: poster, scale: 1).resizable().scaledToFill() }
                        Image(systemName: "play.fill").font(.system(size: 18, weight: .bold)).foregroundStyle(.white)
                            .frame(width: 46, height: 46).background(Circle().fill(Color.black.opacity(0.6)))
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(starting)
                .accessibilityLabel("Play video \(attachment.name)")
                .accessibilityHint(openViewer == nil ? "" : "Opens the video viewer")
                .accessibilityIdentifier("attachment-\(attachment.id)")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(AttachmentFrame(attachment: attachment, fallback: (320, 180)))
        .task(id: attachment.previewUrl) {
            guard attachment.previewUrl != nil else { return }
            poster = await loadAttachmentImage(attachment, preview: true, chat: chat).image ?? poster
        }
        .onDisappear { player?.pause() }
    }

    private func start() {
        guard !starting else { return }
        starting = true
        Task { @MainActor in
            defer { starting = false }
            let url: URL?
            if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
            guard let url else { return }
            let player = AVPlayer(url: url)
            self.player = player
            player.play()
        }
    }
}

private struct AttachmentAudioView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @State private var player: AVPlayer?
    @State private var playing = false

    var body: some View {
        HStack(spacing: 10) {
            Button(action: toggle) {
                Image(systemName: playing ? "pause.fill" : "play.fill").font(.system(size: 13, weight: .bold)).foregroundStyle(.white)
                    .frame(width: 32, height: 32).background(Circle().fill(CaperTheme.terracotta))
                    #if os(iOS)
                    .frame(minWidth: 44, minHeight: 44)
                    #endif
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(playing ? "Pause \(attachment.name)" : "Play \(attachment.name)")
            VStack(alignment: .leading, spacing: 2) {
                Text(attachment.name).font(CaperTheme.font(13, weight: .bold)).lineLimit(1).truncationMode(.middle)
                Text(detail).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: 360, alignment: .leading)
        .background(CaperTheme.surface)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .onReceive(NotificationCenter.default.publisher(for: NSNotification.Name.AVPlayerItemDidPlayToEndTime)) { notification in
            guard let item = notification.object as? AVPlayerItem, item === player?.currentItem else { return }
            playing = false
            player?.seek(to: .zero)
        }
        .onDisappear { player?.pause(); playing = false }
    }

    private var detail: String {
        let size = AttachmentPolicy.formatBytes(attachment.size)
        guard let duration = attachment.durationMs else { return size }
        let seconds = duration / 1000
        return "\(seconds / 60):\(String(format: "%02d", seconds % 60)) · \(size)"
    }

    private func toggle() {
        if let player {
            if playing { player.pause() } else { player.play() }
            playing.toggle()
            return
        }
        Task { @MainActor in
            let url: URL?
            if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
            guard let url else { return }
            let player = AVPlayer(url: url)
            self.player = player
            player.play()
            playing = true
        }
    }
}

private struct AttachmentOpenCard: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @Environment(\.openURL) private var openURL
    var body: some View {
        AttachmentFileCard(attachment: attachment) { openAttachment(attachment, chat: chat, openURL: openURL) }
    }
}

struct AttachmentFileCard: View {
    let attachment: ChatAttachment
    let action: (() -> Void)?

    var body: some View {
        if let action {
            Button(action: action) { card.contentShape(Rectangle()) }
                .buttonStyle(.plain)
                .accessibilityLabel("File \(attachment.name), \(AttachmentPolicy.formatBytes(attachment.size))")
                .accessibilityHint("Opens the file")
        } else {
            card.accessibilityElement(children: .combine)
        }
    }

    private var card: some View {
        HStack(spacing: 10) {
            Image(systemName: attachment.unavailable ? "trash" : "doc.text").font(.system(size: 17)).foregroundStyle(CaperTheme.muted)
            VStack(alignment: .leading, spacing: 2) {
                Text(attachment.name).font(CaperTheme.font(13, weight: .bold)).lineLimit(1).truncationMode(.middle)
                    .foregroundStyle(attachment.unavailable ? CaperTheme.muted : CaperTheme.text)
                Text(attachment.unavailable ? "File removed" : AttachmentPolicy.formatBytes(attachment.size))
                    .font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: 360, alignment: .leading)
        .background(CaperTheme.surface)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
    }
}

// MARK: Server processing

private func processingLabel(_ percent: Int?) -> String {
    percent.map { "Processing… \($0)%" } ?? "Processing…"
}

/// A file the media worker is still compressing: sized from `width`/`height`
/// when known, showing the server preview (a video poster appears first) or
/// this device's own original, with a spinner and the latest percent.
private struct AttachmentProcessingView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @State private var preview: CGImage?

    private var percent: Int? { chat?.attachmentProgress[attachment.id] }
    private var localCopy: URL? { chat?.localCopy(for: attachment.id) }
    private var visual: Bool {
        attachment.kind == .image || attachment.kind == .video || attachment.previewUrl != nil
            || AttachmentPolicy.displaySize(width: attachment.width, height: attachment.height) != nil
            || AttachmentKind.local(contentType: attachment.contentType) == .image
    }

    var body: some View {
        Group {
            if visual {
                ZStack {
                    if let preview { Image(decorative: preview, scale: 1).resizable().scaledToFill() }
                    VStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text(processingLabel(percent)).font(CaperTheme.font(11)).monospacedDigit()
                    }
                    .foregroundStyle(CaperTheme.text)
                    .padding(.horizontal, 10).padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(preview == nil ? 0 : 0.6)))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .modifier(AttachmentFrame(attachment: attachment, fallback: (240, 180)))
            } else {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(attachment.name).font(CaperTheme.font(13, weight: .bold)).lineLimit(1).truncationMode(.middle)
                        Text(processingLabel(percent)).font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted).monospacedDigit()
                    }
                    Spacer(minLength: 0)
                }
                .padding(10)
                .frame(maxWidth: 360, alignment: .leading)
                .background(CaperTheme.surface)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(attachment.name), processing")
        .accessibilityValue(percent.map { "\($0) percent" } ?? "")
        .task(id: attachment.previewUrl ?? localCopy?.path) {
            guard visual else { return }
            if attachment.previewUrl != nil {
                if let loaded = await loadAttachmentImage(attachment, preview: true, chat: chat).image { preview = loaded }
            } else if let localCopy, AttachmentKind.local(contentType: attachment.contentType) == .image {
                // The sender sees its own original until the server's preview or result arrives.
                if let loaded = try? await AttachmentImageLoader.shared.load(key: "local:\(attachment.id)", url: localCopy, maxPixelSize: 1200) {
                    preview = loaded
                }
            }
        }
    }
}

private struct AttachmentFailedCard: View {
    let attachment: ChatAttachment

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle").font(.system(size: 16)).foregroundStyle(CaperTheme.muted)
            VStack(alignment: .leading, spacing: 2) {
                Text(attachment.name).font(CaperTheme.font(13, weight: .bold)).lineLimit(1).truncationMode(.middle)
                    .foregroundStyle(CaperTheme.muted)
                Text("Couldn’t process this file").font(CaperTheme.font(11)).foregroundStyle(CaperTheme.muted)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: 360, alignment: .leading)
        .background(CaperTheme.surface)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .accessibilityElement(children: .combine)
    }
}

// MARK: Animations (GIFs stored as silent looping MP4)

/// Keeps an `AVPlayerLooper` alive for as long as its player is shown.
@MainActor
private final class LoopingPlayback {
    let player: AVQueuePlayer
    private let looper: AVPlayerLooper

    init(url: URL) {
        let player = AVQueuePlayer()
        player.isMuted = true
        player.preventsDisplaySleepDuringVideoPlayback = false
        self.player = player
        looper = AVPlayerLooper(player: player, templateItem: AVPlayerItem(url: url))
    }

    func stop() {
        player.pause()
        looper.disableLooping()
        player.removeAllItems()
    }

    /// Waits for the file: true once it plays, false when it fails to load
    /// (usually an expired signature) or the wait is cancelled.
    @MainActor func loads() async -> Bool {
        for await status in looper.publisher(for: \.status).values {
            switch status {
            case .ready: return true
            case .failed, .cancelled: return false
            default: continue
            }
        }
        return false
    }
}

#if os(iOS)
private final class PlayerLayerUIView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
}

/// A bare video surface: no controls, no audio, no hit testing.
private struct PlayerSurface: UIViewRepresentable {
    let player: AVPlayer
    /// Fills the frame inline; the media viewer fits the whole video.
    var gravity: AVLayerVideoGravity = .resizeAspectFill

    func makeUIView(context: Context) -> PlayerLayerUIView {
        let view = PlayerLayerUIView()
        view.isUserInteractionEnabled = false
        view.playerLayer.videoGravity = gravity
        view.playerLayer.player = player
        return view
    }

    func updateUIView(_ view: PlayerLayerUIView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
    }
}
#else
private final class PlayerLayerNSView: NSView {
    let playerLayer = AVPlayerLayer()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        playerLayer.videoGravity = .resizeAspectFill
        layer = playerLayer
        wantsLayer = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layout() {
        super.layout()
        playerLayer.frame = bounds
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// A bare video surface: no controls, no audio, no hit testing.
private struct PlayerSurface: NSViewRepresentable {
    let player: AVPlayer
    /// Fills the frame inline; the media viewer fits the whole video.
    var gravity: AVLayerVideoGravity = .resizeAspectFill

    func makeNSView(context: Context) -> PlayerLayerNSView {
        let view = PlayerLayerNSView()
        view.playerLayer.videoGravity = gravity
        view.playerLayer.player = player
        return view
    }

    func updateNSView(_ view: PlayerLayerNSView, context: Context) {
        if view.playerLayer.player !== player { view.playerLayer.player = player }
    }
}
#endif

/// Plays inline, muted and looping without controls, like a GIF. With Reduce
/// Motion it waits on the poster until tapped; a tap pauses or resumes it,
/// or opens the media viewer where there is one.
private struct AttachmentAnimationView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    /// Opens the media viewer; nil makes a tap pause or resume.
    let openViewer: (() -> Void)?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var playback: LoopingPlayback?
    @State private var poster: CGImage?
    @State private var playing = false
    @State private var userPaused = false
    @State private var failed = false
    @State private var attempt = 0

    var body: some View {
        ZStack {
            if let poster { Image(decorative: poster, scale: 1).resizable().scaledToFill() }
            if let playback { PlayerSurface(player: playback.player) }
            if !playing {
                Image(systemName: "play.fill").font(.system(size: 14, weight: .bold)).foregroundStyle(.white)
                    .frame(width: 36, height: 36).background(Circle().fill(Color.black.opacity(0.6)))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(AttachmentFrame(attachment: attachment, fallback: (240, 180)))
        .contentShape(Rectangle())
        .onTapGesture { if let openViewer { openViewer() } else { toggle() } }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Animation \(attachment.name)")
        .accessibilityValue(playing ? "Playing" : "Paused")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { if let openViewer { openViewer() } else { toggle() } }
        .accessibilityAction(named: Text(playing ? "Pause" : "Play")) { toggle() }
        .accessibilityIdentifier("attachment-\(attachment.id)")
        .task(id: attachment.previewUrl) {
            guard attachment.previewUrl != nil else { return }
            poster = await loadAttachmentImage(attachment, preview: true, chat: chat).image ?? poster
        }
        .task(id: "\(reduceMotion):\(attempt)") {
            if reduceMotion || userPaused { pause() } else { await play() }
        }
        .onChange(of: attachment.url) { _, _ in
            // The fresh signature a failed load asked for: retry once with it.
            guard failed, attempt == 0 else { return }
            failed = false
            attempt += 1
        }
        .onDisappear {
            playback?.stop()
            playback = nil
            playing = false
        }
    }

    @MainActor private func toggle() {
        if playing {
            userPaused = true
            pause()
        } else {
            userPaused = false
            Task { @MainActor in await play() }
        }
    }

    @MainActor private func pause() {
        playback?.player.pause()
        playing = false
    }

    @MainActor private func play() async {
        if let playback {
            playback.player.play()
            playing = true
            return
        }
        let url: URL?
        if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
        guard let url, playback == nil, !Task.isCancelled else { return }
        let created = LoopingPlayback(url: url)
        playback = created
        created.player.play()
        playing = true
        let loaded = await created.loads()
        guard !loaded, !Task.isCancelled, playback === created else { return }
        // Usually an expired signature: one fresh URL retries (see `attachment.url`).
        created.stop()
        playback = nil
        playing = false
        failed = true
        if attempt == 0 { chat?.requestFreshAttachmentURLs(ids: [attachment.id]) }
    }
}

// MARK: Media viewer

/// One message's images and videos (`MediaViewerPolicy.items`), opened at
/// the one chosen.
struct MediaViewerRequest: Identifiable {
    let id = UUID()
    let items: [ChatAttachment]
    let start: Int
    /// Follows refreshed URLs and removals; nil for another conversation's
    /// files, shown as delivered.
    let chat: ChatModel?
}

/// Opens the media viewer from attachments below a `MediaViewerHost`.
struct MediaViewerAction {
    let perform: (MediaViewerRequest) -> Void
    func callAsFunction(_ request: MediaViewerRequest) { perform(request) }
}

private struct MediaViewerKey: EnvironmentKey {
    static var defaultValue: MediaViewerAction? { nil }
}

extension EnvironmentValues {
    /// Nil where nothing presents the viewer; attachments then open their URL.
    var mediaViewer: MediaViewerAction? {
        get { self[MediaViewerKey.self] }
        set { self[MediaViewerKey.self] = newValue }
    }
}

/// Presents the media viewer for attachments below it: full screen on
/// iPhone and iPad, and over the whole window (or sheet) on the Mac, where
/// what it covers is disabled like the workspace's dialogs. Only the
/// frontmost presentation can present, so sheets and covers that show
/// attachments add their own host.
struct MediaViewerHost: ViewModifier {
    @State private var request: MediaViewerRequest?

    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content
            .environment(\.mediaViewer, MediaViewerAction { request = $0 })
            .fullScreenCover(item: $request) { shown in
                MediaViewer(request: shown) { request = nil }.presentationBackground(.black).preferredColorScheme(.dark)
            }
        #else
        ZStack {
            content
                .environment(\.mediaViewer, MediaViewerAction { request = $0 })
                .disabled(request != nil)
                .accessibilityHidden(request != nil)
            if let shown = request {
                MediaViewer(request: shown) { request = nil }.id(shown.id)
            }
        }
        #endif
    }
}

/// Discord-style viewer for one message's images and videos. Pages swipe on
/// iPhone and iPad; Previous, Next and the arrow keys step on the Mac, on
/// iPad and with VoiceOver. Close, Escape, a click outside the media on the
/// Mac, or a swipe down on phones closes it.
private struct MediaViewer: View {
    let request: MediaViewerRequest
    let close: () -> Void
    @Environment(\.openURL) private var openURL
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
    #else
    @FocusState private var focused: Bool
    #endif
    @State private var index: Int

    init(request: MediaViewerRequest, close: @escaping () -> Void) {
        self.request = request
        self.close = close
        _index = State(initialValue: request.start)
    }

    /// The set with refreshed URLs and removals, in the order it opened in.
    private var items: [ChatAttachment] {
        guard let chat = request.chat else { return request.items }
        return MediaViewerPolicy.latest(request.items, in: chat.messages + chat.pinnedMessages)
    }

    /// Phones swipe between files; the Mac, iPad and VoiceOver get buttons.
    private var showsArrows: Bool {
        #if os(iOS)
        return sizeClass == .regular || voiceOver
        #else
        return true
        #endif
    }

    var body: some View {
        let items = self.items
        let position = min(max(0, index), max(0, items.count - 1))
        ZStack {
            Color.black.opacity(0.92).ignoresSafeArea()
                #if os(macOS)
                .onTapGesture(perform: close)
                #endif
                .accessibilityHidden(true)
            pages(items)
            if items.indices.contains(position) { chrome(items[position], position: position, count: items.count) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(items.indices.contains(position) && items[position].kind == .video ? "Video viewer" : "Image viewer")
        .accessibilityAddTraits(.isModal)
        .accessibilityIdentifier("media-viewer")
        .onChange(of: index) { _, value in
            guard items.indices.contains(value) else { return }
            announceMediaPosition("\(items[value].name), \(value + 1) of \(items.count)")
        }
        #if os(macOS)
        // Takes the keyboard from the composer underneath.
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onExitCommand(perform: close)
        #endif
    }

    @ViewBuilder private func pages(_ items: [ChatAttachment]) -> some View {
        #if os(iOS)
        TabView(selection: $index) {
            ForEach(items.indices, id: \.self) { position in
                MediaViewerPage(item: items[position], chat: request.chat, active: position == index, close: close)
                    .tag(position)
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        #else
        let position = min(max(0, index), max(0, items.count - 1))
        if items.indices.contains(position) {
            MediaViewerPage(item: items[position], chat: request.chat, active: true, close: close)
                .id(items[position].id)
                .padding(.horizontal, 72)
                .padding(.vertical, 60)
        }
        #endif
    }

    private func chrome(_ item: ChatAttachment, position: Int, count: Int) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.name).font(CaperTheme.font(14, weight: .bold)).foregroundStyle(CaperTheme.text)
                        .lineLimit(1).truncationMode(.middle)
                    if let counter = MediaViewerPolicy.counter(position: position, count: count) {
                        Text(counter).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted).monospacedDigit()
                            .accessibilityLabel("\(position + 1) of \(count)")
                            .accessibilityIdentifier("media-viewer-counter")
                    }
                }
                Spacer(minLength: 8)
                if MediaViewerPolicy.isViewable(item), let raw = item.url, let url = URL(string: raw) {
                    shareButton(item, url: url)
                    Button { openAttachment(item, chat: request.chat, openURL: openURL) } label: {
                        Image(systemName: "arrow.up.right.square")
                    }
                    .buttonStyle(MediaViewerButton())
                    .help("Open in browser")
                    .accessibilityLabel("Open in browser")
                    .accessibilityIdentifier("media-viewer-open")
                }
                Button(action: close) { Image(systemName: "xmark") }
                    .buttonStyle(MediaViewerButton())
                    .keyboardShortcut(.cancelAction)
                    .help("Close")
                    .accessibilityLabel("Close viewer")
                    .accessibilityIdentifier("media-viewer-close")
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(LinearGradient(gradient: Gradient(colors: [Color.black.opacity(0.7), Color.black.opacity(0)]), startPoint: .top, endPoint: .bottom))
            Spacer(minLength: 0)
        }
        .overlay {
            if showsArrows && count > 1 {
                HStack {
                    Button { move(-1, count: count) } label: { Image(systemName: "chevron.left") }
                        .buttonStyle(MediaViewerButton())
                        .keyboardShortcut(.leftArrow, modifiers: [])
                        .disabled(position == 0)
                        .help("Previous file")
                        .accessibilityLabel("Previous file")
                        .accessibilityIdentifier("media-viewer-previous")
                    Spacer()
                    Button { move(1, count: count) } label: { Image(systemName: "chevron.right") }
                        .buttonStyle(MediaViewerButton())
                        .keyboardShortcut(.rightArrow, modifiers: [])
                        .disabled(position >= count - 1)
                        .help("Next file")
                        .accessibilityLabel("Next file")
                        .accessibilityIdentifier("media-viewer-next")
                }
                .padding(.horizontal, 12)
            }
        }
    }

    /// The original file, downloaded only once a share action asks for it, so
    /// the share sheet can save it (Photos, Files) or send it on.
    @ViewBuilder private func shareButton(_ item: ChatAttachment, url: URL) -> some View {
        ShareLink(item: SharedAttachmentFile(item, url: url), preview: SharePreview(item.name)) {
            Image(systemName: "square.and.arrow.up")
        }
        .buttonStyle(MediaViewerButton())
        .help("Save or share")
        .accessibilityLabel("Save or share")
        .accessibilityIdentifier("media-viewer-download")
    }

    private func move(_ delta: Int, count: Int) {
        let next = MediaViewerPolicy.step(index, by: delta, count: count)
        guard next != index else { return }
        withAnimation(.easeOut(duration: 0.2)) { index = next }
    }
}

/// Reads the new position aloud after Previous, Next or a swipe.
@MainActor private func announceMediaPosition(_ text: String) {
    #if os(iOS)
    UIAccessibility.post(notification: .announcement, argument: text)
    #else
    guard let window = NSApp.mainWindow else { return }
    NSAccessibility.post(element: window, notification: .announcementRequested,
                         userInfo: [.announcement: text, .priority: NSAccessibilityPriorityLevel.high.rawValue])
    #endif
}

/// A viewer file for the share sheet, downloaded once an action asks for
/// it. It offers its own concrete type (a JPEG, an MP4), which the sheet's
/// Save Image and Save Video actions need; other types go as plain data.
private struct SharedAttachmentFile: Transferable, Sendable {
    let url: URL
    let name: String
    let type: UTType?

    init(_ item: ChatAttachment, url: URL) {
        self.url = url
        name = item.name
        type = UTType(mimeType: item.contentType)
    }

    static let saveable: [UTType] = [.jpeg, .png, .heic, .gif, .webP, .mpeg4Movie, .quickTimeMovie]

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .jpeg) { try await $0.downloaded() }.exportingCondition { $0.type == .jpeg }
        FileRepresentation(exportedContentType: .png) { try await $0.downloaded() }.exportingCondition { $0.type == .png }
        FileRepresentation(exportedContentType: .heic) { try await $0.downloaded() }.exportingCondition { $0.type == .heic }
        FileRepresentation(exportedContentType: .gif) { try await $0.downloaded() }.exportingCondition { $0.type == .gif }
        FileRepresentation(exportedContentType: .webP) { try await $0.downloaded() }.exportingCondition { $0.type == .webP }
        FileRepresentation(exportedContentType: .mpeg4Movie) { try await $0.downloaded() }.exportingCondition { $0.type == .mpeg4Movie }
        FileRepresentation(exportedContentType: .quickTimeMovie) { try await $0.downloaded() }.exportingCondition { $0.type == .quickTimeMovie }
        FileRepresentation(exportedContentType: .data) { try await $0.downloaded() }
            .exportingCondition { file in !saveable.contains { $0 == file.type } }
    }

    private func downloaded() async throws -> SentTransferredFile {
        SentTransferredFile(try await AttachmentImageLoader.shared.download(url, name: name))
    }
}

/// The viewer's round controls over its dark backdrop.
private struct MediaViewerButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 36, height: 36)
            .background(Circle().fill(Color.black.opacity(configuration.isPressed ? 0.8 : 0.55)))
            .overlay(Circle().stroke(Color.white.opacity(0.14)))
            #if os(iOS)
            .frame(minWidth: 44, minHeight: 44)
            #endif
            .contentShape(Rectangle())
            .opacity(isEnabled ? 1 : 0.35)
            .modifier(ControlPointer())
    }
}

/// One file of the set. On phones a swipe down closes the viewer unless
/// the file is zoomed.
private struct MediaViewerPage: View {
    let item: ChatAttachment
    let chat: ChatModel?
    let active: Bool
    let close: () -> Void
    @State private var zoomed = false
    @GestureState private var pull: CGFloat = 0

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            #if os(iOS)
            .offset(y: pull)
            // Global coordinates: the page itself moves with the finger.
            .simultaneousGesture(DragGesture(minimumDistance: 16, coordinateSpace: .global)
                .updating($pull) { value, state, _ in
                    if value.translation.height > abs(value.translation.width) { state = value.translation.height }
                }
                .onEnded { value in
                    let downward = value.translation.height > abs(value.translation.width)
                    if downward && (value.translation.height > 120 || value.predictedEndTranslation.height > 360) { close() }
                }, including: zoomed ? .subviews : .all)
            #endif
    }

    @ViewBuilder private var content: some View {
        if !MediaViewerPolicy.isViewable(item) {
            VStack(spacing: 8) {
                Image(systemName: "trash").font(.system(size: 26))
                Text("File removed").font(CaperTheme.font(14, weight: .bold))
            }
            .foregroundStyle(CaperTheme.muted)
            .accessibilityElement(children: .combine)
        } else if item.kind == .image {
            MediaViewerImage(item: item, chat: chat, active: active, zoomed: $zoomed)
        } else if item.animated {
            MediaViewerAnimation(item: item, chat: chat, active: active)
        } else {
            MediaViewerVideo(item: item, chat: chat, active: active)
        }
    }
}

/// An image fitted to the viewer. Double-click or double-tap toggles fit and
/// 2× at that point, a pinch (touch screen or trackpad) zooms, and a drag
/// pans while zoomed. The preview shows while the original loads; an
/// animated GIF, WebP or PNG plays (paused at first with Reduce Motion).
private struct MediaViewerImage: View {
    let item: ChatAttachment
    let chat: ChatModel?
    let active: Bool
    @Binding var zoomed: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var preview: CGImage?
    @State private var original: AttachmentOriginal?
    @State private var failed = false
    /// Bumped once when the fresh URL a refused load asked for arrives.
    @State private var attempt = 0
    /// Tapped away from the default: paused, or played with Reduce Motion.
    @State private var toggled = false
    @State private var started = Date()
    @State private var scale: CGFloat = 1
    @State private var offset: CGSize = .zero
    @GestureState private var pinch: CGFloat = 1
    @GestureState private var drag: CGSize = .zero

    private var animated: Bool { (original?.images.count ?? 0) > 1 }
    private var playing: Bool { active && (reduceMotion ? toggled : !toggled) }

    var body: some View {
        GeometryReader { geometry in
            if let shown = original?.images.first ?? preview {
                let fitted = MediaViewerPolicy.fittedSize(width: Double(shown.width), height: Double(shown.height), in: geometry.size)
                picture(shown)
                    .frame(width: fitted.width, height: fitted.height)
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { point in toggleZoom(at: point, fitted: fitted, container: geometry.size) }
                    .onTapGesture { if animated { toggled.toggle() } }
                    .gesture(magnify(fitted: fitted, container: geometry.size))
                    .gesture(pan(fitted: fitted, container: geometry.size), including: scale > 1 ? .all : .subviews)
                    .scaleEffect(scale * pinch)
                    .offset(x: offset.width + drag.width, y: offset.height + drag.height)
                    .overlay { if original == nil && !failed { ProgressView() } }
                    .frame(width: geometry.size.width, height: geometry.size.height)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(item.name)
                    .accessibilityAddTraits(.isImage)
                    .accessibilityIdentifier("media-viewer-image")
            } else {
                Group {
                    if failed {
                        VStack(spacing: 6) {
                            Image(systemName: "photo").font(.system(size: 22))
                            Text("Image unavailable").font(CaperTheme.font(13))
                        }
                        .foregroundStyle(CaperTheme.muted)
                        .accessibilityElement(children: .combine)
                    } else {
                        ProgressView()
                    }
                }
                .frame(width: geometry.size.width, height: geometry.size.height)
            }
        }
        .overlay(alignment: .bottom) {
            if animated && active { MediaViewerAnimationToggle(playing: playing) { toggled.toggle() } }
        }
        // Not keyed on the URL: opening the original can re-sign it, which
        // would cancel the download it is about to start.
        .task(id: "\(active):\(attempt)") { await load() }
        .onChange(of: item.url) { _, _ in
            // The fresh signature a refused load asked for: retry once with it.
            guard failed, attempt == 0 else { return }
            failed = false
            attempt += 1
        }
        .onChange(of: active) { _, isActive in
            // Each file opens fitted. Pages stay alive once seen, so only the
            // one on screen holds its decoded original (stills stay cached).
            guard !isActive else { return }
            scale = 1
            offset = .zero
            original = nil
            failed = false
        }
        .onChange(of: scale) { _, value in zoomed = value > 1 }
    }

    @ViewBuilder private func picture(_ still: CGImage) -> some View {
        if let original, original.images.count > 1 {
            TimelineView(.animation(minimumInterval: 0.02, paused: !playing)) { timeline in
                let shownFrame = MediaViewerPolicy.frameIndex(at: timeline.date.timeIntervalSince(started), durations: original.durations)
                Image(decorative: original.images[min(shownFrame, original.images.count - 1)], scale: 1).resizable()
            }
        } else {
            Image(decorative: still, scale: 1).resizable()
        }
    }

    private func magnify(fitted: CGSize, container: CGSize) -> some Gesture {
        MagnifyGesture()
            .updating($pinch) { value, state, _ in state = value.magnification }
            .onEnded { value in
                let next = min(MediaViewerPolicy.maxZoom, max(1, scale * value.magnification))
                withAnimation(.easeOut(duration: 0.2)) {
                    scale = next
                    offset = next > 1 ? MediaViewerPolicy.clampedOffset(offset, scale: next, fitted: fitted, container: container) : .zero
                }
            }
    }

    private func pan(fitted: CGSize, container: CGSize) -> some Gesture {
        // Global coordinates: the image is scaled, so its own space is too.
        DragGesture(minimumDistance: 1, coordinateSpace: .global)
            .updating($drag) { value, state, _ in state = value.translation }
            .onEnded { value in
                let moved = CGSize(width: offset.width + value.translation.width, height: offset.height + value.translation.height)
                withAnimation(.easeOut(duration: 0.15)) {
                    offset = MediaViewerPolicy.clampedOffset(moved, scale: scale, fitted: fitted, container: container)
                }
            }
    }

    private func toggleZoom(at point: CGPoint, fitted: CGSize, container: CGSize) {
        withAnimation(.easeOut(duration: 0.2)) {
            if scale > 1 {
                scale = 1
                offset = .zero
            } else {
                let next = MediaViewerPolicy.doubleTapZoom
                scale = next
                offset = MediaViewerPolicy.clampedOffset(MediaViewerPolicy.zoomOffset(at: point, fitted: fitted, scale: next),
                                                         scale: next, fitted: fitted, container: container)
            }
        }
    }

    @MainActor private func load() async {
        guard MediaViewerPolicy.isViewable(item) else { return }
        if preview == nil, original == nil {
            preview = await loadAttachmentImage(item, preview: true, chat: chat).image
        }
        // Only the file on screen downloads its original.
        guard active, original == nil else { return }
        let result = await loadAttachmentOriginal(item, chat: chat)
        if let frames = result.frames {
            original = frames
            failed = false
            started = Date()
        } else if !Task.isCancelled {
            failed = result.failed
            // Usually an expired signature: the task retries once with the
            // fresh URL when it arrives (see `item.url`).
            if result.refusedURL && attempt == 0 {
                chat?.requestFreshAttachmentURLs(ids: [item.id])
            }
        }
    }
}

/// A GIF stored as video: muted and looping without controls, like inline,
/// paused at first with Reduce Motion. A tap pauses or resumes it.
private struct MediaViewerAnimation: View {
    let item: ChatAttachment
    let chat: ChatModel?
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var playback: LoopingPlayback?
    @State private var poster: CGImage?
    /// Tapped away from the default: paused, or played with Reduce Motion.
    @State private var toggled = false
    @State private var failed = false
    @State private var attempt = 0

    private var playing: Bool { active && (reduceMotion ? toggled : !toggled) }

    var body: some View {
        ZStack {
            if let poster { Image(decorative: poster, scale: 1).resizable().scaledToFit() }
            if let playback { PlayerSurface(player: playback.player, gravity: .resizeAspect) }
            if playback == nil && playing && !failed { ProgressView() }
        }
        .modifier(MediaViewerAspect(item: item))
        .contentShape(Rectangle())
        .onTapGesture { toggled.toggle() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Animation \(item.name)")
        .accessibilityAddTraits(.isImage)
        .accessibilityIdentifier("media-viewer-animation")
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(alignment: .bottom) {
            if active { MediaViewerAnimationToggle(playing: playing) { toggled.toggle() } }
        }
        .task(id: item.previewUrl) {
            guard item.previewUrl != nil else { return }
            poster = await loadAttachmentImage(item, preview: true, chat: chat).image ?? poster
        }
        .task(id: "\(playing):\(attempt)") { await update() }
        .onChange(of: item.url) { _, _ in
            // The fresh signature a failed load asked for: retry once with it.
            guard failed, attempt == 0 else { return }
            failed = false
            attempt += 1
        }
        .onDisappear {
            playback?.stop()
            playback = nil
        }
    }

    @MainActor private func update() async {
        guard playing else { playback?.player.pause(); return }
        if playback == nil {
            let url: URL?
            if let chat { url = await chat.currentURL(for: item) } else { url = item.url.flatMap { URL(string: $0) } }
            guard let url, playback == nil, !failed, !Task.isCancelled else { return }
            let created = LoopingPlayback(url: url)
            playback = created
            created.player.play()
            let loaded = await created.loads()
            guard !loaded, !Task.isCancelled, playback === created else { return }
            // Usually an expired signature: one fresh URL retries (see `item.url`).
            created.stop()
            playback = nil
            failed = true
            if attempt == 0 { chat?.requestFreshAttachmentURLs(ids: [item.id]) }
            return
        }
        playback?.player.play()
    }
}

/// Pauses or resumes an animation in the viewer.
private struct MediaViewerAnimationToggle: View {
    let playing: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) { Image(systemName: playing ? "pause.fill" : "play.fill") }
            .buttonStyle(MediaViewerButton())
            .help(playing ? "Pause animation" : "Play animation")
            .accessibilityLabel(playing ? "Pause animation" : "Play animation")
            .accessibilityIdentifier("media-viewer-animation-toggle")
            .padding(.bottom, 16)
    }
}

/// A video in the platform player with its controls. It plays with sound
/// while its page is shown and pauses when left or closed; a failed load
/// asks for a fresh URL once and retries with it.
private struct MediaViewerVideo: View {
    let item: ChatAttachment
    let chat: ChatModel?
    let active: Bool
    @State private var player: AVPlayer?
    @State private var poster: CGImage?
    @State private var failed = false
    @State private var attempt = 0

    var body: some View {
        ZStack {
            if let player, !failed {
                VideoPlayer(player: player)
            } else if let poster {
                Image(decorative: poster, scale: 1).resizable().scaledToFit()
            }
            if failed {
                VStack(spacing: 6) {
                    Image(systemName: "film").font(.system(size: 22))
                    Text("Video unavailable").font(CaperTheme.font(13))
                }
                .foregroundStyle(CaperTheme.muted)
                .padding(12)
                .background(RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(0.6)))
                .accessibilityElement(children: .combine)
            } else if player == nil {
                ProgressView()
            }
        }
        .modifier(MediaViewerAspect(item: item))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        #if os(iOS)
        // Clear of the top bar, so the player's own controls stay reachable.
        .padding(.vertical, 52)
        #endif
        .task(id: item.previewUrl) {
            guard item.previewUrl != nil else { return }
            poster = await loadAttachmentImage(item, preview: true, chat: chat).image ?? poster
        }
        .task(id: "\(active):\(attempt)") {
            if active { await play() } else { player?.pause() }
        }
        .onChange(of: item.url) { _, _ in
            // A refreshed signature after the first failed load: retry once with it.
            guard failed, attempt == 0 else { return }
            failed = false
            player = nil
            attempt += 1
        }
        .onDisappear { player?.pause() }
    }

    @MainActor private func play() async {
        if let player {
            if !failed { player.play() }
            return
        }
        let url: URL?
        if let chat { url = await chat.currentURL(for: item) } else { url = item.url.flatMap { URL(string: $0) } }
        guard let url, player == nil, !Task.isCancelled else { return }
        let created = AVPlayer(url: url)
        player = created
        created.play()
        guard let current = created.currentItem else { return }
        for await status in current.publisher(for: \.status).values {
            if status == .unknown { continue }
            if status == .failed {
                failed = true
                // Usually an expired signature: one fresh URL retries (see `item.url`).
                if attempt == 0 { chat?.requestFreshAttachmentURLs(ids: [item.id]) }
            }
            break
        }
    }
}

/// Fits a video's own aspect ratio when it is known, so the letterbox stays
/// backdrop (a click there closes the viewer on the Mac).
private struct MediaViewerAspect: ViewModifier {
    let item: ChatAttachment

    @ViewBuilder func body(content: Content) -> some View {
        if let width = item.width, let height = item.height, width > 0, height > 0 {
            content.aspectRatio(CGSize(width: width, height: height), contentMode: .fit)
        } else {
            content
        }
    }
}

// MARK: Composer

/// Draft files above the composer: thumbnail, name, size, upload progress, remove.
struct AttachmentDraftsView: View {
    @Bindable var chat: ChatModel

    var body: some View {
        if !chat.attachmentDrafts.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(chat.attachmentDrafts) { draft in
                        AttachmentDraftChip(draft: draft) { chat.removeAttachmentDraft(id: draft.id) }
                    }
                }
            }
            .accessibilityLabel("Files to send")
        }
    }
}

private struct AttachmentDraftChip: View {
    let draft: AttachmentDraft
    let remove: () -> Void
    @State private var thumbnail: CGImage?

    var body: some View {
        HStack(spacing: 8) {
            Group {
                if let thumbnail {
                    Image(decorative: thumbnail, scale: 1).resizable().scaledToFill()
                } else {
                    Image(systemName: icon).font(.system(size: 15)).foregroundStyle(CaperTheme.muted)
                }
            }
            .frame(width: 36, height: 36)
            .background(CaperTheme.raised)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            VStack(alignment: .leading, spacing: 3) {
                Text(draft.name).font(CaperTheme.font(12, weight: .bold)).lineLimit(1).truncationMode(.middle)
                Text(draft.statusLabel).font(CaperTheme.font(10))
                    .foregroundStyle(draft.error == nil ? CaperTheme.muted : CaperTheme.terracottaBright).lineLimit(2)
                if draft.attachment == nil && draft.error == nil {
                    ProgressView(value: draft.progress).progressViewStyle(.linear).tint(CaperTheme.terracottaBright).frame(width: 110)
                        .accessibilityLabel("Uploading \(draft.name)")
                }
            }
            .frame(maxWidth: 160, alignment: .leading)
            Button(action: remove) {
                Image(systemName: "xmark").font(.system(size: 11, weight: .bold)).frame(width: 24, height: 24)
                    #if os(iOS)
                    .frame(minWidth: 44, minHeight: 44)
                    #endif
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(CaperTheme.muted)
            .accessibilityLabel("Remove \(draft.name)")
        }
        .padding(.leading, 6).padding(.trailing, 4).padding(.vertical, 6)
        .background(CaperTheme.surface)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(draft.error == nil ? CaperTheme.border : CaperTheme.terracottaBright))
        .task(id: draft.localURL) {
            guard draft.kind == .image else { thumbnail = nil; return }
            let key = "draft:\(draft.id):\(draft.localURL.lastPathComponent)"
            thumbnail = try? await AttachmentImageLoader.shared.load(key: key, url: draft.localURL, maxPixelSize: 120)
        }
    }

    private var icon: String {
        switch draft.kind {
        case .image: return "photo"
        case .video: return "film"
        case .audio: return "waveform"
        case .file: return "doc.text"
        }
    }
}

/// Imports a photo or video from the Photos picker into the staging area.
struct PickedAttachmentMedia: Transferable, Sendable {
    let file: LocalAttachmentFile

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .movie) { received in
            PickedAttachmentMedia(file: try AttachmentStaging.stage(copying: received.file))
        }
        FileRepresentation(importedContentType: .image) { received in
            PickedAttachmentMedia(file: try AttachmentStaging.stage(copying: received.file))
        }
    }
}

/// The composer's attach control: Photos, or any file. Shown only while
/// uploads are available and the conversation accepts messages.
struct AttachmentPickerButton: View {
    @Bindable var chat: ChatModel
    @State private var showPhotos = false
    @State private var showFiles = false
    @State private var photoItems: [PhotosPickerItem] = []

    private var remaining: Int { max(0, AttachmentPolicy.maxAttachments - chat.attachmentDrafts.count) }

    var body: some View {
        Menu {
            Button { showPhotos = true } label: { Label("Photos and videos", systemImage: "photo.on.rectangle") }
            Button { showFiles = true } label: { Label("Files", systemImage: "doc") }
        } label: {
            Image(systemName: "paperclip").font(.system(size: 16, weight: .semibold)).foregroundStyle(CaperTheme.muted)
                .frame(width: 42, height: 42)
                .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .background(CaperTheme.composer)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
        .disabled(remaining == 0)
        .help(remaining == 0 ? "You can attach up to \(AttachmentPolicy.maxAttachments) files." : "Attach files")
        .accessibilityLabel("Attach files")
        .accessibilityIdentifier("attach-files-button")
        .photosPicker(isPresented: $showPhotos, selection: $photoItems, maxSelectionCount: max(1, remaining),
                      matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            photoItems = []
            Task { @MainActor in
                var files: [LocalAttachmentFile] = []
                for item in items {
                    if let picked = try? await item.loadTransferable(type: PickedAttachmentMedia.self) { files.append(picked.file) }
                }
                chat.addAttachments(files)
                if files.count < items.count && chat.attachmentNotice == nil { chat.attachmentNotice = "Some items couldn’t be added." }
            }
        }
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            let files = urls.prefix(AttachmentPolicy.maxAttachments).compactMap { try? AttachmentStaging.stage(copying: $0) }
            chat.addAttachments(files)
            if files.count < min(urls.count, AttachmentPolicy.maxAttachments) && chat.attachmentNotice == nil {
                chat.attachmentNotice = "Some files couldn’t be added."
            }
        }
    }
}
