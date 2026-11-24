import AVKit
import SwiftUI
import Observation

#if os(macOS)

@MainActor final class AppKitMediaViewer: NSViewController {
    private let state: NativeMediaViewing
    private let observation = AppKitObservation()
    private var canvas: AppKitMediaCanvas!
    private let titleLabel = AppKitKit.label("", size: 14, weight: .bold, wraps: false)
    private let counter = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let fileSize = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let notice = AppKitKit.label("", size: 13)
    private let status = AppKitKit.label("", size: 13)
    private let spinner = NSProgressIndicator()
    private var previous: AppKitKitButton!
    private var next: AppKitKitButton!
    private var hide: AppKitKitButton!
    private var play: AppKitKitButton!
    private var share: AppKitKitButton!
    private var open: AppKitKitButton!
    private var more: AppKitKitButton!
    private var sharing: NSSharingServicePicker?

    init(request: MediaViewerRequest, close: @escaping () -> Void) {
        state = NativeMediaViewing(request: request, close: close); super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        canvas = AppKitMediaCanvas(state: state); view = canvas
        view.setAccessibilityIdentifier("media-viewer"); view.setAccessibilityRole(.group)
        let close = button("Close viewer", icon: "xmark", id: "close") { [state] in state.finish() }
        share = button("Save or share", icon: "square.and.arrow.up", id: "download") { [weak self] in self?.shareFile() }
        open = button("Open in browser", icon: "arrow.up.right.square", id: "open") { [state] in
            state.perform { _, url in NSWorkspace.shared.open(url); return nil }
        }
        more = button("More media actions", icon: "ellipsis", id: "actions") { [weak self] in self?.actions() }
        counter.setAccessibilityIdentifier("media-viewer-counter"); fileSize.setAccessibilityIdentifier("media-viewer-size")
        let details = NSStackView(views: [counter, fileSize]); details.spacing = 8
        let names = AppKitKit.stack([titleLabel, details], spacing: 2)
        let header = NSStackView(views: [names, NSView(), share, open, more, close]); header.spacing = 8
        previous = button("Previous file", icon: "chevron.left", id: "previous") { [weak self] in self?.move(-1) }
        next = button("Next file", icon: "chevron.right", id: "next") { [weak self] in self?.move(1) }
        hide = AppKitKitButton("Hide spoiler") { [state] in state.hideSpoiler() }
        hide.setAccessibilityIdentifier("media-viewer-hide-spoiler")
        play = AppKitKitButton("Play animation") { [state] in state.toggleAnimation() }
        play.setAccessibilityIdentifier("media-viewer-animation-toggle")
        let footer = NSStackView(views: [status, play, hide]); footer.spacing = 12
        // A dark disc keeps the spinner visible over a light poster.
        spinner.style = .spinning; spinner.controlSize = .regular; spinner.isDisplayedWhenStopped = false
        spinner.appearance = NSAppearance(named: .darkAqua); spinner.wantsLayer = true
        spinner.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.55).cgColor; spinner.layer?.cornerRadius = 28
        spinner.setAccessibilityLabel("Loading")
        for child in [header, previous!, next!, spinner, notice, footer] {
            child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child)
        }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.topAnchor, constant: 12),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12), header.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            previous.centerYAnchor.constraint(equalTo: view.centerYAnchor), previous.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12),
            next.centerYAnchor.constraint(equalTo: view.centerYAnchor), next.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            notice.centerXAnchor.constraint(equalTo: view.centerXAnchor), notice.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor), spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            spinner.widthAnchor.constraint(equalToConstant: 56), spinner.heightAnchor.constraint(equalToConstant: 56),
            footer.centerXAnchor.constraint(equalTo: view.centerXAnchor), footer.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16),
        ])
        canvas.step = { [weak self] in self?.move($0) }
    }
    private func button(_ title: String, icon: String, id: String, action: @escaping () -> Void) -> AppKitKitButton {
        // AppKitKitButton draws any title itself, over the icon; the name stays
        // the accessibility label and tooltip.
        let button = AppKitKitButton("", kind: .plain, action: action)
        button.imagePosition = .imageOnly; button.minimumHeight = 36
        button.image = NSImage(systemSymbolName: icon, accessibilityDescription: title)
        button.setAccessibilityLabel(title); button.setAccessibilityIdentifier("media-viewer-\(id)"); button.toolTip = title
        button.widthAnchor.constraint(equalToConstant: 36).isActive = true
        return button
    }
    override func viewDidAppear() {
        super.viewDidAppear(); state.start(); view.window?.makeFirstResponder(canvas)
        observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
    }
    override func viewDidDisappear() { super.viewDidDisappear(); observation.stop(); state.stop(); canvas.refresh() }
    private func move(_ delta: Int) {
        state.move(delta)
        NSAccessibility.post(element: view, notification: .announcementRequested, userInfo: [
            .announcement: "\(state.item?.name ?? ""), \(state.position + 1) of \(state.items.count)",
            .priority: NSAccessibilityPriorityLevel.high.rawValue,
        ])
    }
    private func render() {
        state.synchronize()
        let item = state.item
        titleLabel.stringValue = item?.name ?? ""
        view.setAccessibilityLabel(item?.kind == .video ? "Video viewer" : "Image viewer")
        counter.stringValue = MediaViewerPolicy.counter(position: state.position, count: state.items.count) ?? ""
        counter.isHidden = counter.stringValue.isEmpty
        counter.setAccessibilityLabel("\(state.position + 1) of \(state.items.count)")
        fileSize.stringValue = item.flatMap(MediaViewerPolicy.size) ?? ""
        previous.isHidden = state.items.count < 2; next.isHidden = previous.isHidden
        previous.isEnabled = state.position > 0; next.isEnabled = state.position < state.items.count - 1
        for button in [share!, open!, more!] { button.isHidden = !state.available; button.isEnabled = !state.busy }
        hide.isHidden = item?.spoiler != true
        play.isHidden = !state.animated || state.failed; play.title = state.playing ? "Pause animation" : "Play animation"
        status.stringValue = state.status ?? ""; status.isHidden = state.status == nil
        notice.stringValue = !state.available ? "This file is no longer available." : state.failed ? "Couldn’t load this file. Try opening it in your browser." : ""
        notice.isHidden = notice.stringValue.isEmpty
        if state.spinning { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        canvas.refresh()
    }
    private func shareFile() {
        state.perform { [weak self] item, url in
            let file = try await AttachmentImageLoader.shared.download(url, name: item.name)
            guard let self, self.view.window != nil, !Task.isCancelled else { return nil }
            let picker = NSSharingServicePicker(items: [file]); self.sharing = picker
            picker.show(relativeTo: self.share.bounds, of: self.share, preferredEdge: .minY)
            return nil
        }
    }
    private func actions() {
        let menu = NSMenu()
        if state.item?.kind == .image {
            let copy = NSMenuItem(title: "Copy image", action: #selector(copyImage), keyEquivalent: ""); copy.target = self; menu.addItem(copy)
        }
        let link = NSMenuItem(title: "Copy media link", action: #selector(copyLink), keyEquivalent: ""); link.target = self; menu.addItem(link)
        CaperMenu.show(menu, below: more)
    }
    @objc private func copyImage() { state.copyImage() }
    @objc private func copyLink() {
        state.perform { _, url in
            NSPasteboard.general.clearContents()
            return NSPasteboard.general.setString(url.absoluteString, forType: .string) ? "Media link copied" : "Couldn’t copy media link."
        }
    }
}

@MainActor private final class AppKitMediaCanvas: NSView {
    let state: NativeMediaViewing
    var step: ((Int) -> Void)?
    private let picture = AppKitMediaImage()
    private let video = AVPlayerView()
    private var drag: CGPoint?
    private var offset = CGSize.zero
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    init(state: NativeMediaViewing) {
        self.state = state; super.init(frame: .zero)
        wantsLayer = true; layer?.backgroundColor = NSColor.black.withAlphaComponent(0.96).cgColor; layer?.masksToBounds = true
        picture.imageScaling = .scaleProportionallyUpOrDown; picture.setAccessibilityElement(false)
        addSubview(picture); addSubview(video)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    private var container: CGRect { bounds.insetBy(dx: 72, dy: 64) }
    private var fitted: CGSize {
        MediaViewerPolicy.fittedSize(width: Double(state.image?.width ?? 1), height: Double(state.image?.height ?? 1), in: container.size)
    }
    func refresh() {
        picture.image = state.image.map { NSImage(cgImage: $0, size: .zero) }
        // A video keeps its poster (the picture) on screen until the player
        // is ready, instead of the player's black frame.
        let videoShown = state.player != nil && !state.loading
        picture.isHidden = !state.available || videoShown
        picture.setAccessibilityElement(!picture.isHidden && state.image != nil)
        picture.setAccessibilityIdentifier("media-viewer-image"); picture.setAccessibilityLabel(state.item?.name)
        if video.player !== state.player { video.player = state.player }
        video.isHidden = !videoShown; video.controlsStyle = state.animated ? .none : .floating
        needsLayout = true
    }
    override func layout() {
        super.layout(); video.frame = container
        let size = fitted, offset = MediaViewerPolicy.clampedOffset(state.offset, scale: state.scale, fitted: size, container: container.size)
        picture.frame = CGRect(x: bounds.midX + offset.width - size.width * state.scale / 2,
            y: bounds.midY + offset.height - size.height * state.scale / 2, width: size.width * state.scale, height: size.height * state.scale)
    }
    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .arrow)
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.invalidateCursorRects(for: self)
    }
    override func cancelOperation(_ sender: Any?) { state.finish() }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 123 { step?(-1) } else if event.keyCode == 124 { step?(1) } else { super.keyDown(with: event) }
    }
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard state.player == nil else { if !container.contains(point) { state.finish() }; return }
        guard picture.frame.contains(point) else { state.finish(); return }
        if event.clickCount == 2 {
            state.scale = state.scale > 1 ? 1 : MediaViewerPolicy.doubleTapZoom
            state.offset = state.scale == 1 ? .zero : MediaViewerPolicy.clampedOffset(
                MediaViewerPolicy.zoomOffset(at: CGPoint(x: point.x - picture.frame.minX, y: point.y - picture.frame.minY), fitted: fitted, scale: state.scale),
                scale: state.scale, fitted: fitted, container: container.size)
        } else { drag = point; offset = state.offset }
    }
    override func mouseDragged(with event: NSEvent) {
        guard state.scale > 1, let drag else { return }
        let point = convert(event.locationInWindow, from: nil)
        state.offset = MediaViewerPolicy.clampedOffset(CGSize(width: offset.width + point.x - drag.x, height: offset.height + point.y - drag.y),
            scale: state.scale, fitted: fitted, container: container.size)
    }
    override func mouseUp(with event: NSEvent) {
        if let drag, state.animated, state.scale == 1 {
            let point = convert(event.locationInWindow, from: nil)
            if hypot(point.x - drag.x, point.y - drag.y) < 4 { state.toggleAnimation() }
        }
        drag = nil
    }
    override func magnify(with event: NSEvent) {
        guard state.player == nil else { return }
        state.scale = min(MediaViewerPolicy.maxZoom, max(1, state.scale * (1 + event.magnification)))
        state.offset = MediaViewerPolicy.clampedOffset(state.offset, scale: state.scale, fitted: fitted, container: container.size)
    }
}

