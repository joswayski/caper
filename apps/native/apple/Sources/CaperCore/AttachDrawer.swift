import Foundation

/// The attach drawer's limit line, worded as on every client (web chat/attach.ts).
/// Drafts below the limit show nothing, so only a full one has a label.
enum AttachCapacity {
    static func label(_ attached: Int, max: Int = AttachmentPolicy.maxAttachments) -> String? {
        attached >= max ? "\(max) of \(max) attached" : nil
    }
    /// How to make room once full.
    static func hint(_ attached: Int, max: Int = AttachmentPolicy.maxAttachments) -> String? {
        attached >= max ? "Remove a file to add another." : nil
    }
}

/// Library items copy in the background, but join the draft in the order they
/// were tapped: a finished copy waits for the ones tapped before it.
struct AttachDrawerQueue {
    private(set) var order: [String] = []
    private var staged: [String: LocalAttachmentFile] = [:]

    func isPending(_ source: String) -> Bool { order.contains(source) }

    mutating func enqueue(_ source: String) {
        if !order.contains(source) { order.append(source) }
    }

    /// Stops waiting for `source`; returns its copy if it had finished, for removal.
    mutating func cancel(_ source: String) -> LocalAttachmentFile? {
        order.removeAll { $0 == source }
        return staged.removeValue(forKey: source)
    }

    /// Records a finished copy (nil when it failed) and returns the files now
    /// ready for the draft, in tap order. Callers check `isPending` first.
    mutating func finish(_ source: String, _ file: LocalAttachmentFile?) -> [LocalAttachmentFile] {
        guard order.contains(source) else { return [] }
        if let file { staged[source] = file } else { order.removeAll { $0 == source } }
        var ready: [LocalAttachmentFile] = []
        while let first = order.first, let file = staged.removeValue(forKey: first) {
            order.removeFirst()
            ready.append(file)
        }
        return ready
    }
}

#if os(iOS)
import Photos
import PhotosUI
import ImageIO
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A file in the draft, as the drawer shows it.
struct AttachDrawerFile: Equatable {
    let id: String
    let source: String?
}

/// What the drawer adds to and reads from: the channel/thread draft or a
/// profile DM draft. Closures read live state, so the drawer follows edits
/// made in the composer (removing a chip deselects its tile).
struct AttachDrawerDraft {
    let files: () -> [AttachDrawerFile]
    let add: ([LocalAttachmentFile]) -> Void
    let remove: (String) -> Void
    let canSend: () -> Bool
    let sending: () -> Bool
    let send: () -> Void
    /// False once the draft can't take files (conversation changed, uploads off); the drawer closes.
    let valid: () -> Bool
}

@MainActor
final class AttachDrawerLibrary: NSObject, ObservableObject, PHPhotoLibraryChangeObserver {
    @Published private(set) var status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    @Published private(set) var assets: PHFetchResult<PHAsset>?
    private var observing = false

    var readable: Bool { status == .authorized || status == .limited }

    /// The newest fetch, shared with the composer's warm-up so the drawer opens on it.
    private(set) static var fetched: PHFetchResult<PHAsset>?

    /// Every photo and video, newest first. A fetch result is lazy: tiles read only the assets they show.
    static func fetch() -> PHFetchResult<PHAsset> {
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d",
                                        PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
        let assets = PHAsset.fetchAssets(with: options)
        fetched = assets
        return assets
    }

    /// Asks once, the first time the drawer opens; later opens just read.
    func start() {
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        if status == .notDetermined {
            PHPhotoLibrary.requestAuthorization(for: .readWrite) { _ in
                Task { @MainActor [weak self] in self?.reload() }
            }
        }
        // Open on the composer's warm fetch, then catch up after the sheet is up.
        if readable, let warm = Self.fetched {
            assets = warm
            if !observing { PHPhotoLibrary.shared().register(self); observing = true }
            Task { @MainActor [weak self] in await Task.yield(); self?.reload() }
        } else {
            reload()
        }
    }

