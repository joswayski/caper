import AVFoundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// MARK: Presentation

/// A staged file opened in the editor (docs/media.md, "Editing attachments
/// before sending").
struct AttachmentEditRequest: Identifiable {
    let id = UUID()
    let name: String
    /// `.image` or `.video`.
    let kind: AttachmentKind
    /// The local original. The editor shows it and never changes it.
    let original: URL
    /// What the draft has now.
    let edit: AttachmentEdit
    /// The tool it opens on: Draw from the pencil, any from the preview.
    var tool = AttachmentEditTool.initial
    /// Profile pictures use a square crop and always save on Done.
    var pictureMode = false
    /// Called only for a change: the new edit (the original after Reset) and
    /// a staged render of it for the chip (nil for the original).
    let save: (AttachmentEdit, URL?) -> Void
}

/// Opens the editor from composers below an `AttachmentEditorHost`.
struct AttachmentEditorAction {
    let perform: (AttachmentEditRequest) -> Void
    func callAsFunction(_ request: AttachmentEditRequest) { perform(request) }
}

private struct AttachmentEditorKey: EnvironmentKey {
    static var defaultValue: AttachmentEditorAction? { nil }
}

extension EnvironmentValues {
    /// Nil where nothing presents the editor; chips then offer no pencil.
    var attachmentEditor: AttachmentEditorAction? {
        get { self[AttachmentEditorKey.self] }
        set { self[AttachmentEditorKey.self] = newValue }
    }
}

/// Presents the attachment editor, and the preview of staged files it can
/// open (`AttachmentDraftPreview`), for composers below it, like
/// `MediaViewerHost`: full screen on iPhone and iPad, and over the whole
/// window on the Mac, where what it covers is disabled. Only the frontmost
/// presentation can present, so the iPhone thread cover adds its own.
struct AttachmentEditorHost: ViewModifier {
    @State private var request: AttachmentEditRequest?
    @State private var preview: AttachmentDraftPreviewRequest?

    private var presenting: Bool { request != nil || preview != nil }

    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content
            .environment(\.attachmentEditor, AttachmentEditorAction { shown in withoutPresentationAnimation { request = shown } })
            .environment(\.attachmentDraftPreview, AttachmentDraftPreviewAction { shown in withoutPresentationAnimation { preview = shown } })
            // One cover for both, so a tool in the preview swaps it for the editor in place.
            .fullScreenCover(isPresented: Binding(get: { presenting }, set: { if !$0 { request = nil; preview = nil } })) {
                presented
                    .presentationBackground(.clear)
                    .preferredColorScheme(.dark)
            }
        #else
        ZStack {
            content
                .environment(\.attachmentEditor, AttachmentEditorAction { request = $0 })
                .environment(\.attachmentDraftPreview, AttachmentDraftPreviewAction { preview = $0 })
                .disabled(presenting)
                .accessibilityHidden(presenting)
            presented
        }
        #endif
    }

    @ViewBuilder private var presented: some View {
        if let shown = request {
            AttachmentEditor(request: shown) { withoutPresentationAnimation { request = nil } }.id(shown.id)
        } else if let shown = preview {
            AttachmentDraftPreview(request: shown) { withoutPresentationAnimation { preview = nil } } edit: { tool in
                // Closing the preview is instant; the editor brings its own entrance.
                let next = shown.edit(tool)
                withoutPresentationAnimation { preview = nil; request = next }
            }
            .id(shown.id)
        }
    }
}

/// Presents or closes an editor cover without the system slide: the editor
/// brings its own short entrance, and closing stays instant.
@MainActor func withoutPresentationAnimation(_ change: () -> Void) {
    var transaction = Transaction()
    transaction.disablesAnimations = true
    withTransaction(transaction, change)
}

/// The editor's picture: the upright original (EXIF orientation or the video
/// track's transform applied), downscaled for the screen. Uploads always
/// apply the edit to the original itself.
struct AttachmentEditorPicture: @unchecked Sendable {
    let image: CGImage

    /// A still, or a video's frame from just after the start (as posters
    /// are), at most `maxEdge` on its longer edge. Decoded off the main actor.
    static func load(_ url: URL, kind: AttachmentKind, maxEdge: Int = 2048) async -> AttachmentEditorPicture? {
        if kind == .video {
            let asset = AVURLAsset(url: url)
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: maxEdge, height: maxEdge)
            let duration = await AttachmentPreparer.durationMs(asset)
            let seconds = Double(duration ?? 1000) / 1000
            guard let frame = try? await generator.image(at: CMTime(seconds: min(0.5, seconds / 2), preferredTimescale: 600)).image else { return nil }
            return AttachmentEditorPicture(image: frame)
        }
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let image = AttachmentPreparer.thumbnail(source, index: CGImageSourceGetPrimaryImageIndex(source), maxPixelSize: maxEdge) else { return nil }
        return AttachmentEditorPicture(image: image)
    }
}

