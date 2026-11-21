import Foundation
import Observation
import QuartzCore

/// One immutable render snapshot; actions resolve the draft by id, so an upload
/// finishing while its menu is open cannot resurrect an old file.
@MainActor struct NativeStagedFile {
    let id: String
    let name: String
    let kind: AttachmentKind
    let url: URL
    let thumbnail: URL?
    let status: String
    let failed: Bool
    let busy: String?
    let progress: Double?
    let removable: Bool
    let spoiler: Bool
    let editable: Bool
    let voice: ChatAttachment?
    let preview: () -> Void
    let remove: () -> Void
    let retry: (() -> Void)?
    let toggleSpoiler: (() -> Void)?

    var safeName: String { voice != nil ? "voice message" : spoiler ? "spoiler attachment" : name }
    var accessibilityStatus: String {
        if spoiler, failed { return "Spoiler, upload failed" }
        let progress = progress.flatMap { $0.isFinite ? " \(Int((min(1, max(0, $0)) * 100).rounded()))%" : nil } ?? ""
        let status = busy.map { $0 + progress } ?? (spoiler ? "" : status)
        return spoiler ? "Spoiler\(status.isEmpty ? "" : ", " + status)" : status
    }

    static func chat(_ chat: ChatModel, inThread: Bool, preview: @escaping (AttachmentDraft) -> Void) -> [Self] {
        chat.attachmentFiles(inThread: inThread).map { file in
            Self(id: file.id, name: file.name, kind: file.kind, url: file.localURL, thumbnail: file.editPreviewURL,
                status: file.statusLabel, failed: file.error != nil,
                busy: file.attachment == nil && file.error == nil ? (file.preparing ? "Preparing…" : "Uploading…") : nil,
                progress: !file.preparing ? file.progress : nil, removable: true, spoiler: file.spoiler,
                editable: file.editable && file.original != nil,
                voice: file.voiceDurationMs.map { note(file.id, url: file.localURL, duration: $0, waveform: file.voiceWaveform) },
                preview: { if let current = chat.attachmentFiles(inThread: inThread).first(where: { $0.id == file.id }) { preview(current) } },
                remove: { chat.removeAttachmentDraft(id: file.id) },
                retry: file.error == nil ? nil : { chat.retryAttachmentDraft(id: file.id) },
                toggleSpoiler: { chat.toggleAttachmentSpoiler(id: file.id) })
        }
    }
    static func quick(_ state: NativeQuickDraft) -> [Self] {
        state.draft.files.map { file in
            Self(id: file.id, name: file.local.name, kind: AttachmentKind.local(contentType: file.local.contentType),
                url: file.local.url, thumbnail: file.editPreview, status: file.uploadError ?? AttachmentPolicy.formatBytes(file.local.size),
                failed: file.uploadError != nil, busy: file.progress != nil ? "Uploading…" : nil, progress: file.progress,
                removable: state.editable, spoiler: file.spoiler, editable: file.editable && state.editable,
                voice: file.voiceDurationMs.map { note(file.id, url: file.local.url, duration: $0, waveform: file.voiceWaveform) },
                preview: { if let current = state.draft.files.first(where: { $0.id == file.id }) { state.preview(current) } },
                remove: { if state.editable { state.draft.remove(id: file.id) } }, retry: nil,
                toggleSpoiler: { if state.editable { state.draft.toggleSpoiler(id: file.id) } })
        }
    }
    private static func note(_ id: String, url: URL, duration: Int, waveform: [UInt8]?) -> ChatAttachment {
        ChatAttachment(id: "draft-\(id)", kind: .audio, contentType: VoiceMessage.uploadContentType, name: VoiceMessage.label, size: 0,
            durationMs: duration, url: url.absoluteString, voice: true, waveform: waveform.map(VoiceWaveform.encode))
    }
    static func editRequest(_ file: AttachmentDraft, chat: ChatModel, inThread: Bool) -> AttachmentEditRequest? {
        guard file.editable, let original = file.original else { return nil }
        let id = file.id, shown = file.preparing ? (file.editPreviewURL ?? file.localURL) : file.localURL
        return AttachmentEditRequest(name: file.name, kind: AttachmentKind.local(contentType: original.contentType), original: original.url,
            edit: file.edit, playback: file.kind == .video && shown != file.editPreviewURL ? shown : nil,
            spoiler: AttachmentDraftSpoiler(isOn: { chat.attachmentFiles(inThread: inThread).first { $0.id == id }?.spoiler ?? false },
                toggle: { chat.toggleAttachmentSpoiler(id: id) }), remove: { chat.removeAttachmentDraft(id: id) }) { edit, preview in
            chat.editAttachmentDraft(id: id, edit: edit, preview: preview)
        }
    }
}

