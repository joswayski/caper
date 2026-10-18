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
    @Published var loading = false
    private var observing = false

    func refresh() {
        status = PHPhotoLibrary.authorizationStatus(for: .readWrite)
        guard status == .authorized || status == .limited else { assets = []; return }
        if !observing { PHPhotoLibrary.shared().register(self); observing = true }
        loading = true
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.predicate = NSPredicate(format: "mediaType == %d OR mediaType == %d", PHAssetMediaType.image.rawValue, PHAssetMediaType.video.rawValue)
        let result = PHAsset.fetchAssets(with: options)
        var next: [PHAsset] = []
        result.enumerateObjects { asset, _, _ in next.append(asset) }
        assets = next
        loading = false
    }

    func request() {
        PHPhotoLibrary.requestAuthorization(for: .readWrite) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    nonisolated func photoLibraryDidChange(_ changeInstance: PHChange) {
        Task { @MainActor [weak self] in self?.refresh() }
    }

    deinit { if observing { PHPhotoLibrary.shared().unregisterChangeObserver(self) } }
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
    private var capacity: Int { max(0, AttachmentPolicy.maxAttachments - existingCount()) }

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                Text("\(selection.ids.count) selected · \(max(0, capacity - selection.ids.count)) remaining")
                Text("Up to \(AttachmentPolicy.maxAttachments) attachments, including \(existingCount()) already in this draft.")
                    .font(.footnote).foregroundStyle(CaperTheme.muted)
                if selection.ids.count >= capacity {
                    Text("Attachment limit reached. Deselect an item to choose another.").font(.footnote)
                }
                if let error { Text(error).foregroundStyle(CaperTheme.error).font(.footnote) }
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
                } else if library.loading {
                    ProgressView("Loading photos…")
                } else if library.assets.isEmpty {
                    ContentUnavailableView("No accessible photos or videos", systemImage: "photo", description: Text("Use Files or change photo access to choose other media."))
                } else {
                    Text("Cloud-only previews may be unavailable. Selected originals download when you tap Add.")
                        .font(.footnote).foregroundStyle(CaperTheme.muted)
                    ScrollView {
                        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 3), count: 3), spacing: 3) {
                            ForEach(library.assets, id: \.localIdentifier) { asset in
                                let order = selection.ids.firstIndex(of: asset.localIdentifier).map { $0 + 1 }
                                Button { selection.toggle(asset.localIdentifier, capacity: capacity) } label: {
                                    AttachmentPhotoTile(asset: asset, order: order)
                                        .opacity(order == nil && selection.ids.count >= capacity ? 0.35 : 1)
                                }
                                .buttonStyle(.plain)
                                .disabled(importing != nil)
                                .accessibilityLabel("\(asset.mediaType == .video ? "Video" : "Photo"), \(asset.creationDate?.formatted() ?? "unknown date")")
                                .accessibilityValue(order.map { "Selected \($0)" } ?? "Not selected")
                                .accessibilityHint(order != nil ? "Double tap to deselect" : "Double tap to select")
                                .accessibilityAddTraits(order != nil ? [.isSelected] : [])
                            }
                        }
                    }
                }
                Spacer(minLength: 0)
                Button("Use Files instead") { importing?.cancel(); useFiles(); dismiss() }
                Button(importing == nil ? "Add \(selection.ids.count)" : "Preparing files…") { importSelection() }
                    .fontWeight(.semibold).frame(maxWidth: .infinity).padding(12)
                    .foregroundStyle(CaperTheme.onAction)
                    .background(selection.canAdd(capacity: capacity) && valid() ? CaperTheme.action : CaperTheme.border, in: RoundedRectangle(cornerRadius: 8))
                    .disabled(!selection.canAdd(capacity: capacity) || !valid() || importing != nil)
            }
            .padding(16).foregroundStyle(CaperTheme.text).background(CaperTheme.surface)
            .navigationTitle("Photos and videos").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { importing?.cancel(); dismiss() } } }
        }
        .presentationDetents([.large]).presentationDragIndicator(.visible)
        .task { library.refresh() }
        .onChange(of: scenePhase) { _, phase in if phase == .active { library.refresh() } }
        .onChange(of: library.assets.map(\.localIdentifier)) { _, ids in selection.retain(Set(ids)) }
        .onChange(of: library.status) { _, status in
            if status != .authorized && status != .limited { importing?.cancel() }
        }
        .onChange(of: valid()) { _, value in if !value { importing?.cancel(); dismiss() } }
        .onDisappear { importing?.cancel() }
    }

    private func manageLimitedAccess() {
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first(where: { $0.activationState == .foregroundActive }),
              var controller = scene.windows.first(where: \.isKeyWindow)?.rootViewController else { return }
        while let presented = controller.presentedViewController { controller = presented }
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: controller)
    }

    private func importSelection() {
        let ids = selection.ids
        importing = Task { @MainActor in
            var files: [LocalAttachmentFile] = []
            defer { files.forEach { AttachmentStaging.remove($0.url) }; importing = nil }
            do {
                for id in ids {
                    try Task.checkCancellation()
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
            // PhotoKit streams directly to disk, including iCloud originals; no
            // full-resolution image/video Data is held by the gallery.
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                PHAssetResourceManager.default().writeData(for: resource, toFile: url, options: options) { error in
                    if let error { continuation.resume(throwing: error) } else { continuation.resume() }
                }
            }
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
            .overlay(alignment: .topTrailing) {
                if let order { Text("\(order)").font(.caption.bold()).padding(7).background(CaperTheme.terracotta, in: Circle()).padding(5) }
            }
            .overlay(alignment: .bottomLeading) {
                if asset.mediaType == .video {
                    Text(String(format: "%d:%02d", Int(asset.duration) / 60, Int(asset.duration) % 60))
                        .font(.caption.monospacedDigit()).padding(4).background(CaperTheme.blackout.opacity(0.8))
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
