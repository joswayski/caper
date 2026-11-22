#if os(iOS)
import UIKit
import AVKit
import SwiftUI
import Observation

struct NativeAttachmentEditorBridge: UIViewControllerRepresentable {
    let request: AttachmentEditRequest
    let close: () -> Void
    func makeUIViewController(context: Context) -> UIKitAttachmentEditor { UIKitAttachmentEditor(request: request, close: close) }
    func updateUIViewController(_ controller: UIKitAttachmentEditor, context: Context) {}
}

@MainActor final class UIKitAttachmentEditor: UIViewController, UIColorPickerViewControllerDelegate {
    private let state: NativeAttachmentEditing
    private let player = AVPlayerViewController()
    private var playing: URL?
    private var active = false
    private var task: Task<Void, Never>?
    private var canvas: UIKitAttachmentCanvas!
    private let status = UIKitMessageDialog.label("Loading…")
    private var undo: UIButton!
    private var redo: UIButton!
    private var reset: UIButton!
    private var done: UIButton!
    private var eraser: UIButton!
    private var spoiler: UIButton?
    private var tools: [AttachmentEditTool: UIButton] = [:]
    private var cropOptions: UIStackView!
    private var markOptions: UIStackView!
    private let size = UIKitEditSizeSlider()
    private let cropSize = UISlider()
    private let sizeLabel = UIKitMessageDialog.label("")
    private lazy var sizePreview = UIKitAttachmentSizePreview(state: state)
    private var swatches: [UIButton] = []