/// Both tile implementations use the same waveform geometry and playback
/// clock. The clipping layer preserves the partly played boundary bar.
@MainActor final class NativeVoiceWaveform {
    let layer = CALayer()
    private let unplayed = CAShapeLayer(), played = CAShapeLayer(), clip = CALayer()
    init(playedColor: CGColor, unplayedColor: CGColor) {
        unplayed.fillColor = unplayedColor; played.fillColor = playedColor
        clip.masksToBounds = true; clip.addSublayer(played)
        layer.addSublayer(unplayed); layer.addSublayer(clip)
    }
    func update(_ voice: ChatAttachment?, frame: CGRect) {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        layer.frame = frame; layer.isHidden = voice == nil
        guard let voice else { return }
        let path = CGMutablePath(), width = frame.width, height = frame.height
        let count = VoiceWaveform.barCount(available: width, width: 3, gap: 2)
        for (index, value) in VoiceWaveform.resample(voice.waveformBytes, count: count).enumerated() {
            let barHeight = max(3, CGFloat(value) * height)
            path.addRoundedRect(in: CGRect(x: CGFloat(index) * 5, y: (height - barHeight) / 2, width: 3, height: barHeight), cornerWidth: 1.5, cornerHeight: 1.5)
        }
        unplayed.frame = layer.bounds; unplayed.path = path
        played.frame = layer.bounds; played.path = path
        let fraction = VoicePlaybackPosition.progress(elapsedMs: VoiceMessagePlayer.shared.preciseElapsedMs(for: voice) ?? 0, durationMs: voice.durationMs)
        clip.frame = CGRect(x: 0, y: 0, width: width * fraction, height: height)
    }
}

@MainActor private func stagedProgress(_ layer: CAShapeLayer, file: NativeStagedFile) {
    layer.isHidden = file.busy == nil || file.failed
    layer.strokeEnd = file.progress.flatMap { $0.isFinite ? min(1, max(0, $0)) : nil } ?? 0.75
    let rotating = !layer.isHidden && file.progress == nil && !CaperAccessibility.reduceMotion
    if rotating, layer.animation(forKey: "busy") == nil {
        let animation = CABasicAnimation(keyPath: "transform.rotation.z")
        animation.fromValue = 0; animation.toValue = Double.pi * 2; animation.duration = 1; animation.repeatCount = .infinity
        layer.add(animation, forKey: "busy")
    } else if !rotating { layer.removeAnimation(forKey: "busy") }
}

