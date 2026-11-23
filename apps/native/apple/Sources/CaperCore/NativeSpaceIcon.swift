import Foundation
import Observation
import ImageIO
import UniformTypeIdentifiers

@MainActor @Observable private final class NativeSpaceIcon {
    let model: AppModel
    private let spaceID: String?
    private let epoch: Int
    private var stopped = false
    private var task: Task<Void, Never>?
    private var staged: URL?
    var busy: String?
    var unavailable = false
    var error: String?
    var animation: Data?
    var editing: AttachmentEditRequest?
    init(model: AppModel) { self.model = model; spaceID = model.detail?.space.id; epoch = model.accountEpoch }
    var current: Bool { !stopped && epoch == model.accountEpoch && model.detail?.space.id == spaceID && model.isOwner }
    var canChoose: Bool { current && busy == nil && !unavailable && animation == nil && editing == nil }
    var status: String? { error ?? busy ?? (unavailable ? ProfilePicturePolicy.unavailableMessage : nil) }
    func choose(_ load: @escaping () async throws -> Data, allowAnimation: Bool = true) {
        guard canChoose else { return }; busy = "Opening picture…"; error = nil
        task = Task { [weak self] in
            guard let self else { return }; defer { busy = nil }
            do {
                let data = try await load(); guard data.count <= PictureFile.maxBytes else { throw PictureError.tooLarge }
                let animated = await Task.detached { (CGImageSourceCreateWithData(data as CFData, nil).map(CGImageSourceGetCount) ?? 0) > 1 || ProfilePicturePolicy.isGIF(data) }.value
                guard current, !Task.isCancelled else { return }
                if allowAnimation && animated { animation = data; return }
                let image = try await Task.detached { try ProfilePicturePreparer.editorImage(data) }.value
                guard current, !Task.isCancelled else { return }
                guard let url = AttachmentPreparer.writeStaged(data, name: "space-icon-original") else { throw PictureError.unreadable }; staged = url
                let rect = ProfilePicturePolicy.squareCrop(width: image.width, height: image.height)
                let edit = AttachmentEdit(crop: EditRect(x: rect.minX / Double(image.width), y: rect.minY / Double(image.height), width: rect.width / Double(image.width), height: rect.height / Double(image.height)))
                editing = AttachmentEditRequest(name: "Adjust icon", kind: .image, original: url, edit: edit, tool: .crop, pictureMode: true) { [weak self] edit, _ in self?.send(data, edit: edit) }
            } catch is CancellationError {} catch { if current { failure(error) } }
        }
    }
    func original(_ data: Data) { animation = nil; send(data, edit: nil) }
    func still(_ data: Data) { animation = nil; choose({ data }, allowAnimation: false) }
    func editCurrent() {
        guard let url = model.detail?.space.iconPicture?.url else { return }
        choose {
            let config = URLSessionConfiguration.ephemeral; config.httpCookieAcceptPolicy = .never; config.httpShouldSetCookies = false
            let session = URLSession(configuration: config); defer { session.invalidateAndCancel() }
            return try await ProfilePictureDecoder.fetch(url, session: session)
        }
    }
    func remove() {
        guard current, busy == nil else { return }; busy = "Removing…"; error = nil
        task = Task { [weak self] in
            guard let self else { return }; defer { busy = nil }
            do { try await model.removeSpaceIcon() } catch is CancellationError {} catch { if current { failure(error) } }
        }
    }
    private func send(_ data: Data, edit: AttachmentEdit?) {
        guard current, busy == nil else { return }; busy = "Uploading…"; error = nil
        task = Task { [weak self] in
            guard let self else { return }; defer { busy = nil }
            do {
                let prepared = try await Task.detached { if let edit { return try ProfilePicturePreparer.prepare(data, edit: edit) }; return try ProfilePicturePreparer.prepare(data) }.value
                guard current, !Task.isCancelled else { return }; try await model.changeSpaceIcon(prepared)
            } catch is CancellationError {} catch { if current { failure(error) } }
        }
    }
    private func failure(_ error: Error) { self.error = ProfilePicturePolicy.message(for: error, purpose: .spaceIcon); unavailable = unavailable || ProfilePicturePolicy.uploadsUnavailable(error) }
    func closeCrop() { editing = nil; if let staged { AttachmentStaging.remove(staged) }; staged = nil }
    func stop() { stopped = true; task?.cancel(); task = nil; closeCrop(); animation = nil }
    /// A view that returns to a window (a reused settings page) edits again.
    func resume() { stopped = false }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitSpaceIcon: NSStackView {
    private let state: NativeSpaceIcon
    private let observation = AppKitObservation()
    private let picture = AppKitAvatar(corner: ProfilePicturePolicy.cornerRadius(side: 112))
    private let status = AppKitKit.label("", size: 12)
    private var choose: AppKitKitButton!, edit: AppKitKitButton!, remove: AppKitKitButton!
    private var child: NSWindow?
    private var alert: NSAlert?
    private var picking = false
    private var picker: NSOpenPanel?
    init(model: AppModel) {
        state = NativeSpaceIcon(model: model); super.init(frame: .zero); orientation = .vertical; spacing = 8; alignment = .leading
        picture.translatesAutoresizingMaskIntoConstraints = false; picture.widthAnchor.constraint(equalToConstant: 112).isActive = true; picture.heightAnchor.constraint(equalToConstant: 112).isActive = true
        addArrangedSubview(picture)
        choose = AppKitKitButton("Change space icon") { [weak self] in self?.pick() }; choose.setAccessibilityLabel("Change space icon")
        edit = AppKitKitButton("Edit icon") { [state] in state.editCurrent() }; remove = AppKitKitButton("Remove icon") { [state] in state.remove() }
        let actions = NSStackView(views: [choose, edit, remove]); actions.spacing = 8
        addArrangedSubview(actions); addArrangedSubview(status); AppKitKit.fillWidth(status, of: self); registerForDraggedTypes([.fileURL])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { state.stop() }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { state.resume(); observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop(); state.stop(); picker?.cancel(nil); picker = nil; closeChild(); if let alert { alert.window.sheetParent?.endSheet(alert.window) }; alert = nil }
    }
    private func render() {
        guard state.current else { return }
        let space = state.model.detail?.space
        picture.update(name: space?.name ?? "", avatarId: nil, picture: space?.iconPicture)
        choose.isEnabled = state.canChoose && !picking && child == nil && alert == nil
        edit.isHidden = space?.iconAsset == nil; remove.isHidden = edit.isHidden
        edit.isEnabled = choose.isEnabled; remove.isEnabled = state.busy == nil && child == nil && alert == nil
        status.stringValue = state.status ?? ""; status.textColor = state.error == nil ? AppKitKit.muted : AppKitKit.error; status.isHidden = state.status == nil
        guard !picking, alert == nil, child == nil, let window else { return }
        if let data = state.animation {
            let alert = NSAlert(); alert.messageText = "Animated picture"; alert.informativeText = ProfilePicturePolicy.animationMessage(data)
            alert.addButton(withTitle: "Use original animation").isEnabled = ProfilePicturePolicy.keepsAnimation(data); alert.addButton(withTitle: "Edit a still picture"); alert.addButton(withTitle: "Cancel"); self.alert = alert
            alert.beginSheetModal(for: window) { [weak self] response in
                guard let self else { return }; self.alert = nil
                if response == .alertFirstButtonReturn { self.state.original(data) } else if response == .alertSecondButtonReturn { self.state.still(data) } else { self.state.animation = nil }; self.render()
            }
        } else if let request = state.editing {
            let controller = AppKitAttachmentEditor(request: request) { [weak self] in self?.state.closeCrop(); self?.closeChild(); self?.render() }
            let child = NSWindow(contentViewController: controller); child.setContentSize(NSSize(width: 720, height: 600)); self.child = child; window.beginSheet(child)
        }
    }
    private func pick() {
        guard state.canChoose, !picking, let window else { return }
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.image]; panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
        picking = true; picker = panel; render()
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }; self.picking = false; self.picker = nil
            if response == .OK, let url = panel.url { self.state.choose { try await Task.detached { try PictureFile.read(url) }.value } }; self.render()
        }
    }
    private func closeChild() { if let child { child.sheetParent?.endSheet(child); child.orderOut(nil) }; child = nil }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { state.canChoose && !picking && child == nil && alert == nil ? .copy : [] }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard state.canChoose, !picking, child == nil, alert == nil,
              let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], let url = urls.first else { return false }
        state.choose { try await Task.detached { try PictureFile.read(url) }.value }; return true
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitSpaceIcon: UIStackView, UIDropInteractionDelegate {
    private let state: NativeSpaceIcon
    private let present: (UIViewController) -> Void
    private let picture = UIKitAvatar(side: 112, corner: ProfilePicturePolicy.cornerRadius(side: 112))
    private let status = UIKitMessageDialog.label("")
    private var choose: UIButton!, edit: UIButton!, remove: UIButton!
    private var generation = 0
    private var child: UIViewController?
    private var closing = false
    init(model: AppModel, present: @escaping (UIViewController) -> Void) {
        state = NativeSpaceIcon(model: model); self.present = present; super.init(frame: .zero); axis = .vertical; spacing = 8; alignment = .leading
        addArrangedSubview(picture)
        choose = UIKitMessageDialog.button("Change space icon") { [weak self] in self?.pick() }
        edit = UIKitMessageDialog.button("Edit icon") { [state] in state.editCurrent() }; remove = UIKitMessageDialog.button("Remove icon") { [state] in state.remove() }
        let actions = UIStackView(arrangedSubviews: [choose, edit, remove]); actions.axis = .vertical; actions.spacing = 8
        addArrangedSubview(actions); addArrangedSubview(status); status.widthAnchor.constraint(equalTo: widthAnchor).isActive = true; addInteraction(UIDropInteraction(delegate: self))
    }
    required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    isolated deinit { state.stop() }
    override func didMoveToWindow() {
        super.didMoveToWindow(); generation += 1
        // A full-screen camera can temporarily detach the presenting window.
        if window != nil { state.resume(); observe(generation) }
        else if child == nil { state.stop() }
    }
    private func observe(_ token: Int) {
        guard token == generation, window != nil else { return }
        withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe(token) } }
    }
    private func render() {
        guard state.current else { child?.dismiss(animated: false); child = nil; return }
        let space = state.model.detail?.space
        picture.show(name: space?.name ?? "", avatarID: nil, picture: space?.iconPicture)
        choose.isEnabled = state.canChoose && child == nil; edit.isHidden = space?.iconAsset == nil; remove.isHidden = edit.isHidden
        edit.isEnabled = choose.isEnabled; remove.isEnabled = state.busy == nil && child == nil
        status.text = state.status; status.isHidden = state.status == nil; status.textColor = UIColor(state.error == nil ? CaperTheme.muted : CaperTheme.error)
        guard child == nil, !closing, window != nil else { return }
        if let data = state.animation {
            let alert = UIAlertController(title: "Animated picture", message: ProfilePicturePolicy.animationMessage(data), preferredStyle: .alert)
            let original = UIAlertAction(title: "Use original animation", style: .default) { [weak self] _ in self?.afterAlert { self?.state.original(data) } }; original.isEnabled = ProfilePicturePolicy.keepsAnimation(data); alert.addAction(original)
            alert.addAction(UIAlertAction(title: "Edit a still picture", style: .default) { [weak self] _ in self?.afterAlert { self?.state.still(data) } })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in self?.afterAlert { self?.state.animation = nil } }); show(alert)
        } else if let request = state.editing {
            let editor = UIKitAttachmentEditor(request: request) { [weak self] in self?.state.closeCrop(); self?.closeChild() }
            editor.modalPresentationStyle = .fullScreen; show(editor)
        }
    }
    private func afterAlert(_ action: () -> Void) {
        action(); closeChild()
    }
    private func pick() {
        guard state.canChoose, child == nil else { return }
        let drawer = UIKitAttachDrawer(mode: .picture(title: "Choose an icon", pick: { [weak self] in self?.state.choose($0) })) { [weak self] in self?.closeChild() }; show(drawer)
    }
    private func show(_ controller: UIViewController) { child = controller; present(controller) }
    private func closeChild() {
        guard let child, !closing else { return }; closing = true
        child.dismiss(animated: false) { [weak self] in self?.child = nil; self?.closing = false; self?.render() }
    }
    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool { session.hasItemsConforming(toTypeIdentifiers: [UTType.image.identifier]) }
    func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal { UIDropProposal(operation: state.canChoose && child == nil ? .copy : .forbidden) }
    func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        guard state.canChoose, child == nil, let provider = session.items.first?.itemProvider else { return }
        state.choose {
            let dropped: DroppedAttachment? = await withCheckedContinuation { done in _ = provider.loadTransferable(type: DroppedAttachment.self) { done.resume(returning: try? $0.get()) } }
            guard let dropped else { throw PictureError.unreadable }
            return try await Task.detached { defer { AttachmentStaging.remove(dropped.file.url) }; return try PictureFile.read(dropped.file.url) }.value
        }
    }
}
#endif
