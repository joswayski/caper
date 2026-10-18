import Foundation

/// Selection never truncates when capacity changes: Add is blocked until the
/// person removes enough items. Library removals, unlike capacity, invalidate IDs.
struct AttachmentGallerySelection {
    private(set) var ids: [String] = []
    mutating func toggle(_ id: String, capacity: Int) {
        if ids.contains(id) { ids.removeAll { $0 == id } }
        else if ids.count < capacity { ids.append(id) }
    }
    mutating func retain(_ available: Set<String>) { ids.removeAll { !available.contains($0) } }
    func canAdd(capacity: Int) -> Bool { !ids.isEmpty && ids.count <= capacity }
}

#if os(iOS)
import Photos
import SwiftUI
import UIKit

@MainActor
private final class AttachmentPhotoLibrary: NSObject, ObservableObject, PHPhotoLibraryChangeObserver {
    @Published var status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    @Published var assets: [PHAsset] = []
    @Published var revision = 0
    @Published var hasMore = false
    private var result: PHFetchResult<PHAsset>?
    private var observing = false
    private var active = false

    func start() { active = true; refresh() }

    func refresh() {
        guard active else { return }
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        defer { revision += 1 }
        guard status == .authorized || status == .limited else {
            assets = []; result = nil; hasMore = false
            return
        }
        if !observing { PHPhotoLibrary.shared().register(self); observing = true }
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d", PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
        result = PHAsset.fetchAssets(with: options)
        assets = []
        loadMore()
    }

    func loadMore() {
        guard let result else { return }
        let end = min(assets.count + 60, result.count)
        assets += (assets.count..<end).map { result.object(at: $0) }
        hasMore = end < result.count
    }

    // Selection may include older pages. Never prune against just visible assets.
    func accessibleIDs(_ ids: [String]) -> Set<String> {
        guard status == .authorized || status == .limited, !ids.isEmpty else { return [] }
        let selected = PHAsset.fetchAssets(withLocalIdentifiers: ids, options: nil)
        return Set((0..<selected.count).map { selected.object(at: $0).localIdentifier })
    }

    func request() {
        PHPhotoLibrary.requestAuthorization(for: .readWrite) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
        Task { @MainActor [weak self] in self?.refresh() }
    }

    func stop() {
        active = false
        if observing { PHPhotoLibrary.shared().unregisterChangeObserver(self); observing = false }
    }

    deinit { if observing { PHPhotoLibrary.shared().unregisterChangeObserver(self) } }
}

/// PhotoKit delivers chunks and completion on one serial queue. Only cancellation
/// and request registration cross queues; the lock protects their race. Streaming
/// requestData, rather than writeData, lets Cancel stop an iCloud download too.
private final class AttachmentResourceCopy: @unchecked Sendable {
    private let lock = NSLock()
    private var requestID: PHAssetResourceDataRequestID?
    private var cancelled = false
    private var completed = false
    private let file: FileHandle
    private var writeError: Error?