    func reload() {
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard readable else { assets = nil; return }
        if !observing { PHPhotoLibrary.shared().register(self); observing = true }
        let fresh = Self.fetch()
        // An unchanged library keeps the grid as it is instead of rebuilding every tile.
        if let assets, assets.count == fresh.count,
           assets.firstObject?.localIdentifier == fresh.firstObject?.localIdentifier,
           assets.lastObject?.localIdentifier == fresh.lastObject?.localIdentifier { return }
        assets = fresh
    }

    nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            guard let assets else { reload(); return }
            // iCloud sync reports changes constantly; only ones touching this fetch redraw the grid.
            guard let details = changeInstance.changeDetails(for: assets) else { return }
            self.assets = details.fetchResultAfterChanges
            Self.fetched = details.fetchResultAfterChanges
        }
    }

    func stop() {
        if observing { PHPhotoLibrary.shared().unregisterChangeObserver(self); observing = false }
    }

    /// Lets someone with limited access choose more items, from the drawer.
    func manageLimitedAccess() {
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive }),
              var controller = scene.windows.first(where: \.isKeyWindow)?.rootViewController else { return }
        while let presented = controller.presentedViewController { controller = presented }
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: controller)
    }

    /// Copies an item's original (or its edited version) into staging,
    /// downloading it from iCloud when needed.
    static func stage(_ identifier: String) async -> LocalAttachmentFile? {
        guard let asset = PHAsset.fetchAssets(withLocalIdentifiers: [identifier], options: nil).firstObject else { return nil }
        let resources = PHAssetResource.assetResources(for: asset)
        let video = asset.mediaType == .video
        let preferred: [PHAssetResourceType] = video ? [.fullSizeVideo, .video] : [.fullSizePhoto, .photo]
        guard let resource = preferred.lazy.compactMap({ type in resources.first { $0.type == type } }).first,
              let directory = try? AttachmentStaging.newDirectory() else { return nil }
        let name = AttachmentStaging.sanitized(resource.originalFilename)
        let url = directory.appendingPathComponent(name)
        let options = PHAssetResourceRequestOptions()
        options.isNetworkAccessAllowed = true
        let written: Bool = await withCheckedContinuation { continuation in
            PHAssetResourceManager.default().writeData(for: resource, toFile: url, options: options) { error in
                continuation.resume(returning: error == nil)
            }
        }
        guard written, var file = try? AttachmentStaging.file(at: url, name: name) else {
            try? FileManager.default.removeItem(at: directory)
            return nil
        }
        file.source = AttachDrawer.source(identifier)
        return file
    }

    deinit {
        if observing { PHPhotoLibrary.shared().unregisterChangeObserver(self) }
    }
}

/// The composer's attach drawer on iPhone and iPad: recent photos and videos
/// (with the camera first) to tap into the draft, Files, and Send. Tapping adds
/// to the draft at once, so uploads start while the person keeps choosing;
/// tapping again removes it. At the limit, other tiles dim and a tap nudges
/// the capacity line instead.
struct AttachDrawer: View {
    let draft: AttachDrawerDraft
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @StateObject private var library = AttachDrawerLibrary()
    @State private var queue = AttachDrawerQueue()
    @State private var copies: [String: Task<Void, Never>] = [:]
    @State private var nudges = 0
    @State private var notice: String?
    /// How many library items the grid lists; grows as it scrolls.
    @State private var shown = 240
    @State private var showingCamera = false
    @State private var showingFiles = false
    @State private var showingPhotos = false
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var expanded = false
    @GestureState private var drag: CGFloat = 0

    static func source(_ identifier: String) -> String { "photos:\(identifier)" }

    private var files: [AttachDrawerFile] { draft.files() }
    private var used: Int { files.count + queue.order.count }
    private var full: Bool { used >= AttachmentPolicy.maxAttachments }
    private var cameraAvailable: Bool { UIImagePickerController.isSourceTypeAvailable(.camera) }

