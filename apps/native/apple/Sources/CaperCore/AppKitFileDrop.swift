#if os(macOS)
import AppKit
import SwiftUI

/// The native conversation owns the destination, including file drags over its
/// text editor. Import captures the composer before any background file work.
@MainActor public final class AppKitFileDropView: NSView {
    private let model: AppModel
    private let inThread: Bool
    private var target: ChatModel.AttachmentTarget?
    private var overlay: AppKitDropOverlay?
    public var acceptedDrop: (() -> Void)?
    public override var isFlipped: Bool { true }

    static var dragTypes: [NSPasteboard.PasteboardType] {
        NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) } + [.fileURL]
    }

    public init(model: AppModel, inThread: Bool = false) {
        self.model = model; self.inThread = inThread
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor(CaperTheme.conversation).cgColor
        registerForDraggedTypes(Self.dragTypes)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    static func carriesFiles(_ board: NSPasteboard) -> Bool {
        board.canReadObject(forClasses: [NSFilePromiseReceiver.self, NSURL.self], options: [.urlReadingFileURLsOnly: true])
    }

    private var canReceive: Bool {
        let chat = model.chat
        return chat.canAttach && !chat.sending && !chat.sendRejected && chat.pendingMessage == nil
            && !model.previewingChannel && !model.whatsNew.workspaceModalOpen && window?.attachedSheet == nil
            && (model.selectedDirectMessage.map { $0.status != .incoming && !$0.blocked } ?? true)
    }

    private func operation(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard canReceive, sender.draggingSourceOperationMask.contains(.copy),
              let target, target == model.chat.attachmentTarget(inThread: inThread) else {
            hideOverlay()
            return []
        }
        return .copy
    }

    public override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        resetHover()
        guard canReceive, Self.carriesFiles(sender.draggingPasteboard), sender.draggingSourceOperationMask.contains(.copy) else { return [] }
        target = model.chat.attachmentTarget(inThread: inThread)
        guard target != nil else { return [] }
        let chat = model.chat
        let text = AttachmentPolicy.dropText(channel: chat.channelName,
            direct: model.selectedDirectMessageID != nil || model.visit?.direct == true,
            attached: inThread ? chat.threadAttachmentDrafts.count : chat.attachmentDrafts.count)
        let overlay = AppKitDropOverlay(rootView: FileDropOverlay(title: inThread ? "Add files to thread" : text.title, detail: text.detail))
        overlay.frame = bounds
        overlay.autoresizingMask = [.width, .height]
        overlay.setAccessibilityIdentifier("attachment-drop-overlay")
        addSubview(overlay)
        self.overlay = overlay
        return .copy
    }

    public override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { operation(sender) }
    public override func draggingExited(_ sender: NSDraggingInfo?) { resetHover() }
    public override func draggingEnded(_ sender: NSDraggingInfo) { resetHover() }
    public override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { operation(sender) == .copy }

    public override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard operation(sender) == .copy, let target else { resetHover(); return false }
        // Promise first: an item's URL may be a lower-quality alternative.
        let objects = sender.draggingPasteboard.readObjects(forClasses: [NSFilePromiseReceiver.self, NSURL.self],
            options: [.urlReadingFileURLsOnly: true]) ?? []
        resetHover()
        guard !objects.isEmpty else { return false }
        let chat = model.chat
        let room = AttachmentPolicy.maxAttachments - chat.attachmentFiles(inThread: inThread).count
        guard room > 0 else {
            chat.setAttachmentNotice("You can attach up to \(AttachmentPolicy.maxAttachments) files.", target: target)
            return false
        }
        chat.setAttachmentNotice("Adding files…", target: target)
        AppKitDropImport(objects: objects, limit: room).start { files, failed, overflow in
            chat.addAttachments(files, to: target)
            if chat.attachmentNotice(target: target) == "Adding files…" { chat.setAttachmentNotice(nil, target: target) }
            if chat.attachmentNotice(target: target) == nil {
                if failed { chat.setAttachmentNotice("Some files couldn’t be added. Try selecting them with Files.", target: target) }
                else if overflow { chat.setAttachmentNotice("You can attach up to \(AttachmentPolicy.maxAttachments) files.", target: target) }
            }
        }
        // Focus follows the drop, not a potentially much later promise callback.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.canReceive, target == self.model.chat.attachmentTarget(inThread: self.inThread) else { return }
            self.acceptedDrop?()
        }
        return true
    }

    public override func concludeDragOperation(_ sender: NSDraggingInfo?) { resetHover() }
    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { resetHover() }
    }

    private func hideOverlay() { overlay?.removeFromSuperview(); overlay = nil }
    private func resetHover() { hideOverlay(); target = nil }
}