/// `#RRGGBB` as a SwiftUI colour; unreadable values are black.
private func editColor(_ hex: String) -> Color {
    let rgb = AttachmentEdit.components(hex) ?? (red: 0, green: 0, blue: 0)
    return Color(.sRGB, red: rgb.red, green: rgb.green, blue: rgb.blue, opacity: 1)
}

// MARK: Editor

/// Full-screen editor for a file waiting to be sent: crop and turn it, draw
/// on it and add text. It edits a description of the changes, never the file;
/// the composer prepares the original again with them. The stage paints with
/// the upload's own rules (`AttachmentEditRenderer`), so what you see is what
/// is sent.
struct AttachmentEditor: View {
    let request: AttachmentEditRequest
    let close: () -> Void

    private struct LiveText { let index: Int; let x: Double; let y: Double }
    /// A press in the Text tool: on a text (`index`) it may become a move.
    private struct TextDrag {
        var index: Int?
        var from: (x: Double, y: Double)
        var origin: (x: Double, y: Double)
        var moved = false
        /// The press only closed the open field.
        var closedField = false
    }
    private static let stageSpace = "attachment-editor-stage"
    private static let cropHandles: [AttachmentEdit.CropHandle] = [.nw, .n, .ne, .e, .se, .s, .sw, .w]
    /// Handles keep a crop at least this many points on screen.
    private static let minimumCropPoints = 32.0
    #if os(iOS)
    private static let textButtonHeight: CGFloat = 44
    #else
    private static let textButtonHeight: CGFloat = 36
    #endif

    @State private var picture: AttachmentEditorPicture?
    @State private var failed = false
    @State private var history: AttachmentEditHistory
    @State private var tool: AttachmentEditTool
    @State private var color = AttachmentEdit.colors[0]
    @State private var strokePoints = AttachmentEdit.initialStrokeSize
    @State private var textPoints = AttachmentEdit.initialTextSize
    @State private var liveStroke: StrokeMark?
    @State private var lastStrokeLocation: CGPoint?
    @State private var liveCrop: EditRect?
    @State private var textDrag: TextDrag?
    @State private var liveText: LiveText?
    @State private var field: AttachmentEditTextField?
    /// The picture's on-screen longer edge when the field opened: its size is a fraction of it.
    @State private var fieldEdge = 1.0
    @State private var saving = false
    @State private var brush = BrushTracker()
    @FocusState private var fieldFocused: Bool
    #if os(macOS)
    @FocusState private var editorFocused: Bool
    #endif

    init(request: AttachmentEditRequest, close: @escaping () -> Void) {
        self.request = request
        self.close = close
        _history = State(initialValue: AttachmentEditHistory(request.edit))
        _tool = State(initialValue: request.tool)
    }

    var body: some View {
        VStack(spacing: 0) {
            topBar
            Rectangle().fill(CaperTheme.border).frame(height: 1)
            stageArea
            bottomBar
        }
        .background(CaperTheme.blackout.ignoresSafeArea())
        .background(alignment: .topLeading) { escapeKey }
        .foregroundStyle(CaperTheme.text)
        // Arrives with a short fade and rise; closing is instant.
        .modifier(Entrance(offset: CGSize(width: 0, height: 8), animation: CaperMotion.easeOut(CaperMotion.base)))
        .task { await load() }
        #if os(iOS)
        // Leaving the field (the keyboard's dismiss key) commits it, as Return does.
        .onChange(of: fieldFocused) { _, focused in if !focused, field != nil { commitField() } }
        #else
        // Takes the keyboard from the composer underneath, as the media viewer does.
        .focusable()
        .focusEffectDisabled()
        .focused($editorFocused)
        .onAppear { editorFocused = true }
        #endif
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .accessibilityIdentifier("attachment-editor")
    }

    // MARK: Model

    /// What the stage shows: the committed edit with any drag in progress.
    private var shown: AttachmentEdit {
        var edit = history.current
        if let liveText, edit.marks.indices.contains(liveText.index), case .text(var mark) = edit.marks[liveText.index] {
            mark.x = liveText.x
            mark.y = liveText.y
            edit.marks[liveText.index] = .text(mark)
        }
        // The text being changed shows only in its field.
        if let index = field?.index, edit.marks.indices.contains(index) { edit.marks.remove(at: index) }
        if let liveStroke { edit.marks.append(.stroke(liveStroke)) }
        if let liveCrop { edit.crop = liveCrop }
        return edit
    }

    private var isCustomColor: Bool { !AttachmentEdit.colors.contains(color) }

    private var pictureLabel: String { request.kind == .video ? "Video frame being edited" : "Image being edited" }

    private func load() async {
        guard picture == nil, !failed else { return }
        if let loaded = await AttachmentEditorPicture.load(request.original, kind: request.kind, maxEdge: request.pictureMode ? 4096 : 2048) { picture = loaded } else { failed = true }
    }

