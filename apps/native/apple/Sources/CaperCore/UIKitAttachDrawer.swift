#if os(iOS)
import UIKit
import Photos
import PhotosUI
import UniformTypeIdentifiers
import Combine
import Observation

/// Edge-to-edge photo drawer. PhotoKit and file providers hand over original
/// bytes; the draft still owns upload ordering, limits and cancellation.
@MainActor final class UIKitAttachDrawer: UIViewController, UICollectionViewDataSource, UICollectionViewDelegateFlowLayout,
    UIDocumentPickerDelegate, PHPickerViewControllerDelegate {
    private let mode: AttachDrawerMode
    private let close: () -> Void
    private let library = AttachDrawerLibrary()
    private var changes: AnyCancellable?
    private var foreground: NSObjectProtocol?
    private var queue = AttachDrawerQueue()
    private var copies: [String: (id: UUID, task: Task<Void, Never>)] = [:]
    private var notice: String?
    private let panel = UIView()
    private let titleLabel = UIKitMessageDialog.label("", size: 15, bold: true)
    private let hint = UIKitMessageDialog.label("")
    private let access = UIKitMessageDialog.label("")
    private let grid: UICollectionView
    private var send: UIButton!
    private var manage: UIButton!
    private var settings: UIButton!
    private var resize: UIButton!
    private var panelHeight: NSLayoutConstraint!
    private var expanded = false
    private var drag: CGFloat = 0
    private var active = true
    private var cameraDelegate: AttachCameraPicker.Coordinator?
    private var displayedAssets: PHFetchResult<PHAsset>?
    private var displayedFiles: [AttachDrawerFile] = []
    private var displayedPending: [String] = []
    private var displayedFull = false
    private var gridWidth: CGFloat = 0
    private var appeared = false
    private var draft: AttachDrawerDraft? { if case .attach(let draft) = mode { return draft }; return nil }
    private var picking: Bool { draft == nil }
    private var used: Int { (draft?.files().count ?? 0) + queue.order.count }
    private var full: Bool { !picking && used >= AttachmentPolicy.maxAttachments }
    private var cameraAvailable: Bool { UIImagePickerController.isSourceTypeAvailable(.camera) }

    init(mode: AttachDrawerMode, close: @escaping () -> Void) {
        self.mode = mode; self.close = close
        let layout = UICollectionViewFlowLayout(); layout.minimumInteritemSpacing = 2; layout.minimumLineSpacing = 2
        grid = UICollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(nibName: nil, bundle: nil); modalPresentationStyle = .overFullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { foreground.map(NotificationCenter.default.removeObserver); library.stop() }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = .dark; view.backgroundColor = .clear; view.accessibilityViewIsModal = true
        let scrim = UIButton(); scrim.backgroundColor = UIColor.black.withAlphaComponent(0.55)
        scrim.isAccessibilityElement = false; scrim.addAction(UIAction { [weak self] _ in self?.close() }, for: .touchUpInside)
        scrim.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(scrim)
        panel.backgroundColor = UIColor(CaperTheme.surface); panel.layer.cornerRadius = 16
        panel.layer.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner]; panel.translatesAutoresizingMaskIntoConstraints = false
        panel.accessibilityIdentifier = picking ? "picture-drawer" : "attach-drawer"; view.addSubview(panel)
        let name = picking ? "picture chooser" : "attachment drawer"
        let cancel = UIKitMessageDialog.button("Close") { [weak self] in self?.close() }; cancel.accessibilityLabel = "Close \(name)"
        resize = UIButton(type: .system); resize.setTitle("━", for: .normal); resize.tintColor = UIColor(CaperTheme.muted)
        resize.accessibilityIdentifier = "attach-drawer-resize"
        resize.addAction(UIAction { [weak self] _ in guard let self else { return }; self.expanded.toggle(); self.resizePanel(animated: true) }, for: .touchUpInside)
        resize.heightAnchor.constraint(equalToConstant: 28).isActive = true
        let titleRow = UIStackView(arrangedSubviews: [titleLabel, cancel]); titleRow.spacing = 12
        let header = UIStackView(arrangedSubviews: [resize, titleRow, hint]); header.axis = .vertical; header.spacing = 4
        header.addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(pan)))
        grid.backgroundColor = .clear; grid.dataSource = self; grid.delegate = self
        grid.register(UIKitAttachTile.self, forCellWithReuseIdentifier: "photo"); grid.accessibilityIdentifier = "attach-drawer-grid"
        grid.contentInset.bottom = 72
        manage = UIKitMessageDialog.button("Manage") { [library] in library.manageLimitedAccess() }
        settings = UIKitMessageDialog.button("Open Settings") {
            if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
        }
        let photos = UIKitMessageDialog.button("Photos") { [weak self] in self?.choosePhotos() }; photos.accessibilityIdentifier = "attach-drawer-photos"
        let files = UIKitMessageDialog.button("Files") { [weak self] in self?.chooseFiles() }; files.accessibilityIdentifier = "attach-drawer-files"
        let camera = UIKitMessageDialog.button(picking ? "Take a photo" : "Camera") { [weak self] in self?.chooseCamera() }
        camera.accessibilityIdentifier = "attach-drawer-camera"; camera.isHidden = !cameraAvailable
        let tools = UIStackView(arrangedSubviews: [photos, files, camera]); tools.distribution = .fillEqually; tools.spacing = 8
        let stack = UIStackView(arrangedSubviews: [header, access, manage, settings, grid, tools]); stack.axis = .vertical; stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false; panel.addSubview(stack)
        send = UIKitMessageDialog.button("Send", primary: true) { [weak self] in
            guard let self, let draft = self.draft, draft.valid(), draft.canSend(), self.queue.order.isEmpty else { return }
            draft.send(); self.close()
        }
        send.accessibilityIdentifier = "attach-drawer-send"; send.translatesAutoresizingMaskIntoConstraints = false; panel.addSubview(send)
        panelHeight = panel.heightAnchor.constraint(equalToConstant: 400)
        NSLayoutConstraint.activate([
            scrim.leadingAnchor.constraint(equalTo: view.leadingAnchor), scrim.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrim.topAnchor.constraint(equalTo: view.topAnchor), scrim.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            panel.leadingAnchor.constraint(equalTo: view.leadingAnchor), panel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            panel.bottomAnchor.constraint(equalTo: view.bottomAnchor), panelHeight,
            stack.leadingAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.leadingAnchor, constant: 12), stack.trailingAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: panel.topAnchor, constant: 4), stack.bottomAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.bottomAnchor, constant: -8),
            send.trailingAnchor.constraint(equalTo: stack.trailingAnchor), send.bottomAnchor.constraint(equalTo: tools.topAnchor, constant: -12),
        ])
        changes = library.objectWillChange.sink { [weak self] in DispatchQueue.main.async { [weak self] in self?.render() } }
        foreground = NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.library.reload() }
        }
        library.start(imagesOnly: picking); observe()
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews(); resizePanel(animated: false)
        if gridWidth != grid.bounds.width { gridWidth = grid.bounds.width; grid.collectionViewLayout.invalidateLayout() }
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !appeared else { return }; appeared = true
        guard !CaperAccessibility.reduceMotion else { return }
        panel.alpha = 0; panel.transform = CGAffineTransform(translationX: 0, y: 12)
        UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
            self.panel.alpha = 1; self.panel.transform = .identity
        }.startAnimation()
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        // A camera or system picker temporarily covers the drawer. In-flight
        // copies still belong to the user's taps after the drawer is dismissed.
        if presentingViewController == nil { active = false; library.stop() }
    }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor [weak self] in self?.observe() } }
    }
    private func render() {
        guard isViewLoaded, active else { return }
        if let draft, !draft.valid() { close(); return }
        if case .picture(let title, _) = mode { titleLabel.text = title } else { titleLabel.text = "Attach" }
        hint.text = notice ?? (picking ? nil : AttachCapacity.hint(used)); hint.isHidden = hint.text == nil
        access.text = library.status == .limited ? "Caper can see only the photos you allowed."
            : library.status == .restricted ? "Photo access is restricted on this device."
            : library.status == .denied ? "Caper can’t see your photos. Allow access in Settings, or choose items one at a time."
            : library.assets?.count == 0 ? (picking ? "No photos yet." : "No photos or videos yet.") : nil
        access.isHidden = access.text == nil; manage.isHidden = library.status != .limited; settings.isHidden = library.status != .denied
        send.isHidden = picking || (draft?.files().isEmpty != false && queue.order.isEmpty)
        send.isEnabled = draft?.canSend() == true && queue.order.isEmpty
        send.accessibilityLabel = send.isEnabled ? "Send message" : "Uploading attachments"
        let files = draft?.files() ?? []
        if displayedAssets !== library.assets || displayedFiles != files || displayedPending != queue.order || displayedFull != full {
            displayedAssets = library.assets; displayedFiles = files; displayedPending = queue.order; displayedFull = full; grid.reloadData()
        }
    }
    private func resizePanel(animated: Bool) {
        guard panelHeight != nil else { return }
        let height = max(1, view.bounds.height - view.safeAreaInsets.top)
        // A landscape phone cannot fit the permission controls in half its
        // height. Keep them reachable rather than squeezing the gallery below zero.
        let minimum = height < 500 ? height : height * 0.5
        panelHeight.constant = min(height, max(minimum, (expanded ? height : minimum) - drag))
        resize.accessibilityLabel = "\(expanded ? "Collapse" : "Expand") \(picking ? "picture chooser" : "attachment drawer")"
        if animated, !CaperAccessibility.reduceMotion {
            UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) { self.view.layoutIfNeeded() }.startAnimation()
        }
    }
    @objc private func pan(_ gesture: UIPanGestureRecognizer) {
        drag = gesture.translation(in: view).y
        if gesture.state == .ended || gesture.state == .cancelled {
            let travel = drag + gesture.velocity(in: view).y * 0.15
            if !expanded && travel > 80 { close(); return }
            if abs(travel) > 40 { expanded = travel < 0 }
            drag = 0; resizePanel(animated: true)
        } else { resizePanel(animated: false) }
    }
    private func nudge() { notice = AttachCapacity.hint(AttachmentPolicy.maxAttachments); UINotificationFeedbackGenerator().notificationOccurred(.warning); render() }
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { library.assets?.count ?? 0 }
    func collectionView(_ collectionView: UICollectionView, layout collectionViewLayout: UICollectionViewLayout, sizeForItemAt indexPath: IndexPath) -> CGSize {
        let columns = max(1, floor((collectionView.bounds.width + 2) / 90)), side = floor((collectionView.bounds.width - (columns - 1) * 2) / columns)
        return CGSize(width: side, height: side)
    }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "photo", for: indexPath) as! UIKitAttachTile
        guard let asset = library.assets?.object(at: indexPath.item) else { return cell }
        let source = AttachDrawer.source(asset.localIdentifier), files = draft?.files() ?? []
        let order = files.firstIndex { $0.source == source }.map { $0 + 1 }
        cell.configure(asset, order: order, pending: queue.isPending(source), full: full) { [weak self] in self?.choose(asset.localIdentifier) }
        return cell
    }
    private func choose(_ identifier: String) {
        if picking {
            choosePicture { guard let file = await AttachDrawerLibrary.stage(identifier) else { throw PictureError.unreadable }; return try await Self.read(file) }
            return
        }
        guard let draft, draft.valid() else { return }
        let source = AttachDrawer.source(identifier)
        if let file = draft.files().first(where: { $0.source == source }) { draft.remove(file.id); notice = nil; render(); return }
        if queue.isPending(source) {
            copies.removeValue(forKey: source)?.task.cancel()
            if let file = queue.cancel(source) { AttachmentStaging.remove(file.url) }
            let ready = queue.takeReady()
            if draft.valid() { draft.add(ready) } else { ready.forEach { AttachmentStaging.remove($0.url) } }
            notice = nil; render(); return
        }
        guard !full else { nudge(); return }
        enqueue(source: source) { await AttachDrawerLibrary.stage(identifier) }
    }
    private func enqueue(source: String, load: @escaping () async -> LocalAttachmentFile?) {
        guard let draft, draft.valid(), !queue.isPending(source) else { return }
        notice = nil; queue.enqueue(source); render(); UISelectionFeedbackGenerator().selectionChanged()
        let id = UUID()
        let task = Task { @MainActor in
            let file = await load()
            guard !Task.isCancelled, copies[source]?.id == id, queue.isPending(source) else { if let file { AttachmentStaging.remove(file.url) }; return }
            copies[source] = nil
            if file == nil { notice = "That item couldn’t be added. Check your connection and try again." }
            let ready = queue.finish(source, file)
            if draft.valid() { draft.add(ready) } else { ready.forEach { AttachmentStaging.remove($0.url) } }
            render()
        }
        copies[source] = (id, task)
    }
    private func choosePicture(_ load: @escaping () async throws -> Data) {
        guard case .picture(_, let pick) = mode else { return }; close(); pick(load)
    }
    private static func read(_ file: LocalAttachmentFile) async throws -> Data {
        try await Task.detached { defer { AttachmentStaging.remove(file.url) }; return try PictureFile.read(file.url) }.value
    }
    private func chooseFiles() {
        guard !full else { nudge(); return }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: picking ? [.image] : [.item], asCopy: true)
        picker.allowsMultipleSelection = !picking; picker.delegate = self; present(picker, animated: !UIAccessibility.isReduceMotionEnabled)
    }
    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        if picking {
            guard let url = urls.first else { return }
            choosePicture { try await Task.detached { try PictureFile.read(url) }.value }; return
        }
        guard let draft, draft.valid() else { return }
        let room = max(0, AttachmentPolicy.maxAttachments - used)
        let files = urls.prefix(room).compactMap { try? AttachmentStaging.stage(copying: $0) }
        for file in files { enqueue(source: "file:\(UUID())") { file } }
        if urls.count > room { notice = "Only \(room) more file\(room == 1 ? "" : "s") can be attached." }
        else if files.count < urls.count { notice = "Some files couldn’t be added." }
        else { close() }
        render()
    }
    private func choosePhotos() {
        guard !full else { nudge(); return }
        var configuration = PHPickerConfiguration(photoLibrary: .shared())
        configuration.selectionLimit = picking ? 1 : max(1, AttachmentPolicy.maxAttachments - used)
        configuration.filter = picking ? .images : .any(of: [.images, .videos]); configuration.preferredAssetRepresentationMode = .current
        let picker = PHPickerViewController(configuration: configuration); picker.delegate = self; present(picker, animated: !UIAccessibility.isReduceMotionEnabled)
    }
    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: false) { [self] in
            guard !results.isEmpty else { return }
            if picking {
                let result = results[0]
                choosePicture { guard let file = await Self.stage(result) else { throw PictureError.unreadable }; return try await Self.read(file) }
            } else {
                for result in results {
                    let source = result.assetIdentifier.map(AttachDrawer.source) ?? "picker:\(UUID())"
                    guard draft?.files().contains(where: { $0.source == source }) != true, !queue.isPending(source) else { continue }
                    guard !full else { nudge(); break }
                    enqueue(source: source) { await Self.stage(result) }
                }
            }
        }
    }
    private static func stage(_ result: PHPickerResult) async -> LocalAttachmentFile? {
        let provider = result.itemProvider
        guard let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true || UTType($0)?.conforms(to: .movie) == true }) else { return nil }
        return await withCheckedContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { url, _ in
                // Copy inside the provider callback, before its temporary file expires.
                var file = url.flatMap { try? AttachmentStaging.stage(copying: $0) }
                if let source = result.assetIdentifier { file?.source = AttachDrawer.source(source) }
                continuation.resume(returning: file)
            }
        }
    }
    private func chooseCamera() {
        guard cameraAvailable, !full else { if full { nudge() }; return }
        let picker = UIImagePickerController(); picker.sourceType = .camera
        picker.mediaTypes = picking ? [UTType.image.identifier] : [UTType.image.identifier, UTType.movie.identifier]
        picker.videoQuality = .typeHigh
        cameraDelegate = AttachCameraPicker.Coordinator { [weak self, weak picker] file in
            picker?.dismiss(animated: false) {
                guard let self else { if let file { AttachmentStaging.remove(file.url) }; return }
                self.cameraDelegate = nil
                guard let file else { return }
                if self.picking { self.choosePicture { try await Self.read(file) } }
                else if let draft = self.draft, draft.valid(), !self.full { self.enqueue(source: "camera:\(UUID())") { file } }
                else { AttachmentStaging.remove(file.url); self.nudge() }
            }
        }
        picker.delegate = cameraDelegate; present(picker, animated: !UIAccessibility.isReduceMotionEnabled)
    }
}