/// Thumbnail loading is owned by a retained tile, never its render pass.
@MainActor private final class StagedThumbnail {
    let layer = CALayer()
    private var source: URL?
    private var task: Task<Void, Never>?
    private var image: CGImage?
    private var hidden = false
    init() { layer.contentsGravity = .resizeAspectFill; layer.masksToBounds = true; layer.cornerRadius = 8 }
    isolated deinit { task?.cancel() }
    func update(_ file: NativeStagedFile) {
        hidden = file.spoiler; layer.contents = hidden ? nil : image
        let source = file.thumbnail ?? file.url
        guard self.source != source else { return }
        self.source = source; task?.cancel(); image = nil; layer.contents = nil
        let key = "draft:\(file.id):\(source.path)"
        task = Task { [weak self] in
            let image: CGImage?
            if file.thumbnail != nil || file.kind == .image { image = try? await AttachmentImageLoader.shared.load(key: key, url: source, maxPixelSize: 312) }
            else if file.kind == .video { image = try? await AttachmentImageLoader.shared.loadVideo(key: key, url: source, maxPixelSize: 312) }
            else { image = nil }
            guard !Task.isCancelled, let self, self.source == source else { return }
            self.image = image; self.layer.contents = self.hidden ? nil : image
        }
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitStagedFiles: NSView {
    var heightChanged: (() -> Void)?
    private let files: () -> [NativeStagedFile]
    private let observation = AppKitObservation()
    private var tiles: [String: AppKitStagedFile] = [:]
    private var order: [String] = []
    private let notice = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private var height: CGFloat = 0
    override var isFlipped: Bool { true }
    init(files: @escaping () -> [NativeStagedFile]) {
        self.files = files; super.init(frame: .zero); addSubview(notice)
        setAccessibilityRole(.group); setAccessibilityLabel("Files to send")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop(); tiles.values.forEach { $0.stop() } }
    }
    func render() {
        let files = files(); order = files.map(\.id)
        for id in Array(tiles.keys) where !order.contains(id) { tiles.removeValue(forKey: id)?.removeFromSuperview() }
        for file in files {
            let tile = tiles[file.id] ?? AppKitStagedFile(file)
            if tiles[file.id] == nil { tiles[file.id] = tile; addSubview(tile) }
            tile.update(file)
        }
        notice.stringValue = stagedFilesNotice(files); notice.isHidden = notice.stringValue.isEmpty
        notice.setAccessibilityIdentifier(files.contains { $0.status == AttachmentMetadata.refusalMessage } ? "attachment-metadata-refusal" : "attachment-upload-notice")
        needsLayout = true; invalidateIntrinsicContentSize()
    }
    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: measured(bounds.width)) }
    private func measured(_ width: CGFloat) -> CGFloat {
        guard !order.isEmpty else { return 0 }
        let columns = max(1, Int((width + 8) / 112)), side = min(104, max(0, width))
        notice.preferredMaxLayoutWidth = max(1, width)
        return (notice.isHidden ? 0 : ceil(notice.fittingSize.height) + 8) + CGFloat((order.count + columns - 1) / columns) * (side + 8) - 8
    }
    override func layout() {
        super.layout(); let width = bounds.width
        let columns = max(1, Int((width + 8) / 112)), side = min(104, max(0, width))
        notice.preferredMaxLayoutWidth = max(1, width)
        let top = notice.isHidden ? 0 : ceil(notice.fittingSize.height) + 8
        notice.frame = NSRect(x: 0, y: 0, width: width, height: max(0, top - 8))
        for (index, id) in order.enumerated() { tiles[id]?.frame = NSRect(x: CGFloat(index % columns) * (side + 8), y: top + CGFloat(index / columns) * (side + 8), width: side, height: side) }
        let next = measured(width)
        if next != height { height = next; invalidateIntrinsicContentSize(); heightChanged?() }
    }
}