    private func undo() { closeField(); history.undo() }
    private func redo() { closeField(); history.redo() }
    /// Back to the original, as one undo step.
    private func reset() { closeField(); history.commit(request.pictureMode ? request.edit : .original) }
    private func rotateLeft() { history.commit(history.current.rotatedLeft()) }

    private func commitCrop(_ crop: EditRect) {
        var next = history.current
        next.crop = crop
        history.commit(next)
    }

    private func openField(_ next: AttachmentEditTextField, edge: Double) {
        fieldEdge = edge
        field = next
    }

    private func commitField() {
        guard let field else { return }
        history.commit(field.applied(to: history.current, edge: fieldEdge))
        closeField()
    }

    private func closeField() {
        field = nil
        #if os(macOS)
        editorFocused = true
        #endif
    }

    /// Escape closes an open text field, else cancels the editor.
    private func escape() {
        if field != nil { closeField() } else { close() }
    }

    private func chooseTool(_ next: AttachmentEditTool) {
        if field != nil { commitField() }
        if next != .draw { brush.pointer = AttachmentEditBrushPointer() }
        tool = next
    }

    private func chooseColor(_ hex: String) {
        color = hex
        field?.color = hex
    }

    private func chooseSize(_ points: Double) {
        if tool == .draw {
            strokePoints = points
        } else {
            textPoints = points
            field?.points = points
        }
    }

    /// Done with no change from what the draft had just closes.
    private func done() {
        guard !saving, let picture else { return }
        let result = field.map { $0.applied(to: history.current, edge: fieldEdge) } ?? history.current
        guard request.pictureMode || result != request.edit else { close(); return }
        saving = true
        request.save(result, !request.pictureMode && result.isEdited ? AttachmentEditRenderer.stagePreview(picture.image, edit: result) : nil)
        close()
    }

    /// The topmost text under a point (fractions of the picture), by its on-screen box.
    private func textIndex(at point: (x: Double, y: Double), layout: AttachmentEditLayout) -> Int? {
        for index in history.current.marks.indices.reversed() {
            guard case .text(let mark) = history.current.marks[index] else { continue }
            let points = mark.size * layout.edge
            let halfWidth = Double(AttachmentEditRenderer.textWidth(mark.text, size: CGFloat(points))) / 2 + 8
            let halfHeight = points * 0.6 + 8
            if abs((point.x - mark.x) * layout.fullWidth) <= halfWidth, abs((point.y - mark.y) * layout.fullHeight) <= halfHeight { return index }
        }
        return nil
    }

    // MARK: Bars