/// Decorative only: never replace the destination or steal composer focus.
private final class AppKitDropOverlay: NSHostingView<FileDropOverlay> {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// One accepted drag. Only disk work runs on the serial background queue;
/// results and the model callback stay on the main actor. Source files are
/// never modified, and promises share one private destination as AppKit requires.
@MainActor private final class AppKitDropImport {
    private enum Source {
        case file(URL)
        case promise(NSFilePromiseReceiver)
        var count: Int {
            switch self {
            case .file: 1
            case .promise(let receiver): receiver.fileNames.count
            }
        }
    }
    private let sources: [Source]
    private let overflow: Bool
    private let queue = OperationQueue()
    private var remaining = 0
    private var files: [Int: LocalAttachmentFile] = [:]
    private var receivedIndices: Set<Int> = []
    private var failed = false
    private var destination: URL?
    private var completed: (([LocalAttachmentFile], Bool, Bool) -> Void)?

    // AppKit returns [Any]; classify the supported values below rather than
    // force-casting the whole pasteboard result to a protocol array.
    init(objects: [Any], limit: Int) {
        var sources: [Source] = [], count = 0, overflow = false
        for object in objects {
            let source: Source
            if let receiver = object as? NSFilePromiseReceiver { source = .promise(receiver) }
            else if let url = object as? URL, url.isFileURL { source = .file(url) }
            else { continue }
            // A multi-file promise cannot be partially requested. Refuse a group
            // larger than the remaining allowance instead of importing unbounded files.
            guard source.count > 0 else { continue }
            guard count + source.count <= limit else { overflow = true; continue }
            sources.append(source); count += source.count
        }
        self.sources = sources; self.overflow = overflow; remaining = count
        queue.maxConcurrentOperationCount = 1
        queue.qualityOfService = .userInitiated
    }

    func start(_ completed: @escaping ([LocalAttachmentFile], Bool, Bool) -> Void) {
        self.completed = completed
        guard remaining > 0 else { finish(); return }
        if sources.contains(where: { if case .promise = $0 { true } else { false } }) {
            do { destination = try AttachmentStaging.newDirectory() }
            catch { failed = true; finish(); return }
        }
        var offset = 0
        for source in sources {
            let start = offset
            offset += source.count
            switch source {
            case .file(let url):
                queue.addOperation {
                    let file = try? Self.stage(url)
                    Task { @MainActor in self.received(file, index: start) }
                }
            case .promise(let receiver):
                guard let destination else { continue }
                let names = receiver.fileNames
                // AppKit calls this once per promised file, on our serial queue.
                receiver.receivePromisedFiles(atDestination: destination, options: [:], operationQueue: queue) { url, error in
                    let file = error == nil ? try? Self.stage(url) : nil
                    Task { @MainActor in
                        let index = names.indices.first { names[$0] == url.lastPathComponent && !self.receivedIndices.contains(start + $0) }.map { start + $0 }
                            ?? (start..<(start + names.count)).first { !self.receivedIndices.contains($0) } ?? start
                        self.received(file, index: index)
                    }
                }
            }
        }
    }

    nonisolated private static func stage(_ url: URL) throws -> LocalAttachmentFile {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        guard url.isFileURL, try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else {
            throw CocoaError(.fileReadUnsupportedScheme)
        }
        return try AttachmentStaging.stage(copying: url)
    }

    private func received(_ file: LocalAttachmentFile?, index: Int) {
        receivedIndices.insert(index)
        if let file { files[index] = file } else { failed = true }
        remaining -= 1
        if remaining == 0 { finish() }
    }

    private func finish() {
        if let destination { try? FileManager.default.removeItem(at: destination) }
        destination = nil
        let callback = completed; completed = nil
        callback?(files.sorted { $0.key < $1.key }.map(\.value), failed, overflow)
    }
}
#endif
