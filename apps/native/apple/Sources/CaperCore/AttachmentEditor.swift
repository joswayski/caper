import AVFoundation
import ImageIO
import SwiftUI

// MARK: Presentation

/// A staged file opened in the editor (docs/media.md, "Editing attachments
/// before sending"). Tapping a staged file the editor can open, or its
/// pencil, opens it here viewing, with no tool picked.
struct AttachmentEditRequest: Identifiable {
    let id = UUID()
    let name: String
    /// `.image` or `.video`.
    let kind: AttachmentKind
    /// The local original. The editor shows it and never changes it.
    let original: URL
    /// What the draft has now.
    let edit: AttachmentEdit
    /// The tool it opens on; nil opens it viewing, with no tool picked.
    var tool: AttachmentEditTool?
    /// Profile pictures use a square crop and always save on Done.
    var pictureMode = false
    /// Applying a profile crop only changes its local draft; space icons still save immediately.
    var deferredPicture = false
    /// A staged video as it will be sent, played while viewing with no
    /// unsaved change; nil shows the editor's frame.
    var playback: URL?
    /// The file's spoiler mark and its toggle; nil where it can't be marked.
    var spoiler: AttachmentDraftSpoiler?
    /// Removes the file (the editor then closes, dropping unsaved edits);
    /// nil where it can't be removed.
    var remove: (() -> Void)?
    /// Called only for a change: the new edit (the original after Reset) and
    /// a staged render of it for the chip (nil for the original).
    let save: (AttachmentEdit, URL?) -> Void

    init(name: String, kind: AttachmentKind, original: URL, edit: AttachmentEdit, tool: AttachmentEditTool? = nil,
         pictureMode: Bool = false, deferredPicture: Bool = false, playback: URL? = nil, spoiler: AttachmentDraftSpoiler? = nil, remove: (() -> Void)? = nil,
         save: @escaping (AttachmentEdit, URL?) -> Void) {
        self.name = name
        self.kind = kind
        self.original = original
        self.edit = edit
        self.tool = tool
        self.pictureMode = pictureMode
        self.deferredPicture = deferredPicture
        self.playback = playback
        self.spoiler = spoiler
        self.remove = remove
        self.save = save
    }

    /// The bottom row, left to right.
    var row: [AttachmentEditorRowItem] { AttachmentEditorRowItem.row(spoiler: spoiler != nil, remove: remove != nil) }
    var saveLabel: String { deferredPicture ? "Apply crop" : "Done" }
}

/// A staged file's spoiler mark, read live from its composer (the editor's
/// toggle and the tile both reflect it).
struct AttachmentDraftSpoiler {
    let isOn: () -> Bool
    let toggle: () -> Void
}

/// A button in the editor's bottom row.
enum AttachmentEditorRowItem: Hashable {
    case spoiler
    case tool(AttachmentEditTool)
    case remove

    /// Spoiler where the composer offers it, the tools, then Remove where it can.
    static func row(spoiler: Bool, remove: Bool) -> [AttachmentEditorRowItem] {
        var row: [AttachmentEditorRowItem] = []
        if spoiler { row.append(.spoiler) }
        row.append(contentsOf: AttachmentEditTool.allCases.map { AttachmentEditorRowItem.tool($0) })
        if remove { row.append(.remove) }
        return row
    }
}

extension AttachmentEditTool {
    /// The tool picked after tapping `tapped`: tapping the selected tool
    /// returns to viewing (nil), except for a profile picture, which stays on it.
    static func picking(_ tapped: AttachmentEditTool, from current: AttachmentEditTool?, pictureMode: Bool = false) -> AttachmentEditTool? {
        current == tapped && !pictureMode ? nil : tapped
    }
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
    /// Nil where nothing presents the editor; staged files then preview in QuickLook.
    var attachmentEditor: AttachmentEditorAction? {
        get { self[AttachmentEditorKey.self] }
        set { self[AttachmentEditorKey.self] = newValue }
    }
}

/// Presents the attachment editor for composers below it, like
/// `MediaViewerHost`: full screen on iPhone and iPad, and over the whole
/// window on the Mac, where what it covers is disabled. Only the frontmost
/// presentation can present, so the iPhone thread cover adds its own.
#if os(iOS)
struct AttachmentEditorHost: ViewModifier {
    @State private var request: AttachmentEditRequest?

    @ViewBuilder func body(content: Content) -> some View {
        content
            .environment(\.attachmentEditor, AttachmentEditorAction { shown in withoutPresentationAnimation { request = shown } })
            .fullScreenCover(isPresented: Binding(get: { request != nil }, set: { if !$0 { request = nil } })) {
                presented
                    .presentationBackground(.clear)
                    .preferredColorScheme(CaperTheme.colorScheme)
            }
    }

    @ViewBuilder private var presented: some View {
        if let shown = request {
            AttachmentEditor(request: shown) { withoutPresentationAnimation { request = nil } }.id(shown.id)
        }
    }
}
#endif

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

/// Compatibility presentation for SwiftUI callers; editing is native on both platforms.
#if os(iOS)
struct AttachmentEditor: View {
    let request: AttachmentEditRequest
    let close: () -> Void
    var body: some View { NativeAttachmentEditorBridge(request: request, close: close) }
}
#endif