    private var topBar: some View {
        HStack(spacing: 6) {
            Button(action: close) { CaperIcon(name: "x", size: 20) }
                .buttonStyle(EditorIconButton())
                .help("Cancel")
                .accessibilityLabel("Cancel editing")
            VStack(alignment: .leading, spacing: 2) {
                Text(request.kind == .video ? "Edit video" : "Edit image")
                    .font(CaperTheme.font(14, weight: .bold)).foregroundStyle(CaperTheme.text)
                    .lineLimit(1)
                Text(request.name)
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    .lineLimit(1).truncationMode(.middle)
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 6)
            Button(action: undo) { CaperIcon(name: "undo-2", size: 18) }
                .buttonStyle(EditorIconButton())
                .disabled(picture == nil || !history.canUndo)
                // While a field is open, the shortcut undoes its typing instead.
                .keyboardShortcut(field == nil ? KeyboardShortcut("z", modifiers: .command) : nil)
                .help("Undo")
                .accessibilityLabel("Undo")
            Button(action: redo) { CaperIcon(name: "redo-2", size: 18) }
                .buttonStyle(EditorIconButton())
                .disabled(picture == nil || !history.canRedo)
                .keyboardShortcut(field == nil ? KeyboardShortcut("z", modifiers: [.command, .shift]) : nil)
                .help("Redo")
                .accessibilityLabel("Redo")
            Button("Reset", action: reset)
                .buttonStyle(CaperSecondaryButton(minHeight: Self.textButtonHeight))
                .fixedSize()
                .disabled(picture == nil || !history.current.isEdited)
            Button(action: done) { Text("Done").padding(.horizontal, 16) }
                .buttonStyle(CaperPrimaryButton())
                .fixedSize()
                .disabled(picture == nil || saving)
                .accessibilityIdentifier("attachment-editor-done")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(CaperTheme.surface)
    }

    /// Keyboards (the Mac, iPad): Escape.
    private var escapeKey: some View {
        Button("Close", action: escape)
            .keyboardShortcut(.cancelAction)
            .opacity(0)
            .frame(width: 0, height: 0)
            .accessibilityHidden(true)
    }

    private var bottomBar: some View {
        VStack(spacing: 10) {
            if request.pictureMode, tool == .crop {
                Text("Drag the square to reposition. Adjust crop size to zoom.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if request.kind == .video, picture != nil {
                Text("Drawings and text stay on screen for the whole video.")
                    .font(CaperTheme.font(12)).foregroundStyle(CaperTheme.muted)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            options.frame(minHeight: 44)
            tools
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity)
        .background(CaperTheme.surface)
        .overlay(alignment: .top) { Rectangle().fill(CaperTheme.border).frame(height: 1) }
        .disabled(picture == nil)
    }

    @ViewBuilder private var options: some View {
        if tool == .crop {
            if request.pictureMode, let picture {
                let size = AttachmentEdit.rotatedSize(width: Double(picture.image.width), height: Double(picture.image.height), rotation: history.current.rotation)
                Slider(value: Binding(get: {
                    history.current.crop.width * size.width / min(size.width, size.height)
                }, set: { fraction in
                    let crop = history.current.crop
                    let side = fraction * min(size.width, size.height)
                    commitCrop(AttachmentEdit.clampCrop(EditRect(x: crop.x + crop.width / 2 - side / size.width / 2,
                                                               y: crop.y + crop.height / 2 - side / size.height / 2,
                                                               width: side / size.width, height: side / size.height), min: 0))
                }), in: 0.1...1)
                .accessibilityLabel("Crop size")
            }
            Button(action: rotateLeft) {
                HStack(spacing: 6) {
                    CaperIcon(name: "rotate-ccw", size: 16)
                    Text("Rotate left")
                }
            }
            .buttonStyle(CaperSecondaryButton(minHeight: Self.textButtonHeight))
        } else {
            #if os(iOS)
            VStack(spacing: 6) {
                colorChoices
                sizeControl
            }
            #else
            HStack(spacing: 20) {
                colorChoices
                sizeControl
            }
            #endif
        }
    }

    private var colorChoices: some View {
        HStack(spacing: 2) {
            ForEach(AttachmentEdit.colors, id: \.self) { preset in
                Button { chooseColor(preset) } label: { EditorSwatch(color: editColor(preset), selected: color == preset) }
                    .buttonStyle(.plain)
                    .help(AttachmentEdit.colorNames[preset] ?? preset)
                    .accessibilityLabel(AttachmentEdit.colorNames[preset] ?? preset)
                    .accessibilityAddTraits(color == preset ? .isSelected : [])
            }
            ColorPicker("Custom colour", selection: customColor, supportsOpacity: false)
                .labelsHidden()
                .padding(4)
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(isCustomColor ? CaperTheme.terracottaBright : Color.clear, lineWidth: 2))
                .help("Custom colour")
                .accessibilityLabel("Custom colour")
        }
    }

    /// The platform colour picker, in and out of web's `#RRGGBB`.
    private var customColor: Binding<Color> {
        Binding(get: { editColor(color) }, set: { chooseColor(Self.hex($0)) })
    }

    private static func hex(_ color: Color) -> String {
        #if os(iOS)
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        _ = UIColor(color).getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        return AttachmentEdit.hex(red: Double(red), green: Double(green), blue: Double(blue))
        #else
        let srgb = NSColor(color).usingColorSpace(.sRGB)
        return AttachmentEdit.hex(red: Double(srgb?.redComponent ?? 0), green: Double(srgb?.greenComponent ?? 0),
                                  blue: Double(srgb?.blueComponent ?? 0))
        #endif
    }

    private var sizeControl: some View {
        let drawing = tool == .draw
        let range = drawing ? AttachmentEdit.strokeSizes : AttachmentEdit.textSizes
        let value = drawing ? strokePoints : textPoints
        return HStack(spacing: 10) {
            Text("Size").font(CaperTheme.font(12, weight: .medium)).foregroundStyle(CaperTheme.muted)
                .accessibilityHidden(true)
            Slider(value: Binding(get: { value }, set: { chooseSize($0) }), in: range, step: 1)
                .tint(CaperTheme.terracottaBright)
                .frame(minWidth: 120, maxWidth: 240)
                .accessibilityLabel(drawing ? "Line size" : "Text size")
                .accessibilityValue("\(Int(value)) points")
            sizePreview(drawing: drawing)
                .frame(width: 44, height: 44)
                .accessibilityHidden(true)
        }
    }

    @ViewBuilder private func sizePreview(drawing: Bool) -> some View {
        if drawing {
            Circle().fill(editColor(color))
                .overlay(Circle().strokeBorder(Color.white.opacity(0.3), lineWidth: 1))
                .frame(width: min(strokePoints, 40), height: min(strokePoints, 40))
        } else {
            Text("Aa")
                .font(.custom(AttachmentEditRenderer.fontName as String, fixedSize: min(textPoints, 36)))
                .foregroundStyle(editColor(color))
        }
    }

    private var tools: some View {
        HStack(spacing: 8) {
            ForEach(AttachmentEditTool.allCases, id: \.self) { value in
                AttachmentEditToolButton(tool: value, selected: tool == value) { chooseTool(value) }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Editing tools")
    }

    // MARK: Stage

    private var stageArea: some View {
        GeometryReader { geometry in
            if let picture {
                stage(picture.image, size: geometry.size)
            } else if failed {
                Text("This file can’t be edited here.")
                    .font(CaperTheme.font(13)).foregroundStyle(CaperTheme.muted)
                    .frame(width: geometry.size.width, height: geometry.size.height)
            } else {
                ProgressView().controlSize(.small)
                    .frame(width: geometry.size.width, height: geometry.size.height)
                    .accessibilityLabel("Loading")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// The picture fitted inside the stage: cropped in Draw and Text, whole
    /// with the crop frame in Crop (with more room for its handles).
    @ViewBuilder private func stage(_ image: CGImage, size: CGSize) -> some View {
        let edit = shown
        let region = tool == .crop ? EditRect.full : history.current.crop
        let rotated = AttachmentEdit.rotatedSize(width: Double(image.width), height: Double(image.height), rotation: edit.rotation)
        if let layout = AttachmentEditLayout.fit(pictureWidth: rotated.width, pictureHeight: rotated.height, region: region,
                                                 boxWidth: Double(size.width), boxHeight: Double(size.height),
                                                 padding: tool == .crop ? 28 : 16) {
            ZStack(alignment: .topLeading) {
                pictureLayer(image, edit: edit, region: region, layout: layout)
                interaction(edit: edit, region: region, layout: layout)
                if let field { textField(field, region: region, layout: layout) }
                if tool == .draw {
                    BrushRing(tracker: brush, color: editColor(color), diameter: layout.brushDiameter(points: strokePoints))
                }
            }
            .frame(width: layout.width, height: layout.height)
            .coordinateSpace(NamedCoordinateSpace.named(Self.stageSpace))
            .frame(width: size.width, height: size.height)
        }
    }

    /// The whole rotated picture with its marks, moved so `region` fills the
    /// frame. The image turns on the GPU; the marks paint in the picture's
    /// on-screen points, so stroke widths and text sizes match the upload.
    private func pictureLayer(_ image: CGImage, edit: AttachmentEdit, region: EditRect, layout: AttachmentEditLayout) -> some View {
        let upright = AttachmentEdit.rotatedSize(width: layout.fullWidth, height: layout.fullHeight, rotation: edit.rotation)
        let marks = edit.marks
        return ZStack(alignment: .topLeading) {
            Image(decorative: image, scale: 1)
                .resizable()
                .interpolation(.high)
                .frame(width: upright.width, height: upright.height)
                .rotationEffect(.degrees(Double(edit.rotation)))
                .frame(width: layout.fullWidth, height: layout.fullHeight)
            Canvas { context, size in
                context.withCGContext { cg in
                    AttachmentEditRenderer.paintMarks(cg, marks, width: size.width, height: size.height)
                }
            }
            .frame(width: layout.fullWidth, height: layout.fullHeight)
        }
        .frame(width: layout.fullWidth, height: layout.fullHeight, alignment: .topLeading)
        .offset(x: -region.x * layout.fullWidth, y: -region.y * layout.fullHeight)
        .frame(width: layout.width, height: layout.height, alignment: .topLeading)
        .clipped()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    @ViewBuilder private func interaction(edit: AttachmentEdit, region: EditRect, layout: AttachmentEditLayout) -> some View {
        switch tool {
        case .crop:
            cropFrame(edit.crop, layout: layout)
        case .draw:
            Color.clear
                .contentShape(Rectangle())
                .gesture(drawGesture(region: region, layout: layout))
                // A hovering pointer gets the brush ring in place of its cursor.
                .onContinuousHover(coordinateSpace: CoordinateSpace.local) { phase in
                    switch phase {
                    case .active(let location): brush.pointer.hovered(at: location)
                    case .ended: brush.pointer.hoverEnded()
                    }
                }
                .background(HiddenPointerRegion().allowsHitTesting(false).accessibilityHidden(true))
                .accessibilityElement()
                .accessibilityLabel(pictureLabel)
                .accessibilityHint("Drag on the picture to draw.")
                .accessibilityAddTraits(.isImage)
        case .text:
            Color.clear
                .contentShape(Rectangle())
                .gesture(textGesture(region: region, layout: layout))
                .accessibilityElement()
                .accessibilityLabel(pictureLabel)
                .accessibilityHint(Self.textHint)
                .accessibilityAddTraits(.isImage)
        }
    }

    #if os(iOS)
    private static let textHint = "Tap the picture to add text. Tap text to change it, or drag it to move it."
    #else
    private static let textHint = "Click the picture to add text. Click text to change it, or drag it to move it."
    #endif

    /// A press draws; points closer than 1.5 points are skipped, and a tap is
    /// a dot. A hovering pointer's ring follows the stroke's tip.
    private func drawGesture(region: EditRect, layout: AttachmentEditLayout) -> some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                brush.pointer.pressed(at: value.location, starting: liveStroke == nil)
                let point = layout.fraction(x: Double(value.location.x), y: Double(value.location.y), region: region)
                if var stroke = liveStroke, let last = lastStrokeLocation {
                    guard hypot(Double(value.location.x - last.x), Double(value.location.y - last.y)) >= 1.5 else { return }
                    stroke.points += [point.x, point.y]
                    liveStroke = stroke
                } else {
                    liveStroke = StrokeMark(color: color, width: layout.strokeWidth(points: strokePoints), points: [point.x, point.y])
                }
                lastStrokeLocation = value.location
            }
            .onEnded { _ in
                if let liveStroke {
                    var next = history.current
                    next.marks.append(.stroke(liveStroke))
                    history.commit(next)
                }
                liveStroke = nil
                lastStrokeLocation = nil
                brush.pointer.released()
            }
    }

    /// A tap on the picture opens a field there, a tap on a text opens it for
    /// changes, and a drag on a text moves it. With a field open, a press
    /// anywhere else only commits it.
    private func textGesture(region: EditRect, layout: AttachmentEditLayout) -> some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                if textDrag == nil {
                    let start = layout.fraction(x: Double(value.startLocation.x), y: Double(value.startLocation.y), region: region)
                    if field != nil {
                        commitField()
                        textDrag = TextDrag(index: nil, from: start, origin: start, closedField: true)
                        return
                    }
                    let index = textIndex(at: start, layout: layout)
                    var origin = start
                    if let index, case .text(let mark) = history.current.marks[index] { origin = (mark.x, mark.y) }
                    textDrag = TextDrag(index: index, from: start, origin: origin)
                }
                guard var drag = textDrag, !drag.closedField, let index = drag.index else { return }
                if !drag.moved {
                    guard hypot(Double(value.translation.width), Double(value.translation.height)) > 4 else { return }
                    drag.moved = true
                    textDrag = drag
                }
                let point = layout.fraction(x: Double(value.location.x), y: Double(value.location.y), region: region)
                liveText = LiveText(index: index, x: min(1, max(0, drag.origin.x + point.x - drag.from.x)),
                                    y: min(1, max(0, drag.origin.y + point.y - drag.from.y)))
            }
            .onEnded { _ in
                defer { textDrag = nil; liveText = nil }
                guard let drag = textDrag, !drag.closedField else { return }
                guard let index = drag.index, history.current.marks.indices.contains(index),
                      case .text(let mark) = history.current.marks[index] else {
                    openField(AttachmentEditTextField(index: nil, x: drag.from.x, y: drag.from.y, value: "", color: color, points: textPoints),
                              edge: layout.edge)
                    return
                }
                if drag.moved, let liveText {
                    var next = history.current
                    var moved = mark
                    moved.x = liveText.x
                    moved.y = liveText.y
                    next.marks[index] = .text(moved)
                    history.commit(next)
                } else {
                    // Its colour and size become the controls' while it is open.
                    let points = mark.size * layout.edge
                    color = mark.color
                    textPoints = min(AttachmentEdit.textSizes.upperBound, max(AttachmentEdit.textSizes.lowerBound, points.rounded()))
                    openField(AttachmentEditTextField(index: index, x: mark.x, y: mark.y, value: mark.text, color: mark.color, points: points),
                              edge: layout.edge)
                }
            }
    }

    /// The crop over the whole picture: outside dimmed, inside drags to move,
    /// corners and edges resize. Kept inside the picture and at least 2% (and
    /// in practice 32 points) on each side.
    private func cropFrame(_ crop: EditRect, layout: AttachmentEditLayout) -> some View {
        let rect = CGRect(x: crop.x * layout.width, y: crop.y * layout.height, width: crop.width * layout.width, height: crop.height * layout.height)
        return ZStack(alignment: .topLeading) {
            Path { path in
                path.addRect(CGRect(x: 0, y: 0, width: layout.width, height: layout.height))
                path.addRect(rect)
            }
            .fill(Color.black.opacity(0.55), style: FillStyle(eoFill: true))
            .allowsHitTesting(false)
            Rectangle()
                .strokeBorder(Color.white.opacity(0.9), lineWidth: 1)
                .contentShape(Rectangle())
                .frame(width: rect.width, height: rect.height)
                .offset(x: rect.minX, y: rect.minY)
                .gesture(cropGesture(.move, layout: layout))
                .accessibilityElement()
                .accessibilityLabel("Crop frame")
                .accessibilityHint(request.pictureMode ? "Drag inside the square to reposition it. Adjust Crop size to zoom." : "Drag the frame’s corners or edges to crop, or inside it to move it.")
            ForEach(request.pictureMode ? [] : Self.cropHandles, id: \.self) { handle in
                CropHandleMark(handle: handle)
                    .position(Self.point(of: handle, in: rect))
                    .gesture(cropGesture(handle, layout: layout))
                    .accessibilityHidden(true)
            }
        }
        .frame(width: layout.width, height: layout.height, alignment: .topLeading)
    }

    private static func point(of handle: AttachmentEdit.CropHandle, in rect: CGRect) -> CGPoint {
        let x = handle.movesLeft ? rect.minX : handle.movesRight ? rect.maxX : rect.midX
        let y = handle.movesTop ? rect.minY : handle.movesBottom ? rect.maxY : rect.midY
        return CGPoint(x: x, y: y)
    }

    /// Measured in the stage's own space, so a handle that moves with the
    /// crop doesn't change the translation it reports.
    private func cropGesture(_ handle: AttachmentEdit.CropHandle, layout: AttachmentEditLayout) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: NamedCoordinateSpace.named(Self.stageSpace))
            .onChanged { value in
                liveCrop = AttachmentEdit.dragCrop(history.current.crop, handle: handle,
                                                   dx: Double(value.translation.width) / layout.width,
                                                   dy: Double(value.translation.height) / layout.height,
                                                   minWidth: min(1, max(0.02, Self.minimumCropPoints / layout.width)),
                                                   minHeight: min(1, max(0.02, Self.minimumCropPoints / layout.height)))
            }
            .onEnded { _ in
                if let liveCrop { commitCrop(AttachmentEdit.clampCrop(liveCrop, min: request.pictureMode ? 0 : 0.02)) }
                liveCrop = nil
            }
    }

    /// One line over the picture, in the mark's font, colour and size, centred where it will be painted.
    private func textField(_ field: AttachmentEditTextField, region: EditRect, layout: AttachmentEditLayout) -> some View {
        let position = layout.point(x: field.x, y: field.y, region: region)
        let measured = Double(AttachmentEditRenderer.textWidth(field.value.isEmpty ? "M" : field.value, size: CGFloat(field.points)))
        return TextField("", text: fieldText)
            .textFieldStyle(.plain)
            .font(.custom(AttachmentEditRenderer.fontName as String, fixedSize: field.points))
            .foregroundStyle(editColor(field.color))
            .multilineTextAlignment(.center)
            .focused($fieldFocused)
            .onSubmit(commitField)
            #if os(iOS)
            .submitLabel(.done)
            #endif
            .frame(width: max(48, measured + field.points))
            .padding(.horizontal, 4)
            .background(Color.black.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(CaperTheme.terracottaBright, lineWidth: 1.5))
            .position(x: position.x, y: position.y)
            .onAppear { DispatchQueue.main.async { fieldFocused = true } }
            .accessibilityLabel(field.index == nil ? "New text" : "Edit text")
    }

    /// One line, at most 200 characters.
    private var fieldText: Binding<String> {
        Binding(get: { field?.value ?? "" }, set: { value in
            let line = value.replacingOccurrences(of: "\n", with: " ")
            field?.value = String(line.prefix(AttachmentEdit.maxTextLength))
        })
    }
}

/// A tool in the editor's switcher, and in a staged file's preview, where it
/// opens the editor on that tool.
struct AttachmentEditToolButton: View {
    let tool: AttachmentEditTool
    var selected = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 4) {
                CaperIcon(name: tool.icon, size: 18)
                Text(tool.title).font(CaperTheme.font(11, weight: .bold))
            }
            .foregroundStyle(selected ? CaperTheme.terracottaBright : CaperTheme.muted)
            .frame(minWidth: 64, minHeight: 48)
            .background(selected ? CaperTheme.terracotta.opacity(0.16) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(selected ? CaperTheme.terracotta : Color.clear, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(ControlPointer())
        .accessibilityLabel(tool.title)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Draw's pointer. Only the ring reads it, so a pointer moving over the
/// picture redraws the ring rather than the stage.
@Observable private final class BrushTracker {
    var pointer = AttachmentEditBrushPointer()
}

/// The brush in Draw, where the cursor would be: its colour at 30% inside a
/// white ring with a dark edge either side, so it reads on light and dark
/// pictures. It never animates.
private struct BrushRing: View {
    let tracker: BrushTracker
    let color: Color
    let diameter: Double

    var body: some View {
        if let location = tracker.pointer.location {
            ZStack {
                Circle().fill(color.opacity(0.3))
                Circle().strokeBorder(CaperTheme.blackout.opacity(0.7), lineWidth: 1).padding(-1)
                Circle().strokeBorder(Color.white, lineWidth: 1.5)
                Circle().strokeBorder(CaperTheme.blackout.opacity(0.7), lineWidth: 1).padding(1.5)
            }
            .frame(width: CGFloat(diameter), height: CGFloat(diameter))
            .position(location)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

#if os(macOS)
/// Hides the cursor over the picture in Draw, where the brush ring stands in
/// for it. AppKit owns it through a cursor rect, as `ControlPointer` does, so
/// it comes back wherever the rect ends or goes away.
private struct HiddenPointerRegion: NSViewRepresentable {
    func makeNSView(context: Context) -> RegionView { RegionView() }
    func updateNSView(_ view: RegionView, context: Context) { view.window?.invalidateCursorRects(for: view) }

    final class RegionView: NSView {
        private let blank = NSCursor(image: NSImage(size: NSSize(width: 16, height: 16), flipped: false) { _ in true },
                                     hotSpot: NSPoint(x: 8, y: 8))
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func resetCursorRects() {
            super.resetCursorRects()
            addCursorRect(visibleRect, cursor: blank)
        }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.invalidateCursorRects(for: self)
        }
        override func viewWillMove(toWindow newWindow: NSWindow?) {
            // Gone from under a still pointer (Escape, another tool by keyboard):
            // the arrow now, not at the next move.
            if newWindow == nil, let window, bounds.contains(convert(window.mouseLocationOutsideOfEventStream, from: nil)) {
                NSCursor.arrow.set()
            }
            super.viewWillMove(toWindow: newWindow)
        }
    }
}
#else
/// Hides an iPad's pointer over the picture in Draw, where the brush ring
/// stands in for it. The interaction sits on the window, so it never takes a
/// touch from the drawing gesture, and claims only this view's frame where
/// nothing (a colour picker, say) covers it.
private struct HiddenPointerRegion: UIViewRepresentable {
    func makeUIView(context: Context) -> RegionView {
        let view = RegionView()
        view.isUserInteractionEnabled = false
        return view
    }
    func updateUIView(_ view: RegionView, context: Context) {}

    final class RegionView: UIView, UIPointerInteractionDelegate {
        private var interaction: UIPointerInteraction?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if let interaction { interaction.view?.removeInteraction(interaction) }
            interaction = nil
            guard let window else { return }
            let added = UIPointerInteraction(delegate: self)
            window.addInteraction(added)
            interaction = added
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            interaction?.invalidate()
        }

        func pointerInteraction(_ interaction: UIPointerInteraction, regionFor request: UIPointerRegionRequest,
                                defaultRegion: UIPointerRegion) -> UIPointerRegion? {
            guard let window, interaction.view === window else { return nil }
            let frame = convert(bounds, to: window)
            // Only over this editor's own content: the hit view is one of this view's ancestors.
            guard frame.contains(request.location), let hit = window.hitTest(request.location, with: nil), isDescendant(of: hit) else { return nil }
            return UIPointerRegion(rect: frame, identifier: "attachment-editor-brush")
        }

        func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
            UIPointerStyle.hidden()
        }
    }
}
#endif

/// The editor's top-bar icon buttons.
private struct EditorIconButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(CaperTheme.text)
            .frame(width: 36, height: 36)
            .background(RoundedRectangle(cornerRadius: 8).fill(configuration.isPressed ? CaperTheme.raised : Color.clear))
            #if os(iOS)
            .frame(minWidth: 44, minHeight: 44)
            #endif
            .contentShape(Rectangle())
            .opacity(isEnabled ? 1 : 0.35)
            .modifier(ControlPointer())
    }
}

/// A markup colour; terracotta rings the selected one.
private struct EditorSwatch: View {
    let color: Color
    let selected: Bool
    var body: some View {
        Circle().fill(color)
            .overlay(Circle().strokeBorder(Color.white.opacity(0.25), lineWidth: 1))
            .frame(width: 24, height: 24)
            .padding(3)
            .overlay(Circle().strokeBorder(selected ? CaperTheme.terracottaBright : Color.clear, lineWidth: 2))
            #if os(iOS)
            .frame(width: 44, height: 44)
            #else
            .frame(width: 34, height: 34)
            #endif
            .contentShape(Rectangle())
            .modifier(ControlPointer())
    }
}

/// A bright crop handle: a square at corners, a bar along edges, with a
/// 44-point target.
private struct CropHandleMark: View {
    let handle: AttachmentEdit.CropHandle
    var body: some View {
        let corner = (handle.movesLeft || handle.movesRight) && (handle.movesTop || handle.movesBottom)
        let across = handle == .n || handle == .s
        RoundedRectangle(cornerRadius: 2)
            .fill(Color.white)
            .frame(width: corner ? 14 : across ? 24 : 5, height: corner ? 14 : across ? 5 : 24)
            .shadow(color: .black.opacity(0.5), radius: 1.5)
            .frame(width: 44, height: 44)
            .contentShape(Rectangle())
    }
}
