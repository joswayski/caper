#if os(macOS)
import AppKit
import AVKit
import SwiftUI

struct NativeAttachmentEditorBridge: NSViewControllerRepresentable {
    let request: AttachmentEditRequest
    let close: () -> Void
    func makeNSViewController(context: Context) -> AppKitAttachmentEditor { AppKitAttachmentEditor(request: request, close: close) }
    func updateNSViewController(_ controller: AppKitAttachmentEditor, context: Context) {}
}

@MainActor final class AppKitAttachmentEditor: NSViewController {
    private let state: NativeAttachmentEditing
    private let observation = AppKitObservation()
    private var task: Task<Void, Never>?
    private var canvas: AppKitAttachmentCanvas!
    private let status = AppKitKit.label("Loading…", size: 13, color: AppKitKit.muted)
    private var undo: AppKitKitButton!
    private var redo: AppKitKitButton!
    private var reset: AppKitKitButton!
    private var done: AppKitKitButton!
    private var rotate: AppKitKitButton!
    private var eraser: AppKitKitButton!
    private var spoiler: AppKitKitButton?
    private var tools: [AttachmentEditTool: AppKitKitButton] = [:]
    private let options = NSView()
    private var cropOptions: NSStackView!
    private var markOptions: NSStackView!
    private let size = AppKitEditSizeSlider(value: 6, minValue: 2, maxValue: 40, target: nil, action: nil)
    private let cropSize = NSSlider(value: 1, minValue: 0.1, maxValue: 1, target: nil, action: nil)
    private let sizeLabel = AppKitKit.label("", size: 12)
    private lazy var sizePreview = AppKitAttachmentSizePreview(state: state)
    private let color = NSColorWell()
    private var swatches: [AppKitKitButton] = []

