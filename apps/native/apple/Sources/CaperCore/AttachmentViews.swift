import AVKit
import Combine
import ImageIO
import PhotosUI
import QuickLook
import SwiftUI
import UniformTypeIdentifiers

// MARK: Image loading

enum AttachmentLoadError: Error { case status(Int), undecodable }

/// Decoded images in the colour space the main window composites in. Core
/// Animation re-renders, and keeps, a second copy of any layer image in
/// another space; drawing each decoded image into this space once, off the
/// main thread, leaves one copy. The Mac host sets it from its window's screen;
/// nil (iOS, extended-range screens) keeps images as decoded.
public enum DisplayPixels {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var target: CGColorSpace?

    public static var colorSpace: CGColorSpace? {
        get { lock.withLock { target } }
        set {
            let usable = newValue.flatMap { $0.model == .rgb && !CGColorSpaceUsesExtendedRange($0) ? $0 : nil }
            lock.withLock { target = usable }
        }
    }

    static func prepare(_ image: CGImage) -> CGImage {
        guard let space = colorSpace, image.colorSpace != space else { return image }
        let opaque = [.none, .noneSkipFirst, .noneSkipLast].contains(image.alphaInfo)
        let alpha: CGImageAlphaInfo = opaque ? .noneSkipFirst : .premultipliedFirst
        guard let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                                      bitmapInfo: alpha.rawValue | CGBitmapInfo.byteOrder32Little.rawValue) else { return image }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage() ?? image
    }
}

/// Cancellation may arrive before the continuation installs its task delegate.
private final class AttachmentDownloadCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var task: URLSessionDataTask?
    private var cancelled = false

    func start(_ task: URLSessionDataTask) {
        lock.withLock {
            self.task = task
            task.resume()
            if cancelled { task.cancel() }
        }
    }

    func cancel() {
        lock.withLock {
            cancelled = true
            task?.cancel()
        }
    }
}