@MainActor private final class AppKitStagedFile: NSView {
    private var file: NativeStagedFile
    private let thumbnail = StagedThumbnail()
    private let waveform = NativeVoiceWaveform(playedColor: AppKitKit.text.cgColor, unplayedColor: AppKitKit.muted.withAlphaComponent(0.45).cgColor)
    private var preview: AppKitKitButton!
    private var remove: AppKitKitButton!
    private var edit: AppKitKitButton!
    private var retry: AppKitKitButton!
    private let time = AppKitKit.label("", size: 11, color: AppKitKit.muted)
    private let progress = CAShapeLayer()
    override var isFlipped: Bool { true }
    init(_ file: NativeStagedFile) {
        self.file = file; super.init(frame: .zero); wantsLayer = true
        layer?.backgroundColor = AppKitKit.surface.cgColor; layer?.cornerRadius = 8; layer?.borderWidth = 1
        layer?.addSublayer(thumbnail.layer)
        layer?.addSublayer(waveform.layer)
        preview = AppKitKitButton("", kind: .plain) { [weak self] in
            guard let self else { return }; if let voice = self.file.voice { VoiceMessagePlayer.shared.toggle(voice, chat: nil) } else { self.file.preview() }
        }
        remove = corner("xmark") { [weak self] in self?.file.remove() }
        edit = corner("pencil") { [weak self] in guard let self else { return }; if self.file.editable { self.file.preview() } else { self.showMenu() } }
        retry = corner("arrow.clockwise") { [weak self] in self?.file.retry?() }
        [preview!, time, remove!, edit!, retry!].forEach { $0.translatesAutoresizingMaskIntoConstraints = true; addSubview($0) }
        progress.fillColor = nil; progress.strokeColor = AppKitKit.terracottaBright.cgColor; progress.lineWidth = 2
        remove.wantsLayer = true; remove.layer?.addSublayer(progress); update(file)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { stop() }
    func stop() { if let voice = file.voice, VoiceMessagePlayer.shared.isCurrent(voice) { VoiceMessagePlayer.shared.stop() } }
    private func corner(_ icon: String, action: @escaping () -> Void) -> AppKitKitButton {
        let button = AppKitKitButton("", kind: .plain, action: action)
        button.image = NSImage(systemSymbolName: icon, accessibilityDescription: nil); button.imagePosition = .imageOnly
        return button
    }
    func update(_ file: NativeStagedFile) {
        self.file = file; thumbnail.update(file)
        let playing = file.voice.map { VoiceMessagePlayer.shared.isPlaying($0) } ?? false
        preview.title = file.voice != nil ? (playing ? "Ⅱ" : "▶") : file.spoiler ? "Spoiler" : (file.kind == .file ? "File" : file.kind == .audio ? "Audio" : "")
        preview.setAccessibilityLabel(file.voice != nil ? (playing ? "Pause voice message" : "Play voice message") : "Preview \(file.safeName)")
        preview.setAccessibilityValue(file.accessibilityStatus); preview.toolTip = file.safeName
        remove.setAccessibilityLabel("Remove \(file.safeName)"); remove.setAccessibilityValue(file.accessibilityStatus)
        remove.setAccessibilityIdentifier(file.voice == nil ? "staged-file-remove" : "voice-draft-remove")
        edit.setAccessibilityLabel("Edit \(file.safeName)"); edit.setAccessibilityIdentifier("staged-file-edit")
        retry.setAccessibilityLabel("Retry uploading \(file.safeName)"); retry.setAccessibilityIdentifier("staged-file-retry")
        remove.isHidden = !file.removable; edit.isHidden = !file.removable || file.voice != nil; retry.isHidden = file.retry == nil
        time.stringValue = file.voice.map { file.failed ? file.status : file.busy ?? VoiceDuration.playerLabel(durationMs: $0.durationMs, elapsedMs: VoiceMessagePlayer.shared.elapsed(for: $0)) } ?? ""
        time.isHidden = file.voice == nil
        layer?.borderColor = (file.failed ? AppKitKit.error : AppKitKit.border).cgColor
        stagedProgress(progress, file: file)
        setAccessibilityIdentifier(file.voice == nil ? "staged-file-\(file.id)" : "voice-draft")
        needsLayout = true
    }
    override func layout() {
        super.layout(); thumbnail.layer.frame = bounds
        preview.frame = file.voice == nil ? bounds : NSRect(x: 6, y: 4, width: 36, height: 36)
        waveform.update(file.voice, frame: CGRect(x: 8, y: bounds.height - 48, width: max(0, bounds.width - 16), height: 18))
        remove.frame = NSRect(x: max(0, bounds.width - 28), y: 0, width: 28, height: 28)
        edit.frame = NSRect(x: 0, y: max(0, bounds.height - 28), width: 28, height: 28)
        retry.frame = NSRect(x: bounds.midX - 17, y: bounds.midY - 17, width: 34, height: 34)
        time.frame = NSRect(x: 6, y: bounds.height - 24, width: max(0, bounds.width - 12), height: 20)
        progress.frame = remove.bounds; progress.path = CGPath(ellipseIn: remove.bounds.insetBy(dx: 4, dy: 4), transform: nil)
    }
    override func rightMouseDown(with event: NSEvent) { showMenu() }
    private func showMenu() {
        guard file.removable else { return }; let menu = NSMenu()
        if file.editable { let item = NSMenuItem(title: "Edit", action: #selector(editFile), keyEquivalent: ""); item.target = self; menu.addItem(item) }
        if file.toggleSpoiler != nil, file.voice == nil { let item = NSMenuItem(title: file.spoiler ? "Remove spoiler" : "Mark as spoiler", action: #selector(spoiler), keyEquivalent: ""); item.target = self; menu.addItem(item) }
        let item = NSMenuItem(title: "Remove attachment", action: #selector(removeFile), keyEquivalent: ""); item.target = self; item.caperRole = .destructive; menu.addItem(item)
        CaperMenu.show(menu, below: self)
    }
    @objc private func editFile() { if file.removable { file.preview() } }
    @objc private func spoiler() { if file.removable { file.toggleSpoiler?() } }
    @objc private func removeFile() { if file.removable { file.remove() } }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitStagedFiles: UIView {
    var heightChanged: (() -> Void)?
    private let files: () -> [NativeStagedFile]
    private var tiles: [String: UIKitStagedFile] = [:]
    private var order: [String] = []
    private let notice = UIKitMessageDialog.label("", error: true)
    private var height: CGFloat = 0
    private var active = false, generation = 0
    init(files: @escaping () -> [NativeStagedFile]) { self.files = files; super.init(frame: .zero); addSubview(notice); accessibilityLabel = "Files to send" }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func didMoveToWindow() { super.didMoveToWindow(); active = window != nil; generation += 1; if active { observe(generation) } else { tiles.values.forEach { $0.stop() } } }
    private func observe(_ token: Int) { guard active, token == generation else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } } }
    func render() {
        let files = files(); order = files.map(\.id)
        for id in Array(tiles.keys) where !order.contains(id) { tiles.removeValue(forKey: id)?.removeFromSuperview() }
        for file in files {
            let tile = tiles[file.id] ?? UIKitStagedFile(file)
            if tiles[file.id] == nil { tiles[file.id] = tile; addSubview(tile) }; tile.update(file)
        }
        notice.text = stagedFilesNotice(files); notice.isHidden = notice.text?.isEmpty != false
        notice.accessibilityIdentifier = files.contains { $0.status == AttachmentMetadata.refusalMessage } ? "attachment-metadata-refusal" : "attachment-upload-notice"
        setNeedsLayout(); invalidateIntrinsicContentSize()
    }
    private func metrics(_ width: CGFloat) -> (columns: Int, side: CGFloat, top: CGFloat) {
        let columns = min(4, max(1, Int((min(width, 440) + 8) / 98)))
        return (columns, min(104, max(0, (width - CGFloat(columns - 1) * 8) / CGFloat(columns))),
            notice.isHidden ? 0 : ceil(notice.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height) + 8)
    }
    override func sizeThatFits(_ size: CGSize) -> CGSize {
        let m = metrics(size.width)
        return CGSize(width: size.width, height: order.isEmpty ? 0 : m.top + CGFloat((order.count + m.columns - 1) / m.columns) * (m.side + 8) - 8)
    }
    override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: sizeThatFits(bounds.size).height) }
    override func layoutSubviews() {
        super.layoutSubviews(); let m = metrics(bounds.width)
        notice.frame = CGRect(x: 0, y: 0, width: bounds.width, height: max(0, m.top - 8))
        for (index, id) in order.enumerated() { tiles[id]?.frame = CGRect(x: CGFloat(index % m.columns) * (m.side + 8), y: m.top + CGFloat(index / m.columns) * (m.side + 8), width: m.side, height: m.side) }
        let next = sizeThatFits(bounds.size).height
        if next != height { height = next; invalidateIntrinsicContentSize(); heightChanged?() }
    }
}

@MainActor private final class UIKitStagedFile: UIView, UIContextMenuInteractionDelegate {
    private var file: NativeStagedFile
    private let thumbnail = StagedThumbnail()
    private let waveform = NativeVoiceWaveform(playedColor: UIColor(CaperTheme.text).cgColor, unplayedColor: UIColor(CaperTheme.muted).withAlphaComponent(0.45).cgColor)
    private let preview = UIButton(type: .custom), remove = UIButton(type: .system), edit = UIButton(type: .system), retry = UIButton(type: .system)
    private let time = UIKitMessageDialog.label("", size: 11)
    private let progress = CAShapeLayer()
    init(_ file: NativeStagedFile) {
        self.file = file; super.init(frame: .zero); backgroundColor = UIColor(CaperTheme.surface); layer.cornerRadius = 8; layer.borderWidth = 1
        layer.addSublayer(thumbnail.layer)
        layer.addSublayer(waveform.layer)
        [preview, time, remove, edit, retry].forEach(addSubview)
        preview.addAction(UIAction { [weak self] _ in guard let self else { return }; if let voice = self.file.voice { VoiceMessagePlayer.shared.toggle(voice, chat: nil) } else { self.file.preview() } }, for: .touchUpInside)
        remove.setImage(UIImage(systemName: "xmark"), for: .normal); edit.setImage(UIImage(systemName: "pencil"), for: .normal); retry.setImage(UIImage(systemName: "arrow.clockwise"), for: .normal)
        remove.addAction(UIAction { [weak self] _ in self?.file.remove() }, for: .touchUpInside)
        edit.addAction(UIAction { [weak self] _ in if self?.file.editable == true { self?.file.preview() } }, for: .touchUpInside)
        retry.addAction(UIAction { [weak self] _ in self?.file.retry?() }, for: .touchUpInside)
        for button in [remove, edit, retry] { button.tintColor = UIColor(CaperTheme.text) }
        progress.fillColor = nil; progress.strokeColor = UIColor(CaperTheme.terracottaBright).cgColor; progress.lineWidth = 2; remove.layer.addSublayer(progress)
        preview.addInteraction(UIContextMenuInteraction(delegate: self)); update(file)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { stop() }
    func stop() { if let voice = file.voice, VoiceMessagePlayer.shared.isCurrent(voice) { VoiceMessagePlayer.shared.stop() } }
    func update(_ file: NativeStagedFile) {
        self.file = file; thumbnail.update(file)
        let playing = file.voice.map { VoiceMessagePlayer.shared.isPlaying($0) } ?? false
        preview.setTitle(file.voice != nil ? nil : file.spoiler ? "Spoiler" : file.kind == .file ? "File" : file.kind == .audio ? "Audio" : nil, for: .normal)
        preview.setImage(file.voice == nil ? nil : UIImage(systemName: playing ? "pause.fill" : "play.fill"), for: .normal)
        preview.accessibilityLabel = file.voice != nil ? (playing ? "Pause voice message" : "Play voice message") : "Preview \(file.safeName)"
        preview.accessibilityValue = file.accessibilityStatus
        remove.accessibilityLabel = "Remove \(file.safeName)"; remove.accessibilityValue = file.accessibilityStatus
        remove.accessibilityIdentifier = file.voice == nil ? "staged-file-remove" : "voice-draft-remove"
        edit.accessibilityLabel = "Edit \(file.safeName)"; edit.accessibilityIdentifier = "staged-file-edit"
        retry.accessibilityLabel = "Retry uploading \(file.safeName)"; retry.accessibilityIdentifier = "staged-file-retry"
        remove.isHidden = !file.removable; edit.isHidden = !file.removable || file.voice != nil; retry.isHidden = file.retry == nil
        edit.showsMenuAsPrimaryAction = !file.editable; edit.menu = file.editable ? nil : menu()
        time.text = file.voice.map { file.failed ? file.status : file.busy ?? VoiceDuration.playerLabel(durationMs: $0.durationMs, elapsedMs: VoiceMessagePlayer.shared.elapsed(for: $0)) }
        time.isHidden = file.voice == nil
        layer.borderColor = UIColor(file.failed ? CaperTheme.error : CaperTheme.border).cgColor
        stagedProgress(progress, file: file)
        accessibilityIdentifier = file.voice == nil ? "staged-file-\(file.id)" : "voice-draft"; setNeedsLayout()
    }
    override func layoutSubviews() {
        super.layoutSubviews(); thumbnail.layer.frame = bounds
        preview.frame = file.voice == nil ? bounds : CGRect(x: 0, y: 0, width: 44, height: 44)
        waveform.update(file.voice, frame: CGRect(x: 8, y: bounds.height - 48, width: max(0, bounds.width - 16), height: 18))
        remove.frame = CGRect(x: max(0, bounds.width - 44), y: 0, width: 44, height: 44)
        edit.frame = CGRect(x: 0, y: max(0, bounds.height - 44), width: 44, height: 44)
        retry.frame = CGRect(x: bounds.midX - 22, y: bounds.midY - 22, width: 44, height: 44)
        time.frame = CGRect(x: 6, y: bounds.height - 24, width: max(0, bounds.width - 12), height: 20)
        progress.frame = remove.bounds; progress.path = CGPath(ellipseIn: remove.bounds.insetBy(dx: 11, dy: 11), transform: nil)
    }
    private func menu() -> UIMenu {
        guard file.removable else { return UIMenu() }; var actions: [UIAction] = []
        if file.editable { actions.append(UIAction(title: "Edit") { [weak self] _ in if self?.file.removable == true { self?.file.preview() } }) }
        if file.toggleSpoiler != nil, file.voice == nil { actions.append(UIAction(title: file.spoiler ? "Remove spoiler" : "Mark as spoiler") { [weak self] _ in if self?.file.removable == true { self?.file.toggleSpoiler?() } }) }
        actions.append(UIAction(title: "Remove attachment", attributes: .destructive) { [weak self] _ in if self?.file.removable == true { self?.file.remove() } })
        return UIMenu(children: actions)
    }
    func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard file.removable else { return nil }; return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { [weak self] _ in self?.menu() }
    }
}
#endif

@MainActor private func stagedFilesNotice(_ files: [NativeStagedFile]) -> String {
    let failures = files.filter(\.failed).count
    var lines = failures > 0 ? ["\(failures) file\(failures == 1 ? "" : "s") failed to upload. Send will retry \(failures == 1 ? "it" : "them"), or you can remove \(failures == 1 ? "it" : "them")."] : []
    lines += files.filter { $0.status == AttachmentMetadata.refusalMessage }.map { "\($0.safeName): \(AttachmentMetadata.refusalMessage)" }
    return lines.joined(separator: "\n")
}