    init(request: AttachmentEditRequest, close: @escaping () -> Void) {
        state = NativeAttachmentEditing(request: request, close: close)
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NSView(); view.wantsLayer = true; view.layer?.backgroundColor = NSColor(CaperTheme.blackout).cgColor
        view.setAccessibilityElement(true); view.setAccessibilityRole(.group); view.setAccessibilityIdentifier("attachment-editor")
        view.setAccessibilityLabel(state.request.name)
        let close = AppKitKitButton("Close", kind: .plain) { [state] in state.close() }
        close.setAccessibilityIdentifier("attachment-editor-close")
        undo = AppKitKitButton("Undo") { [state] in state.undo() }
        redo = AppKitKitButton("Redo") { [state] in state.redo() }
        reset = AppKitKitButton("Reset") { [state] in state.reset() }
        done = AppKitKitButton(state.request.saveLabel, kind: .primary) { [state] in state.done() }
        done.setAccessibilityIdentifier("attachment-editor-done")
        let title = AppKitKit.label(state.request.name, size: 14, weight: .bold, wraps: false)
        let top = NSStackView(views: [close, title, NSView(), undo, redo, reset, done]); top.spacing = 8
        canvas = AppKitAttachmentCanvas(state: state)
        canvas.addSubview(status); status.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([status.centerXAnchor.constraint(equalTo: canvas.centerXAnchor), status.centerYAnchor.constraint(equalTo: canvas.centerYAnchor)])
        let row = NSStackView(); row.spacing = 6
        for item in state.request.row {
            let button: AppKitKitButton
            switch item {
            case .spoiler:
                button = AppKitKitButton("Spoiler") { [state] in state.request.spoiler?.toggle() }
                button.setAccessibilityIdentifier("attachment-editor-spoiler"); spoiler = button
            case .tool(let tool):
                button = AppKitKitButton(tool.title) { [state] in state.choose(tool) }
                button.setAccessibilityIdentifier("attachment-editor-\(tool.rawValue)"); tools[tool] = button
            case .remove:
                button = AppKitKitButton("Remove", kind: .destructive) { [state] in state.remove() }
                button.setAccessibilityIdentifier("attachment-editor-remove")
            }
            row.addArrangedSubview(button)
        }
        rotate = AppKitKitButton("Rotate left") { [state] in state.rotate() }
        cropSize.target = self; cropSize.action = #selector(cropChanged); cropSize.setAccessibilityLabel("Crop size")
        cropSize.widthAnchor.constraint(equalToConstant: 240).isActive = true
        cropSize.isHidden = !state.request.pictureMode
        cropOptions = NSStackView(views: [cropSize, rotate]); cropOptions.spacing = 12
        markOptions = NSStackView(); markOptions.spacing = 6
        for hex in AttachmentEdit.colors {
            let button = AppKitKitButton("●", kind: .plain) { [state] in state.chooseColor(hex) }
            button.setAccessibilityLabel(AttachmentEdit.colorNames[hex]); button.toolTip = AttachmentEdit.colorNames[hex]
            button.widthAnchor.constraint(equalToConstant: 32).isActive = true
            // Color is artwork rather than a selection tint; the ring remains terracotta.
            let image = NSImage(size: NSSize(width: 22, height: 22), flipped: false) { rect in
                NSColor(cgColor: AttachmentEditRenderer.cgColor(hex))?.setFill(); NSBezierPath(ovalIn: rect.insetBy(dx: 2, dy: 2)).fill(); return true
            }
            button.image = image; button.imagePosition = .imageOnly
            markOptions.addArrangedSubview(button); swatches.append(button)
        }
        color.target = self; color.action = #selector(colorChanged); color.setAccessibilityLabel("Custom colour")
        color.widthAnchor.constraint(equalToConstant: 36).isActive = true
        eraser = AppKitKitButton("Eraser") { [state] in state.erasing.toggle() }
        size.target = self; size.action = #selector(sizeChanged)
        size.widthAnchor.constraint(greaterThanOrEqualToConstant: 120).isActive = true
        for child in [color, eraser, size, sizeLabel, sizePreview] as [NSView] { markOptions.addArrangedSubview(child) }
        options.translatesAutoresizingMaskIntoConstraints = false
        options.heightAnchor.constraint(equalToConstant: 52).isActive = true
        for strip in [cropOptions!, markOptions!] {
            strip.translatesAutoresizingMaskIntoConstraints = false; options.addSubview(strip)
            NSLayoutConstraint.activate([strip.centerXAnchor.constraint(equalTo: options.centerXAnchor), strip.centerYAnchor.constraint(equalTo: options.centerYAnchor),
                strip.leadingAnchor.constraint(greaterThanOrEqualTo: options.leadingAnchor), strip.trailingAnchor.constraint(lessThanOrEqualTo: options.trailingAnchor)])
        }
        let hint = AppKitKit.label(state.request.pictureMode ? "Drag the square to reposition. Adjust crop size to zoom."
            : state.request.kind == .video ? "Drawings and text stay on screen for the whole video." : "", size: 12, color: AppKitKit.muted)
        hint.isHidden = hint.stringValue.isEmpty
        let bottom = AppKitKit.stack([hint, options, row], spacing: 8, alignment: .centerX)
        AppKitKit.fillWidth(options, of: bottom)
        for child in [top, canvas!, bottom] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        NSLayoutConstraint.activate([
            top.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12), top.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            top.topAnchor.constraint(equalTo: view.topAnchor, constant: 8),
            canvas.topAnchor.constraint(equalTo: top.bottomAnchor, constant: 8), canvas.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            canvas.trailingAnchor.constraint(equalTo: view.trailingAnchor), canvas.bottomAnchor.constraint(equalTo: bottom.topAnchor, constant: -8),
            bottom.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16), bottom.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            bottom.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -12),
        ])
    }
    override func viewDidAppear() {
        super.viewDidAppear(); view.window?.makeFirstResponder(canvas)
        observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
        task = Task { [state] in await state.load() }
        if !CaperAccessibility.reduceMotion {
            view.alphaValue = 0
            NSAnimationContext.runAnimationGroup { context in
                context.duration = CaperMotion.base; context.timingFunction = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1)
                view.animator().alphaValue = 1
            }
        }
    }
    override func viewDidDisappear() { super.viewDidDisappear(); task?.cancel(); observation.stop(); canvas.stop() }
    @objc private func sizeChanged() { state.sizeFromSlider(size.doubleValue, dragging: NSApp.currentEvent?.type == .leftMouseDragged) }
    @objc private func cropChanged() { state.resizeCrop(cropSize.doubleValue) }
    @objc private func colorChanged() {
        guard let value = color.color.usingColorSpace(.sRGB) else { return }
        state.chooseColor(AttachmentEdit.hex(red: value.redComponent, green: value.greenComponent, blue: value.blueComponent))
    }
    private func render() {
        let loaded = state.picture != nil
        status.isHidden = loaded; status.stringValue = state.failed ? "This file can’t be edited here." : "Loading…"
        undo.isEnabled = loaded && state.history.canUndo; redo.isEnabled = loaded && state.history.canRedo
        reset.isEnabled = loaded && state.history.current.isEdited; done.isEnabled = !state.request.pictureMode || loaded
        for (tool, button) in tools { button.isEnabled = loaded; button.showsSelection = state.tool == tool }
        spoiler?.showsSelection = state.spoilered; spoiler?.setAccessibilityValue(state.spoilered ? "On" : "Off")
        cropOptions.isHidden = state.tool != .crop; markOptions.isHidden = state.tool != .draw && state.tool != .text
        rotate.isEnabled = loaded; cropSize.isEnabled = loaded; cropSize.doubleValue = state.cropSize
        eraser.isHidden = state.tool != .draw; eraser.showsSelection = state.erasing
        let drawing = state.tool == .draw, range = drawing ? AttachmentEdit.strokeSizes : AttachmentEdit.textSizes
        size.minValue = range.lowerBound; size.maxValue = range.upperBound; size.doubleValue = drawing ? state.strokePoints : state.textPoints
        size.normal = drawing ? AttachmentEdit.initialStrokeSize : AttachmentEdit.initialTextSize
        size.setAccessibilityLabel(drawing ? state.erasing ? "Eraser size" : "Line size" : "Text size")
        sizeLabel.stringValue = "\(Int(size.doubleValue))"
        sizePreview.needsDisplay = true
        for (button, hex) in zip(swatches, AttachmentEdit.colors) { button.showsSelection = !state.erasing && state.color == hex }
        color.color = NSColor(cgColor: AttachmentEditRenderer.cgColor(state.color)) ?? .red
        // Reading drawing state subscribes to gestures without replacing the canvas or field.
        _ = state.shown; _ = state.scale; _ = state.offset; _ = state.brush
        canvas.refresh()
    }
}