    init(request: AttachmentEditRequest, close: @escaping () -> Void) {
        state = NativeAttachmentEditing(request: request, close: close)
        super.init(nibName: nil, bundle: nil); modalPresentationStyle = .overFullScreen
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); overrideUserInterfaceStyle = .dark
        view.backgroundColor = UIColor(CaperTheme.blackout); view.accessibilityIdentifier = "attachment-editor"
        view.accessibilityViewIsModal = true
        let close = UIKitMessageDialog.button("Close") { [state] in state.close() }
        close.accessibilityIdentifier = "attachment-editor-close"
        let title = UIKitMessageDialog.label(state.request.name, size: 14, bold: true)
        title.numberOfLines = 1; title.lineBreakMode = .byTruncatingMiddle
        let header = UIStackView(arrangedSubviews: [title, close]); header.spacing = 8
        close.setContentHuggingPriority(.required, for: .horizontal)
        undo = UIKitMessageDialog.button("Undo") { [state] in state.undo() }
        redo = UIKitMessageDialog.button("Redo") { [state] in state.redo() }
        reset = UIKitMessageDialog.button("Reset") { [state] in state.reset() }
        done = UIKitMessageDialog.button(state.request.saveLabel, primary: true) { [state] in state.done() }
        done.accessibilityIdentifier = "attachment-editor-done"
        let actions = UIStackView(arrangedSubviews: [undo, redo, reset, done]); actions.spacing = 6; actions.distribution = .fillEqually
        let top = UIStackView(arrangedSubviews: [header, actions]); top.axis = .vertical; top.spacing = 4
        addChild(player)
        canvas = UIKitAttachmentCanvas(state: state, playerView: player.view)
        player.didMove(toParent: self); player.videoGravity = .resizeAspect
        canvas.addSubview(status); status.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([status.centerXAnchor.constraint(equalTo: canvas.centerXAnchor), status.centerYAnchor.constraint(equalTo: canvas.centerYAnchor)])
        let row = UIStackView(); row.spacing = 6
        for item in state.request.row {
            let button: UIButton
            switch item {
            case .spoiler:
                button = UIKitMessageDialog.button("Spoiler") { [state] in state.request.spoiler?.toggle() }
                button.accessibilityIdentifier = "attachment-editor-spoiler"; spoiler = button
            case .tool(let tool):
                button = UIKitMessageDialog.button(tool.title) { [state] in state.choose(tool) }
                button.accessibilityIdentifier = "attachment-editor-\(tool.rawValue)"; tools[tool] = button
            case .remove:
                button = UIKitMessageDialog.button("Remove") { [state] in state.remove() }
                button.accessibilityIdentifier = "attachment-editor-remove"
            }
            row.addArrangedSubview(button)
        }
        let rotate = UIKitMessageDialog.button("Rotate left") { [state] in state.rotate() }
        cropSize.minimumValue = 0.1; cropSize.maximumValue = 1; cropSize.accessibilityLabel = "Crop size"
        cropSize.addTarget(self, action: #selector(cropChanged), for: .valueChanged)
        cropSize.isHidden = !state.request.pictureMode
        cropOptions = UIStackView(arrangedSubviews: [cropSize, rotate]); cropOptions.axis = .vertical; cropOptions.spacing = 4
        let colors = UIStackView(); colors.distribution = .fillEqually; colors.spacing = 2
        for hex in AttachmentEdit.colors {
            let button = UIButton(type: .system)
            button.setImage(UIImage(systemName: "circle.fill"), for: .normal)
            button.tintColor = UIColor(cgColor: AttachmentEditRenderer.cgColor(hex))
            button.accessibilityLabel = AttachmentEdit.colorNames[hex]
            button.addAction(UIAction { [state] _ in state.chooseColor(hex) }, for: .touchUpInside)
            button.heightAnchor.constraint(equalToConstant: 44).isActive = true
            colors.addArrangedSubview(button); swatches.append(button)
        }
        let custom = UIButton(type: .system); custom.setImage(UIImage(systemName: "paintpalette"), for: .normal)
        custom.accessibilityLabel = "Custom color"; custom.tintColor = UIColor(CaperTheme.text)
        custom.addAction(UIAction { [weak self] _ in self?.pickColor() }, for: .touchUpInside); colors.addArrangedSubview(custom)
        eraser = UIKitMessageDialog.button("Eraser") { [state] in state.erasing.toggle() }
        size.addTarget(self, action: #selector(sizeChanged), for: .valueChanged)
        let sizing = UIStackView(arrangedSubviews: [eraser, size, sizeLabel, sizePreview]); sizing.spacing = 8
        size.setContentHuggingPriority(.defaultLow, for: .horizontal)
        markOptions = UIStackView(arrangedSubviews: [colors, sizing]); markOptions.axis = .vertical; markOptions.spacing = 4
        let options = UIView(); options.heightAnchor.constraint(equalToConstant: 92).isActive = true
        for strip in [cropOptions!, markOptions!] {
            options.addSubview(strip); strip.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([strip.leadingAnchor.constraint(equalTo: options.leadingAnchor), strip.trailingAnchor.constraint(equalTo: options.trailingAnchor),
                strip.centerYAnchor.constraint(equalTo: options.centerYAnchor)])
        }
        let hint = UIKitMessageDialog.label(state.request.pictureMode ? "Drag the square to reposition. Adjust crop size to zoom."
            : state.request.kind == .video ? "Drawings and text stay on screen for the whole video." : "")
        hint.isHidden = hint.text?.isEmpty == true
        let toolScroll = UIScrollView(); toolScroll.showsHorizontalScrollIndicator = false
        row.translatesAutoresizingMaskIntoConstraints = false; toolScroll.addSubview(row)
        NSLayoutConstraint.activate([toolScroll.heightAnchor.constraint(equalToConstant: 48),
            row.leadingAnchor.constraint(equalTo: toolScroll.contentLayoutGuide.leadingAnchor), row.trailingAnchor.constraint(equalTo: toolScroll.contentLayoutGuide.trailingAnchor),
            row.topAnchor.constraint(equalTo: toolScroll.contentLayoutGuide.topAnchor), row.bottomAnchor.constraint(equalTo: toolScroll.contentLayoutGuide.bottomAnchor),
            row.heightAnchor.constraint(equalTo: toolScroll.frameLayoutGuide.heightAnchor)])
        let bottom = UIStackView(arrangedSubviews: [hint, options, toolScroll]); bottom.axis = .vertical; bottom.spacing = 6
        // Keep the canvas usable with a landscape keyboard. The contents can
        // scroll instead of forcing all three regions into the keyboard gap.
        let viewport = UIScrollView(); viewport.keyboardDismissMode = .interactive
        viewport.delaysContentTouches = false; viewport.canCancelContentTouches = false
        let content = UIView()
        viewport.translatesAutoresizingMaskIntoConstraints = false; content.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(viewport); viewport.addSubview(content)
        for child in [top, canvas!, bottom] { child.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(child) }
        let fills = content.heightAnchor.constraint(equalTo: viewport.frameLayoutGuide.heightAnchor)
        fills.priority = .defaultLow; fills.isActive = true
        NSLayoutConstraint.activate([
            viewport.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor), viewport.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            viewport.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor), viewport.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            content.leadingAnchor.constraint(equalTo: viewport.contentLayoutGuide.leadingAnchor), content.trailingAnchor.constraint(equalTo: viewport.contentLayoutGuide.trailingAnchor),
            content.topAnchor.constraint(equalTo: viewport.contentLayoutGuide.topAnchor), content.bottomAnchor.constraint(equalTo: viewport.contentLayoutGuide.bottomAnchor),
            content.widthAnchor.constraint(equalTo: viewport.frameLayoutGuide.widthAnchor), content.heightAnchor.constraint(greaterThanOrEqualTo: viewport.frameLayoutGuide.heightAnchor),
            top.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 12), top.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -12),
            top.topAnchor.constraint(equalTo: content.topAnchor, constant: 8),
            canvas.topAnchor.constraint(equalTo: top.bottomAnchor, constant: 8), canvas.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            canvas.trailingAnchor.constraint(equalTo: content.trailingAnchor), canvas.bottomAnchor.constraint(equalTo: bottom.topAnchor, constant: -8),
            canvas.heightAnchor.constraint(greaterThanOrEqualToConstant: 180),
            bottom.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 12), bottom.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -12),
            bottom.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -8),
        ])
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated); active = true; observe(); canvas.becomeFirstResponder()
        if state.picture == nil { task = Task { [state] in await state.load() } }
        if !CaperAccessibility.reduceMotion {
            view.alpha = 0
            UIViewPropertyAnimator(duration: CaperMotion.base, controlPoint1: CGPoint(x: 0.16, y: 1), controlPoint2: CGPoint(x: 0.3, y: 1)) {
                self.view.alpha = 1
            }.startAnimation()
        }
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated); active = false; task?.cancel()
        player.player?.pause(); player.player = nil; playing = nil
        state.cancelGesture()
    }
    private func observe() {
        guard active else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } }
    }
    @objc private func sizeChanged() { state.sizeFromSlider(Double(size.value), dragging: size.isTracking) }
    @objc private func cropChanged() { state.resizeCrop(Double(cropSize.value)) }
    private func pickColor() {
        let picker = UIColorPickerViewController(); picker.supportsAlpha = false
        picker.selectedColor = UIColor(cgColor: AttachmentEditRenderer.cgColor(state.color)); picker.delegate = self
        present(picker, animated: true)
    }
    func colorPickerViewControllerDidSelectColor(_ viewController: UIColorPickerViewController) {
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        viewController.selectedColor.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        state.chooseColor(AttachmentEdit.hex(red: red, green: green, blue: blue))
    }
    private func selected(_ button: UIButton?, _ on: Bool) {
        button?.isSelected = on; button?.accessibilityTraits = on ? [.button, .selected] : .button
        button?.layer.borderWidth = on ? 2 : 0; button?.layer.borderColor = UIColor(CaperTheme.terracotta).cgColor
        button?.layer.cornerRadius = 8
    }
    private func render() {
        let loaded = state.picture != nil
        status.isHidden = loaded; status.text = state.failed ? "This file can’t be edited here." : "Loading…"
        undo.isEnabled = loaded && state.history.canUndo; redo.isEnabled = loaded && state.history.canRedo
        reset.isEnabled = loaded && state.history.current.isEdited; done.isEnabled = !state.request.pictureMode || loaded
        for (tool, button) in tools { button.isEnabled = loaded; selected(button, state.tool == tool) }
        selected(spoiler, state.spoilered); spoiler?.accessibilityValue = state.spoilered ? "On" : "Off"
        cropOptions.isHidden = state.tool != .crop; markOptions.isHidden = state.tool != .draw && state.tool != .text
        cropSize.value = Float(state.cropSize)
        eraser.isHidden = state.tool != .draw; selected(eraser, state.erasing)
        let drawing = state.tool == .draw, range = drawing ? AttachmentEdit.strokeSizes : AttachmentEdit.textSizes
        size.minimumValue = Float(range.lowerBound); size.maximumValue = Float(range.upperBound)
        size.value = Float(drawing ? state.strokePoints : state.textPoints)
        size.normal = Float(drawing ? AttachmentEdit.initialStrokeSize : AttachmentEdit.initialTextSize)
        size.accessibilityLabel = drawing ? state.erasing ? "Eraser size" : "Line size" : "Text size"
        sizeLabel.text = "\(Int(size.value))"
        sizePreview.setNeedsDisplay()
        for (button, hex) in zip(swatches, AttachmentEdit.colors) { selected(button, !state.erasing && state.color == hex) }
        _ = state.shown; _ = state.scale; _ = state.offset; _ = state.brush
        if playing != state.playback {
            player.player?.pause(); player.player = nil; playing = state.playback
            if let playing { player.player = AVPlayer(url: playing); player.player?.play() }
        }
        player.view.isHidden = playing == nil; canvas.refresh()
    }
}