@MainActor private final class UIKitAttachTile: UICollectionViewCell {
    private let button = UIButton(type: .custom)
    private let picture = UIImageView()
    private let badge = UILabel()
    private var request: PHImageRequestID?
    private var animation: Task<Void, Never>?
    private var motion: NSObjectProtocol?
    private var asset: PHAsset?
    private var pick: (() -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame); contentView.addSubview(button); button.addSubview(picture); button.addSubview(badge)
        picture.contentMode = .scaleAspectFill; picture.clipsToBounds = true; picture.isUserInteractionEnabled = false
        badge.textColor = .white; badge.backgroundColor = UIColor.black.withAlphaComponent(0.7); badge.textAlignment = .center
        badge.layer.cornerRadius = 8; badge.clipsToBounds = true; badge.isUserInteractionEnabled = false
        button.addAction(UIAction { [weak self] _ in self?.pick?() }, for: .touchUpInside)
        motion = NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { guard let self else { return }; self.clear(); if self.window != nil, let asset = self.asset { self.load(asset) } }
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { motion.map(NotificationCenter.default.removeObserver) }
    override func layoutSubviews() { super.layoutSubviews(); button.frame = contentView.bounds; picture.frame = button.bounds; badge.frame = CGRect(x: max(0, bounds.width - 54), y: 4, width: 50, height: 24) }
    override func prepareForReuse() { super.prepareForReuse(); clear() }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { clear() } else if let asset { load(asset) } }
    private func clear() {
        if let request { PHImageManager.default().cancelImageRequest(request) }; request = nil
        animation?.cancel(); animation = nil; picture.layer.removeAllAnimations(); picture.image = nil
    }
    func configure(_ asset: PHAsset, order: Int?, pending: Bool, full: Bool, pick: @escaping () -> Void) {
        clear(); self.asset = asset; self.pick = pick
        let date = asset.creationDate?.formatted(date: .abbreviated, time: .omitted) ?? ""
        button.accessibilityIdentifier = "attach-drawer-tile"
        button.accessibilityLabel = "\(asset.mediaType == .video ? "Video, \(Int(asset.duration)) seconds" : AttachGalleryCache.isGIF(asset) ? "GIF" : "Photo"), \(date)"
        button.accessibilityValue = order.map { "Attached \($0)" } ?? (pending ? "Adding" : full ? "Attachment limit reached" : "")
        button.accessibilityTraits = order == nil ? .button : [.button, .selected]
        badge.text = order.map(String.init) ?? (pending ? "…" : asset.mediaType == .video ? VoiceDuration.format(ms: Int(asset.duration * 1000)) : AttachGalleryCache.isGIF(asset) ? "GIF" : nil)
        badge.isHidden = badge.text == nil; picture.alpha = full && order == nil && !pending ? 0.4 : 1
        load(asset)
    }
    private func load(_ asset: PHAsset) {
        guard request == nil else { return }
        let key = AttachGalleryCache.key(asset)
        picture.image = AttachGalleryCache.thumbnail(key)
        request = AttachGalleryCache.requestThumbnail(asset, scale: window?.screen.scale ?? 2) { [weak self] image in
            if let self, self.asset.map(AttachGalleryCache.key) == key { self.picture.image = image }
        }
        guard !CaperAccessibility.reduceMotion, AttachGalleryCache.isGIF(asset) else { return }
        animation = Task { [weak self] in
            guard let original = await AttachGalleryCache.animation(for: asset), !Task.isCancelled, !CaperAccessibility.reduceMotion,
                  let self, self.asset.map(AttachGalleryCache.key) == key else { return }
            let total = original.durations.reduce(0, +)
            guard total > 0, original.images.count > 1, original.images.count == original.durations.count else { return }
            var elapsed = 0.0
            var times: [NSNumber] = [0]
            for duration in original.durations { elapsed += duration; times.append(NSNumber(value: min(1, elapsed / total))) }
            let frames = CAKeyframeAnimation(keyPath: "contents")
            frames.values = original.images + [original.images[0]]; frames.keyTimes = times; frames.calculationMode = .discrete
            frames.duration = total; frames.repeatCount = .infinity
            self.picture.layer.add(frames, forKey: "frames")
        }
    }
}
#endif