@MainActor private final class AppKitAttachmentSizePreview: NSView {
    private let state: NativeAttachmentEditing
    override var isFlipped: Bool { true }
    init(state: NativeAttachmentEditing) {
        self.state = state; super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false; setAccessibilityElement(false)
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: 44), heightAnchor.constraint(equalToConstant: 44)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func draw(_ dirtyRect: NSRect) {
        if let context = NSGraphicsContext.current?.cgContext { state.paintSizePreview(in: context, size: bounds.size) }
    }
}

@MainActor private final class AppKitAttachmentCanvas: NSView, NSTextFieldDelegate {
    private let state: NativeAttachmentEditing
    private var text: NSTextField?
    private let player = AVPlayerView()
    private var playing: URL?
    private var pan: CGPoint?
    private var panOffset = CGSize.zero
    private var tracking: NSTrackingArea?
    private let blankCursor = NSCursor(image: NSImage(size: NSSize(width: 16, height: 16), flipped: false) { _ in true }, hotSpot: NSPoint(x: 8, y: 8))
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    init(state: NativeAttachmentEditing) {
        self.state = state; super.init(frame: .zero)
        wantsLayer = true; layer?.masksToBounds = true
        player.controlsStyle = .floating; addSubview(player)
        setAccessibilityElement(true); setAccessibilityRole(.group)
        setAccessibilityLabel(state.request.kind == .video ? "Video frame being edited" : "Image being edited")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func stop() { player.player?.pause(); player.player = nil; playing = nil; state.cancelGesture(); NSCursor.arrow.set() }
    func refresh() {
        needsDisplay = true; needsLayout = true
        setAccessibilityRole(state.picture != nil && state.field == nil && state.playback == nil ? .image : .group)
        setAccessibilityIdentifier(state.picture == nil ? nil : "attachment-editor-image")
        window?.invalidateCursorRects(for: self)
        if playing != state.playback {
            stop(); playing = state.playback
            if let playing { player.player = AVPlayer(url: playing); player.player?.play() }
        }
        player.isHidden = playing == nil
        if let field = state.field {
            let new = text == nil
            let text = self.text ?? NSTextField()
            if new {
                text.isBordered = true; text.drawsBackground = true; text.backgroundColor = NSColor.black.withAlphaComponent(0.7)
                text.delegate = self; text.target = self; text.action = #selector(commit); text.alignment = .center
                addSubview(text); self.text = text
            }
            if (text.currentEditor() as? NSTextView)?.hasMarkedText() != true, text.stringValue != field.value { text.stringValue = field.value }
            text.font = AppKitKit.font(field.points, weight: .bold); text.textColor = NSColor(cgColor: AttachmentEditRenderer.cgColor(field.color))
            text.setAccessibilityLabel(field.index == nil ? "New text" : "Edit text")
            if new { window?.makeFirstResponder(text) }
        } else if let text { text.removeFromSuperview(); self.text = nil; window?.makeFirstResponder(self) }
    }
    override func layout() {
        super.layout(); player.frame = state.rect(in: bounds.size)
        if let field = state.field, let layout = state.layout(in: bounds.size), let text {
            let rect = state.rect(in: bounds.size), point = layout.point(x: field.x, y: field.y, region: state.region)
            let width = max(48, AttachmentEditRenderer.textWidth(field.value.isEmpty ? "M" : field.value, size: field.points) + field.points + 8)
            text.frame = CGRect(x: rect.minX + point.x - width / 2, y: rect.minY + point.y - field.points * 0.75, width: width, height: field.points * 1.5)
        }
    }
    override func draw(_ dirtyRect: NSRect) { if let context = NSGraphicsContext.current?.cgContext { state.paint(in: context, size: bounds.size) } }
    func controlTextDidChange(_ notification: Notification) {
        if let text, (text.currentEditor() as? NSTextView)?.hasMarkedText() != true { state.setText(text.stringValue) }
    }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.cancelOperation(_:)) { state.field = nil; return true }; return false
    }
    @objc private func commit() { state.commitField() }
    override func cancelOperation(_ sender: Any?) { state.escape() }
    override func keyDown(with event: NSEvent) {
        if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers?.lowercased() == "z" {
            if event.modifierFlags.contains(.shift) { state.redo() } else { state.undo() }
        } else { super.keyDown(with: event) }
    }
    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let point = convert(event.locationInWindow, from: nil)
        if state.tool == nil {
            if event.clickCount == 2 { zoom(at: point) }
            else { pan = point; panOffset = state.offset }
        } else { state.begin(at: point, size: bounds.size) }
    }
    override func mouseDragged(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if state.tool == nil, state.scale > 1, let pan {
            state.offset = MediaViewerPolicy.clampedOffset(CGSize(width: panOffset.width + point.x - pan.x, height: panOffset.height + point.y - pan.y),
                scale: state.scale, fitted: state.rect(in: bounds.size).size, container: bounds.size)
        } else { state.drag(to: point, size: bounds.size) }
    }
    override func mouseUp(with event: NSEvent) { state.end(size: bounds.size); pan = nil }
    override func magnify(with event: NSEvent) {
        guard state.tool == nil, playing == nil else { return }
        state.scale = min(MediaViewerPolicy.maxZoom, max(1, state.scale * (1 + event.magnification)))
        state.offset = MediaViewerPolicy.clampedOffset(state.offset, scale: state.scale, fitted: state.rect(in: bounds.size).size, container: bounds.size)
    }
    private func zoom(at point: CGPoint) {
        guard playing == nil else { return }
        if state.scale > 1 { state.scale = 1; state.offset = .zero }
        else {
            state.scale = MediaViewerPolicy.doubleTapZoom
            let rect = state.rect(in: bounds.size)
            state.offset = MediaViewerPolicy.clampedOffset(MediaViewerPolicy.zoomOffset(at: CGPoint(x: point.x - rect.minX, y: point.y - rect.minY), fitted: rect.size, scale: state.scale),
                scale: state.scale, fitted: rect.size, container: bounds.size)
        }
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas(); if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseMoved(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil), rect = state.rect(in: bounds.size)
        if rect.contains(point) { state.brush.hovered(at: CGPoint(x: point.x - rect.minX, y: point.y - rect.minY)) }
        else { state.brush.hoverEnded() }
    }
    override func mouseExited(with event: NSEvent) { state.brush.hoverEnded() }
    override func resetCursorRects() {
        super.resetCursorRects()
        if state.tool == .draw { addCursorRect(state.rect(in: bounds.size).intersection(visibleRect), cursor: blankCursor) }
    }
}

private final class AppKitEditSizeSlider: NSSlider {
    var normal = AttachmentEdit.initialStrokeSize { didSet { needsDisplay = true } }
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        let fraction = (normal - minValue) / (maxValue - minValue)
        AppKitKit.muted.setFill()
        NSRect(x: 9 + max(0, bounds.width - 18) * fraction, y: 1, width: 2, height: 4).fill()
    }
}
#endif