@MainActor private final class UIKitAttachmentSizePreview: UIView {
    private let state: NativeAttachmentEditing
    init(state: NativeAttachmentEditing) {
        self.state = state; super.init(frame: .zero)
        backgroundColor = .clear; isUserInteractionEnabled = false; isAccessibilityElement = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: 44), heightAnchor.constraint(equalToConstant: 44)])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func draw(_ rect: CGRect) {
        if let context = UIGraphicsGetCurrentContext() { state.paintSizePreview(in: context, size: bounds.size) }
    }
}

@MainActor private final class UIKitAttachmentCanvas: UIView, UITextFieldDelegate, UIGestureRecognizerDelegate, UIPointerInteractionDelegate {
    private let state: NativeAttachmentEditing
    private let playerView: UIView
    private var text: UITextField?
    private var pan: CGPoint?
    private var panOffset = CGSize.zero
    override var canBecomeFirstResponder: Bool { true }
    init(state: NativeAttachmentEditing, playerView: UIView) {
        self.state = state; self.playerView = playerView; super.init(frame: .zero)
        backgroundColor = .clear; clipsToBounds = true; isMultipleTouchEnabled = true
        addInteraction(UIPointerInteraction(delegate: self))
        addSubview(playerView)
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(magnify)); pinch.delegate = self; addGestureRecognizer(pinch)
        let double = UITapGestureRecognizer(target: self, action: #selector(zoom)); double.numberOfTapsRequired = 2; double.delegate = self; addGestureRecognizer(double)
        let hover = UIHoverGestureRecognizer(target: self, action: #selector(hovered)); addGestureRecognizer(hover)
        accessibilityLabel = state.request.kind == .video ? "Video frame being edited" : "Image being edited"
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool { state.tool == nil && state.playback == nil }
    func refresh() {
        setNeedsDisplay(); setNeedsLayout()
        isAccessibilityElement = state.picture != nil && state.field == nil && state.playback == nil
        accessibilityTraits = .image; accessibilityIdentifier = state.picture == nil ? nil : "attachment-editor-image"
        if let field = state.field {
            let new = text == nil, text = self.text ?? UITextField()
            if new {
                text.borderStyle = .roundedRect; text.backgroundColor = .black.withAlphaComponent(0.7)
                text.delegate = self; text.returnKeyType = .done; text.textAlignment = .center
                text.addTarget(self, action: #selector(textChanged), for: .editingChanged)
                addSubview(text); self.text = text
            }
            if text.markedTextRange == nil, text.text != field.value { text.text = field.value }
            text.font = UIFont(name: "Inrond40-Bold", size: field.points) ?? .boldSystemFont(ofSize: field.points)
            text.textColor = UIColor(cgColor: AttachmentEditRenderer.cgColor(field.color))
            text.accessibilityLabel = field.index == nil ? "New text" : "Edit text"
            if new { text.becomeFirstResponder() }
        } else if let text { text.removeFromSuperview(); self.text = nil; becomeFirstResponder() }
    }
    override func layoutSubviews() {
        super.layoutSubviews(); playerView.frame = state.rect(in: bounds.size)
        if let field = state.field, let layout = state.layout(in: bounds.size), let text {
            let rect = state.rect(in: bounds.size), point = layout.point(x: field.x, y: field.y, region: state.region)
            let width = max(48, AttachmentEditRenderer.textWidth(field.value.isEmpty ? "M" : field.value, size: field.points) + field.points + 8)
            text.frame = CGRect(x: rect.minX + point.x - width / 2, y: rect.minY + point.y - field.points * 0.75, width: width, height: field.points * 1.5)
        }
    }
    override func draw(_ rect: CGRect) { if let context = UIGraphicsGetCurrentContext() { state.paint(in: context, size: bounds.size) } }
    @objc private func textChanged() { if let text, text.markedTextRange == nil { state.setText(text.text ?? "") } }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool { state.commitField(); return true }
    func textFieldDidEndEditing(_ textField: UITextField) { if state.field != nil { state.commitField() } }
    override var keyCommands: [UIKeyCommand]? {
        [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(escape)),
         UIKeyCommand(input: "z", modifierFlags: .command, action: #selector(undoEdit)),
         UIKeyCommand(input: "z", modifierFlags: [.command, .shift], action: #selector(redoEdit))]
    }
    @objc private func escape() { state.escape() }
    @objc private func undoEdit() { if state.field == nil { state.undo() } }
    @objc private func redoEdit() { if state.field == nil { state.redo() } }
    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let point = touch.location(in: self)
        if state.tool == nil { pan = point; panOffset = state.offset } else { state.begin(at: point, size: bounds.size) }
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        let point = touch.location(in: self)
        if state.tool == nil, state.scale > 1, let pan {
            state.offset = MediaViewerPolicy.clampedOffset(CGSize(width: panOffset.width + point.x - pan.x, height: panOffset.height + point.y - pan.y),
                scale: state.scale, fitted: state.rect(in: bounds.size).size, container: bounds.size)
        } else { state.drag(to: point, size: bounds.size) }
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { state.end(size: bounds.size); pan = nil }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { state.cancelGesture(); pan = nil }
    @objc private func magnify(_ recognizer: UIPinchGestureRecognizer) {
        guard state.tool == nil, state.playback == nil else { return }
        state.scale = min(MediaViewerPolicy.maxZoom, max(1, state.scale * recognizer.scale)); recognizer.scale = 1
        state.offset = MediaViewerPolicy.clampedOffset(state.offset, scale: state.scale, fitted: state.rect(in: bounds.size).size, container: bounds.size)
    }
    @objc private func zoom(_ recognizer: UITapGestureRecognizer) {
        guard state.tool == nil, state.playback == nil else { return }
        if state.scale > 1 { state.scale = 1; state.offset = .zero }
        else {
            state.scale = MediaViewerPolicy.doubleTapZoom
            let rect = state.rect(in: bounds.size), point = recognizer.location(in: self)
            state.offset = MediaViewerPolicy.clampedOffset(MediaViewerPolicy.zoomOffset(at: CGPoint(x: point.x - rect.minX, y: point.y - rect.minY), fitted: rect.size, scale: state.scale),
                scale: state.scale, fitted: rect.size, container: bounds.size)
        }
    }
    @objc private func hovered(_ recognizer: UIHoverGestureRecognizer) {
        let point = recognizer.location(in: self), rect = state.rect(in: bounds.size)
        if recognizer.state == .ended || !rect.contains(point) { state.brush.hoverEnded() }
        else { state.brush.hovered(at: CGPoint(x: point.x - rect.minX, y: point.y - rect.minY)) }
    }
    func pointerInteraction(_ interaction: UIPointerInteraction, regionFor request: UIPointerRegionRequest, defaultRegion: UIPointerRegion) -> UIPointerRegion? {
        let rect = state.rect(in: bounds.size)
        return state.tool == .draw && rect.contains(request.location) ? UIPointerRegion(rect: rect, identifier: "attachment-editor-brush") : nil
    }
    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? { .hidden() }
}

private final class UIKitEditSizeSlider: UISlider {
    var normal = Float(AttachmentEdit.initialStrokeSize) { didSet { setNeedsDisplay() } }
    override func draw(_ rect: CGRect) {
        super.draw(rect)
        let fraction = CGFloat((normal - minimumValue) / (maximumValue - minimumValue))
        UIColor(CaperTheme.muted).setFill()
        UIRectFill(CGRect(x: 10 + max(0, bounds.width - 20) * fraction, y: bounds.midY + 10, width: 2, height: 4))
    }
}
#endif