    var body: some View {
        GeometryReader { geometry in
            let height = geometry.size.height
            let collapsedHeight = height * 0.5
            let panelHeight = expanded ? height : collapsedHeight
            ZStack(alignment: .bottom) {
                Color.black.opacity(0.55).ignoresSafeArea()
                    .onTapGesture { close() }
                    .accessibilityHidden(true)
                    .modifier(Entrance.scrim)
                panel
                    .padding(.leading, geometry.safeAreaInsets.leading)
                    .padding(.trailing, geometry.safeAreaInsets.trailing)
                    .frame(height: min(height, max(collapsedHeight, panelHeight - drag)))
                    .background {
                        UnevenRoundedRectangle(topLeadingRadius: 16, topTrailingRadius: 16)
                            .fill(CaperTheme.surface).ignoresSafeArea(edges: .bottom)
                    }
                    .offset(y: max(0, drag - (panelHeight - collapsedHeight)))
                    .modifier(Entrance(offset: CGSize(width: 0, height: 12), animation: CaperMotion.easeOut(CaperMotion.base)))
            }
            // The surface reaches both window edges, including in landscape;
            // the controls still respect the notch and home-indicator safe areas.
            .frame(width: geometry.size.width + geometry.safeAreaInsets.leading + geometry.safeAreaInsets.trailing,
                   height: height)
            .offset(x: -geometry.safeAreaInsets.leading)
        }
        .foregroundStyle(CaperTheme.text)
        .font(CaperTheme.font(14))
        // A system medium sheet floats away from the edges on iOS 26.
        // Hosts use a transparent full-screen cover; this drawer owns its height.
        .presentationBackground(.clear)
        .accessibilityAction(.escape) { close() }
        .task { library.start() }
        .onChange(of: scenePhase) { _, phase in if phase == .active { library.reload() } }
        .onChange(of: draft.valid()) { _, valid in if !valid { close() } }
        .onDisappear {
            // Copies still running belong to taps the person made; let them land
            // in the draft. Only the observer stops.
            library.stop()
        }
        .fullScreenCover(isPresented: $showingCamera) {
            AttachCameraPicker { file in
                showingCamera = false
                guard let file else { return }
                guard draft.valid(), !full else { AttachmentStaging.remove(file.url); nudge(); return }
                draft.add([file])
            }
            .ignoresSafeArea()
        }
        .fileImporter(isPresented: $showingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            let room = max(0, AttachmentPolicy.maxAttachments - used)
            let files = urls.prefix(room).compactMap { try? AttachmentStaging.stage(copying: $0) }
            draft.add(files)
            if urls.count > room { notice = "Only \(room) more file\(room == 1 ? "" : "s") can be attached." }
            else if files.count < urls.count { notice = "Some files couldn’t be added." }
            else { close() }
        }
        .photosPicker(isPresented: $showingPhotos, selection: $photoItems,
                      maxSelectionCount: max(1, AttachmentPolicy.maxAttachments - used),
                      matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            photoItems = []
            Task { @MainActor in
                var picked: [LocalAttachmentFile] = []
                for item in items {
                    if let media = try? await item.loadTransferable(type: PickedAttachmentMedia.self) { picked.append(media.file) }
                }
                guard draft.valid() else { picked.forEach { AttachmentStaging.remove($0.url) }; return }
                draft.add(picked)
                if picked.count < items.count { notice = "Some items couldn’t be added." }
            }
        }
    }

