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

/// The tile's ✕ and Edit faces sit on pictures, so they are dark with a chalk
/// glyph in every theme, not themed.
@MainActor private enum StagedFace {
    static let blackout = CGColor(red: 9 / 255, green: 10 / 255, blue: 12 / 255, alpha: 0.72)
    static let chalk = CGColor(red: 244 / 255, green: 244 / 255, blue: 246 / 255, alpha: 1)
    static let rim = CGColor(red: 1, green: 1, blue: 1, alpha: 0.2)
    static let shadow = CGColor(red: 0, green: 0, blue: 0, alpha: 0.35)
}

/// While a file prepares or uploads: a scrim over the tile and a centred ring
/// filling clockwise from 12 o'clock (a spinning quarter while the fraction is
/// unknown). Decoration only; the buttons carry the status text.
@MainActor private final class StagedProgress {
    let layer = CALayer()
    private let track = CAShapeLayer(), fill = CAShapeLayer()
    init() {
        layer.backgroundColor = CGColor(red: 9 / 255, green: 10 / 255, blue: 12 / 255, alpha: 0.45); layer.cornerRadius = 8
        for ring in [track, fill] { ring.fillColor = nil; ring.lineWidth = 3; ring.lineCap = .round; layer.addSublayer(ring) }
        track.strokeColor = CGColor(red: 1, green: 1, blue: 1, alpha: 0.25); fill.strokeColor = StagedFace.chalk
    }
    func update(busy: Bool, fraction: Double?) {
        layer.isHidden = !busy
        // Nothing sent yet reads as waiting, so the ring turns instead of sitting empty.
        let fraction = fraction.flatMap { $0.isFinite && $0 > 0 ? min(1, $0) : nil }
        fill.strokeEnd = fraction ?? 0.25
        let spinning = busy && fraction == nil && !CaperAccessibility.reduceMotion
        if spinning, fill.animation(forKey: "busy") == nil {
            let animation = CABasicAnimation(keyPath: "transform.rotation.z")
            animation.fromValue = 0; animation.toValue = Double.pi * 2; animation.duration = 1; animation.repeatCount = .infinity
            fill.add(animation, forKey: "busy")
        } else if !spinning { fill.removeAnimation(forKey: "busy") }
    }
    func layout(_ bounds: CGRect) {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        layer.frame = bounds
        // Angles grow clockwise on the tile's y-down layers, so this starts at 12 o'clock.
        let path = CGMutablePath()
        path.addArc(center: CGPoint(x: 14, y: 14), radius: 12.5, startAngle: -.pi / 2, endAngle: .pi * 1.5, clockwise: false)
        for ring in [track, fill] { ring.frame = CGRect(x: bounds.midX - 14, y: bounds.midY - 14, width: 28, height: 28); ring.path = path }
    }
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
    private var remove: AppKitStagedFace!
    private var edit: AppKitStagedFace!
    private var retry: AppKitKitButton!
    private let time = AppKitKit.label("", size: 11, color: AppKitKit.muted)
    private let progress = StagedProgress()
    override var isFlipped: Bool { true }
    init(_ file: NativeStagedFile) {
        self.file = file; super.init(frame: .zero); wantsLayer = true
        layer?.backgroundColor = AppKitKit.surface.cgColor; layer?.cornerRadius = 8; layer?.borderWidth = 1
        layer?.addSublayer(thumbnail.layer)
        layer?.addSublayer(waveform.layer)
        layer?.addSublayer(progress.layer)
        preview = AppKitKitButton("", kind: .plain) { [weak self] in
            guard let self else { return }; if let voice = self.file.voice { VoiceMessagePlayer.shared.toggle(voice, chat: nil) } else { self.file.preview() }
        }
        remove = AppKitStagedFace(symbol: "xmark") { [weak self] in self?.file.remove() }
        edit = AppKitStagedFace(symbol: "pencil", label: "Edit") { [weak self] in guard let self else { return }; if self.file.editable { self.file.preview() } else { self.showMenu() } }
        retry = AppKitKitButton("", kind: .plain) { [weak self] in self?.file.retry?() }
        retry.image = NSImage(systemSymbolName: "arrow.clockwise", accessibilityDescription: nil); retry.imagePosition = .imageOnly
        [preview!, time, remove!, edit!, retry!].forEach { $0.translatesAutoresizingMaskIntoConstraints = true; addSubview($0) }
        update(file)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { stop() }
    func stop() { if let voice = file.voice, VoiceMessagePlayer.shared.isCurrent(voice) { VoiceMessagePlayer.shared.stop() } }
    func update(_ file: NativeStagedFile) {
        self.file = file; thumbnail.update(file)
        let playing = file.voice.map { VoiceMessagePlayer.shared.isPlaying($0) } ?? false
        // While busy the progress ring owns the centre.
        preview.title = file.voice != nil ? (playing ? "Ⅱ" : "▶") : file.busy != nil ? "" : file.spoiler ? "Spoiler" : (file.kind == .file ? "File" : file.kind == .audio ? "Audio" : "")
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
        // Voice chips keep their own "Uploading…" line instead of a scrim over the waveform.
        progress.update(busy: file.busy != nil && !file.failed && file.voice == nil, fraction: file.progress)
        setAccessibilityIdentifier(file.voice == nil ? "staged-file-\(file.id)" : "voice-draft")
        needsLayout = true
    }
    override func layout() {
        super.layout(); thumbnail.layer.frame = bounds
        preview.frame = file.voice == nil ? bounds : NSRect(x: 6, y: 4, width: 36, height: 36)
        waveform.update(file.voice, frame: CGRect(x: 8, y: bounds.height - 48, width: max(0, bounds.width - 16), height: 18))
        // Pointer targets hug the corners; each face is drawn 6pt inside the tile's edges.
        remove.frame = NSRect(x: max(0, bounds.width - 36), y: 0, width: 36, height: 36)
        edit.frame = NSRect(x: 0, y: max(0, bounds.height - 36), width: edit.targetWidth, height: 36)
        retry.frame = NSRect(x: bounds.midX - 17, y: bounds.midY - 17, width: 34, height: 34)
        time.frame = NSRect(x: 6, y: bounds.height - 24, width: max(0, bounds.width - 12), height: 20)
        progress.layout(bounds)
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

/// A ✕ (top-right) or labelled Edit pill (bottom-left) drawn 6pt inside the
/// tile's corner; the button itself is the larger pointer target there.
@MainActor private final class AppKitStagedFace: NSButton {
    private let glyph: NSImage?, label: NSAttributedString?, handler: () -> Void
    private var hovered = false, pressed = false, tracking: NSTrackingArea?
    override var isFlipped: Bool { true }
    init(symbol: String, label: String? = nil, action: @escaping () -> Void) {
        let chalk = NSColor(cgColor: StagedFace.chalk) ?? .white
        glyph = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?.withSymbolConfiguration(
            NSImage.SymbolConfiguration(pointSize: 12, weight: label == nil ? .bold : .semibold).applying(NSImage.SymbolConfiguration(paletteColors: [chalk])))
        self.label = label.map { NSAttributedString(string: $0, attributes: [.font: AppKitKit.font(12, weight: .semibold), .foregroundColor: chalk]) }
        handler = action; super.init(frame: .zero)
        title = ""; isBordered = false; imagePosition = .noImage; focusRingType = .none
        target = self; self.action = #selector(fire)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func fire() { handler() }
    /// 24 high; the pill is 7 leading + pencil + 4 + "Edit" + 9 trailing.
    private var faceWidth: CGFloat { label.map { 7 + (glyph?.size.width ?? 12) + 4 + ceil($0.size().width) + 9 } ?? 24 }
    var targetWidth: CGFloat { faceWidth + 12 }
    private var face: NSRect {
        label == nil ? NSRect(x: bounds.width - 30, y: 6, width: 24, height: 24) : NSRect(x: 6, y: bounds.height - 30, width: faceWidth, height: 24)
    }
    private var focused: Bool { window?.firstResponder === self && window?.isKeyWindow == true }
    override func draw(_ dirtyRect: NSRect) {
        let face = face, path = NSBezierPath(roundedRect: face, xRadius: 12, yRadius: 12)
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow(); shadow.shadowOffset = NSSize(width: 0, height: -1); shadow.shadowBlurRadius = 3
        shadow.shadowColor = NSColor(cgColor: StagedFace.shadow); shadow.set()
        (NSColor(cgColor: StagedFace.blackout) ?? .black).setFill(); path.fill()
        NSGraphicsContext.restoreGraphicsState()
        if hovered || pressed { NSColor.white.withAlphaComponent(pressed ? 0.2 : 0.12).setFill(); path.fill() }
        (NSColor(cgColor: StagedFace.rim) ?? .white).setStroke()
        let rim = NSBezierPath(roundedRect: face.insetBy(dx: 0.5, dy: 0.5), xRadius: 11.5, yRadius: 11.5); rim.lineWidth = 1; rim.stroke()
        if focused && isEnabled {
            AppKitKit.terracottaBright.setStroke()
            let ring = NSBezierPath(roundedRect: face.insetBy(dx: -2, dy: -2), xRadius: 14, yRadius: 14); ring.lineWidth = 2; ring.stroke()
        }
        if let glyph {
            let x = label == nil ? face.midX - glyph.size.width / 2 : face.minX + 7
            glyph.draw(in: NSRect(x: x, y: face.midY - glyph.size.height / 2, width: glyph.size.width, height: glyph.size.height),
                       from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        }
        if let label {
            let size = label.size()
            label.draw(at: NSPoint(x: face.minX + 7 + (glyph?.size.width ?? 12) + 4, y: face.midY - size.height / 2))
        }
    }
    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func keyDown(with event: NSEvent) {
        guard event.keyCode == 48 else { super.keyDown(with: event); return }
        KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self)
    }
    override func becomeFirstResponder() -> Bool { needsDisplay = true; return super.becomeFirstResponder() }
    override func resignFirstResponder() -> Bool { needsDisplay = true; return super.resignFirstResponder() }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { if AppKitKit.windowActive(window) { hovered = true; needsDisplay = true } }
    override func mouseExited(with event: NSEvent) { hovered = false; needsDisplay = true }
    override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        pressed = true; needsDisplay = true
        super.mouseDown(with: event)
        pressed = false; needsDisplay = true
    }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
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
    private let preview = UIButton(type: .custom), remove = UIButton(type: .custom), edit = UIButton(type: .custom), retry = UIButton(type: .system)
    private let time = UIKitMessageDialog.label("", size: 11)
    private let progress = StagedProgress()
    init(_ file: NativeStagedFile) {
        self.file = file; super.init(frame: .zero); backgroundColor = UIColor(CaperTheme.surface); layer.cornerRadius = 8; layer.borderWidth = 1
        layer.addSublayer(thumbnail.layer)
        layer.addSublayer(waveform.layer)
        layer.addSublayer(progress.layer)
        [preview, time, remove, edit, retry].forEach(addSubview)
        preview.addAction(UIAction { [weak self] _ in guard let self else { return }; if let voice = self.file.voice { VoiceMessagePlayer.shared.toggle(voice, chat: nil) } else { self.file.preview() } }, for: .touchUpInside)
        retry.setImage(UIImage(systemName: "arrow.clockwise"), for: .normal)
        remove.addAction(UIAction { [weak self] _ in self?.file.remove() }, for: .touchUpInside)
        edit.addAction(UIAction { [weak self] _ in if self?.file.editable == true { self?.file.preview() } }, for: .touchUpInside)
        retry.addAction(UIAction { [weak self] _ in self?.file.retry?() }, for: .touchUpInside)
        retry.tintColor = UIColor(CaperTheme.text)
        // Remove and Edit sit on small dark faces so they read over pale pictures. Face, glyph and
        // label are one image, so nothing can draw between them; the shadow follows the face.
        for button in [remove, edit] {
            button.setImage(Self.face(edit: button === edit, wash: 0), for: .normal); button.setImage(Self.face(edit: button === edit, wash: 0.2), for: .highlighted)
            button.layer.shadowColor = UIColor.black.cgColor; button.layer.shadowOpacity = 0.35
            button.layer.shadowOffset = CGSize(width: 0, height: 1); button.layer.shadowRadius = 1.5
            button.addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hovered(_:))))
        }
        preview.addInteraction(UIContextMenuInteraction(delegate: self)); update(file)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { stop() }
    func stop() { if let voice = file.voice, VoiceMessagePlayer.shared.isCurrent(voice) { VoiceMessagePlayer.shared.stop() } }
    func update(_ file: NativeStagedFile) {
        self.file = file; thumbnail.update(file)
        let playing = file.voice.map { VoiceMessagePlayer.shared.isPlaying($0) } ?? false
        // While busy the progress ring owns the centre.
        preview.setTitle(file.voice != nil || file.busy != nil ? nil : file.spoiler ? "Spoiler" : file.kind == .file ? "File" : file.kind == .audio ? "Audio" : nil, for: .normal)
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
        // Voice chips keep their own "Uploading…" line instead of a scrim over the waveform.
        progress.update(busy: file.busy != nil && !file.failed && file.voice == nil, fraction: file.progress)
        accessibilityIdentifier = file.voice == nil ? "staged-file-\(file.id)" : "voice-draft"; setNeedsLayout()
    }
    override func layoutSubviews() {
        super.layoutSubviews(); thumbnail.layer.frame = bounds
        preview.frame = file.voice == nil ? bounds : CGRect(x: 0, y: 0, width: 44, height: 44)
        waveform.update(file.voice, frame: CGRect(x: 8, y: bounds.height - 48, width: max(0, bounds.width - 16), height: 18))
        // 44pt targets anchored to their corners; each image draws its face 6pt inside the tile's edges.
        let width = edit.image(for: .normal)?.size.width ?? 44
        remove.frame = CGRect(x: max(0, bounds.width - 44), y: 0, width: 44, height: 44)
        edit.frame = CGRect(x: 0, y: max(0, bounds.height - 44), width: width, height: 44)
        for (button, face) in [(remove, CGRect(x: 14, y: 6, width: 24, height: 24)), (edit, CGRect(x: 6, y: 14, width: width - 12, height: 24))] {
            button.layer.shadowPath = UIBezierPath(roundedRect: face, cornerRadius: 12).cgPath
            button.focusEffect = UIFocusHaloEffect(roundedRect: face, cornerRadius: 12, curve: .continuous)
        }
        retry.frame = CGRect(x: bounds.midX - 22, y: bounds.midY - 22, width: 44, height: 44)
        time.frame = CGRect(x: 6, y: bounds.height - 24, width: max(0, bounds.width - 12), height: 20)
        progress.layout(bounds)
    }
    @objc private func hovered(_ recognizer: UIHoverGestureRecognizer) {
        guard let button = recognizer.view as? UIButton else { return }
        switch recognizer.state {
        case .began: button.setImage(Self.face(edit: button === edit, wash: 0.12), for: .normal)
        case .ended, .cancelled, .failed: button.setImage(Self.face(edit: button === edit, wash: 0), for: .normal)
        default: break
        }
    }
    /// The ✕ (a 24pt circle in the top-right) or the Edit pill (pencil and "Edit", bottom-left),
    /// Blackout @ 72% with a white @ 20% rim, drawn as it is (not tinted) in a 44pt-high image;
    /// `wash` lightens it for hover and press.
    private static func face(edit: Bool, wash: CGFloat) -> UIImage {
        let chalk = UIColor(cgColor: StagedFace.chalk)
        let glyph = UIImage(systemName: edit ? "pencil" : "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: edit ? .semibold : .bold))?
            .withTintColor(chalk, renderingMode: .alwaysOriginal)
        let label = edit ? NSAttributedString(string: "Edit", attributes: [.font: UIFont(name: "Figtree-Bold", size: 12) ?? UIFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: chalk]) : nil
        let glyphWidth = glyph?.size.width ?? 12
        let faceWidth = label.map { 7 + glyphWidth + 4 + ceil($0.size().width) + 9 } ?? 24
        let face = edit ? CGRect(x: 6, y: 14, width: faceWidth, height: 24) : CGRect(x: 14, y: 6, width: 24, height: 24)
        return UIGraphicsImageRenderer(size: CGSize(width: edit ? faceWidth + 12 : 44, height: 44)).image { _ in
            let path = UIBezierPath(roundedRect: face, cornerRadius: 12)
            UIColor(cgColor: StagedFace.blackout).setFill(); path.fill()
            if wash > 0 { UIColor.white.withAlphaComponent(wash).setFill(); path.fill() }
            let rim = UIBezierPath(roundedRect: face.insetBy(dx: 0.5, dy: 0.5), cornerRadius: 11.5)
            UIColor(cgColor: StagedFace.rim).setStroke(); rim.lineWidth = 1; rim.stroke()
            if let glyph { glyph.draw(at: CGPoint(x: edit ? face.minX + 7 : face.midX - glyph.size.width / 2, y: face.midY - glyph.size.height / 2)) }
            if let label { label.draw(at: CGPoint(x: face.minX + 7 + glyphWidth + 4, y: face.midY - label.size().height / 2)) }
        }.withRenderingMode(.alwaysOriginal)
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
