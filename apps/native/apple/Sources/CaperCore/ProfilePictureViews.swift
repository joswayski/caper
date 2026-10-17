import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Loads a custom profile picture or space icon at display size and hands
/// `content` the decoded frames: nil while it loads, after a failure, or
/// with no picture (show the saved mascot, initials or space initial then,
/// never a broken image). Decoded frames are shared through the cache, so a
/// picture another row already decoded shows at once.
struct PictureLoading<Content: View>: View {
    let picture: CaperPicture?
    let size: CGFloat
    private let content: (ProfilePictureFrames?) -> Content
    @Environment(\.displayScale) private var displayScale
    /// Holds the frames on screen even if the cache evicts them.
    @State private var loaded: Loaded?

    private struct Loaded { let key: String; let frames: ProfilePictureFrames }

    init(picture: CaperPicture?, size: CGFloat, @ViewBuilder content: @escaping (ProfilePictureFrames?) -> Content) {
        self.picture = picture
        self.size = size
        self.content = content
    }

    var body: some View {
        // Read on the main actor; the task below uses these copies.
        let picture = self.picture
        let pixelSize = ProfilePicturePolicy.displayPixelSize(points: size, scale: displayScale)
        let key = picture.map { ProfilePictureLoader.key(asset: $0.asset, pixelSize: pixelSize) }
        let frames = key.flatMap { key in loaded?.key == key ? loaded?.frames : ProfilePictureLoader.shared.cached(key) }
        content(frames.flatMap { $0.images.isEmpty ? nil : $0 })
            // A new asset, decode size or signature (after a failure) loads again.
            .task(id: picture.map { "\(pixelSize):\($0.asset):\($0.url.absoluteString)" }) {
                guard let picture, let key else { if loaded != nil { loaded = nil }; return }
                if loaded?.key == key { return }
                guard let frames = await ProfilePictureLoader.shared.load(asset: picture.asset, url: picture.url, pixelSize: pixelSize),
                      !Task.isCancelled else { return }
                loaded = Loaded(key: key, frames: frames)
            }
            // Frames stay cached (bounded); the view lets go of them offscreen.
            .onDisappear { loaded = nil }
    }
}

/// Decoded picture frames aspect-filled in a `size`-point square over a
/// neutral backdrop (never the mascot through transparent pixels). GIFs
/// play, other people's included; Reduce Motion shows the first frame.
struct PictureFrames: View {
    let frames: ProfilePictureFrames
    let size: CGFloat
    /// A muted space's icon steps back.
    var pictureOpacity: Double = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var started = Date()

    var body: some View {
        ZStack {
            CaperTheme.raised
            Group {
                if let first = frames.images.first, reduceMotion || !frames.animated {
                    Image(decorative: first, scale: 1).resizable().scaledToFill()
                } else if frames.animated {
                    TimelineView(.animation(minimumInterval: 0.02)) { timeline in
                        let index = MediaViewerPolicy.frameIndex(at: timeline.date.timeIntervalSince(started), durations: frames.durations)
                        Image(decorative: frames.images[index], scale: 1).resizable().scaledToFill()
                    }
                }
            }
            .frame(width: size, height: size)
            .opacity(pictureOpacity)
        }
        .frame(width: size, height: size)
        .clipped()
    }
}

/// A picture filling a `size`-point square, or `fallback` while it loads and
/// whenever it fails. Callers clip it to their shape (space tiles).
struct PictureImage<Fallback: View>: View {
    let picture: CaperPicture?
    let size: CGFloat
    /// Applied to a loaded picture only (a muted space's tile).
    var pictureOpacity: Double = 1
    private let fallback: Fallback

    init(picture: CaperPicture?, size: CGFloat, pictureOpacity: Double = 1, @ViewBuilder fallback: () -> Fallback) {
        self.picture = picture
        self.size = size
        self.pictureOpacity = pictureOpacity
        self.fallback = fallback()
    }

    var body: some View {
        PictureLoading(picture: picture, size: size) { frames in
            if let frames {
                PictureFrames(frames: frames, size: size, pictureOpacity: pictureOpacity)
            } else {
                fallback.frame(width: size, height: size)
            }
        }
        .frame(width: size, height: size)
        .clipped()
    }
}

/// A space's tile: its icon filling the rounded square, or its initial.
struct SpaceIconTile: View {
    let name: String
    let picture: CaperPicture?
    let size: CGFloat
    var fill: Color = CaperTheme.raised
    var fontSize: CGFloat = 13
    var corner: CGFloat = 12
    /// A muted space: the initial or icon steps back.
    var dimmed = false

    var body: some View {
        PictureImage(picture: picture, size: size, pictureOpacity: dimmed ? 0.5 : 1) {
            Text(String(name.prefix(1)).uppercased()).font(CaperTheme.font(fontSize, weight: .black))
                .opacity(dimmed ? 0.5 : 1)
                .frame(width: size, height: size)
                .background(fill)
        }
        .clipShape(RoundedRectangle(cornerRadius: corner))
    }
}

/// Edit profile's picture and Manage space's icon: the current picture in a
/// frame with an edit badge on its corner. The frame opens a menu: Photos or
/// Files (GIF bytes are kept unchanged) and, while a custom one is set,
/// **Remove**. Progress and inline errors show beneath it.
struct PictureEditor<Preview: View>: View {
    let purpose: PicturePurpose
    /// A custom picture is set (Remove is offered).
    let hasPicture: Bool
    let upload: (PreparedPicture) async throws -> Void
    let remove: () async throws -> Void
    private let preview: Preview
    @State private var phase: Phase?
    @State private var error: String?
    /// The server has no upload storage (503): only removal stays offered.
    @State private var unavailable = false
    @State private var showPhotos = false
    @State private var showFiles = false
    @State private var photoItem: PhotosPickerItem?