    private var panel: some View {
        let files = files
        return VStack(alignment: .leading, spacing: 0) {
            VStack(spacing: 0) {
                Button {
                    withAnimation(reduceMotion ? nil : CaperMotion.easeOut(CaperMotion.base)) { expanded.toggle() }
                } label: {
                    Capsule().fill(CaperTheme.muted).frame(width: 36, height: 5)
                        .frame(maxWidth: .infinity, minHeight: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(expanded ? "Collapse attachment drawer" : "Expand attachment drawer")
                .accessibilityIdentifier("attach-drawer-resize")
                header(files.count)
            }
            .contentShape(Rectangle())
            .simultaneousGesture(DragGesture()
                .updating($drag) { value, state, _ in state = value.translation.height }
                .onEnded { value in
                    let travel = value.predictedEndTranslation.height
                    if !expanded && travel > 80 { close() }
                    else if abs(travel) > 40 {
                        withAnimation(reduceMotion ? nil : CaperMotion.easeOut(CaperMotion.base)) { expanded = travel < 0 }
                    }
                })
            content(files)
            Divider().overlay(CaperTheme.border)
            toolbar
        }
        .overlay(alignment: .bottomTrailing) { sendButton(files) }
        .overlay(alignment: .topTrailing) {
            Button { close() } label: {
                CaperIcon(name: "x", size: 18).frame(width: 44, height: 28).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(CaperTheme.muted)
            .accessibilityLabel("Close attachment drawer")
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("attach-drawer")
    }

    private func close() { withoutPresentationAnimation { dismiss() } }

    // MARK: Header

    private func header(_ attached: Int) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                Text("Attach").font(CaperTheme.font(15, weight: .bold))
                Spacer(minLength: 8)
                // Only a full draft says how many files it holds.
                if let label = AttachCapacity.label(used) {
                    HStack(spacing: 8) {
                        HStack(spacing: 3) {
                            ForEach(0..<AttachmentPolicy.maxAttachments, id: \.self) { _ in
                                Circle().fill(CaperTheme.terracotta).frame(width: 6, height: 6)
                            }
                        }
                        Text(label).font(CaperTheme.font(12).monospacedDigit()).foregroundStyle(CaperTheme.terracottaBright)
                    }
                    .modifier(LimitNudge(nudges: reduceMotion ? 0 : CGFloat(nudges)))
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("attach-drawer-capacity")
                }
            }
            if let hint = AttachCapacity.hint(used) ?? notice {
                Text(hint).font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
            }
        }
        .padding(.horizontal, 16).padding(.top, 18).padding(.bottom, 10)
        .animation(reduceMotion ? nil : CaperMotion.easeOut(CaperMotion.slow), value: nudges)
    }

    // MARK: Library

    @ViewBuilder private func content(_ files: [AttachDrawerFile]) -> some View {
        switch library.status {
        case .denied, .restricted:
            accessMessage(library.status == .restricted
                ? "Photo access is restricted on this device."
                : "Caper can’t see your photos. Allow access in Settings, or choose items one at a time.",
                settings: library.status == .denied)
        default:
            VStack(spacing: 0) {
                if library.status == .limited {
                    HStack(spacing: 8) {
                        Text("Caper can see only the photos you allowed.").font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                        Spacer(minLength: 4)
                        Button("Manage") { library.manageLimitedAccess() }
                            .font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.text)
                    }
                    .padding(.horizontal, 16).padding(.bottom, 8)
                }
                grid(files)
            }
        }
    }

