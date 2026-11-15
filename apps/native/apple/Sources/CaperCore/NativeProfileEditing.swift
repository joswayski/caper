import ImageIO
import Observation

/// The editor owns its draft and local picture preparation. Account mutation
/// still goes through ProfileEditDraft's retry-safe, session-checked save.
@MainActor @Observable final class NativeProfileEditing {
    let model: AppModel
    let draft: ProfileEditDraft?
    private let epoch: Int
    let close: () -> Void
    var error: String?
    var confirmingDiscard = false
    var pictureBusy = false
    private(set) var saving = false
    var animationData: Data?
    var editing: AttachmentEditRequest?
    var history: NativeProfileHistory?
    private var stagedURL: URL?
    private var task: Task<Void, Never>?
    private var stopped = false

    init(model: AppModel, close: @escaping () -> Void) {
        self.model = model; self.close = close; epoch = model.accountEpoch
        draft = model.account.map(ProfileEditDraft.init)
    }
    var current: Bool { !stopped && model.accountEpoch == epoch && model.account?.id == draft?.baseline.id }
    var busy: Bool { pictureBusy || saving }
    var hasPicture: Bool { draft?.preparedPicture != nil || draft?.avatarAsset != nil }
    var canSave: Bool { current && !busy && !model.busy && draft?.canSave == true }
    func requestClose() {
        guard !saving else { return }
        if pictureBusy || draft?.hasChanges == true { confirmingDiscard = true }
        else { stop(); close() }
    }
    func discard() { guard !saving else { return }; stop(); close() }
    func save() {
        guard canSave, let draft else { return }
        error = nil; saving = true
        task = Task { [weak self] in
            guard let self else { return }
            defer { saving = false }
            do {
                try await model.saveProfile(draft: draft)
                if current { stop(); close() }
            } catch is CancellationError {} catch { if current { self.error = ProfileEditDraft.saveError(error) } }
        }
    }
    func choose(_ load: @escaping () async throws -> Data, allowAnimation: Bool = true) {
        guard current, !busy, editing == nil else { return }
        pictureBusy = true; error = nil; animationData = nil
        task = Task { [weak self] in
            guard let self else { return }
            defer { pictureBusy = false }
            do {
                let data = try await load()
                guard data.count <= PictureFile.maxBytes else { throw PictureError.tooLarge }
                let animated = await Task.detached {
                    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return false }
                    return CGImageSourceGetCount(source) > 1 || ProfilePicturePolicy.isGIF(data)
                }.value
                guard current, !Task.isCancelled else { return }
                if allowAnimation && animated { animationData = data; return }
                let image = try await Task.detached(priority: .userInitiated) { try ProfilePicturePreparer.editorImage(data) }.value
                guard current, !Task.isCancelled else { return }
                guard let url = AttachmentPreparer.writeStaged(data, name: "picture-original") else { throw PictureError.unreadable }
                stagedURL = url
                let rect = ProfilePicturePolicy.squareCrop(width: image.width, height: image.height)
                let edit = AttachmentEdit(crop: EditRect(x: rect.minX / Double(image.width), y: rect.minY / Double(image.height),
                    width: rect.width / Double(image.width), height: rect.height / Double(image.height)))
                editing = AttachmentEditRequest(name: "Adjust picture", kind: .image, original: url, edit: edit,
                    tool: .crop, pictureMode: true, deferredPicture: true) { [weak self] edit, _ in self?.prepare(data, edit: edit) }
            } catch is CancellationError {} catch { if current { self.error = ProfilePicturePolicy.message(for: error, purpose: .avatar) } }
        }
    }
    func editCurrent() {
        guard let draft else { return }
        choose {
            if let data = draft.preparedPicture?.data { return data }
            guard let url = draft.avatarPicture?.url else { throw PictureError.unreadable }
            let configuration = URLSessionConfiguration.ephemeral
            configuration.httpCookieAcceptPolicy = .never; configuration.httpShouldSetCookies = false
            let session = URLSession(configuration: configuration); defer { session.invalidateAndCancel() }
            return try await ProfilePictureDecoder.fetch(url, session: session)
        }
    }
    func useAnimation(_ data: Data) { animationData = nil; prepare(data, edit: nil) }
    func editStill(_ data: Data) { animationData = nil; choose({ data }, allowAnimation: false) }
    private func prepare(_ data: Data, edit: AttachmentEdit?) {
        guard current, !busy else { return }
        pictureBusy = true; error = nil
        task = Task { [weak self] in
            guard let self else { return }
            defer { pictureBusy = false }
            do {
                let prepared = try await Task.detached(priority: .userInitiated) {
                    if let edit { return try ProfilePicturePreparer.prepare(data, edit: edit) }
                    return try ProfilePicturePreparer.prepare(data)
                }.value
                guard current, !Task.isCancelled else { return }
                draft?.usePicture(prepared)
            } catch is CancellationError {} catch { if current { self.error = ProfilePicturePolicy.message(for: error, purpose: .avatar) } }
        }
    }
    func closeCrop() { editing = nil; if let stagedURL { AttachmentStaging.remove(stagedURL) }; stagedURL = nil }
    func openHistory() {
        guard current, !busy, let draft else { return }
        history = NativeProfileHistory(expected: draft.avatarAsset, load: { [weak self] cursor in
            guard let self, self.current else { throw CancellationError() }
            return try await self.model.avatarHistory(cursor: cursor)
        }, select: { [weak self] picture in
            guard let self, self.current else { return }
            self.draft?.useHistory(picture); self.history = nil
        })
    }
    func stop() {
        stopped = true; task?.cancel(); task = nil; closeCrop(); animationData = nil; history?.stop(); history = nil
    }
}

@MainActor @Observable final class NativeProfileHistory: Identifiable {
    let id = UUID()
    let expected: String?
    var pictures: [AvatarHistoryPicture] = []
    var selected: AvatarHistoryPicture?
    var cursor: String?
    var loaded = false
    var loading = false
    var error: String?
    private let loadPage: (String?) async throws -> AvatarHistory
    private let select: (AvatarHistoryPicture) -> Void
    private var task: Task<Void, Never>?
    init(expected: String?, load: @escaping (String?) async throws -> AvatarHistory, select: @escaping (AvatarHistoryPicture) -> Void) {
        self.expected = expected; loadPage = load; self.select = select
    }
    var canUse: Bool { !loading && selected?.avatarAsset != nil && selected?.avatarAsset != expected }
    func use() { if canUse, let selected { select(selected) } }
    func load() {
        guard !loading else { return }; loading = true; error = nil
        task = Task { [weak self] in
            guard let self else { return }; defer { loading = false }
            do {
                let page = try await loadPage(cursor)
                guard !Task.isCancelled else { return }
                for item in page.pictures where !pictures.contains(where: { $0.avatarAsset == item.avatarAsset }) { pictures.append(item) }
                cursor = page.nextCursor; loaded = true
            } catch is CancellationError {} catch { if !Task.isCancelled { self.error = "Couldn’t load pictures. Please retry." } }
        }
    }
    func stop() { task?.cancel(); task = nil }
}