    private enum Phase { case uploading, removing }

    /// The frame's side, in points.
    static var side: CGFloat { 112 }

    init(purpose: PicturePurpose, hasPicture: Bool, upload: @escaping (PreparedPicture) async throws -> Void,
         remove: @escaping () async throws -> Void, @ViewBuilder preview: () -> Preview) {
        self.purpose = purpose
        self.hasPicture = hasPicture
        self.upload = upload
        self.remove = remove
        self.preview = preview()
    }

    private var avatar: Bool { purpose == .avatar }
    private var noun: String { avatar ? "picture" : "icon" }
    private var status: String? {
        switch phase {
        case .uploading: "Uploading…"
        case .removing: "Removing…"
        case nil: unavailable ? ProfilePicturePolicy.unavailableMessage : nil
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Menu {
                Button { showPhotos = true } label: { Label("Photos", systemImage: "photo.on.rectangle") }
                    .disabled(unavailable)
                Button { showFiles = true } label: { Label("Files", systemImage: "doc") }
                    .disabled(unavailable)
                if hasPicture {
                    Divider()
                    Button(role: .destructive, action: removePicture) { Label("Remove \(noun)", systemImage: "trash") }
                }
            } label: {
                framedPicture
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .disabled(phase != nil || (unavailable && !hasPicture))
            .accessibilityLabel(avatar ? "Change profile picture" : "Change space icon")
            .accessibilityIdentifier(avatar ? "change-profile-picture" : "change-space-icon")
            if let error {
                Text(error).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.error)
                    .fixedSize(horizontal: false, vertical: true)
            } else if let status {
                Text(status).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        // `.current` keeps the library's original bytes, so GIFs stay animated.
        .photosPicker(isPresented: $showPhotos, selection: $photoItem, matching: .images, preferredItemEncoding: .current)
        .onChange(of: photoItem) { _, item in
            guard let item else { return }
            photoItem = nil
            start { try await PictureFile.data(from: item) }
        }
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.image]) { result in
            guard case .success(let url) = result else { return }
            start { try await Task.detached(priority: .userInitiated) { try PictureFile.read(url) }.value }
        }
    }

    /// The picture, framed in the rounded square it has everywhere, with the
    /// eggshell edit badge on its corner.
    private var framedPicture: some View {
        let shape = Avatar.pictureShape(Self.side)
        return preview
            .frame(width: Self.side, height: Self.side)
            .background(CaperTheme.raised)
            .clipShape(shape)
            .overlay(shape.stroke(CaperTheme.border))
            .opacity(phase == nil ? 1 : 0.55)
            .overlay(alignment: .bottomTrailing) {
                CaperIcon(name: "pencil", size: 15)
                    .foregroundStyle(phase == nil ? CaperTheme.blackout : CaperTheme.muted)
                    .frame(width: 28, height: 28)
                    .background(phase == nil ? CaperTheme.action : CaperTheme.border, in: RoundedRectangle(cornerRadius: 8))
                    .padding(3)
                    .background(CaperTheme.surface, in: RoundedRectangle(cornerRadius: 10))
                    .offset(x: 6, y: 6)
            }
            .padding([.trailing, .bottom], 6)
            .contentShape(Rectangle())
    }

    /// Loads the picked bytes, prepares them off the main actor, then uploads.
    private func start(_ load: @escaping () async throws -> Data) {
        guard phase == nil else { return }
        phase = .uploading
        error = nil
        Task { @MainActor in
            do {
                let data = try await load()
                let prepared = try await Task.detached(priority: .userInitiated) { try ProfilePicturePreparer.prepare(data) }.value
                try await upload(prepared)
            } catch is CancellationError {
            } catch {
                self.error = ProfilePicturePolicy.message(for: error, purpose: purpose)
                if ProfilePicturePolicy.uploadsUnavailable(error) { unavailable = true }
            }
            phase = nil
        }
    }

    private func removePicture() {
        guard phase == nil else { return }
        phase = .removing
        error = nil
        Task { @MainActor in
            do { try await remove() }
            catch is CancellationError {}
            catch { self.error = ProfilePicturePolicy.message(for: error, purpose: purpose) }
            phase = nil
        }
    }
}

/// Reading a picked picture's bytes, unchanged (a GIF keeps every frame),
/// whatever its size.
enum PictureFile {
    static func data(from item: PhotosPickerItem) async throws -> Data {
        guard let data = try await item.loadTransferable(type: Data.self) else { throw PictureError.unreadable }
        return data
    }

    /// A file importer's security-scoped URL.
    static func read(_ url: URL) throws -> Data {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        return try Data(contentsOf: url)
    }
}

/// Edit profile's frame without a custom picture: the saved mascot holding up
/// a "change me!" sign (one vector over it, in the mascot's own square).
struct ChangeMeMascot: View {
    let name: String
    let avatarID: Int?
    let size: CGFloat

    var body: some View {
        ZStack {
            Avatar(name: name, size: size, avatarID: avatarID)
            Image("caper-change-me", bundle: caperResourceBundle).renderingMode(.original)
                .resizable().scaledToFit().frame(width: size, height: size)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}