private final class AppKitMediaImage: NSImageView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    // A non-editable image view otherwise reports a generic role, so
    // VoiceOver would not announce the viewed file as an image.
    override func accessibilityRole() -> NSAccessibility.Role? { .image }
}
#elseif os(iOS)
struct NativeMediaViewerBridge: UIViewControllerRepresentable {
    let request: MediaViewerRequest
    let close: () -> Void
    func makeUIViewController(context: Context) -> UIKitMediaViewer { UIKitMediaViewer(request: request, close: close) }
    func updateUIViewController(_ controller: UIKitMediaViewer, context: Context) {}
}

@MainActor final class UIKitMediaViewer: UIViewController, UIScrollViewDelegate, UIGestureRecognizerDelegate {
    private let state: NativeMediaViewing
    private let scroll = UIScrollView()
    private let image = UIImageView()
    private let video = AVPlayerViewController()
    private let titleLabel = UIKitMessageDialog.label("", size: 14, bold: true)
    private let counter = UIKitMessageDialog.label("", size: 12)
    private let fileSize = UIKitMessageDialog.label("", size: 12)
    private let notice = UIKitMessageDialog.label("")
    private let status = UIKitMessageDialog.label("")
    private let spinner = UIActivityIndicatorView(style: .large)
    private var previous: UIButton!
    private var nextButton: UIButton!
    private var hide: UIButton!
    private var play: UIButton!
    private var more: UIButton!
    private var active = false
    private var displayedID: String?
    private var lastBounds = CGSize.zero
    private var fittedSize = CGSize.zero
    private var pullingDown = false
    override var canBecomeFirstResponder: Bool { true }
    init(request: MediaViewerRequest, close: @escaping () -> Void) {
        state = NativeMediaViewing(request: request, close: close)
        super.init(nibName: nil, bundle: nil); modalPresentationStyle = .overFullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = .dark; view.backgroundColor = .black
        view.accessibilityIdentifier = "media-viewer"; view.accessibilityViewIsModal = true
        scroll.delegate = self; scroll.minimumZoomScale = 1; scroll.maximumZoomScale = MediaViewerPolicy.maxZoom
        scroll.showsHorizontalScrollIndicator = false; scroll.showsVerticalScrollIndicator = false
        image.contentMode = .scaleAspectFit; scroll.addSubview(image); view.addSubview(scroll)
        addChild(video); view.addSubview(video.view); video.didMove(toParent: self)
        let close = control("Close viewer", icon: "xmark", id: "close") { [state] in state.finish() }
        more = control("More media actions", icon: "ellipsis", id: "actions") {}; more.showsMenuAsPrimaryAction = true
        titleLabel.numberOfLines = 1; titleLabel.lineBreakMode = .byTruncatingMiddle
        counter.accessibilityIdentifier = "media-viewer-counter"; fileSize.accessibilityIdentifier = "media-viewer-size"
        for label in [counter, fileSize] { label.numberOfLines = 1; label.textColor = UIColor.white.withAlphaComponent(0.6) }
        let details = UIStackView(arrangedSubviews: [counter, fileSize]); details.spacing = 8
        // Name and size read as one left-aligned block, centred on the buttons.
        let names = UIStackView(arrangedSubviews: [titleLabel, details]); names.axis = .vertical; names.alignment = .leading; names.spacing = 2
        let header = UIStackView(arrangedSubviews: [names, more, close]); header.alignment = .center; header.spacing = 8
        names.setContentHuggingPriority(.defaultLow, for: .horizontal)
        // A dark disc keeps the spinner visible over a light poster.
        spinner.color = .white; spinner.hidesWhenStopped = true; spinner.accessibilityLabel = "Loading"
        spinner.backgroundColor = UIColor.black.withAlphaComponent(0.55); spinner.layer.cornerRadius = 32
        previous = control("Previous file", icon: "chevron.left", id: "previous") { [weak self] in self?.move(-1) }
        nextButton = control("Next file", icon: "chevron.right", id: "next") { [weak self] in self?.move(1) }
        play = UIKitMessageDialog.button("Play animation") { [state] in state.toggleAnimation() }
        play.accessibilityIdentifier = "media-viewer-animation-toggle"
        hide = UIKitMessageDialog.button("Hide spoiler") { [state] in state.hideSpoiler() }; hide.accessibilityIdentifier = "media-viewer-hide-spoiler"
        let actions = UIStackView(arrangedSubviews: [play, hide]); actions.spacing = 8
        let footer = UIStackView(arrangedSubviews: [status, actions]); footer.axis = .vertical; footer.alignment = .center; footer.spacing = 4
        for child in [header, previous!, nextButton!, spinner, notice, footer] {
            child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child)
        }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 4),
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor), spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            spinner.widthAnchor.constraint(equalToConstant: 64), spinner.heightAnchor.constraint(equalToConstant: 64),
            header.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            header.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            previous.centerYAnchor.constraint(equalTo: view.centerYAnchor), previous.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 8),
            nextButton.centerYAnchor.constraint(equalTo: view.centerYAnchor), nextButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -8),
            notice.centerXAnchor.constraint(equalTo: view.centerXAnchor), notice.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            notice.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
            footer.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -8), footer.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            footer.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -24),
        ])
        let paging = UIPanGestureRecognizer(target: self, action: #selector(paged)); paging.delegate = self
        paging.maximumNumberOfTouches = 1; view.addGestureRecognizer(paging)
        scroll.panGestureRecognizer.require(toFail: paging)
        let double = UITapGestureRecognizer(target: self, action: #selector(zoom)); double.numberOfTapsRequired = 2; scroll.addGestureRecognizer(double)
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped)); tap.require(toFail: double)
        scroll.addGestureRecognizer(tap)
    }
    private func control(_ title: String, icon: String, id: String, action: @escaping () -> Void) -> UIButton {
        var configuration = UIButton.Configuration.plain()
        configuration.image = UIImage(systemName: icon, withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold))
        configuration.baseForegroundColor = .white; configuration.cornerStyle = .capsule
        configuration.background.backgroundColor = UIColor.white.withAlphaComponent(0.14)
        let button = UIButton(configuration: configuration)
        button.accessibilityLabel = title; button.accessibilityIdentifier = "media-viewer-\(id)"
        button.addAction(UIAction { _ in action() }, for: .touchUpInside)
        button.widthAnchor.constraint(equalToConstant: 44).isActive = true; button.heightAnchor.constraint(equalToConstant: 44).isActive = true
        return button
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); active = true; state.start(); observe(); becomeFirstResponder() }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); active = false; state.stop(); video.player = nil }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    private func render() {
        state.synchronize()
        let item = state.item
        if displayedID != item?.id { displayedID = item?.id; scroll.setZoomScale(1, animated: false) }
        titleLabel.text = item?.name; view.accessibilityLabel = item?.kind == .video ? "Video viewer" : "Image viewer"
        counter.text = MediaViewerPolicy.counter(position: state.position, count: state.items.count); counter.isHidden = counter.text == nil
        counter.accessibilityLabel = "\(state.position + 1) of \(state.items.count)"; fileSize.text = item.flatMap(MediaViewerPolicy.size)
        fileSize.isHidden = fileSize.text == nil
        previous.isHidden = state.items.count < 2; nextButton.isHidden = previous.isHidden
        previous.isEnabled = state.position > 0; nextButton.isEnabled = state.position < state.items.count - 1
        more.isHidden = !state.available; more.isEnabled = !state.busy
        hide.isHidden = item?.spoiler != true; play.isHidden = !state.animated || state.failed
        var configuration = play.configuration; configuration?.title = state.playing ? "Pause animation" : "Play animation"; play.configuration = configuration
        status.text = state.status; status.isHidden = state.status == nil
        notice.text = !state.available ? "This file is no longer available." : state.failed ? "Couldn’t load this file. Try opening it in your browser." : nil
        notice.isHidden = notice.text == nil
        if state.spinning { spinner.startAnimating() } else { spinner.stopAnimating() }
        // A video keeps its poster (the image) on screen until the player is
        // ready, instead of the player's black frame.
        let videoShown = state.player != nil && !state.loading
        image.image = state.image.map { UIImage(cgImage: $0) }; scroll.isHidden = !state.available || videoShown
        image.isAccessibilityElement = !scroll.isHidden && state.image != nil
        image.accessibilityTraits = .image; image.accessibilityIdentifier = "media-viewer-image"; image.accessibilityLabel = item?.name
        if video.player !== state.player { video.player = state.player }
        video.view.isHidden = !videoShown; video.showsPlaybackControls = !state.animated
        var actions: [UIAction] = []
        if item?.kind == .image { actions.append(UIAction(title: "Copy image", image: UIImage(systemName: "doc.on.doc")) { [state] _ in state.copyImage() }) }
        actions.append(UIAction(title: "Save or share", image: UIImage(systemName: "square.and.arrow.up")) { [weak self] _ in self?.shareFile() })
        actions.append(UIAction(title: "Copy media link", image: UIImage(systemName: "link")) { [state] _ in
            state.perform { _, url in await MainActor.run { UIPasteboard.general.url = url }; return "Media link copied" }
        })
        actions.append(UIAction(title: "Open in browser", image: UIImage(systemName: "arrow.up.right.square")) { [state] _ in
            state.perform { _, url in await UIApplication.shared.open(url); return nil }
        })
        more.menu = UIMenu(children: actions); view.setNeedsLayout()
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let rect = view.bounds.inset(by: UIEdgeInsets(top: view.safeAreaInsets.top + 52, left: 0, bottom: view.safeAreaInsets.bottom + 56, right: 0))
        let fitted = MediaViewerPolicy.fittedSize(width: Double(state.image?.width ?? 1), height: Double(state.image?.height ?? 1), in: rect.size)
        if lastBounds != rect.size || fittedSize != fitted {
            lastBounds = rect.size; scroll.setZoomScale(1, animated: false)
            fittedSize = fitted; scroll.bounds.size = rect.size
            image.frame = CGRect(origin: .zero, size: fitted); scroll.contentSize = fitted
            scrollViewDidZoom(scroll)
        }
        // A paging/dismissal gesture transforms these views. Changing frame
        // under that transform changes their size and snaps the gesture back.
        scroll.center = CGPoint(x: rect.midX, y: rect.midY)
        video.view.bounds = CGRect(origin: .zero, size: rect.size)
        video.view.center = scroll.center
    }
    func viewForZooming(in scrollView: UIScrollView) -> UIView? { image }
    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        image.center = CGPoint(x: max(scrollView.bounds.width, scrollView.contentSize.width) / 2,
            y: max(scrollView.bounds.height, scrollView.contentSize.height) / 2)
    }
    @objc private func tapped(_ recognizer: UITapGestureRecognizer) {
        if !image.frame.contains(recognizer.location(in: scroll)), scroll.zoomScale == 1 { state.finish() }
        else if state.animated { state.toggleAnimation() }
    }
    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard scroll.zoomScale == 1 else { return false }
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer else { return true }
        let velocity = pan.velocity(in: view)
        return abs(velocity.x) > abs(velocity.y) || velocity.y > 0
    }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        var target = touch.view
        while let view = target {
            if view is UIControl { return false }
            target = view.superview
        }
        return scroll.frame.contains(touch.location(in: view))
    }
    @objc private func paged(_ recognizer: UIPanGestureRecognizer) {
        let moved = recognizer.translation(in: view), velocity = recognizer.velocity(in: view)
        if recognizer.state == .began { pullingDown = velocity.y > abs(velocity.x) }
        switch recognizer.state {
        case .began, .changed:
            transformMedia(CGAffineTransform(translationX: pullingDown ? 0 : moved.x,
                y: pullingDown ? max(0, moved.y) + min(0, moved.y) / 3 : 0))
        case .ended:
            if pullingDown {
                if moved.y > 120 || moved.y > 24 && velocity.y > 700 { state.finish() }
                else { settleMedia() }
            } else if abs(moved.x) > view.bounds.width * 0.2 || abs(moved.x) > 24 && abs(velocity.x) > 550 {
                move(moved.x < 0 ? 1 : -1)
            } else { settleMedia() }
        case .cancelled, .failed: settleMedia()
        default: break
        }
    }
    private func transformMedia(_ transform: CGAffineTransform) { scroll.transform = transform; video.view.transform = transform }
    private func settleMedia() {
        if CaperAccessibility.reduceMotion { transformMedia(.identity) }
        else {
            UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
                self.transformMedia(.identity)
            }.startAnimation()
        }
    }
    @objc private func zoom(_ recognizer: UITapGestureRecognizer) {
        if scroll.zoomScale > 1 { scroll.setZoomScale(1, animated: !CaperAccessibility.reduceMotion) }
        else {
            let point = recognizer.location(in: image), scale = MediaViewerPolicy.doubleTapZoom
            let size = CGSize(width: scroll.bounds.width / scale, height: scroll.bounds.height / scale)
            scroll.zoom(to: CGRect(x: point.x - size.width / 2, y: point.y - size.height / 2, width: size.width, height: size.height), animated: !CaperAccessibility.reduceMotion)
        }
    }
    private func move(_ delta: Int) {
        let previous = state.position
        state.move(delta)
        if state.position != previous, !CaperAccessibility.reduceMotion {
            transformMedia(CGAffineTransform(translationX: CGFloat(delta) * view.bounds.width, y: 0))
        }
        settleMedia()
        UIAccessibility.post(notification: .announcement, argument: "\(state.item?.name ?? ""), \(state.position + 1) of \(state.items.count)")
    }
    private func shareFile() {
        state.perform { [weak self] item, url in
            let file = try await AttachmentImageLoader.shared.download(url, name: item.name)
            guard let self, self.view.window != nil, !Task.isCancelled else { return nil }
            let share = UIActivityViewController(activityItems: [file], applicationActivities: nil)
            share.popoverPresentationController?.sourceView = self.more; share.popoverPresentationController?.sourceRect = self.more.bounds
            self.present(share, animated: true); return nil
        }
    }
    override var keyCommands: [UIKeyCommand]? {
        [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(closeViewer)),
         UIKeyCommand(input: UIKeyCommand.inputLeftArrow, modifierFlags: [], action: #selector(previousFile)),
         UIKeyCommand(input: UIKeyCommand.inputRightArrow, modifierFlags: [], action: #selector(nextFile))]
    }
    @objc private func closeViewer() { state.finish() }
    @objc private func previousFile() { move(-1) }
    @objc private func nextFile() { move(1) }
}
#endif