/// Data-task callbacks deliver chunks, avoiding both an unbounded download and
/// per-byte async iteration. State is confined to URLSession's serial delegate queue.
private final class AttachmentImageDownload: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    static let maximumBytes = 40 * 1024 * 1024
    private var body = Data()
    private var failure: Error?
    private let continuation: CheckedContinuation<Data, Error>

    init(_ continuation: CheckedContinuation<Data, Error>) { self.continuation = continuation }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            failure = AttachmentLoadError.status(http.statusCode)
        } else if response.expectedContentLength > Int64(Self.maximumBytes) {
            failure = URLError(.dataLengthExceedsMaximum)
        }
        completionHandler(failure == nil ? .allow : .cancel)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard failure == nil else { return }
        guard data.count <= Self.maximumBytes - body.count else {
            failure = URLError(.dataLengthExceedsMaximum)
            body = Data()
            dataTask.cancel()
            return
        }
        body.append(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error = failure ?? error {
            body = Data()
            continuation.resume(throwing: error)
        } else {
            continuation.resume(returning: body)
        }
    }
}

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

    init(configuration: URLSessionConfiguration = .ephemeral) {
        KlipyFixtureTransport.install(into: configuration)
        configuration.httpCookieAcceptPolicy = .never
        configuration.httpShouldSetCookies = false
        session = URLSession(configuration: configuration)
        cache.totalCostLimit = 160 * 1024 * 1024
    }

    func cached(_ key: String) -> CGImage? { cache.object(forKey: key as NSString)?.image }

    /// A remounted message must paint its previous pixels before its task starts.
    /// This fallback never satisfies a load of a different server variant.
    func cachedPreview(for id: String) -> CGImage? {
        cached("\(id):preview") ?? cached("\(id):full") ?? cached("local:\(id)")
            ?? cached("\(id):inline-gif") ?? cached("poster:\(id):inline-gif")
    }

    private func fetch(_ url: URL) async throws -> Data {
        if url.isFileURL { return try Data(contentsOf: url) }
        let cancellation = AttachmentDownloadCancellation()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let task = session.dataTask(with: url)
                task.delegate = AttachmentImageDownload(continuation)
                cancellation.start(task)
            }
        } onCancel: {
            cancellation.cancel()
        }
    }

    /// Copy original pixels, with orientation applied; animations contribute their first frame.
    func clipboardImage(_ url: URL) async throws -> CGImage {
        let data = try await fetch(url)
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any],
              let width = properties[kCGImagePropertyPixelWidth as String] as? Int,
              let height = properties[kCGImagePropertyPixelHeight as String] as? Int,
              let image = Self.decodeStill(source, maxPixelSize: max(width, height), forDisplay: false) else {
            throw AttachmentLoadError.undecodable
        }
        return image
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
              let decoded = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { throw AttachmentLoadError.undecodable }
        let image = DisplayPixels.prepare(decoded)
        cache.setObject(CachedAttachmentImage(image), forKey: key as NSString, cost: image.bytesPerRow * image.height)
        return image
    }

    /// Local draft posters do not wait for compression or an upload. Keep the
    /// same orientation and early-frame policy as uploaded video previews.
    func loadVideo(key: String, url: URL, maxPixelSize: Int) async throws -> CGImage {
        if let hit = cached(key) { return hit }
        let asset = AVURLAsset(url: url)
        let duration = try await asset.load(.duration).seconds
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        generator.maximumSize = CGSize(width: maxPixelSize, height: maxPixelSize)
        let seconds = duration.isFinite && duration > 0 ? min(0.5, duration / 2) : 0
        let image = DisplayPixels.prepare(try await generator.image(at: CMTime(seconds: seconds, preferredTimescale: 600)).image)
        try Task.checkCancellation()
        cache.setObject(CachedAttachmentImage(image), forKey: key as NSString, cost: image.bytesPerRow * image.height)
        return image
    }

    /// Bound decode work and retained pixels. Lower playback resolution when
    /// needed instead of turning an otherwise ordinary GIF into a still.
    private static let maxAnimationFrames = 600
    private static let maxAnimationBytes = 192 * 1024 * 1024

    /// The media viewer's original: every frame of an animated GIF, WebP or
    /// PNG, or one still (cached like `load`).
    func loadFrames(key: String, url: URL, maxPixelSize: Int, animationByteLimit: Int? = nil) async throws -> AttachmentOriginal {
        if let hit = cached(key) { return AttachmentOriginal(images: [hit], durations: []) }
        let data = try await fetch(url)
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { throw AttachmentLoadError.undecodable }
        if let animation = try Self.decodeAnimation(source, maxPixelSize: maxPixelSize, maxFrames: Self.maxAnimationFrames,
                                                    byteLimit: animationByteLimit ?? Self.maxAnimationBytes) {
            if let first = animation.images.first {
                // Keep only a still for transitions; caching it as `key` would stop future playback.
                cache.setObject(CachedAttachmentImage(first), forKey: "poster:\(key)" as NSString, cost: first.bytesPerRow * first.height)
            }
            return animation
        }
        guard let image = Self.decodeStill(source, maxPixelSize: maxPixelSize) else { throw AttachmentLoadError.undecodable }
        cache.setObject(CachedAttachmentImage(image), forKey: key as NSString, cost: image.bytesPerRow * image.height)
        return AttachmentOriginal(images: [image], durations: [])
    }

    /// Every frame of an animated GIF, WebP or PNG, each at most
    /// `maxPixelSize` on its longest edge, with how long it shows. When the
    /// frames would retain more than `byteLimit` bytes, playback resolution
    /// drops until they fit rather than turning an ordinary GIF into a still.
    /// nil for a still or an animation with more than `maxFrames` frames:
    /// callers then show one still, never a truncated loop. Shared by
    /// inline/viewer attachments and profile pictures.
    static func decodeAnimation(_ source: CGImageSource, maxPixelSize: Int, maxFrames: Int, byteLimit: Int) throws -> AttachmentOriginal? {
        let count = CGImageSourceGetCount(source)
        guard count > 1, count <= maxFrames, frameDelay(source, at: 0) != nil else { return nil }
        var edge = maxPixelSize
        while edge > 0 {
            try Task.checkCancellation()
            let frameOptions: [CFString: Any] = [
                kCGImageSourceShouldCacheImmediately: true,
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceThumbnailMaxPixelSize: edge,
            ]
            guard let first = CGImageSourceCreateThumbnailAtIndex(source, 0, frameOptions as CFDictionary).map(DisplayPixels.prepare) else { return nil }
            let firstBytes = first.bytesPerRow * first.height
            if firstBytes > byteLimit / count {
                // Use the actual row stride, not width × 4; ImageIO may
                // align rows or use a wider pixel representation.
                let scale = sqrt(Double(max(0, byteLimit / count)) / Double(firstBytes))
                edge = min(edge - 1, Int(Double(max(first.width, first.height)) * scale))
                continue
            }
            var images = [first]
            var durations = [MediaViewerPolicy.frameDuration(frameDelay(source, at: 0))]
            var bytes = firstBytes
            for index in 1..<count {
                try Task.checkCancellation()
                guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, frameOptions as CFDictionary).map(DisplayPixels.prepare) else { break }
                bytes += image.bytesPerRow * image.height
                guard bytes <= byteLimit else { break }
                images.append(image)
                durations.append(MediaViewerPolicy.frameDuration(frameDelay(source, at: index)))
            }
            if images.count == count { return AttachmentOriginal(images: images, durations: durations) }
            // If later frames need more space than the first, retry at a
            // smaller size. Never loop only the prefix that happened to fit.
            edge /= 2
        }
        return nil
    }

    /// The first image, orientation applied, at most `maxPixelSize` on its
    /// longest edge; in the display's colour space unless it leaves the app.
    static func decodeStill(_ source: CGImageSource, maxPixelSize: Int, forDisplay: Bool = true) -> CGImage? {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        return forDisplay ? image.map(DisplayPixels.prepare) : image
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
func loadAttachmentImage(_ attachment: ChatAttachment, preview: Bool, chat: ChatModel?, maxPixelSize: Int = 1200) async -> (image: CGImage?, failed: Bool) {
    let usePreview = preview && attachment.previewUrl != nil
    guard let raw = usePreview ? attachment.previewUrl : attachment.url, let url = URL(string: raw) else { return (nil, true) }
    let key = url.isFileURL ? "local:\(attachment.id)" : "\(attachment.id):\(usePreview ? "preview" : "full")"
    if let cached = AttachmentImageLoader.shared.cached(key) { return (cached, false) }
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
func loadAttachmentOriginal(_ attachment: ChatAttachment, chat: ChatModel?, inline: Bool = false) async -> (frames: AttachmentOriginal?, failed: Bool, refusedURL: Bool) {
    let url: URL?
    if let chat { url = await chat.currentURL(for: attachment) } else { url = attachment.url.flatMap { URL(string: $0) } }
    guard let url else { return (nil, true, false) }
    do {
        let frames = try await AttachmentImageLoader.shared.loadFrames(
            key: "\(attachment.id):\(inline ? "inline-gif" : "original")", url: url,
            maxPixelSize: inline ? 720 : 4096, animationByteLimit: inline ? 32 * 1024 * 1024 : nil
        )
        return (frames, false, false)
    } catch AttachmentLoadError.status(let status) where AttachmentURLPolicy.refreshesAfterFailure(status: status) {
        return (nil, true, true)
    } catch {
        return (nil, !(error is CancellationError) && !Task.isCancelled, false)
    }
}

// MARK: Message attachments

/// A quote is navigation, not a media player or a spoiler reveal surface.
struct ReplyAttachmentThumbnail: View {
    let attachment: ChatAttachment
    let chat: ChatModel
    @State private var image: CGImage?
    private var source: String? {
        guard !attachment.spoiler, !attachment.unavailable, attachment.status == .ready else { return nil }
        guard attachment.kind == .image || attachment.kind == .video else { return nil }
        return attachment.previewUrl ?? (attachment.kind == .image && !attachment.animated && attachment.contentType != "image/gif" ? attachment.url : nil)
    }
    var body: some View {
        ZStack {
            CaperTheme.text.opacity(0.06)
            if source != nil, let image {
                Image(decorative: image, scale: 1).resizable().scaledToFill()
            } else {
                Image(systemName: attachment.spoiler ? "eye.slash" : attachment.voice ? "mic" : "paperclip")
                    .font(.system(size: 14)).foregroundStyle(CaperTheme.muted)
            }
        }
        .frame(width: 44, height: 44).clipShape(RoundedRectangle(cornerRadius: 6))
        .accessibilityHidden(true)
        .task(id: source) {
            image = nil
            guard source != nil else { return }
            let result = await loadAttachmentImage(attachment, preview: true, chat: chat, maxPixelSize: 132)
            guard !Task.isCancelled else { return }
            image = result.image
        }
    }
}

/// Files under a message's text. `chat` is nil for the optimistic pending
/// row, whose attachments point at local copies, and for another
/// conversation's files. Below a `MediaViewerHost`, images and videos open
/// the in-app viewer; the pending row's local copies still open on their own.
struct MessageAttachmentsView: View {
    let attachments: [ChatAttachment]
    let chat: ChatModel?
    var isAvailable: (() -> Bool)? = nil
    @Environment(\.mediaViewer) private var mediaViewer
    @State private var revealed: Set<String> = []

    private var visuals: [ChatAttachment] { attachments.filter { $0.kind == .image || $0.kind == .video } }
    private var files: [ChatAttachment] { attachments.filter { $0.kind != .image && $0.kind != .video && !$0.voice } }
    /// Voice players take their own width rather than a file-grid column.
    private var voiceNotes: [ChatAttachment] { attachments.filter { $0.kind != .image && $0.kind != .video && $0.voice } }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if visuals.count > 1 {
                SentGalleryLayout {
                    ForEach(Array(visuals.enumerated()), id: \.element.id) { index, attachment in
                        GeometryReader { geometry in
                            let frames = SentGalleryGeometry.frames(count: visuals.count, width: 480)
                            let cell = frames[index]
                            let height = frames.map(\.maxY).max() ?? 0
                            let right = abs(cell.maxX - 480) < 0.01
                            let bottom = abs(cell.maxY - height) < 0.01
                            let shape = UnevenRoundedRectangle(
                                topLeadingRadius: cell.minX == 0 && cell.minY == 0 ? 12 : 1,
                                bottomLeadingRadius: cell.minX == 0 && bottom ? 12 : 1,
                                bottomTrailingRadius: right && bottom ? 12 : 1,
                                topTrailingRadius: right && cell.minY == 0 ? 12 : 1)
                            attachmentView(attachment)
                                .environment(\.sentGalleryCell, true)
                                .frame(width: geometry.size.width, height: geometry.size.height)
                                .background(CaperTheme.surface)
                                .clipShape(shape)
                        }
                    }
                }
            } else if let attachment = visuals.first {
                attachmentView(attachment)
            }
            ForEach(voiceNotes) { attachment in attachmentView(attachment) }
            SentFileLayout {
                ForEach(files) { attachment in attachmentView(attachment) }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func attachmentView(_ attachment: ChatAttachment) -> some View {
        if attachment.spoiler && !revealed.contains(attachment.id) {
            Button { revealed.insert(attachment.id) } label: {
                AttachmentSpoilerView(attachment: attachment, chat: chat)
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel("Reveal spoiler attachment")
            .accessibilityHint("Shows the hidden file")
        } else {
            revealedAttachmentView(attachment)
                .overlay(alignment: .topTrailing) {
                    if attachment.spoiler {
                        Button { revealed.remove(attachment.id) } label: {
                            Label {
                                Text("Hide spoiler").font(CaperTheme.font(11, weight: .bold)).lineLimit(2)
                            } icon: { Image(systemName: "eye.slash").font(.system(size: 14)) }
                        }
                        .buttonStyle(MediaViewerButton(labelled: true))
                        .labelStyle(.titleAndIcon)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(4)
                    }
                }
                .transaction { if attachment.spoiler { $0.animation = nil } }
        }
    }

    @ViewBuilder private func revealedAttachmentView(_ attachment: ChatAttachment) -> some View {
        if attachment.unavailable {
            SentAttachmentPlaceholder(attachment: attachment, label: "File removed")
        } else if attachment.status == .failed {
            SentAttachmentPlaceholder(attachment: attachment, label: "Processing failed")
        } else if attachment.status == .processing {
            AttachmentProcessingView(attachment: attachment, chat: chat)
        } else if attachment.url == nil {
            SentAttachmentPlaceholder(attachment: attachment, label: "File unavailable")
        } else {
            switch attachment.kind {
            case .image: AttachmentImageView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
            case .video:
                if attachment.animated {
                    AttachmentAnimationView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
                } else {
                    AttachmentVideoView(attachment: attachment, chat: chat, openViewer: viewer(attachment))
                }
            case .audio:
                // Recorded voice messages get the voice player and transcript; uploaded audio files keep the card.
                if attachment.voice {
                    VoiceMessageView(attachment: attachment, chat: chat)
                } else {
                    AttachmentAudioView(attachment: attachment, chat: chat)
                }
            case .file: AttachmentOpenCard(attachment: attachment, chat: chat)
            }
        }
    }

    /// Opens this message's images and videos at `attachment`; nil keeps the
    /// file's own behavior (no viewer here, or not a viewable file).
    private func viewer(_ attachment: ChatAttachment) -> (() -> Void)? {
        guard let mediaViewer else { return nil }
        let items = MediaViewerPolicy.items(attachments, revealedSpoilerIDs: revealed)
        guard let start = items.firstIndex(where: { $0.id == attachment.id }) else { return nil }
        return { mediaViewer(MediaViewerRequest(items: items, start: start, chat: chat,
                                                hideSpoiler: { revealed.remove($0) }, isAvailable: isAvailable)) }
    }
}

private struct SentGalleryCellKey: EnvironmentKey {
    static let defaultValue = false
}

private extension EnvironmentValues {
    var sentGalleryCell: Bool {
        get { self[SentGalleryCellKey.self] }
        set { self[SentGalleryCellKey.self] = newValue }
    }
}

/// Geometry depends only on count, never readiness or downloaded dimensions.
enum SentGalleryGeometry {
    static func frames(count: Int, width: CGFloat) -> [CGRect] {
        guard count > 1 else { return [] }
        let width = max(0, min(480, width))
        let height = width * 2 / 3
        if count == 2 {
            let side = max(0, (width - 4) / 2)
            return [CGRect(x: 0, y: 0, width: side, height: height),
                    CGRect(x: side + 4, y: 0, width: side, height: height)]
        }
        if count == 3 {
            let left = max(0, (width - 4) * 2 / 3)
            let right = max(0, (width - 4) / 3)
            let half = max(0, (height - 4) / 2)
            return [CGRect(x: 0, y: 0, width: left, height: height),
                    CGRect(x: left + 4, y: 0, width: right, height: half),
                    CGRect(x: left + 4, y: half + 4, width: right, height: half)]
        }
        let rows = (count + 2) / 3
        let shortRows = rows * 3 - count
        let rowHeight = max(0, count == 4 ? (width - 4) / 2 : (width - 8) / 3)
        return (0..<rows).flatMap { row -> [CGRect] in
            let columns = row < shortRows ? 2 : 3
            let cellWidth = max(0, (width - CGFloat(columns - 1) * 4) / CGFloat(columns))
            return (0..<columns).map { column in
                CGRect(x: CGFloat(column) * (cellWidth + 4), y: CGFloat(row) * (rowHeight + 4), width: cellWidth, height: rowHeight)
            }
        }
    }
}

private struct SentGalleryLayout: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = max(0, min(480, proposal.width ?? 480))
        let frames = SentGalleryGeometry.frames(count: subviews.count, width: width)
        return CGSize(width: width, height: frames.map(\.maxY).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (index, frame) in SentGalleryGeometry.frames(count: subviews.count, width: bounds.width).enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                                  anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }
}

/// Keep cards readable; wrap rather than shrinking filenames and audio controls.
private struct SentFileLayout: Layout {
    private func layout(width: CGFloat, subviews: Subviews) -> [CGRect] {
        let columns = max(1, Int((width + 6) / 226))
        let cardWidth = min(360, max(0, (width - CGFloat(columns - 1) * 6) / CGFloat(columns)))
        var frames: [CGRect] = []
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        for (index, subview) in subviews.enumerated() {
            if index > 0, index % columns == 0 { y += rowHeight + 6; rowHeight = 0 }
            let size = subview.sizeThatFits(ProposedViewSize(width: cardWidth, height: nil))
            frames.append(CGRect(x: CGFloat(index % columns) * (cardWidth + 6), y: y, width: cardWidth, height: size.height))
            rowHeight = max(rowHeight, size.height)
        }
        return frames
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard !subviews.isEmpty else { return .zero }
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? 360
        let frames = layout(width: width, subviews: subviews)
        return CGSize(width: width, height: frames.map(\.maxY).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (index, frame) in layout(width: bounds.width, subviews: subviews).enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                                  anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }
}

private struct SentAttachmentPlaceholder: View {
    let attachment: ChatAttachment
    let label: String
    @Environment(\.sentGalleryCell) private var galleryCell

    var body: some View {
        if galleryCell {
            VStack(spacing: 4) {
                Text(attachment.name).lineLimit(1).truncationMode(.middle)
                Text(label).lineLimit(2)
            }
            .font(CaperTheme.font(11))
            .foregroundStyle(CaperTheme.muted)
            .padding(8)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityElement(children: .combine)
        } else if attachment.status == .failed && !attachment.unavailable {
            AttachmentFailedCard(attachment: attachment)
        } else {
            AttachmentFileCard(attachment: attachment, action: nil)
        }
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
    @Environment(\.sentGalleryCell) private var galleryCell

    @ViewBuilder func body(content: Content) -> some View {
        let size = AttachmentPolicy.displaySize(width: attachment.width, height: attachment.height) ?? fallback
        // Reserve the final aspect ratio before anything loads.
        if galleryCell {
            Color.clear.overlay { content }.clipped()
        } else {
            Color.clear
            .aspectRatio(CGSize(width: size.width, height: size.height), contentMode: .fit)
            .overlay { content }
            .background(CaperTheme.surface)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
            .frame(maxWidth: size.width, alignment: .leading)
        }
    }
}

/// A still only: never constructs the image animation or video playback views.
private struct AttachmentSpoilerView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @Environment(\.sentGalleryCell) private var galleryCell
    @State private var image: CGImage?
    @State private var hovered = false

    private var visual: Bool {
        !attachment.unavailable && attachment.status != .failed &&
            (attachment.kind == .image || attachment.kind == .video)
    }

    var body: some View {
        Group {
            if galleryCell || visual {
                SpoilerThumbnail(image: image, revealHint: true, emphasized: hovered)
                    .modifier(AttachmentFrame(attachment: attachment, fallback: (240, 180)))
            } else {
                SpoilerThumbnail(image: nil, revealHint: true, emphasized: hovered)
                    .frame(maxWidth: .infinity, minHeight: 80, maxHeight: 80)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
        .accessibilityHidden(true)
        .onHover { hovered = $0 }
        .task(id: "\(attachment.id):\(attachment.previewUrl ?? ""):\(attachment.url ?? ""):\(visual)") {
            image = nil
            guard visual else { return }
            // The existing loader caches by id/variant and requests refreshed
            // signatures. Both URLs key this task so either refresh retries.
            if attachment.previewUrl != nil {
                let result = await loadAttachmentImage(attachment, preview: true, chat: chat)
                guard !Task.isCancelled else { return }
                if let still = result.image { image = still; return }
            }
            // Videos without a usable poster stay neutral; do not start a
            // player to obtain one. ImageIO decodes only frame zero of images.
            guard attachment.kind == .image else { return }
            let result = await loadAttachmentImage(attachment, preview: false, chat: chat)
            guard !Task.isCancelled else { return }
            image = result.image
        }
    }
}

/// Blur is unconditional on the image branch, clipped before the readable
/// label is overlaid. No transition can briefly expose the original pixels.
private struct SpoilerThumbnail: View {
    let image: CGImage?
    /// False on a staged tile, which centres a crossed eye instead.
    var labelled = true
    var revealHint = false
    var emphasized = false

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                CaperTheme.surface
                if let image {
                    Image(decorative: image, scale: 1).resizable().scaledToFill()
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .blur(radius: max(geometry.size.width, geometry.size.height) * 0.24, opaque: true)
                        .clipped()
                }
                CaperTheme.blackout.opacity(0.55)
                Canvas { context, size in
                    var lines = Path()
                    for x in stride(from: -size.height, through: size.width, by: 12) {
                        lines.move(to: CGPoint(x: x, y: 0))
                        lines.addLine(to: CGPoint(x: x + size.height, y: size.height))
                    }
                    context.stroke(lines, with: .color(.white.opacity(0.035)), lineWidth: 1)
                }
                .allowsHitTesting(false)
                if labelled { SpoilerLabel(revealHint: revealHint, emphasized: emphasized) }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
        }
        .transaction { $0.animation = nil }
        .accessibilityHidden(true)
    }
}

/// Compact badge on sent spoiler covers (and a spoiler in the attachment editor).
struct SpoilerLabel: View {
    var revealHint = false
    var emphasized = false

    var body: some View {
        VStack(spacing: 3) {
            HStack(spacing: 5) {
                Image(systemName: "eye.slash").font(.system(size: 14)).accessibilityHidden(true)
                Text("Spoiler").font(CaperTheme.font(12, weight: .bold))
            }
            if revealHint {
                #if os(iOS)
                Text("Tap to reveal").font(CaperTheme.font(11))
                #else
                Text("Click to reveal").font(CaperTheme.font(11))
                #endif
            }
        }
        .foregroundStyle(CaperTheme.text)
        .padding(.horizontal, 10).padding(.vertical, 7)
        .background(CaperTheme.blackout.opacity(emphasized ? 0.98 : 0.9), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(emphasized ? CaperTheme.muted : CaperTheme.border))
    }
}

private struct AttachmentImageView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    /// Opens the media viewer; nil opens the file's URL.
    let openViewer: (() -> Void)?
    @Environment(\.openURL) private var openURL
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var image: CGImage?
    @State private var original: AttachmentOriginal?
    @State private var started = Date()
    @State private var attempt = 0
    @State private var failed = false

    private var gif: Bool { attachment.contentType == "image/gif" }

    var body: some View {
        Button {
            // A drawer swipe that started on the image lifts on it too.
            guard !SwipeTapGuard.swiping else { return }
            if let openViewer { openViewer() } else { openAttachment(attachment, chat: chat, openURL: openURL) }
        } label: {
            ZStack {
                if let original, let first = original.images.first {
                    if reduceMotion || original.images.count == 1 {
                        Image(decorative: first, scale: 1).resizable().scaledToFill()
                    } else {
                        TimelineView(.animation(minimumInterval: 0.02)) { timeline in
                            let index = MediaViewerPolicy.frameIndex(at: timeline.date.timeIntervalSince(started), durations: original.durations)
                            Image(decorative: original.images[index], scale: 1).resizable().scaledToFill()
                        }
                    }
                } else if let image = image ?? AttachmentImageLoader.shared.cachedPreview(for: attachment.id) {
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
        .task(id: gif ? "gif:\(attempt)" : (attachment.previewUrl ?? attachment.url)) {
            if gif {
                let result = await loadAttachmentOriginal(attachment, chat: chat, inline: true)
                guard !Task.isCancelled else { return }
                original = result.frames
                failed = result.failed
                started = Date()
                if result.refusedURL, attempt == 0 { chat?.requestFreshAttachmentURLs(ids: [attachment.id]) }
                return
            }
            let result = await loadAttachmentImage(attachment, preview: true, chat: chat)
            if let loaded = result.image { image = loaded; failed = false } else if image == nil { failed = result.failed }
        }
        .onChange(of: attachment.url) { _, _ in
            guard gif, failed, attempt == 0 else { return }
            attempt += 1
        }
        .onDisappear { original = nil }
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
                Button {
                    guard !SwipeTapGuard.swiping else { return }
                    if let openViewer { openViewer() } else { start() }
                } label: {
                    ZStack {
                        if let poster = poster ?? AttachmentImageLoader.shared.cachedPreview(for: attachment.id) {
                            Image(decorative: poster, scale: 1).resizable().scaledToFill()
                        }
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
            Button(action: SwipeTapGuard.unlessSwiping(toggle)) {
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
            Button(action: SwipeTapGuard.unlessSwiping(action)) { card.contentShape(Rectangle()) }
                .buttonStyle(.plain)
                .accessibilityLabel("File \(attachment.name), \(AttachmentPolicy.formatBytes(attachment.size))")
                .accessibilityHint("Opens the file")
        } else {
            card.accessibilityElement(children: .combine)
        }
    }

    private var card: some View {
        HStack(spacing: 10) {
            // Web's FileText, for removed files too.
            CaperIcon(name: "file-text", size: 18).foregroundStyle(CaperTheme.muted)
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
        let preview = self.preview ?? AttachmentImageLoader.shared.cachedPreview(for: attachment.id)
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
                if let loaded = await loadAttachmentImage(attachment, preview: true, chat: chat).image { self.preview = loaded }
            } else if let localCopy, AttachmentKind.local(contentType: attachment.contentType) == .image {
                // The sender sees its own original until the server's preview or result arrives.
                if let loaded = try? await AttachmentImageLoader.shared.load(key: "local:\(attachment.id)", url: localCopy, maxPixelSize: 1200) {
                    self.preview = loaded
                }
            }
        }
    }
}

private struct AttachmentFailedCard: View {
    let attachment: ChatAttachment

    var body: some View {
        HStack(spacing: 10) {
            CaperIcon(name: "file-warning", size: 18).foregroundStyle(CaperTheme.muted)
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
/// Also plays link preview GIF videos (EmbedViews).
@MainActor
final class LoopingPlayback {
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
        // Only settled states reach the loop: KVO's initial `.unknown` would
        // use up the async sequence's one-value demand, and the change to
        // `.ready` arriving before the next request was dropped, so this
        // never returned and looping clips never started.
        for await status in looper.publisher(for: \.status).filter({ $0 != .unknown }).values {
            return status == .ready
        }
        return false
    }
}

#if os(iOS)
final class PlayerLayerUIView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
}

/// A bare video surface: no controls, no audio, no hit testing.
struct PlayerSurface: UIViewRepresentable {
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
final class PlayerLayerNSView: NSView {
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
struct PlayerSurface: NSViewRepresentable {
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
    /// The poster stays alone until the file can play: the player surface
    /// adds nothing before its first frame.
    @State private var playbackReady = false
    @State private var poster: CGImage?
    @State private var playing = false
    @State private var userPaused = false
    @State private var failed = false
    @State private var attempt = 0

    var body: some View {
        ZStack {
            if let poster = poster ?? AttachmentImageLoader.shared.cachedPreview(for: attachment.id) {
                Image(decorative: poster, scale: 1).resizable().scaledToFill()
            }
            if let playback, playbackReady { PlayerSurface(player: playback.player) }
            if !playing {
                Image(systemName: "play.fill").font(.system(size: 14, weight: .bold)).foregroundStyle(.white)
                    .frame(width: 36, height: 36).background(Circle().fill(Color.black.opacity(0.6)))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(AttachmentFrame(attachment: attachment, fallback: (240, 180)))
        .contentShape(Rectangle())
        .onTapGesture {
            guard !SwipeTapGuard.swiping else { return }
            if let openViewer { openViewer() } else { toggle() }
        }
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
            playbackReady = false
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
        if loaded, playback === created { playbackReady = true }
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

/// Decode the original, not an avatar-sized thumbnail, for the native clipboard.
@MainActor func copyMediaImage(_ url: URL) async throws -> Bool {
    let image = try await AttachmentImageLoader.shared.clipboardImage(url)
    #if os(iOS)
    UIPasteboard.general.image = UIImage(cgImage: image)
    return true
    #else
    NSPasteboard.general.clearContents()
    return NSPasteboard.general.writeObjects([NSImage(cgImage: image, size: .zero)])
    #endif
}

/// One message's images and videos (`MediaViewerPolicy.items`), opened at
/// the one chosen.
struct MediaViewerRequest: Identifiable {
    let id = UUID()
    let items: [ChatAttachment]
    let start: Int
    /// Follows refreshed URLs and removals; nil for another conversation's
    /// files, shown as delivered.
    let chat: ChatModel?
    let hideSpoiler: (String) -> Void
    var isAvailable: (() -> Bool)? = nil
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
#if os(iOS)
struct MediaViewerHost: ViewModifier {
    @State private var request: MediaViewerRequest?

    @ViewBuilder func body(content: Content) -> some View {
        content
            .environment(\.mediaViewer, MediaViewerAction { request = $0 })
            .fullScreenCover(item: $request) { shown in
                MediaViewer(request: shown) { request = nil }.presentationBackground(.black).preferredColorScheme(.dark)
            }
    }
}
#endif

/// Discord-style viewer for one message's images and videos. Pages swipe on
/// iPhone and iPad; Previous, Next and the arrow keys step on the Mac, on
/// iPad and with VoiceOver. Close, Escape, a click outside the media on the
/// Mac, or a swipe down on phones closes it.
#if os(iOS)
struct MediaViewer: View {
    let request: MediaViewerRequest
    let close: () -> Void
    var body: some View { NativeMediaViewerBridge(request: request, close: close) }
}
#endif

/// Round viewer controls, with a compact labelled variant for spoiler hiding.
private struct MediaViewerButton: ButtonStyle {
    var labelled = false
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, labelled ? 8 : 0)
            .frame(width: labelled ? nil : 36, height: 36)
            .background {
                if labelled {
                    RoundedRectangle(cornerRadius: 8).fill(CaperTheme.blackout.opacity(configuration.isPressed ? 0.98 : 0.9))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(CaperTheme.border))
                } else {
                    Circle().fill(Color.black.opacity(configuration.isPressed ? 0.8 : 0.55))
                        .overlay(Circle().stroke(Color.white.opacity(0.14)))
                }
            }
            #if os(iOS)
            .frame(minWidth: 44, minHeight: 44)
            #endif
            .contentShape(Rectangle())
            .opacity(isEnabled ? 1 : 0.35)
            .modifier(ControlPointer())
    }
}

// MARK: Composer

/// Compact, wrapping draft previews shared by channel and thread composers.
#if os(iOS)
struct AttachmentDraftsView: View {
    @Bindable var chat: ChatModel
    var inThread = false
    @State private var previewID: String?
    @Environment(\.attachmentEditor) private var editor
    private var files: [AttachmentDraft] { chat.attachmentFiles(inThread: inThread) }

    var body: some View {
        if !files.isEmpty {
            let failed = files.filter { $0.error != nil }.count
            if failed > 0 {
                Text(failed == 1 ? "1 file failed to upload. Send will retry it, or you can remove it." : "\(failed) files failed to upload. Send will retry them, or you can remove them.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                    .padding(8).frame(maxWidth: .infinity, alignment: .leading)
                    .background(CaperTheme.terracotta.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
            }
            ForEach(files.filter { $0.error == AttachmentMetadata.refusalMessage }) { draft in
                Text("\(draft.spoiler ? "Spoiler attachment" : draft.name): \(AttachmentMetadata.refusalMessage)")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("attachment-metadata-refusal")
            }
            StagedFileLayout() {
                ForEach(files) { draft in chip(draft).modifier(Entrance.chip) }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Files to send")
            .quickLookPreview(Binding(
                get: { files.first(where: { $0.id == previewID }).map { Self.shownURL($0) } },
                set: { if $0 == nil { previewID = nil } }
            ))
        }
    }

    /// A staged voice note gets its own chip (play, waveform, duration, Remove).
    @ViewBuilder private func chip(_ draft: AttachmentDraft) -> some View {
        if let duration = draft.voiceDurationMs {
            VoiceDraftChip(id: draft.id, url: draft.localURL, durationMs: duration, waveform: draft.voiceWaveform,
                           status: draft.error ?? (draft.attachment == nil ? "Uploading…" : nil), failed: draft.error != nil,
                           retry: draft.error == nil ? nil : { chat.retryAttachmentDraft(id: draft.id) },
                           remove: { chat.removeAttachmentDraft(id: draft.id) })
        } else {
            AttachmentDraftChip(draft: draft, previewable: canEdit(draft), preview: { preview(draft) },
                                retry: { chat.retryAttachmentDraft(id: draft.id) },
                                toggleSpoiler: { chat.toggleAttachmentSpoiler(id: draft.id) },
                                remove: { chat.removeAttachmentDraft(id: draft.id) })
        }
    }

    /// While an edit prepares, its render rather than the unedited original.
    private static func shownURL(_ draft: AttachmentDraft) -> URL {
        draft.preparing ? (draft.editPreviewURL ?? draft.localURL) : draft.localURL
    }

    /// Files the editor can open show in it, viewing with no tool picked,
    /// with Spoiler and Remove beside its tools (the pencil opens it too);
    /// QuickLook shows the rest.
    private func preview(_ draft: AttachmentDraft) {
        guard let editor, canEdit(draft), let request = editRequest(draft) else { previewID = draft.id; return }
        editor(request)
    }

    /// Files the editor can open, below an `AttachmentEditorHost`.
    private func canEdit(_ draft: AttachmentDraft) -> Bool { editor != nil && draft.editable && draft.original != nil }

    private func editRequest(_ draft: AttachmentDraft) -> AttachmentEditRequest? {
        guard canEdit(draft), let original = draft.original else { return nil }
        let id = draft.id
        let chat = chat, inThread = inThread
        let shown = Self.shownURL(draft)
        let spoiler = AttachmentDraftSpoiler(
            isOn: { chat.attachmentFiles(inThread: inThread).first(where: { $0.id == id })?.spoiler ?? false },
            toggle: { chat.toggleAttachmentSpoiler(id: id) })
        return AttachmentEditRequest(name: draft.name, kind: AttachmentKind.local(contentType: original.contentType),
                                     original: original.url, edit: draft.edit,
                                     // The staged video as it will be sent; not its still render while it prepares.
                                     playback: draft.kind == .video && shown != draft.editPreviewURL ? shown : nil,
                                     spoiler: spoiler, remove: { chat.removeAttachmentDraft(id: id) }) { edit, preview in
            chat.editAttachmentDraft(id: id, edit: edit, preview: preview)
        }
    }
}
#endif

private struct AttachmentDraftChip: View {
    let draft: AttachmentDraft
    let previewable: Bool
    let preview: () -> Void
    let retry: () -> Void
    let toggleSpoiler: () -> Void
    let remove: () -> Void

    var body: some View {
        StagedFileChip(id: draft.id, name: draft.name, kind: draft.kind, localURL: draft.localURL, thumbnailURL: draft.editPreviewURL,
                       status: draft.statusLabel, failed: draft.error != nil,
                       busyLabel: draft.attachment == nil && draft.error == nil ? (draft.preparing ? "Preparing…" : "Uploading…") : nil,
                       // Compression reports no progress, so its ring spins; then the upload's fills.
                       progress: draft.attachment == nil && draft.error == nil && !draft.preparing ? draft.progress : nil,
                       spoiler: draft.spoiler, toggleSpoiler: toggleSpoiler,
                       retry: draft.error == nil ? nil : retry, previewable: previewable, preview: preview, remove: remove)
    }
}

/// Compact squares wrap at the composer's edge on macOS; iOS caps at four
/// columns. A single attachment never stretches to fill the available width.
#if os(iOS)
struct StagedFileLayout: Layout {
    private func metrics(width: CGFloat?) -> (columns: Int, side: CGFloat) {
        let width = max(0, width ?? 440)
        let columns = min(4, max(1, Int(min(width, 440) + 8) / 98))
        return (columns, min(104, max(0, (width - CGFloat(columns - 1) * 8) / CGFloat(columns))))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard !subviews.isEmpty else { return .zero }
        let (columns, side) = metrics(width: proposal.width)
        let rows = (subviews.count + columns - 1) / columns
        return CGSize(width: proposal.width ?? (CGFloat(columns) * (side + 8) - 8),
                      height: CGFloat(rows) * (side + 8) - 8)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (columns, side) = metrics(width: bounds.width)
        for (index, subview) in subviews.enumerated() {
            subview.place(at: CGPoint(x: bounds.minX + CGFloat(index % columns) * (side + 8),
                                      y: bounds.minY + CGFloat(index / columns) * (side + 8)),
                          anchor: .topLeading, proposal: ProposedViewSize(width: side, height: side))
        }
    }
}
#endif

/// A staged square. Tapping it previews it; Remove (×) sits in the top-right
/// corner and Edit (pencil) in the bottom-left, diagonal so their targets never
/// overlap and a mis-tap can't remove. While the file prepares or uploads,
/// Remove's face is its progress ring; a failed upload shows a centred Retry.
/// Filenames and full status stay accessible without covering thumbnails.
struct StagedFileChip: View {
    let id: String
    let name: String
    let kind: AttachmentKind
    let localURL: URL
    /// An edited file's render, shown in place of `localURL` (videos too).
    var thumbnailURL: URL?
    let status: String
    var failed = false
    /// "Preparing…" or "Uploading…" until it's uploaded: Remove shows the ring.
    var busyLabel: String?
    /// Progress (0…1) when known; nil while busy shows an indeterminate ring.
    var progress: Double?
    /// False while sending: no corner buttons and no menu items.
    var removable = true
    /// Marked as a spoiler: the thumbnail is blurred and labels leave the name unsaid.
    var spoiler = false
    /// Marks or unmarks it, from the pencil's menu, the context menu or the
    /// editor; unavailable while it can't be removed (sending).
    var toggleSpoiler: (() -> Void)?
    /// Offered for a failed upload.
    var retry: (() -> Void)?
    /// `preview` opens the editor, viewing, with its Spoiler, editing and
    /// Remove tools (stills and videos it can open), so the pencil opens it
    /// too. Other files' pencil opens a menu instead.
    var previewable = false
    let preview: () -> Void
    let remove: () -> Void
    @State private var thumbnail: CGImage?

    var body: some View {
        ZStack {
            Button(action: preview) {
                GeometryReader { geometry in
                    Group {
                        if spoiler {
                            SpoilerThumbnail(image: failed ? nil : thumbnail, labelled: false)
                        } else if let thumbnail {
                            Image(decorative: thumbnail, scale: 1).resizable().scaledToFill()
                        } else if kind == .video {
                            Color.clear
                        } else {
                            VStack(spacing: 6) {
                                if kind == .file {
                                    CaperIcon(name: "file-text", size: 22)
                                } else {
                                    Image(systemName: icon).font(.system(size: 22))
                                }
                                Text(kindLabel).font(CaperTheme.font(11, weight: .bold))
                            }
                            .foregroundStyle(CaperTheme.muted)
                        }
                    }
                    .frame(width: geometry.size.width, height: geometry.size.height)
                    .clipped()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipped()
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(spoiler ? "Preview spoiler attachment" : "Preview \(name)")
            .accessibilityValue(accessibilityState)
            .help(spoiler ? "Spoiler attachment" : name)
            .contextMenu {
                menuItems(Self.menuActions(spoilerToggle: spoilerToggleAvailable, removable: removable, previewable: previewable))
            }
            centreGlyph.allowsHitTesting(false)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(CaperTheme.surface)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(failed ? Color(red: 138/255, green: 67/255, blue: 52/255) : CaperTheme.border))
        // Retry sits above the corners, so a tap where their targets meet retries rather than removes.
        .overlay(alignment: .topTrailing) { if removable { removeButton } }
        .overlay(alignment: .bottomLeading) { if removable { editButton } }
        .overlay { if failed, let retry { retryButton(retry) } }
        .task(id: thumbnailURL ?? localURL) {
            let source = thumbnailURL ?? localURL
            // Each prepared or edited file has its own staging directory; its name may repeat.
            let key = "draft:\(id):\(source.path)"
            thumbnail = nil
            let loaded: CGImage?
            if thumbnailURL != nil || kind == .image {
                // An edit's render is a still, for videos too.
                loaded = try? await AttachmentImageLoader.shared.load(key: key, url: source, maxPixelSize: 312)
            } else if kind == .video {
                loaded = try? await AttachmentImageLoader.shared.loadVideo(key: key, url: source, maxPixelSize: 312)
            } else {
                loaded = nil
            }
            if !Task.isCancelled { thumbnail = loaded }
        }
    }

    /// The context menu, top to bottom: Edit (opens the editor) for files it
    /// can show, the spoiler toggle where offered, then Remove. Nothing while
    /// the file can't be removed (sending).
    static func menuActions(spoilerToggle: Bool, removable: Bool, previewable: Bool) -> [StagedFileAction] {
        guard removable else { return [] }
        var actions: [StagedFileAction] = []
        if previewable { actions.append(.edit) }
        if spoilerToggle { actions.append(.spoiler) }
        actions.append(.remove)
        return actions
    }

    /// The pencil's menu on files the editor can't open (PDFs, audio, others).
    static func pencilMenu(spoilerToggle: Bool) -> [StagedFileAction] {
        spoilerToggle ? [.spoiler, .remove] : [.remove]
    }

    private var spoilerToggleAvailable: Bool { toggleSpoiler != nil && removable }
    private var busy: Bool { busyLabel != nil && !failed }

    /// The status Remove's ring shows, for assistive technologies. Never the
    /// file's name, so it's spoiler-safe.
    private var busyStatus: String? {
        guard busy else { return nil }
        let label = busyLabel ?? "Uploading…"
        guard let progress, progress.isFinite else { return label }
        return "\(label) \(Int((min(1, max(0, progress)) * 100).rounded()))%"
    }

    private var accessibilityState: String {
        guard spoiler else { return busyStatus ?? status }
        if failed { return "Spoiler, upload failed" }
        return busyStatus.map { "Spoiler, \($0)" } ?? "Spoiler"
    }

    /// The crossed eye for a spoiler, or a video's play badge.
    @ViewBuilder private var centreGlyph: some View {
        if failed && retry != nil {
            // Retry takes the centre.
            EmptyView()
        } else if spoiler {
            Image(systemName: "eye.slash")
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(CaperTheme.text)
                .accessibilityHidden(true)
        } else if kind == .video {
            Image(systemName: "play.fill")
                .font(.system(size: 14, weight: .bold))
                .foregroundStyle(.white.opacity(0.72))
                .frame(width: 32, height: 32)
                .background(.black.opacity(0.32), in: Circle())
                .accessibilityLabel("Video")
        }
    }

    @ViewBuilder private func menuItems(_ actions: [StagedFileAction]) -> some View {
        ForEach(actions, id: \.self) { action in
            switch action {
            case .edit:
                Button(action: preview) { Label("Edit", systemImage: "pencil") }
            case .spoiler:
                Button { toggleSpoiler?() } label: {
                    Label(spoiler ? "Remove spoiler" : "Mark as spoiler", systemImage: spoiler ? "eye" : "eye.slash")
                }
            case .remove:
                Button(role: .destructive, action: remove) { Label("Remove attachment", systemImage: "trash") }
            }
        }
    }

    /// × in the top-right corner; its face is the progress ring while the file
    /// prepares or uploads. Tapping it removes either way.
    private var removeButton: some View {
        Button(action: remove) {
            ZStack {
                if busy { StagedFileProgressRing(state: StagedFileProgress.ring(busy: busy, progress: progress)) }
                Image(systemName: "xmark").font(.system(size: 11, weight: .semibold))
            }
        }
        .buttonStyle(StagedFileCornerStyle(corner: .topTrailing, ringed: busy))
        .help("Remove")
        .accessibilityLabel(spoiler ? "Remove spoiler attachment" : "Remove \(name)")
        .accessibilityValue(busyStatus ?? "")
        .accessibilityIdentifier("staged-file-remove")
    }

    /// The pencil in the bottom-left corner: the editor, viewing, or a menu
    /// for files it can't open.
    @ViewBuilder private var editButton: some View {
        if previewable {
            Button(action: preview) { pencil }
                .buttonStyle(StagedFileCornerStyle(corner: .bottomLeading))
                .help("Edit")
                .accessibilityLabel(spoiler ? "Edit spoiler attachment" : "Edit \(name)")
                .accessibilityHint("Opens the editor with spoiler, editing and remove tools")
                .accessibilityIdentifier("staged-file-edit")
        } else {
            Menu {
                menuItems(Self.pencilMenu(spoilerToggle: spoilerToggleAvailable))
            } label: {
                StagedFileCornerFace(corner: .bottomLeading) { pencil }
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Edit")
            .accessibilityLabel(spoiler ? "Edit spoiler attachment" : "Edit \(name)")
            .accessibilityIdentifier("staged-file-edit")
        }
    }

    private var pencil: some View {
        Image(systemName: "pencil").font(.system(size: 11, weight: .semibold))
    }

    private func retryButton(_ retry: @escaping () -> Void) -> some View {
        Button(action: retry) {
            Image(systemName: "arrow.clockwise").font(.system(size: 13, weight: .bold))
        }
        .buttonStyle(StagedFileRetryStyle())
        .help("Retry upload")
        .accessibilityLabel(spoiler ? "Retry uploading spoiler attachment" : "Retry uploading \(name)")
        .accessibilityHint(spoiler ? "Upload failed" : status)
        .accessibilityIdentifier("staged-file-retry")
    }

    private var kindLabel: String {
        switch kind {
        case .image: return "Image"
        case .video: return "Video"
        case .audio: return "Audio"
        case .file: return "File"
        }
    }

    private var icon: String {
        switch kind {
        case .image: return "photo"
        case .video: return "film"
        case .audio: return "waveform"
        case .file: return "doc.text"
        }
    }
}

/// A staged tile's menu items (the pencil's menu on files the editor can't
/// open, and the context menu).
enum StagedFileAction: Hashable { case edit, spoiler, remove }

/// Remove's progress ring on a staged tile.
enum StagedFileProgress: Equatable {
    case idle
    /// Spinning (still under Reduce Motion): preparing, or nothing reported yet.
    case indeterminate
    case determinate(Double)

    /// Compression progress when it's reported, else the upload's (callers
    /// pass whichever applies); indeterminate when neither is known.
    static func ring(busy: Bool, progress: Double?) -> StagedFileProgress {
        guard busy else { return .idle }
        guard let progress, progress.isFinite else { return .indeterminate }
        return .determinate(min(1, max(0, progress)))
    }
}

/// The corner buttons' size: a 22-point face 4 points from the tile's edges,
/// inside a 44-point target anchored in that corner on iPhone and iPad (28 on
/// the Mac's pointer).
enum StagedFileCorner {
    static let face: CGFloat = 22
    static let inset: CGFloat = 4
    #if os(iOS)
    static let target: CGFloat = 44
    static let retryTarget: CGFloat = 44
    #else
    static let target: CGFloat = 28
    static let retryTarget: CGFloat = 34
    #endif
    static let retryFace: CGFloat = 30
}

/// A terracotta arc over a faint white track, in Remove's face.
private struct StagedFileProgressRing: View {
    let state: StagedFileProgress
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private static let lineWidth: CGFloat = 2

    var body: some View {
        ZStack {
            Circle().inset(by: Self.lineWidth / 2).stroke(Color.white.opacity(0.16), lineWidth: Self.lineWidth)
            switch state {
            case .idle:
                EmptyView()
            case .determinate(let value):
                arc(to: value)
                    // Glides between progress events instead of jumping.
                    .animation(reduceMotion ? nil : .linear(duration: CaperMotion.base), value: value)
            case .indeterminate:
                if reduceMotion {
                    arc(to: 0.28)
                } else {
                    TimelineView(.animation) { context in
                        let turn = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1)
                        arc(to: 0.28).rotationEffect(.degrees(turn * 360))
                    }
                }
            }
        }
        .frame(width: StagedFileCorner.face, height: StagedFileCorner.face)
        .accessibilityHidden(true)
    }

    private func arc(to end: Double) -> some View {
        Circle()
            .inset(by: Self.lineWidth / 2)
            .trim(from: 0, to: end)
            .stroke(CaperTheme.terracottaBright, style: StrokeStyle(lineWidth: Self.lineWidth, lineCap: .round))
            .rotationEffect(.degrees(-90))
    }
}

private struct StagedFileCornerStyle: ButtonStyle {
    let corner: Alignment
    var ringed = false

    func makeBody(configuration: Configuration) -> some View {
        StagedFileCornerFace(corner: corner, ringed: ringed, pressed: configuration.isPressed) { configuration.label }
    }
}

/// A round blackout face (78%) with a faint white edge, lightening under a
/// hovering pointer (12%) or a press (18%), with a terracotta ring for
/// keyboard focus. The target is a box in the tile's corner around it.
private struct StagedFileCornerFace<Content: View>: View {
    let corner: Alignment
    let ringed: Bool
    let pressed: Bool
    let content: Content
    @Environment(\.isFocused) private var focused
    @State private var hovered = false

    init(corner: Alignment, ringed: Bool = false, pressed: Bool = false, @ViewBuilder content: () -> Content) {
        self.corner = corner
        self.ringed = ringed
        self.pressed = pressed
        self.content = content()
    }

    var body: some View {
        content
            .foregroundStyle(CaperTheme.text)
            .frame(width: StagedFileCorner.face, height: StagedFileCorner.face)
            .background {
                ZStack {
                    Circle().fill(CaperTheme.blackout.opacity(0.78))
                    Circle().fill(Color.white.opacity(pressed ? 0.18 : hovered ? 0.12 : 0))
                    // The ring's track takes the edge's place.
                    if !ringed { Circle().strokeBorder(Color.white.opacity(0.12), lineWidth: 1) }
                }
            }
            .overlay {
                Circle()
                    .inset(by: -2)
                    .strokeBorder(CaperTheme.terracotta, lineWidth: 2)
                    .opacity(focused ? 1 : 0)
                    .allowsHitTesting(false)
            }
            .padding(StagedFileCorner.inset)
            .frame(width: StagedFileCorner.target, height: StagedFileCorner.target, alignment: corner)
            .contentShape(Rectangle())
            .focusEffectDisabled()
            .onHover { hovered = $0 }
            #if os(iOS)
            // The face's own highlight, not the iPad pointer's platter.
            .hoverEffectDisabled()
            #endif
            .modifier(ControlPointer())
    }
}

/// The eggshell primary action as a round 30-point face, centred on a failed tile.
private struct StagedFileRetryStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { StagedFileRetryFace(configuration: configuration) }
}

private struct StagedFileRetryFace: View {
    let configuration: ButtonStyleConfiguration
    @Environment(\.isFocused) private var focused
    @State private var hovered = false

    var body: some View {
        configuration.label
            .foregroundStyle(CaperTheme.blackout)
            .frame(width: StagedFileCorner.retryFace, height: StagedFileCorner.retryFace)
            .background(configuration.isPressed ? CaperTheme.actionPress : hovered ? CaperTheme.actionHover : CaperTheme.action, in: Circle())
            .overlay {
                Circle()
                    .inset(by: -2)
                    .strokeBorder(CaperTheme.terracotta, lineWidth: 2)
                    .opacity(focused ? 1 : 0)
                    .allowsHitTesting(false)
            }
            .frame(width: StagedFileCorner.retryTarget, height: StagedFileCorner.retryTarget)
            .contentShape(Rectangle())
            .focusEffectDisabled()
            .onHover { hovered = $0 }
            #if os(iOS)
            .hoverEffectDisabled()
            #endif
            .modifier(ControlPointer())
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

/// A file dropped on the conversation. Photos and videos (from Photos, Files
/// or Finder) arrive as their media; other files from Finder or Files as
/// file URLs. Web links are not files and are refused.
struct DroppedAttachment: Transferable, Sendable {
    let file: LocalAttachmentFile

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .movie) { received in
            DroppedAttachment(file: try AttachmentStaging.stage(copying: received.file))
        }
        FileRepresentation(importedContentType: .image) { received in
            DroppedAttachment(file: try AttachmentStaging.stage(copying: received.file))
        }
        FileRepresentation(importedContentType: .audio) { received in
            DroppedAttachment(file: try AttachmentStaging.stage(copying: received.file))
        }
        FileRepresentation(importedContentType: .pdf) { received in
            DroppedAttachment(file: try AttachmentStaging.stage(copying: received.file))
        }
        ProxyRepresentation(importing: { (url: URL) in
            guard url.isFileURL else { throw CocoaError(.fileReadUnsupportedScheme) }
            return DroppedAttachment(file: try AttachmentStaging.stage(copying: url))
        })
    }
}

/// The drop overlay's motion, as on web (`apps/web/src/chat/drop.ts`), Android
/// and Rust desktop (`file_drop.rs`). Each open picks a random seed, so the cards
/// drift and the character hops differently every time; every client derives
/// the same motion from a seed (shared/messages/drop-motion-cases.json). Times
/// are seconds since the drag arrived; lengths are points.
enum FileDropMotion {
    /// Resting pose per card, left to right: offset and turn in degrees.
    static let poses: [(x: Double, y: Double, turn: Double)] = [(-46, 10, -16), (0, -26, 0), (46, 10, 16)]
    static let gravity = 420.0
    static let crouch = 0.12

    /// mulberry32: small, and identical on every client's integer arithmetic.
    struct Random {
        var state: UInt32
        init(seed: UInt32) { state = seed }
        mutating func next() -> Double {
            state = state &+ 0x6D2B_79F5
            var t = (state ^ (state >> 15)) &* (1 | state)
            t = (t &+ ((t ^ (t >> 7)) &* (61 | t))) ^ t
            return Double(t ^ (t >> 14)) / 4_294_967_296
        }
    }

    struct Card {
        /// Offset from the resting pose, so no two opens rest the same way.
        var jitter: (x: Double, y: Double, turn: Double)
        var delay: Double
        var duration: Double
        /// Two (period, phase) waves each for x, y and turn.
        var waves: [Double]
    }

    struct Plan {
        var cards: [Card]
        var tilt: (period: Double, phase: Double)
        var hops: [(start: Double, height: Double)]

        init(seed: UInt32) {
            var random = Random(seed: seed)
            func spread(_ size: Double) -> Double { (random.next() * 2 - 1) * size }
            cards = poses.map { _ in
                let jitter = (x: spread(3), y: spread(3), turn: spread(3))
                let delay = 0.04 + random.next() * 0.14
                let duration = 1.3 + random.next() * 0.6
                let waves = (0..<6).flatMap { _ in [4 + random.next() * 5, random.next() * 2 * .pi] }
                return Card(jitter: jitter, delay: delay, duration: duration, waves: waves)
            }
            let period = 5 + random.next() * 3
            tilt = (period, random.next() * 2 * .pi)
            hops = []
            var start = 0.75 + random.next() * 0.5
            for _ in 0..<32 {
                let height = 5 + random.next() * 7
                let quick = random.next() < 0.25
                let pause = random.next()
                hops.append((start, height))
                start += crouch + air(height) + air(height * 0.2) + (quick ? 0.06 + pause * 0.1 : 0.7 + pause * 1.8)
            }
        }
    }

    struct Pose {
        /// Offset from the stage's bottom centre, turn in degrees, and opacity.
        var cards: [(x: Double, y: Double, turn: Double, opacity: Double)]
        /// How far below its resting place, squash, turn and opacity.
        var character: (y: Double, scaleX: Double, scaleY: Double, turn: Double, opacity: Double)
    }

    /// Seconds in the air for a hop `height` high.
    static func air(_ height: Double) -> Double { 2 * (2 * height / gravity).squareRoot() }

    /// A damped spring's response `elapsed` after a kick, peaking near `size`.
    static func spring(_ size: Double, _ elapsed: Double) -> Double {
        elapsed <= 0 ? 0 : size * exp(-6 * elapsed) * sin(16 * elapsed) / 0.6
    }

    static func progress(_ elapsed: Double, delay: Double, duration: Double) -> Double {
        min(max((elapsed - delay) / duration, 0), 1)
    }

    static func easeOutCubic(_ t: Double) -> Double { 1 - pow(1 - t, 3) }

    /// Where everything is `elapsed` seconds in; `still` is the reduced-motion resting pose.
    static func pose(_ plan: Plan, _ elapsed: Double, still: Bool = false) -> Pose {
        let t = still ? 0 : elapsed
        func wave(_ waves: [Double], _ at: Int) -> Double {
            0.6 * sin(2 * .pi * t / waves[at] + waves[at + 1]) + 0.4 * sin(2 * .pi * t / waves[at + 2] + waves[at + 3])
        }
        let cards = plan.cards.enumerated().map { index, card in
            let rest = poses[index]
            let out = easeOutCubic(still ? 1 : progress(t, delay: card.delay, duration: card.duration))
            let drift = still ? 0 : out
            return (x: (rest.x + card.jitter.x) * out + 2.5 * wave(card.waves, 0) * drift,
                    // Each card slides out and up from just below the middle.
                    y: 24 + (rest.y + card.jitter.y - 24) * out + 3.5 * wave(card.waves, 4) * drift,
                    turn: rest.turn + card.jitter.turn + 2.5 * wave(card.waves, 8) * drift,
                    opacity: still ? 1 : progress(t, delay: card.delay, duration: 0.3))
        }
        let rise = still ? 1 : easeOutCubic(progress(t, delay: 0.08, duration: 0.5))
        var lift = 0.0, squash = 0.0
        if !still {
            for hop in plan.hops {
                let since = t - hop.start
                if since < 0 { break }
                let up = air(hop.height), rebound = air(hop.height * 0.2), flying = since - crouch
                if flying >= 0 && flying < up {
                    lift = gravity / 2 * flying * (up - flying)
                } else if flying >= up && flying < up + rebound {
                    lift = gravity / 2 * (flying - up) * (up + rebound - flying)
                }
                // Crouch, spring up, land, and settle after the rebound; higher hops squash more.
                let size = hop.height / 9
                squash += spring(-0.05 * size, since) + spring(0.09 * size, flying)
                    + spring(-0.1 * size, flying - up) + spring(-0.035 * size, flying - up - rebound)
            }
        }
        let turn = still ? 0 : 2 * sin(2 * .pi * t / plan.tilt.period + plan.tilt.phase) * rise
        return Pose(cards: cards,
                    character: (y: 28 * (1 - rise) - lift, scaleX: 1 - 0.8 * squash, scaleY: 1 + squash,
                                turn: turn, opacity: still ? 1 : progress(t, delay: 0.08, duration: 0.2)))
    }
}

/// Covers the conversation while files are dragged over it: a random Caper
/// character hops in front of three file cards that drift apart behind it.
/// Each open picks a new character and a new seed for the motion.
struct FileDropOverlay: View {
    let title: String
    let detail: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var start = Date()
    private static let fixture = ProcessInfo.processInfo.environment["CAPER_TEST_MODE"] == "parity"
    @State private var character = Self.fixture ? 0 : Int.random(in: 0..<CaperDailyIcon.count)
    @State private var plan = FileDropMotion.Plan(seed: Self.fixture ? 0 : UInt32.random(in: .min ... .max))

    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { timeline in
            let elapsed = timeline.date.timeIntervalSince(start)
            let rest = reduceMotion
            let pop: Double = rest ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.26)
            let scrim: Double = rest ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.14)
            let shown: Double = rest ? 1 : FileDropMotion.progress(elapsed, delay: 0, duration: 0.12)
            ZStack {
                CaperTheme.blackout.opacity(0.82 * scrim)
                VStack(spacing: 6) {
                    art(FileDropMotion.pose(plan, elapsed, still: rest)).padding(.bottom, 8)
                    Text(title).font(CaperTheme.font(18, weight: .bold)).foregroundStyle(CaperTheme.text)
                    Text(detail).font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                }
                .multilineTextAlignment(.center)
                .padding(.top, 20).padding(.bottom, 26).padding(.horizontal, 24)
                .frame(maxWidth: 360)
                .background(RoundedRectangle(cornerRadius: 8).fill(CaperTheme.surface))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(CaperTheme.border))
                .overlay(Rectangle().stroke(CaperTheme.terracotta, style: StrokeStyle(lineWidth: 1.5, dash: [5, 4])).padding(9.75))
                .shadow(color: .black.opacity(0.45), radius: 24, y: 18)
                .opacity(shown)
                .offset(y: 10 * (1 - FileDropMotion.easeOutCubic(pop)))
                .padding(16)
            }
        }
        .allowsHitTesting(false)
        .accessibilityElement(children: .combine)
    }

    /// A 176×128 stage: the middle card first, behind the other two, then the character in front.
    private func art(_ pose: FileDropMotion.Pose) -> some View {
        ZStack {
            ForEach([1, 0, 2], id: \.self) { index in
                let card = pose.cards[index]
                DropFileCard(symbol: ["photo", "doc.text", "film"][index])
                    .rotationEffect(.degrees(card.turn), anchor: .bottom)
                    .opacity(card.opacity)
                    // The card's bottom edge sits 98 points down the stage; less half its 66-point height.
                    .position(x: 88 + card.x, y: 98 + card.y - 33)
            }
            let character = pose.character
            Image("caper-branding-\(self.character)", bundle: caperResourceBundle)
                .renderingMode(.original).resizable().scaledToFit()
                .frame(width: 76, height: 76)
                .scaleEffect(x: character.scaleX, y: character.scaleY, anchor: .bottom)
                .rotationEffect(.degrees(character.turn), anchor: .bottom)
                .opacity(character.opacity)
                .position(x: 88, y: 128 + 4 + character.y - 38)
        }
        .frame(width: 176, height: 128)
        .accessibilityHidden(true)
    }
}

private struct DropFileCard: View {
    let symbol: String
    var body: some View {
        RoundedRectangle(cornerRadius: 8).fill(CaperTheme.raised)
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(CaperTheme.border))
            .overlay(Image(systemName: symbol).font(.system(size: 18, weight: .medium)).foregroundStyle(CaperTheme.muted))
            .frame(width: 54, height: 66)
    }
}

/// Keep the attach control in place while capability/session checks finish.
/// Photos and files remain disabled until the conversation accepts uploads.
struct AttachmentPickerButton: View {
    @Bindable var chat: ChatModel
    var inThread = false
    @State private var showFiles = false
    @State private var selectionTarget: ChatModel.AttachmentTarget?

    private var remaining: Int { max(0, AttachmentPolicy.maxAttachments - chat.attachmentFiles(inThread: inThread).count) }
    /// Why the control is disabled, worded as on every client.
    private var disabledReason: String? {
        !chat.canAttach ? chat.attachUnavailableReason : remaining == 0 ? "You can attach up to \(AttachmentPolicy.maxAttachments) files." : nil
    }

    #if os(iOS)
    @State private var showDrawer = false
    @State private var drawerTarget: ChatModel.AttachmentTarget?

    /// Phones and iPads open the attach drawer; it stays openable at the
    /// limit, where it shows why nothing more can be added.
    var body: some View {
        Button {
            drawerTarget = chat.attachmentTarget(inThread: inThread)
            withoutPresentationAnimation { showDrawer = drawerTarget != nil }
        } label: {
            Image("caper-plus").renderingMode(.template).resizable().frame(width: 18, height: 18)
                .foregroundStyle(showDrawer ? CaperTheme.text : CaperTheme.muted)
                .frame(width: 42, height: 42)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize()
        .disabled(!chat.canAttach)
        .opacity(chat.canAttach ? 1 : 0.4)
        .accessibilityLabel("Attach files")
        .accessibilityValue(!chat.canAttach ? chat.attachUnavailableReason : AttachCapacity.hint(AttachmentPolicy.maxAttachments - remaining) ?? "")
        .accessibilityIdentifier("attach-files-button")
        .fullScreenCover(isPresented: $showDrawer) {
            if let target = drawerTarget { AttachDrawer(draft: drawerDraft(target)) }
        }
        // Prepare the gallery while the composer is up, so the drawer opens filled.
        .task { AttachGalleryCache.warm() }
    }

    private func drawerDraft(_ target: ChatModel.AttachmentTarget) -> AttachDrawerDraft {
        let chat = chat, inThread = inThread
        return AttachDrawerDraft(
            files: { chat.attachmentFiles(inThread: inThread).map { AttachDrawerFile(id: $0.id, source: $0.source) } },
            add: { chat.addAttachments($0, to: target) },
            remove: { chat.removeAttachmentDraft(id: $0) },
            canSend: { chat.canPressSend(inThread: inThread) },
            send: { Task { await chat.send(inThread: inThread) } },
            valid: { chat.canAttach && chat.attachmentTarget(inThread: inThread) == target })
    }
    #else
    /// The Mac, like web on desktop, has one unfiltered "Upload a file"; the
    /// menu still opens at the limit to say so.
    var body: some View {
        Menu {
            if let hint = AttachCapacity.hint(AttachmentPolicy.maxAttachments - remaining) { Text(hint) }
            Button {
                selectionTarget = chat.attachmentTarget(inThread: inThread)
                showFiles = selectionTarget != nil
            } label: { Label("Upload a file", systemImage: "arrow.up.doc") }
                .disabled(remaining == 0)
        } label: {
            // The shared Lucide plus (CaperIcons.xcassets), as on web and in the profile composer.
            Image("caper-plus").renderingMode(.template).resizable().frame(width: 18, height: 18)
                .foregroundStyle(CaperTheme.muted)
                .frame(width: 42, height: 42)
                .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .menuOrder(.fixed)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(!chat.canAttach)
        .opacity(chat.canAttach ? 1 : 0.4)
        .help(disabledReason ?? "Attach files")
        .accessibilityLabel("Attach files")
        .accessibilityValue(disabledReason ?? "")
        .accessibilityIdentifier("attach-files-button")
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result, let target = selectionTarget else { return }
            let files = urls.prefix(AttachmentPolicy.maxAttachments).compactMap { try? AttachmentStaging.stage(copying: $0) }
            chat.addAttachments(files, to: target)
            if files.count < min(urls.count, AttachmentPolicy.maxAttachments) && chat.attachmentNotice(target: target) == nil {
                chat.setAttachmentNotice("Some files couldn’t be added.", target: target)
            }
        }
    }
    #endif
}