    private func grid(_ files: [AttachDrawerFile]) -> some View {
        // Library source -> (position in the draft, draft ID).
        var order: [String: (position: Int, id: String)] = [:]
        for (index, file) in files.enumerated() {
            if let source = file.source, order[source] == nil { order[source] = (index + 1, file.id) }
        }
        let count = min(library.assets?.count ?? 0, shown)
        return ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 88), spacing: 2)], spacing: 2) {
                if cameraAvailable {
                    Button { if full { nudge() } else { showingCamera = true } } label: {
                        Color.clear.aspectRatio(1, contentMode: .fit)
                            .background(CaperTheme.raised)
                            .overlay(CaperIcon(name: "camera", size: 26).foregroundStyle(CaperTheme.muted))
                            .opacity(full ? 0.4 : 1)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Camera")
                    .accessibilityIdentifier("attach-drawer-camera")
                }
                if let assets = library.assets {
                    ForEach(0..<count, id: \.self) { index in
                        let asset = assets.object(at: index)
                        let source = Self.source(asset.localIdentifier)
                        let selected = order[source]
                        let copying = queue.isPending(source)
                        Button { toggle(source, identifier: asset.localIdentifier, draftID: selected?.id) } label: {
                            AttachLibraryTile(asset: asset, order: selected?.position, copying: copying,
                                              dimmed: full && selected == nil && !copying)
                            .id(asset.localIdentifier)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(AttachLibraryTile.label(asset))
                        .accessibilityValue(selected.map { "Attached \($0.position)" }
                            ?? (copying ? "Adding" : full ? "Attachment limit reached" : ""))
                        .accessibilityAddTraits(selected != nil ? .isSelected : [])
                        .accessibilityIdentifier("attach-drawer-tile")
                        .onAppear { if index >= count - 30 && count < assets.count { shown += 240 } }
                    }
                }
            }
            .padding(.horizontal, 2)
            // Keep the last row clear of the floating Send button.
            .padding(.bottom, 72)
        }
        .accessibilityIdentifier("attach-drawer-grid")
        .overlay {
            if library.status == .notDetermined {
                ProgressView().tint(CaperTheme.muted)
            } else if library.readable && library.assets?.count == 0 {
                Text("No photos or videos yet.").font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
            }
        }
    }

    private func accessMessage(_ text: String, settings: Bool) -> some View {
        VStack(spacing: 14) {
            Text(text).multilineTextAlignment(.center).foregroundStyle(CaperTheme.muted)
            if settings {
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                }
                .buttonStyle(CaperPrimaryButton())
            }
            Button("Choose photos and videos") { if full { nudge() } else { showingPhotos = true } }
                .font(CaperTheme.font(14, weight: .bold))
            if cameraAvailable {
                Button("Take a photo or video") { if full { nudge() } else { showingCamera = true } }
                    .font(CaperTheme.font(14, weight: .bold))
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: Toolbar and Send

    private var toolbar: some View {
        HStack(spacing: 0) {
            toolbarItem("Photos", icon: "images", selected: true) {}
                .accessibilityAddTraits(.isSelected)
            toolbarItem("Files", icon: "paperclip", selected: false) { if full { nudge() } else { showingFiles = true } }
                .accessibilityIdentifier("attach-drawer-files")
        }
        .padding(.horizontal, 12).padding(.top, 6).padding(.bottom, 4)
    }

    private func toolbarItem(_ title: String, icon: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                CaperIcon(name: icon, size: 20)
                Text(title).font(CaperTheme.font(11, weight: selected ? .bold : .regular))
            }
            .foregroundStyle(selected ? CaperTheme.text : CaperTheme.muted)
            .frame(maxWidth: .infinity, minHeight: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder private func sendButton(_ files: [AttachDrawerFile]) -> some View {
        if !files.isEmpty || !queue.order.isEmpty {
            let ready = draft.canSend() && queue.order.isEmpty
            Button {
                draft.send()
                close()
            } label: {
                ZStack {
                    if ready || draft.sending() { CaperIcon(name: "send", size: 20) }
                    else { ProgressView().tint(CaperTheme.muted) }
                }
                .frame(width: 52, height: 52)
                .foregroundStyle(ready ? CaperTheme.onAction : CaperTheme.muted)
                .background(ready ? CaperTheme.action : CaperTheme.raised, in: Circle())
                .overlay(Circle().strokeBorder(ready ? Color.clear : CaperTheme.border))
            }
            .buttonStyle(.plain)
            .disabled(!ready)
            .padding(.trailing, 16).padding(.bottom, 72)
            .accessibilityLabel(ready ? "Send message" : "Uploading attachments")
            .accessibilityIdentifier("attach-drawer-send")
            .modifier(Entrance.chip)
        }
    }

    // MARK: Selection

    private func toggle(_ source: String, identifier: String, draftID: String?) {
        if let draftID { draft.remove(draftID); notice = nil; return }
        if queue.isPending(source) {
            copies.removeValue(forKey: source)?.cancel()
            if let staged = queue.cancel(source) { AttachmentStaging.remove(staged.url) }
            notice = nil
            return
        }
        guard !full else { nudge(); return }
        notice = nil
        queue.enqueue(source)
        UISelectionFeedbackGenerator().selectionChanged()
        copies[source] = Task { @MainActor in
            let file = await AttachDrawerLibrary.stage(identifier)
            copies[source] = nil
            guard !Task.isCancelled, queue.isPending(source) else {
                if let file { AttachmentStaging.remove(file.url) }
                return
            }
            if file == nil { notice = "That item couldn’t be added. Check your connection and try again." }
            let ready = queue.finish(source, file)
            guard draft.valid() else { ready.forEach { AttachmentStaging.remove($0.url) }; return }
            draft.add(ready)
        }
    }

    private func nudge() {
        nudges += 1
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
    }
}

/// A full drawer refuses more files with a short sideways shake.
private struct LimitNudge: GeometryEffect {
    var nudges: CGFloat
    var animatableData: CGFloat {
        get { nudges }
        set { nudges = newValue }
    }
    func effectValue(size: CGSize) -> ProjectionTransform {
        ProjectionTransform(CGAffineTransform(translationX: 4 * sin(nudges * .pi * 4), y: 0))
    }
}

/// What gallery tiles keep between scrolls and drawer openings, so a tile shows
/// its thumbnail and plays its GIF as soon as it appears. The composer warms it
/// before the drawer opens. NSCache gives the memory back under pressure.
@MainActor
enum AttachGalleryCache {
    private final class Frames {
        let original: AttachmentOriginal
        init(_ original: AttachmentOriginal) { self.original = original }
    }

    private static let thumbnails: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.totalCostLimit = 64 * 1024 * 1024
        return cache
    }()
    private static let animations: NSCache<NSString, Frames> = {
        let cache = NSCache<NSString, Frames>()
        cache.totalCostLimit = 96 * 1024 * 1024
        return cache
    }()
    /// GIFs whose originals could not be decoded within the limits; they stay stills.
    private static var stills: Set<String> = []
    private static var warmed: Date?

    /// Tiles are about 90-130pt wide; larger thumbnails only cost decode time and memory.
    private static let thumbnailPoints: CGFloat = 120
    /// The newest items the composer prepares before the drawer opens: more than a tall drawer shows.
    private static let warmCount = 60

    /// An edited item keeps its identifier, so it is cached by version too.
    static func key(_ asset: PHAsset) -> String {
        "\(asset.localIdentifier)|\(asset.modificationDate?.timeIntervalSince1970 ?? 0)"
    }

    /// Read from the fetched asset itself: asking PhotoKit for its resources on
    /// every render made scrolling stutter. The original's type is still checked
    /// before it is decoded.
    static func isGIF(_ asset: PHAsset) -> Bool {
        asset.mediaType == .image && asset.playbackStyle == .imageAnimated
    }

    static func thumbnail(_ key: String) -> UIImage? { thumbnails.object(forKey: key as NSString) }

    /// Asks PhotoKit for a tile-sized thumbnail. `deliver` may run twice, a quick
    /// degraded image first; only the final one is cached.
    @discardableResult
    static func requestThumbnail(_ asset: PHAsset, scale: CGFloat,
                                 deliver: @escaping @MainActor (UIImage) -> Void = { _ in }) -> PHImageRequestID {
        let key = key(asset)
        let options = PHImageRequestOptions()
        options.deliveryMode = .opportunistic
        options.resizeMode = .fast
        options.isNetworkAccessAllowed = true
        let side = thumbnailPoints * scale
        return PHImageManager.default().requestImage(for: asset, targetSize: CGSize(width: side, height: side),
                                                     contentMode: .aspectFill, options: options) { result, info in
            let degraded = (info?[PHImageResultIsDegradedKey] as? Bool) ?? false
            Task { @MainActor in
                guard let result else { return }
                if !degraded {
                    let pixels = result.size.width * result.scale * result.size.height * result.scale
                    thumbnails.setObject(result, forKey: key as NSString, cost: Int(pixels) * 4)
                }
                deliver(result)
            }
        }
    }

    /// A GIF's frames, decoded once and then shared by every tile showing it.
    /// Cancelling stops the decode; a cancelled GIF is tried again next time.
    static func animation(for asset: PHAsset) async -> AttachmentOriginal? {
        let key = key(asset)
        if let cached = animations.object(forKey: key as NSString) { return cached.original }
        guard isGIF(asset), !stills.contains(key), let original = await originalData(asset), !Task.isCancelled else { return nil }
        guard original.data.count <= 16 * 1024 * 1024, let type = original.type, UTType(type)?.conforms(to: .gif) == true else {
            stills.insert(key)
            return nil
        }
        let data = original.data
        let worker = Task.detached(priority: .userInitiated) { () -> AttachmentOriginal? in
            guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
            return try? AttachmentImageLoader.decodeAnimation(source, maxPixelSize: 240, maxFrames: 200, byteLimit: 2 * 1024 * 1024)
        }
        let result = await withTaskCancellationHandler { await worker.value } onCancel: { worker.cancel() }
        guard !Task.isCancelled else { return nil }
        guard let result else { stills.insert(key); return nil }
        let bytes = result.images.reduce(0) { $0 + $1.bytesPerRow * $1.height }
        animations.setObject(Frames(result), forKey: key as NSString, cost: bytes)
        return result
    }

    /// The local original; cloud-only GIFs are not downloaded just to animate a tile.
    private static func originalData(_ asset: PHAsset) async -> (data: Data, type: String?)? {
        let options = PHImageRequestOptions()
        options.version = .original
        options.deliveryMode = .highQualityFormat
        options.isNetworkAccessAllowed = false
        return await withCheckedContinuation { continuation in
            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) { data, type, _, _ in
                continuation.resume(returning: data.map { ($0, type) })
            }
        }
    }

    /// Called while a composer is on screen: reads the library and prepares the
    /// newest thumbnails and GIFs, so the drawer opens filled and already playing.
    /// Never asks for access.
    static func warm() {
        let status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard status == .authorized || status == .limited else { return }
        if let warmed, Date().timeIntervalSince(warmed) < 30 { return }
        warmed = Date()
        let scale = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.screen.scale }.first ?? 3
        let assets = AttachDrawerLibrary.fetch()
        let newest = (0..<min(assets.count, warmCount)).map { assets.object(at: $0) }
        for asset in newest where thumbnail(key(asset)) == nil { requestThumbnail(asset, scale: scale) }
        Task { @MainActor in
            for asset in newest where isGIF(asset) { _ = await animation(for: asset) }
        }
    }
}

