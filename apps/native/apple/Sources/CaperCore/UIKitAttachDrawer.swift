#if os(iOS)
import UIKit
import Photos
import PhotosUI
import UniformTypeIdentifiers
import Combine
import Observation

/// Edge-to-edge photo drawer. PhotoKit and file providers hand over original
/// bytes; the draft still owns upload ordering, limits and cancellation.
/// Docked (the composer's drawer), it replaces the keyboard below the message
/// box, as Discord's does: picks queue in the composer, which sends them, and
/// its emoji and microphone stay in reach. Otherwise (pictures, the quick
/// composer) it is a modal sheet with its own Send.
@MainActor final class UIKitAttachDrawer: UIViewController, UICollectionViewDataSource, UICollectionViewDelegateFlowLayout,
    UIDocumentPickerDelegate, PHPickerViewControllerDelegate {
    private let mode: AttachDrawerMode
    private let close: () -> Void
    /// Docked: who presents the camera and pickers, and the home indicator's inset.
    private let presenter: (() -> UIViewController?)?
    private let dockInset: CGFloat
    private var docked: Bool { presenter != nil }
    private let library = AttachDrawerLibrary()
    private var changes: AnyCancellable?
    private var foreground: NSObjectProtocol?
    private var queue = AttachDrawerQueue()
    private var copies: [String: (id: UUID, task: Task<Void, Never>)] = [:]
    private var notice: String?
    private let panel = UIView()
    private let scrim = UIButton()
    private let titleLabel = UIKitMessageDialog.label("", size: 17, bold: true)
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
    private var sendShown = false
    private var draft: AttachDrawerDraft? { if case .attach(let draft) = mode { return draft }; return nil }
    private var picking: Bool { draft == nil }
    private var used: Int { (draft?.files().count ?? 0) + queue.order.count }
    private var full: Bool { !picking && used >= AttachmentPolicy.maxAttachments }
    private var cameraAvailable: Bool { UIImagePickerController.isSourceTypeAvailable(.camera) }

    init(mode: AttachDrawerMode, presenter: (() -> UIViewController?)? = nil, dockInset: CGFloat = 0, close: @escaping () -> Void) {
        self.mode = mode; self.close = close; self.presenter = presenter; self.dockInset = dockInset
        let layout = UICollectionViewFlowLayout(); layout.minimumInteritemSpacing = Self.spacing; layout.minimumLineSpacing = Self.spacing
        grid = UICollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(nibName: nil, bundle: nil); modalPresentationStyle = .overFullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { foreground.map(NotificationCenter.default.removeObserver); library.stop() }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = CaperTheme.interfaceStyle; view.backgroundColor = .clear; view.accessibilityViewIsModal = !docked
        scrim.backgroundColor = UIColor.black.withAlphaComponent(0.55)
        scrim.isAccessibilityElement = false; scrim.addAction(UIAction { [weak self] _ in self?.close() }, for: .touchUpInside)
        scrim.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(scrim); scrim.isHidden = docked
        panel.backgroundColor = UIColor(CaperTheme.surface); panel.layer.cornerRadius = 18
        panel.layer.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner]; panel.translatesAutoresizingMaskIntoConstraints = false
        panel.accessibilityIdentifier = picking ? "picture-drawer" : "attach-drawer"; view.addSubview(panel)
        let name = picking ? "picture chooser" : "attachment drawer"
        // The handle: a short capsule across the top; tapping it expands the drawer.
        resize = UIButton(type: .custom); resize.accessibilityIdentifier = "attach-drawer-resize"; resize.accessibilityLabel = "Expand \(name)"
        resize.addAction(UIAction { [weak self] _ in guard let self, !self.docked else { return }; self.expanded.toggle(); CaperHaptics.tap(); self.resizePanel(animated: true) }, for: .touchUpInside)
        // Docked, the handle only drags the drawer down; the composer's X closes it.
        resize.isAccessibilityElement = !docked
        let grabber = UIView(); grabber.backgroundColor = UIColor(CaperTheme.muted).withAlphaComponent(0.6); grabber.layer.cornerRadius = 2.5
        grabber.isUserInteractionEnabled = false; grabber.translatesAutoresizingMaskIntoConstraints = false; resize.addSubview(grabber)
        let cancel = UIButton(type: .system); cancel.setImage(attachIcon("x", size: 18), for: .normal); cancel.tintColor = UIColor(CaperTheme.muted)
        cancel.accessibilityLabel = "Close \(name)"; cancel.addAction(UIAction { [weak self] _ in self?.close() }, for: .touchUpInside)
        hint.textColor = UIColor(CaperTheme.muted)
        let titles = UIStackView(arrangedSubviews: [titleLabel, hint]); titles.axis = .vertical; titles.spacing = 2
        let titleRow = UIStackView(arrangedSubviews: [titles, cancel]); titleRow.alignment = .center; titleRow.spacing = 12
        // The title takes the slack so Close keeps its own width at the trailing edge.
        titles.setContentHuggingPriority(.defaultLow, for: .horizontal); cancel.setContentHuggingPriority(.required, for: .horizontal)
        let header = UIStackView(arrangedSubviews: [resize, titleRow]); header.axis = .vertical
        // Docked, the composer above says what it is and closes it; only the limit line shows.
        titleLabel.isHidden = docked; cancel.isHidden = docked
        header.addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(pan)))
        grid.backgroundColor = .clear; grid.dataSource = self; grid.delegate = self
        grid.register(UIKitAttachTile.self, forCellWithReuseIdentifier: "photo"); grid.register(UIKitAttachCameraTile.self, forCellWithReuseIdentifier: "camera")
        grid.accessibilityIdentifier = "attach-drawer-grid"
        // Keep the last row clear of the floating Send button (sheets only).
        grid.contentInset.bottom = docked ? 8 : 76
        manage = UIKitMessageDialog.button("Manage") { [library] in library.manageLimitedAccess() }
        settings = UIKitMessageDialog.button("Open Settings") {
            if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
        }
        // Tabs, as on Android: Photos is the grid above (tap for the whole
        // library), Files opens the document picker. The camera is the grid's first tile.
        let photos = Self.tab("Photos", icon: "images", selected: true) { [weak self] in self?.choosePhotos() }; photos.accessibilityIdentifier = "attach-drawer-photos"
        let files = Self.tab("Files", icon: "paperclip", selected: false) { [weak self] in self?.chooseFiles() }; files.accessibilityIdentifier = "attach-drawer-files"
        let tools = UIStackView(arrangedSubviews: [photos, files]); tools.distribution = .fillEqually
        let divider = UIView(); divider.backgroundColor = UIColor(CaperTheme.border)
        let stack = UIStackView(arrangedSubviews: [header, access, manage, settings, grid, divider, tools]); stack.axis = .vertical; stack.spacing = 8
        stack.setCustomSpacing(4, after: header); stack.setCustomSpacing(4, after: divider)
        stack.translatesAutoresizingMaskIntoConstraints = false; panel.addSubview(stack)
        var sendConfig = UIButton.Configuration.filled(); sendConfig.image = attachIcon("send", size: 20); sendConfig.cornerStyle = .capsule
        send = UIButton(configuration: sendConfig, primaryAction: UIAction { [weak self] _ in
            guard let self, let draft = self.draft, draft.valid(), draft.canSend(), self.queue.order.isEmpty else { return }
            draft.send(); self.close()
        })
        // Chalk white once everything is in; neutral chrome while items copy or upload.
        send.configurationUpdateHandler = { button in
            var config = button.configuration
            config?.baseBackgroundColor = UIColor(button.isEnabled ? CaperTheme.action : CaperTheme.raised)
            config?.baseForegroundColor = UIColor(button.isEnabled ? CaperTheme.onAction : CaperTheme.muted)
            config?.background.strokeColor = button.isEnabled ? .clear : UIColor(CaperTheme.border); config?.background.strokeWidth = 1
            button.configuration = config
        }
        send.accessibilityIdentifier = "attach-drawer-send"; send.translatesAutoresizingMaskIntoConstraints = false; send.isHidden = true; panel.addSubview(send)
        panelHeight = panel.heightAnchor.constraint(equalToConstant: 400)
        NSLayoutConstraint.activate(docked ? [
            panel.topAnchor.constraint(equalTo: view.topAnchor),
            stack.bottomAnchor.constraint(equalTo: panel.bottomAnchor, constant: -(dockInset + 4)),
        ] : [
            panelHeight,
            stack.bottomAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.bottomAnchor, constant: -4),
        ])
        NSLayoutConstraint.activate([
            scrim.leadingAnchor.constraint(equalTo: view.leadingAnchor), scrim.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrim.topAnchor.constraint(equalTo: view.topAnchor), scrim.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            panel.leadingAnchor.constraint(equalTo: view.leadingAnchor), panel.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            panel.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.leadingAnchor, constant: 12), stack.trailingAnchor.constraint(equalTo: panel.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: panel.topAnchor),
            resize.heightAnchor.constraint(equalToConstant: 28),
            grabber.widthAnchor.constraint(equalToConstant: 36), grabber.heightAnchor.constraint(equalToConstant: 5),
            grabber.centerXAnchor.constraint(equalTo: resize.centerXAnchor), grabber.centerYAnchor.constraint(equalTo: resize.centerYAnchor),
            cancel.widthAnchor.constraint(equalToConstant: 44), cancel.heightAnchor.constraint(equalToConstant: 44),
            divider.heightAnchor.constraint(equalToConstant: 1),
            send.widthAnchor.constraint(equalToConstant: 52), send.heightAnchor.constraint(equalToConstant: 52),
            send.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -4), send.bottomAnchor.constraint(equalTo: divider.topAnchor, constant: -16),
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
    // The sheet rises from the bottom edge as the scrim fades in, set up
    // before the first frame so it never shows in place first. Closing stays instant.
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        guard !appeared, !docked, !CaperAccessibility.reduceMotion else { return }
        view.layoutIfNeeded()
        scrim.alpha = 0; panel.transform = CGAffineTransform(translationX: 0, y: panelHeight.constant)
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !appeared else { return }; appeared = true
        guard !docked, !CaperAccessibility.reduceMotion else { return }
        UIViewPropertyAnimator(duration: CaperMotion.slow, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
            self.scrim.alpha = 1; self.panel.transform = .identity
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
        let shows = !picking && !docked && (draft?.files().isEmpty == false || !queue.order.isEmpty)
        send.isEnabled = draft?.canSend() == true && queue.order.isEmpty
        send.accessibilityLabel = send.isEnabled ? "Send message" : "Uploading attachments"
        if shows != sendShown {
            sendShown = shows; send.isHidden = !shows
            // Send pops in with the first pick, as attached files do in the composer.
            if shows, appeared, !CaperAccessibility.reduceMotion {
                send.alpha = 0; send.transform = CGAffineTransform(scaleX: 0.94, y: 0.94)
                UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
                    self.send.alpha = 1; self.send.transform = .identity
                }.startAnimation()
            }
        }
        let files = draft?.files() ?? []
        if displayedAssets !== library.assets {
            displayedAssets = library.assets; displayedFiles = files; displayedPending = queue.order; displayedFull = full; grid.reloadData()
        } else if displayedFiles != files || displayedPending != queue.order || displayedFull != full {
            // A tap changes only badges and dimming: update the tiles in place
            // so thumbnails and GIFs keep playing instead of reloading.
            displayedFiles = files; displayedPending = queue.order; displayedFull = full
            grid.reconfigureItems(at: grid.indexPathsForVisibleItems)
        }
    }
    private func resizePanel(animated: Bool) {
        guard panelHeight != nil, !docked else { return }
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
            if docked { drag = 0; return }
            // Snapping between half and full height taps once.
            if abs(travel) > 40, expanded != (travel < 0) { expanded = travel < 0; CaperHaptics.tap() }
            drag = 0; resizePanel(animated: true)
        } else if !docked { resizePanel(animated: false) }
    }
    private func nudge() { notice = AttachCapacity.hint(AttachmentPolicy.maxAttachments); UINotificationFeedbackGenerator().notificationOccurred(.warning); render() }
    private static let spacing: CGFloat = 3
    /// The camera leads the grid when the device has one.
    private var cameraTiles: Int { cameraAvailable ? 1 : 0 }
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { (library.assets?.count ?? 0) + cameraTiles }
    func collectionView(_ collectionView: UICollectionView, layout collectionViewLayout: UICollectionViewLayout, sizeForItemAt indexPath: IndexPath) -> CGSize {
        // Three large tiles across a phone, more on wider screens.
        let width = collectionView.bounds.width
        let columns = max(3, floor((width + Self.spacing) / 124)), side = floor((width - (columns - 1) * Self.spacing) / columns)
        return CGSize(width: side, height: side)
    }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: indexPath.item < cameraTiles ? "camera" : "photo", for: indexPath)
        configure(cell, at: indexPath)
        return cell
    }
    // A prefetched tile missed the in-place update while offscreen.
    func collectionView(_ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell, forItemAt indexPath: IndexPath) {
        configure(cell, at: indexPath)
    }
    private func configure(_ cell: UICollectionViewCell, at indexPath: IndexPath) {
        if let camera = cell as? UIKitAttachCameraTile {
            camera.configure(label: picking ? "Take a photo" : "Camera", dimmed: full) { [weak self] in self?.chooseCamera() }
            return
        }
        let index = indexPath.item - cameraTiles
        guard let cell = cell as? UIKitAttachTile, let assets = library.assets, index >= 0, index < assets.count else { return }
        let asset = assets.object(at: index)
        let source = AttachDrawer.source(asset.localIdentifier), files = draft?.files() ?? []
        // A tap shows its place at once; the copy joins the draft in tap order.
        let order = files.firstIndex { $0.source == source }.map { $0 + 1 } ?? queue.order.firstIndex(of: source).map { files.count + $0 + 1 }
        cell.configure(asset, order: order, pending: queue.isPending(source), full: full) { [weak self] in self?.choose(asset.localIdentifier) }
    }
    /// A toolbar tab: its icon over its name, bright and bold when selected.
    private static func tab(_ title: String, icon: String, selected: Bool, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.image = attachIcon(icon, size: 20); config.imagePlacement = .top; config.imagePadding = 4; config.title = title
        config.baseForegroundColor = UIColor(selected ? CaperTheme.text : CaperTheme.muted)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
            var attributes = incoming
            attributes.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(for: UIFont(name: selected ? "Figtree-Bold" : "Figtree-Regular", size: 11) ?? .systemFont(ofSize: 11))
            return attributes
        }
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 52).isActive = true
        if selected { button.accessibilityTraits.insert(.selected) }
        return button
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
        picker.allowsMultipleSelection = !picking; picker.delegate = self; show(picker)
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
        let picker = PHPickerViewController(configuration: configuration); picker.delegate = self; show(picker)
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
        picker.delegate = cameraDelegate; show(picker)
    }
    /// A docked drawer lives in the keyboard's place, outside any controller, so
    /// the conversation presents its camera and pickers.
    private func show(_ controller: UIViewController) {
        (presenter?() ?? self).present(controller, animated: !CaperAccessibility.reduceMotion)
    }
}

