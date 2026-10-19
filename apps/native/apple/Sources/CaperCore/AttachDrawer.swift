import Foundation

/// The attach drawer's capacity line, worded as on every client (web chat/attach.ts).
enum AttachCapacity {
    static func label(_ attached: Int, max: Int = AttachmentPolicy.maxAttachments) -> String {
        let count = Swift.max(0, Swift.min(attached, max))
        return count == 0 ? "Up to \(max) files" : "\(count) of \(max) attached"
    }
    /// Shown once full: how to make room.
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

    /// Asks once, the first time the drawer opens; later opens just read.
    func start() {
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        if status == .notDetermined {
            PHPhotoLibrary.requestAuthorization(for: .readWrite) { _ in
                Task { @MainActor [weak self] in self?.reload() }
            }
        }
        reload()
    }

    func reload() {
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard readable else { assets = nil; return }
        if !observing { PHPhotoLibrary.shared().register(self); observing = true }
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d",
                                        PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
        // A fetch result is lazy: tiles read only the assets they show.
        assets = PHAsset.fetchAssets(with: options)
    }

    nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
        Task { @MainActor [weak self] in self?.reload() }
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

    static func source(_ identifier: String) -> String { "photos:\(identifier)" }

    private var files: [AttachDrawerFile] { draft.files() }
    private var used: Int { files.count + queue.order.count }
    private var full: Bool { used >= AttachmentPolicy.maxAttachments }
    private var cameraAvailable: Bool { UIImagePickerController.isSourceTypeAvailable(.camera) }

    var body: some View {
        let files = files
        VStack(alignment: .leading, spacing: 0) {
            header(files.count)
            content(files)
            Divider().overlay(CaperTheme.border)
            toolbar
        }
        .overlay(alignment: .bottomTrailing) { sendButton(files) }
        .background(CaperTheme.surface)
        .foregroundStyle(CaperTheme.text)
        .font(CaperTheme.font(14))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(CaperTheme.surface)
        .presentationCornerRadius(16)
        .task { library.start() }
        .onChange(of: scenePhase) { _, phase in if phase == .active { library.reload() } }
        .onChange(of: draft.valid()) { _, valid in if !valid { dismiss() } }
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
            else { dismiss() }
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

    // MARK: Header

    private func header(_ attached: Int) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                Text("Attach").font(CaperTheme.font(15, weight: .bold))
                Spacer(minLength: 8)
                HStack(spacing: 8) {
                    HStack(spacing: 3) {
                        ForEach(0..<AttachmentPolicy.maxAttachments, id: \.self) { index in
                            Circle().fill(full ? CaperTheme.terracotta : index < used ? CaperTheme.text : CaperTheme.border)
                                .frame(width: 6, height: 6)
                        }
                    }
                    Text(AttachCapacity.label(used)).font(CaperTheme.font(12).monospacedDigit())
                        .foregroundStyle(full ? CaperTheme.terracottaBright : CaperTheme.muted)
                }
                .modifier(LimitNudge(nudges: reduceMotion ? 0 : CGFloat(nudges)))
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("attach-drawer-capacity")
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
                dismiss()
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

private struct AttachLibraryTile: View {
    let asset: PHAsset
    let order: Int?
    let copying: Bool
    let dimmed: Bool
    @State private var image: UIImage?
    @State private var request: PHImageRequestID?
    @State private var loaded: String?
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        Color.clear.aspectRatio(1, contentMode: .fit)
            .background(CaperTheme.raised)
            .overlay {
                if let image { Image(uiImage: image).resizable().scaledToFill() }
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
                if asset.mediaType == .video {
                    Text(String(format: "%d:%02d", Int(asset.duration) / 60, Int(asset.duration) % 60))
                        .font(CaperTheme.font(11, weight: .bold).monospacedDigit()).foregroundStyle(CaperTheme.text)
                        .padding(.horizontal, 5).padding(.vertical, 2)
                        .background(CaperTheme.blackout.opacity(0.7), in: RoundedRectangle(cornerRadius: 4))
                        .padding(5)
                }
            }
            .opacity(dimmed ? 0.4 : 1)
            .contentShape(Rectangle())
            .onAppear { load() }
            .onChange(of: asset.localIdentifier) { _, _ in load() }
            .onDisappear {
                if let request { PHImageManager.default().cancelImageRequest(request) }
                request = nil; image = nil; loaded = nil
            }
    }

    /// Read by the tile's button, which VoiceOver activates.
    static func label(_ asset: PHAsset) -> String {
        let date = asset.creationDate?.formatted(date: .abbreviated, time: .omitted) ?? ""
        return asset.mediaType == .video ? "Video, \(Int(asset.duration)) seconds, \(date)" : "Photo, \(date)"
    }

    private func load() {
        let identifier = asset.localIdentifier
        guard loaded != identifier else { return }
        if let request { PHImageManager.default().cancelImageRequest(request) }
        loaded = identifier
        image = nil
        let options = PHImageRequestOptions()
        options.deliveryMode = .opportunistic
        options.resizeMode = .fast
        options.isNetworkAccessAllowed = true
        let side = 200 * displayScale
        request = PHImageManager.default().requestImage(for: asset, targetSize: CGSize(width: side, height: side),
                                                        contentMode: .aspectFill, options: options) { result, _ in
            Task { @MainActor in if loaded == identifier, let result { image = result } }
        }
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
