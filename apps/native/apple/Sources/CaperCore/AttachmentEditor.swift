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

/// Presents the attachment editor for composers below it, like
/// `MediaViewerHost`: full screen on iPhone and iPad, and over the whole
/// window on the Mac, where what it covers is disabled. Only the frontmost
/// presentation can present, so the iPhone thread cover adds its own.
struct AttachmentEditorHost: ViewModifier {
    @State private var request: AttachmentEditRequest?

    @ViewBuilder func body(content: Content) -> some View {
        #if os(iOS)
        content
            .environment(\.attachmentEditor, AttachmentEditorAction { shown in withoutPresentationAnimation { request = shown } })
            .fullScreenCover(item: $request) { shown in
                AttachmentEditor(request: shown) { withoutPresentationAnimation { request = nil } }
                    .presentationBackground(.clear)
                    .preferredColorScheme(.dark)
            }
        #else
        ZStack {
            content
                .environment(\.attachmentEditor, AttachmentEditorAction { request = $0 })
                .disabled(request != nil)
                .accessibilityHidden(request != nil)
            if let shown = request {
                AttachmentEditor(request: shown) { request = nil }.id(shown.id)
            }
        }
        #endif
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

    private enum Tool { case crop, draw, text }
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
    @State private var tool = Tool.draw
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
    @FocusState private var fieldFocused: Bool
    #if os(macOS)
    @FocusState private var editorFocused: Bool
    #endif

    init(request: AttachmentEditRequest, close: @escaping () -> Void) {
        self.request = request
        self.close = close
        _history = State(initialValue: AttachmentEditHistory(request.edit))
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
        if let loaded = await AttachmentEditorPicture.load(request.original, kind: request.kind) { picture = loaded } else { failed = true }
    }

    private func undo() { closeField(); history.undo() }
    private func redo() { closeField(); history.redo() }
    /// Back to the original, as one undo step.
    private func reset() { closeField(); history.commit(.original) }
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

    private func chooseTool(_ next: Tool) {
        if field != nil { commitField() }
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
        guard result != request.edit else { close(); return }
        saving = true
        request.save(result, result.isEdited ? AttachmentEditRenderer.stagePreview(picture.image, edit: result) : nil)
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
            toolButton(.crop, title: "Crop", icon: "crop")
            toolButton(.draw, title: "Draw", icon: "brush")
            toolButton(.text, title: "Text", icon: "type")
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Editing tools")
    }

    private func toolButton(_ value: Tool, title: String, icon: String) -> some View {
        let selected = tool == value
        return Button { chooseTool(value) } label: {
            VStack(spacing: 4) {
                CaperIcon(name: icon, size: 18)
                Text(title).font(CaperTheme.font(11, weight: .bold))
            }
            .foregroundStyle(selected ? CaperTheme.terracottaBright : CaperTheme.muted)
            .frame(minWidth: 64, minHeight: 48)
            .background(selected ? CaperTheme.terracotta.opacity(0.16) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(selected ? CaperTheme.terracotta : Color.clear, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .modifier(ControlPointer())
        .accessibilityLabel(title)
        .accessibilityAddTraits(selected ? .isSelected : [])
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

    /// A press draws; points closer than 1.5 points are skipped, and a tap is a dot.
    private func drawGesture(region: EditRect, layout: AttachmentEditLayout) -> some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                let point = layout.fraction(x: Double(value.location.x), y: Double(value.location.y), region: region)
                if var stroke = liveStroke, let last = lastStrokeLocation {
                    guard hypot(Double(value.location.x - last.x), Double(value.location.y - last.y)) >= 1.5 else { return }
                    stroke.points += [point.x, point.y]
                    liveStroke = stroke
                } else {
                    liveStroke = StrokeMark(color: color, width: strokePoints / layout.edge, points: [point.x, point.y])
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
                .accessibilityHint("Drag the frame’s corners or edges to crop, or inside it to move it.")
            ForEach(Self.cropHandles, id: \.self) { handle in
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
                if let liveCrop { commitCrop(AttachmentEdit.clampCrop(liveCrop)) }
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