/// A Lucide icon from the app's catalog (CaperIcons.xcassets), drawn at `size` as a template.
@MainActor private func attachIcon(_ name: String, size: CGFloat) -> UIImage? {
    guard let image = UIImage(named: "caper-\(name)") ?? UIImage(named: "caper-\(name)", in: caperResourceBundle, compatibleWith: nil) else { return nil }
    return UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { _ in
        image.draw(in: CGRect(x: 0, y: 0, width: size, height: size))
    }.withRenderingMode(.alwaysTemplate)
}

/// The grid's first tile opens the camera.
@MainActor private final class UIKitAttachCameraTile: UICollectionViewCell {
    private let button = UIButton(type: .custom)
    private var take: (() -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame)
        contentView.backgroundColor = UIColor(CaperTheme.raised); contentView.layer.cornerRadius = 8; contentView.clipsToBounds = true
        button.setImage(attachIcon("camera", size: 26), for: .normal); button.tintColor = UIColor(CaperTheme.muted)
        button.accessibilityIdentifier = "attach-drawer-camera"
        button.addAction(UIAction { [weak self] _ in self?.take?() }, for: .touchUpInside)
        contentView.addSubview(button)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layoutSubviews() { super.layoutSubviews(); button.frame = contentView.bounds }
    func configure(label: String, dimmed: Bool, take: @escaping () -> Void) {
        button.accessibilityLabel = label; button.alpha = dimmed ? 0.4 : 1; self.take = take
    }
}