    init(url: URL) throws {
        guard FileManager.default.createFile(atPath: url.path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
        file = try FileHandle(forWritingTo: url)
    }

    func register(_ id: PHAssetResourceDataRequestID) {
        lock.lock()
        requestID = id
        let shouldCancel = cancelled && !completed
        lock.unlock()
        if shouldCancel { PHAssetResourceManager.default().cancelDataRequest(id) }
    }

    func cancel() {
        lock.lock()
        cancelled = true
        let id = completed ? nil : requestID
        lock.unlock()
        if let id { PHAssetResourceManager.default().cancelDataRequest(id) }
    }

    func receive(_ data: Data) {
        lock.lock()
        let shouldWrite = !cancelled
        lock.unlock()
        guard shouldWrite, writeError == nil else { return }
        do { try file.write(contentsOf: data) }
        catch { writeError = error; cancel() }
    }

    func finish(_ error: Error?) -> Error? {
        do { try file.close() } catch { if writeError == nil { writeError = error } }
        lock.lock()
        completed = true
        let wasCancelled = cancelled
        lock.unlock()
        return writeError ?? error ?? (wasCancelled ? CancellationError() : nil)
    }
}

/// Owns staged files until the callback synchronously transfers them to a draft.
struct MobileAttachmentGallery: View {
    let existingCount: () -> Int
    let valid: () -> Bool
    let useFiles: () -> Void
    let add: ([LocalAttachmentFile]) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var library = AttachmentPhotoLibrary()
    @State private var selection = AttachmentGallerySelection()
    @State private var importing: Task<Void, Never>?
    @State private var error: String?
    @State private var importProgress = 0
    private var capacity: Int { max(0, AttachmentPolicy.maxAttachments - existingCount()) }

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                Text("\(selection.ids.count) selected · \(max(0, capacity - selection.ids.count)) remaining")
                    .accessibilityIdentifier("attachment-gallery-count")
                Text("Up to \(AttachmentPolicy.maxAttachments) attachments, including \(existingCount()) already in this draft.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                if selection.ids.count >= capacity {
                    Text(capacity == 0 ? "Draft is full. Remove an attachment from your draft to add more." : "Attachment limit reached. Deselect an item to choose another.")
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.terracottaBright)
                }
                if let error { Text(error).foregroundStyle(CaperTheme.error).font(CaperTheme.font(12)) }
                if library.status == .limited {
                    Button("Manage selected photos") { manageLimitedAccess() }.disabled(importing != nil)
                }
                if library.status == .notDetermined {
                    Text("Choose which photos and videos Caper can show. Limited access is supported.")
                    Button("Choose photo access") { library.request() }
                } else if library.status == .denied || library.status == .restricted {
                    Text(library.status == .restricted ? "Photo access is restricted on this device. Use Files instead." : "Photo access is off. Allow access in Settings or use Files.")
                    if library.status == .denied {
                        Button("Open Settings") { if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) } }
                    }
                } else if library.assets.isEmpty {
                    ContentUnavailableView("No accessible photos or videos", systemImage: "photo", description: Text("Use Files or change photo access to choose other media."))
                } else {
                    Text("Cloud-only previews may be unavailable. Selected originals download when you tap Add.")
                        .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    ScrollView {
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 3), count: 3), spacing: 3) {
                            ForEach(library.assets, id: \.localIdentifier) { asset in
                                let order = selection.ids.firstIndex(of: asset.localIdentifier).map { $0 + 1 }
                                let atLimit = order == nil && selection.ids.count >= capacity
                                Button { selection.toggle(asset.localIdentifier, capacity: capacity) } label: {
                                    AttachmentPhotoTile(asset: asset, order: order)
                                        .opacity(atLimit ? 0.35 : 1)
                                }
                                .buttonStyle(.plain)
                                .disabled(importing != nil || atLimit)
                                .accessibilityLabel("\(asset.mediaType == .video ? "Video" : "Photo"), \(asset.creationDate?.formatted() ?? "unknown date")")
                                .accessibilityValue(order.map { "Selected \($0)" } ?? (atLimit ? "Attachment limit reached" : "Not selected"))
                                .accessibilityHint(order != nil ? "Double tap to deselect" : "Double tap to select")
                                .accessibilityAddTraits(order != nil ? [.isSelected] : [])
                            }
                        }
                        if library.hasMore {
                            Button("Load more photos and videos") { library.loadMore() }
                                .onAppear { library.loadMore() }
                        }
                    }
                }
                Spacer(minLength: 0)
                Button("Use Files instead") { importing?.cancel(); useFiles(); dismiss() }
                Button(importing == nil ? "Add \(selection.ids.count)" : "Preparing \(importProgress + 1) of \(selection.ids.count)…") { importSelection() }
                    .buttonStyle(CaperPrimaryButton())
                    .disabled(!selection.canAdd(capacity: capacity) || !valid() || importing != nil)
            }
            .padding(16).font(CaperTheme.font(14)).foregroundStyle(CaperTheme.text).background(CaperTheme.surface)
            .navigationTitle("Photos and videos").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { importing?.cancel(); dismiss() } } }
        }
        .tint(CaperTheme.text)
        .presentationDetents([.large]).presentationDragIndicator(.visible)
        .task { library.start() }
        .onChange(of: scenePhase) { _, phase in if phase == .active { library.refresh() } }
        .onChange(of: library.revision) { _, _ in
            let previous = selection.ids
            selection.retain(library.accessibleIDs(previous))
            if previous != selection.ids {
                importing?.cancel()
                error = "Some selected media is no longer accessible. Review your selection."
            }
        }
        .onChange(of: library.status) { _, status in
            if status != .authorized && status != .limited { importing?.cancel() }
        }
        .onChange(of: valid()) { _, value in if !value { importing?.cancel(); dismiss() } }
        .onDisappear { importing?.cancel(); library.stop() }
    }

    private func manageLimitedAccess() {
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first(where: { $0.activationState == .foregroundActive }),
              var controller = scene.windows.first(where: \.isKeyWindow)?.rootViewController else { return }
        while let presented = controller.presentedViewController { controller = presented }
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: controller)
    }

    private func importSelection() {
        let ids = selection.ids
        importProgress = 0
        error = nil
        importing = Task { @MainActor in
            var files: [LocalAttachmentFile] = []
            defer { files.forEach { AttachmentStaging.remove($0.url) }; importing = nil }
            do {
                for (index, id) in ids.enumerated() {
                    try Task.checkCancellation()
                    importProgress = index
                    guard let asset = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil).firstObject else { throw CocoaError(.fileReadNoSuchFile) }
                    let file = try await Self.stage(asset)
                    files.append(file)
                }
                try Task.checkCancellation()
                let status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
                guard valid(), ids.count <= capacity, status == .authorized || status == .limited,
                      PHAsset.fetchAssets(withLocalIdentifiers: ids, options: nil).count == ids.count else { throw CocoaError(.fileReadNoPermission) }
                add(files)
                files = []
                dismiss()
            } catch {
                if !Task.isCancelled { self.error = "These items couldn’t be added. Check photo access or your connection, then try again or use Files." }
            }
        }
    }

    private static func stage(_ asset: PHAsset) async throws -> LocalAttachmentFile {
        let resources = PHAssetResource.assetResources(for: asset)
        let edited: PHAssetResourceType = asset.mediaType == .video ? .fullSizeVideo : .fullSizePhoto
        let original: PHAssetResourceType = asset.mediaType == .video ? .video : .photo
        guard let resource = resources.first(where: { $0.type == edited }) ?? resources.first(where: { $0.type == original }) else { throw CocoaError(.fileReadUnknown) }
        let directory = try AttachmentStaging.newDirectory()
        let name = AttachmentStaging.sanitized(resource.originalFilename)
        let url = directory.appendingPathComponent(name)
        do {
            let options = PHAssetResourceRequestOptions()
            options.isNetworkAccessAllowed = true
            let copy = try AttachmentResourceCopy(url: url)
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    let id = PHAssetResourceManager.default().requestData(for: resource, options: options, dataReceivedHandler: { data in
                        copy.receive(data)
                    }, completionHandler: { error in
                        if let error = copy.finish(error) { continuation.resume(throwing: error) }
                        else { continuation.resume() }
                    })
                    copy.register(id)
                }
            } onCancel: { copy.cancel() }
            try Task.checkCancellation()
            return try AttachmentStaging.file(at: url, name: name)
        } catch { try? FileManager.default.removeItem(at: directory); throw error }
    }
}

