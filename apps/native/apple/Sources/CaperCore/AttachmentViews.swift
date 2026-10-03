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
/// URL, so refreshed signatures reuse what is already on screen.
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

    func load(key: String, url: URL, maxPixelSize: Int) async throws -> CGImage {
        if let hit = cached(key) { return hit }
        let data: Data
        if url.isFileURL {
            data = try Data(contentsOf: url)
        } else {
            let (body, response) = try await session.data(from: url)
            if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) { throw AttachmentLoadError.status(http.statusCode) }
            data = body
        }
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

// MARK: Message attachments

/// Files under a message's text. `chat` is nil for the optimistic pending
/// row, whose attachments point at local copies.
struct MessageAttachmentsView: View {
    let attachments: [ChatAttachment]
    let chat: ChatModel?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(attachments) { attachment in
                if attachment.unavailable || attachment.url == nil {
                    AttachmentFileCard(attachment: attachment, action: nil)
                } else {
                    switch attachment.kind {
                    case .image: AttachmentImageView(attachment: attachment, chat: chat)
                    case .video: AttachmentVideoView(attachment: attachment, chat: chat)
                    case .audio: AttachmentAudioView(attachment: attachment, chat: chat)
                    case .file: AttachmentOpenCard(attachment: attachment, chat: chat)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
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
    @Environment(\.openURL) private var openURL
    @State private var image: CGImage?
    @State private var failed = false

    var body: some View {
        Button { openAttachment(attachment, chat: chat, openURL: openURL) } label: {
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
        .accessibilityHint("Opens the full-size image")
        .task(id: attachment.previewUrl ?? attachment.url) {
            let result = await loadAttachmentImage(attachment, preview: true, chat: chat)
            if let loaded = result.image { image = loaded; failed = false } else if image == nil { failed = result.failed }
        }
    }
}

private struct AttachmentVideoView: View {
    let attachment: ChatAttachment
    let chat: ChatModel?
    @Environment(\.openURL) private var openURL
    @State private var player: AVPlayer?
    @State private var poster: CGImage?
    @State private var starting = false

    var body: some View {
        ZStack {
            if let player {
                VideoPlayer(player: player)
            } else {
                Button(action: start) {
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

// MARK: Composer

/// Draft files above the composer: thumbnail, name, size change, progress, remove.
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