/// A library tile: once it's in the draft, a green numbered badge over a
/// darkened, green-edged photo (in tap order).
@MainActor private final class UIKitAttachTile: UICollectionViewCell {
    private let button = UIButton(type: .custom)
    private let picture = UIImageView()
    private let shade = UIView()
    private let ring = UILabel()
    private let badge = UILabel()
    private var request: PHImageRequestID?
    private var animation: Task<Void, Never>?
    private var motion: NSObjectProtocol?
    private var asset: PHAsset?
    private var order: Int?
    private var pick: (() -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame)
        contentView.backgroundColor = UIColor(CaperTheme.raised); contentView.layer.cornerRadius = 8; contentView.clipsToBounds = true
        contentView.addSubview(button); [picture, shade, ring, badge].forEach { $0.isUserInteractionEnabled = false; button.addSubview($0) }
        picture.contentMode = .scaleAspectFill; picture.clipsToBounds = true
        shade.backgroundColor = UIColor(CaperTheme.blackout).withAlphaComponent(0.3)
        shade.layer.borderColor = UIColor(CaperTheme.terracotta).cgColor; shade.layer.borderWidth = 3; shade.layer.cornerRadius = 8
        ring.font = UIFont(name: "Figtree-Bold", size: 12) ?? .boldSystemFont(ofSize: 12); ring.textColor = UIColor(CaperTheme.text)
        ring.textAlignment = .center; ring.layer.cornerRadius = 12; ring.clipsToBounds = true
        ring.backgroundColor = UIColor(CaperTheme.terracotta)
        badge.font = (UIFont(name: "Figtree-Bold", size: 11) ?? .boldSystemFont(ofSize: 11)); badge.textColor = UIColor(CaperTheme.text)
        badge.backgroundColor = UIColor(CaperTheme.blackout).withAlphaComponent(0.7); badge.textAlignment = .center
        badge.layer.cornerRadius = 4; badge.clipsToBounds = true
        // The tile button already speaks the attach order and duration.
        ring.isAccessibilityElement = false; badge.isAccessibilityElement = false
        button.addAction(UIAction { [weak self] _ in self?.pick?() }, for: .touchUpInside)
        motion = NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { guard let self else { return }; self.clear(); if self.window != nil, let asset = self.asset { self.load(asset) } }
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { motion.map(NotificationCenter.default.removeObserver) }
    override func layoutSubviews() {
        super.layoutSubviews()
        button.frame = contentView.bounds; picture.frame = button.bounds; shade.frame = button.bounds
        ring.frame = CGRect(x: bounds.width - 30, y: 6, width: 24, height: 24)
        let width = badge.sizeThatFits(CGSize(width: bounds.width, height: 18)).width + 10
        badge.frame = CGRect(x: 5, y: bounds.height - 23, width: min(bounds.width - 10, width), height: 18)
    }
    override func prepareForReuse() { super.prepareForReuse(); clear() }
    override func didMoveToWindow() { super.didMoveToWindow(); if window == nil { clear() } else if let asset { load(asset) } }
    private func clear() {
        if let request { PHImageManager.default().cancelImageRequest(request) }; request = nil
        animation?.cancel(); animation = nil; picture.layer.removeAllAnimations(); picture.image = nil
    }
    func configure(_ asset: PHAsset, order: Int?, pending: Bool, full: Bool, pick: @escaping () -> Void) {
        // The same photo keeps its thumbnail, request and GIF frames.
        let same = self.asset.map(AttachGalleryCache.key) == AttachGalleryCache.key(asset)
        if !same { clear() }
        let arrives = same && self.order == nil && order != nil
        self.asset = asset; self.order = order; self.pick = pick
        let date = asset.creationDate?.formatted(date: .abbreviated, time: .omitted) ?? ""
        button.accessibilityIdentifier = "attach-drawer-tile"
        button.accessibilityLabel = "\(asset.mediaType == .video ? "Video, \(Int(asset.duration)) seconds" : AttachGalleryCache.isGIF(asset) ? "GIF" : "Photo"), \(date)"
        button.accessibilityValue = pending ? "Adding" : order.map { "Attached \($0)" } ?? (full ? "Attachment limit reached" : "")
        button.accessibilityTraits = order == nil ? .button : [.button, .selected]
        shade.isHidden = order == nil
        // Only picked photos carry a mark; the rest stay clean.
        ring.isHidden = order == nil
        ring.text = order.map(String.init)
        badge.text = asset.mediaType == .video ? "\u{25B6}\u{FE0E} \(VoiceDuration.format(ms: Int(asset.duration * 1000)))" : AttachGalleryCache.isGIF(asset) ? "GIF" : nil
        badge.isHidden = badge.text == nil; setNeedsLayout()
        button.alpha = full && order == nil && !pending ? 0.4 : 1
        // The number and edge pop in as an attached file does in the composer.
        if arrives, !CaperAccessibility.reduceMotion {
            ring.transform = CGAffineTransform(scaleX: 0.94, y: 0.94); ring.alpha = 0; shade.alpha = 0
            UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
                self.ring.transform = .identity; self.ring.alpha = 1; self.shade.alpha = 1
            }.startAnimation()
        } else if !arrives { ring.transform = .identity; ring.alpha = 1; shade.alpha = 1 }
        load(asset)
    }
    private func load(_ asset: PHAsset) {
        guard request == nil, animation == nil else { return }
        let key = AttachGalleryCache.key(asset)
        // A cached final thumbnail is never replaced by a blurry interim one.
        if let cached = AttachGalleryCache.thumbnail(key) { picture.image = cached }
        else { request = AttachGalleryCache.requestThumbnail(asset, scale: window?.screen.scale ?? 2) { [weak self] image in
            if let self, self.asset.map(AttachGalleryCache.key) == key { self.picture.image = image }
        } }
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