private struct AttachmentPhotoTile: View {
    let asset: PHAsset
    let order: Int?
    @State private var image: UIImage?
    @State private var request: PHImageRequestID?
    @State private var active = false
    var body: some View {
        Color.clear.aspectRatio(1, contentMode: .fit)
            .background(CaperTheme.raised)
            .overlay {
                if let image { Image(uiImage: image).resizable().scaledToFill() }
                else { Image(systemName: "photo").foregroundStyle(CaperTheme.muted).accessibilityLabel("Preview unavailable") }
            }
            .clipped()
            .overlay { Rectangle().strokeBorder(order != nil ? CaperTheme.terracotta : .clear, lineWidth: 3) }
            .overlay(alignment: .topTrailing) {
                if let order { Text("\(order)").font(CaperTheme.font(12, weight: .bold)).frame(width: 28, height: 28).background(CaperTheme.terracotta, in: Circle()).padding(5) }
                else { Circle().strokeBorder(CaperTheme.text, lineWidth: 1).background(CaperTheme.blackout.opacity(0.6), in: Circle()).frame(width: 24, height: 24).padding(5) }
            }
            .overlay(alignment: .bottomLeading) {
                if asset.mediaType == .video {
                    Text(String(format: "%d:%02d", Int(asset.duration) / 60, Int(asset.duration) % 60))
                        .font(CaperTheme.font(12).monospacedDigit()).padding(4).background(CaperTheme.blackout.opacity(0.8))
                }
            }
            .onAppear {
                active = true
                let options = PHImageRequestOptions()
                options.isNetworkAccessAllowed = false
                options.deliveryMode = .opportunistic
                request = PHImageManager.default().requestImage(for: asset, targetSize: CGSize(width: 360, height: 360), contentMode: .aspectFill, options: options) { result, _ in
                    Task { @MainActor in if active { image = result } }
                }
            }
            .onDisappear {
                active = false
                if let request { PHImageManager.default().cancelImageRequest(request) }
                request = nil
                image = nil
            }
    }
}
#endif