/// Plays decoded GIF frames with Core Animation, so a screen full of GIFs costs
/// no SwiftUI updates per frame.
private struct AttachGalleryAnimationView: UIViewRepresentable {
    let animation: AttachmentOriginal

    final class Coordinator { var first: CGImage? }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        view.clipsToBounds = true
        view.layer.contentsGravity = .resizeAspectFill
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        guard context.coordinator.first !== animation.images.first else { return }
        context.coordinator.first = animation.images.first
        view.layer.removeAnimation(forKey: "frames")
        view.layer.contents = animation.images.first
        let total = animation.durations.reduce(0, +)
        guard animation.images.count > 1, total > 0 else { return }
        // Discrete key times bracket each frame, so there is one more than there are frames.
        var elapsed = 0.0
        var times: [NSNumber] = [0]
        for duration in animation.durations {
            elapsed += duration
            times.append(NSNumber(value: min(1, elapsed / total)))
        }
        let frames = CAKeyframeAnimation(keyPath: "contents")
        frames.values = animation.images
        frames.keyTimes = times
        frames.calculationMode = .discrete
        frames.duration = total
        frames.repeatCount = .infinity
        frames.isRemovedOnCompletion = false
        view.layer.add(frames, forKey: "frames")
    }
}

private struct AttachLibraryTile: View {
    let asset: PHAsset
    let order: Int?
    let copying: Bool
    let dimmed: Bool
    @State private var image: UIImage?
    @State private var request: PHImageRequestID?
    @State private var frames: AttachmentOriginal?
    @State private var shown = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.displayScale) private var displayScale

    /// Every GIF on screen plays, unless motion is reduced or the app is in the background.
    private var playing: Bool {
        shown && !reduceMotion && scenePhase == .active && AttachGalleryCache.isGIF(asset)
    }

    var body: some View {
        Color.clear.aspectRatio(1, contentMode: .fit)
            .background(CaperTheme.raised)
            .overlay {
                if let frames, !frames.images.isEmpty {
                    AttachGalleryAnimationView(animation: frames)
                } else if let still = image ?? AttachGalleryCache.thumbnail(AttachGalleryCache.key(asset)) {
                    Image(uiImage: still).resizable().scaledToFill()
                }
            }
            .clipped()
            .overlay { if order != nil || copying { CaperTheme.blackout.opacity(0.25) } }
            .overlay { Rectangle().strokeBorder(order != nil ? CaperTheme.terracotta : .clear, lineWidth: 3) }
            .overlay(alignment: .topTrailing) {
                Group {
                    if let order {
                        Text("\(order)").font(CaperTheme.font(12, weight: .bold)).foregroundStyle(CaperTheme.text)
                            .frame(width: 24, height: 24).background(CaperTheme.terracotta, in: Circle())
                            .modifier(Entrance.chip)
                    } else if copying {
                        ProgressView().controlSize(.small).tint(CaperTheme.text)
                            .frame(width: 24, height: 24).background(CaperTheme.blackout.opacity(0.7), in: Circle())
                    } else {
                        Circle().strokeBorder(CaperTheme.text.opacity(0.9), lineWidth: 1.5)
                            .background(CaperTheme.blackout.opacity(0.35), in: Circle())
                            .frame(width: 24, height: 24)
                    }
                }
                .padding(6)
            }
            .overlay(alignment: .bottomLeading) {
                if asset.mediaType == .video || AttachGalleryCache.isGIF(asset) {
                    HStack(spacing: 4) {
                        if asset.mediaType == .video {
                            Image(systemName: "play.fill")
                            Text(String(format: "%d:%02d", Int(asset.duration) / 60, Int(asset.duration) % 60))
                        } else {
                            Text("GIF")
                        }
                    }
                        .font(CaperTheme.font(11, weight: .bold).monospacedDigit()).foregroundStyle(CaperTheme.text)
                        .padding(.horizontal, 5).padding(.vertical, 2)
                        .background(CaperTheme.blackout.opacity(0.7), in: RoundedRectangle(cornerRadius: 4))
                        .padding(5)
                }
            }
            .opacity(dimmed ? 0.4 : 1)
            .contentShape(Rectangle())
            .onAppear { shown = true; load() }
            .onDisappear {
                shown = false
                if let request { PHImageManager.default().cancelImageRequest(request) }
                request = nil; image = nil
            }
            .task(id: playing) {
                guard playing else { frames = nil; return }
                frames = await AttachGalleryCache.animation(for: asset)
            }
    }

    /// Read by the tile's button, which VoiceOver activates.
    static func label(_ asset: PHAsset) -> String {
        let date = asset.creationDate?.formatted(date: .abbreviated, time: .omitted) ?? ""
        if asset.mediaType == .video { return "Video, \(Int(asset.duration)) seconds, \(date)" }
        return "\(AttachGalleryCache.isGIF(asset) ? "GIF" : "Photo"), \(date)"
    }

    private func load() {
        guard image == nil, request == nil else { return }
        if let cached = AttachGalleryCache.thumbnail(AttachGalleryCache.key(asset)) { image = cached; return }
        request = AttachGalleryCache.requestThumbnail(asset, scale: displayScale) { result in image = result }
    }
}

/// The system camera for one photo or video, staged like any picked file.
struct AttachCameraPicker: UIViewControllerRepresentable {
    let finish: (LocalAttachmentFile?) -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = [UTType.image.identifier, UTType.movie.identifier]
        picker.videoQuality = .typeHigh
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(finish: finish) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let finish: (LocalAttachmentFile?) -> Void
        init(finish: @escaping (LocalAttachmentFile?) -> Void) { self.finish = finish }

        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            var file: LocalAttachmentFile?
            if let url = info[.mediaURL] as? URL {
                file = try? AttachmentStaging.stage(copying: url, filename: "Video.\(url.pathExtension.isEmpty ? "mov" : url.pathExtension)")
            } else if let image = info[.originalImage] as? UIImage, let data = image.jpegData(compressionQuality: 0.92) {
                file = try? AttachmentStaging.stage(data: data, filename: "Photo.jpg")
            }
            finish(file)
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { finish(nil) }
    }
}
#endif
