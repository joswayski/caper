import Foundation
import Observation
import UniformTypeIdentifiers

#if os(macOS)
import AppKit

/// Presentation lifetime is native too: alerts, file access, crop and history
/// all use the same profile draft without a hidden SwiftUI root.
@MainActor final class AppKitProfilePresentation: NSViewController {
    let state: NativeProfileEditing
    private let observation = AppKitObservation()
    private var childWindow: NSWindow?
    private var childKind: String?
    private var choosing = false
    private var active = true
    private var alert: NSAlert?
    private var picker: NSOpenPanel?
    lazy var form = AppKitProfileEditor(state: state) { [weak self] in self?.choose() }
    init(model: AppModel, close: @escaping () -> Void) { state = NativeProfileEditing(model: model, close: close); super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() { view = form }
    override func viewDidAppear() { super.viewDidAppear(); observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
    isolated deinit { state.stop() }
    func requestClose() { state.requestClose() }
    func stop() {
        active = false; observation.stop(); state.stop(); picker?.cancel(nil); picker = nil
        dismissChild(); if let alert { alert.window.sheetParent?.endSheet(alert.window) }; alert = nil
    }
    private func render() {
        guard active, let window = view.window else { return }
        guard state.current else { stop(); state.close(); return }
        if state.confirmingDiscard, alert == nil {
            let alert = NSAlert(); alert.messageText = "Discard profile changes?"; alert.informativeText = "Your unsaved profile changes will be lost."
            alert.addButton(withTitle: "Keep editing"); alert.addButton(withTitle: "Discard"); self.alert = alert
            alert.beginSheetModal(for: window) { [weak self] response in
                guard let self else { return }; self.alert = nil; self.state.confirmingDiscard = false
                if response == .alertSecondButtonReturn { self.state.discard() }
            }
            return
        }
        guard alert == nil, !choosing else { return }
        if let data = state.animationData {
            let alert = NSAlert(); alert.messageText = "Animated picture"
            alert.informativeText = "Editing saves a still frame, not the animation. Original animation upload is available for GIFs only."
            alert.addButton(withTitle: "Use original animation").isEnabled = ProfilePicturePolicy.isGIF(data)
            alert.addButton(withTitle: "Edit a still picture"); alert.addButton(withTitle: "Cancel"); self.alert = alert
            alert.beginSheetModal(for: window) { [weak self] response in
                guard let self else { return }; self.alert = nil
                if response == .alertFirstButtonReturn { self.state.useAnimation(data) }
                else if response == .alertSecondButtonReturn { self.state.editStill(data) }
                else { self.state.animationData = nil }
            }
        } else if let request = state.editing, childKind == nil {
            show(AppKitAttachmentEditor(request: request) { [weak self] in self?.state.closeCrop(); self?.dismissChild() }, kind: "crop", size: NSSize(width: 720, height: 600))
        } else if let history = state.history, childKind == nil {
            let controller = NSViewController(); controller.view = AppKitProfileHistory(state: history) { [weak self] in
                self?.state.history?.stop(); self?.state.history = nil; self?.dismissChild()
            }
            show(controller, kind: "history", size: NSSize(width: 360, height: 560))
        } else if childKind == "history", state.history == nil { dismissChild() }
        else if childKind == "crop", state.editing == nil { dismissChild() }
    }
    private func choose() {
        guard state.current, !state.busy, childKind == nil, !choosing, let window = view.window else { return }
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.image]; panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
        choosing = true; picker = panel
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }; self.choosing = false; self.picker = nil
            guard self.active else { return }
            if response == .OK, let url = panel.url { self.state.choose { try await Task.detached { try PictureFile.read(url) }.value } }
            self.render()
        }
    }
    private func show(_ controller: NSViewController, kind: String, size: NSSize) {
        guard let window = view.window else { return }
        let child = NSWindow(contentViewController: controller); child.setContentSize(size)
        childWindow = child; childKind = kind; window.beginSheet(child)
    }
    private func dismissChild() {
        if let childWindow { childWindow.sheetParent?.endSheet(childWindow); childWindow.orderOut(nil) }
        childWindow = nil; childKind = nil
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitProfilePresentation: UIViewController, UIAdaptivePresentationControllerDelegate, UIDropInteractionDelegate {
    let state: NativeProfileEditing
    private var active = true
    private var observing = false
    private var closing = false
    private var childKind: String?
    private var child: UIViewController?
    private lazy var form = UIKitProfileEditor(state: state) { [weak self] in self?.choose() }
    init(model: AppModel, close: @escaping () -> Void) { state = NativeProfileEditing(model: model, close: close); super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = UIView(); addChild(form); form.view.frame = view.bounds; form.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(form.view); form.didMove(toParent: self); view.addInteraction(UIDropInteraction(delegate: self))
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated); presentationController?.delegate = self
        if !observing { observing = true; observe() }
    }
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || parent?.isBeingDismissed == true || isMovingFromParent { stop() }
    }
    func fittingHeight(width: CGFloat) -> CGFloat { form.fittingHeight(width: width) }
    isolated deinit { state.stop() }
    func requestClose() { state.requestClose() }
    func stop() { active = false; form.stop(); state.stop(); child?.dismiss(animated: false); child = nil; childKind = nil }
    func presentationControllerShouldDismiss(_ presentationController: UIPresentationController) -> Bool { state.requestClose(); return false }
    private func observe() { guard active else { return }; withObservationTracking { render() } onChange: { [weak self] in Task { @MainActor in self?.observe() } } }
    private func render() {
        guard active, view.window != nil else { return }
        guard state.current else { stop(); state.close(); return }
        isModalInPresentation = state.busy || state.draft?.hasChanges == true
        if childKind == "history", state.history == nil { dismissChild() }
        if childKind == "crop", state.editing == nil { dismissChild() }
        guard child == nil, !closing else { return }
        if state.confirmingDiscard {
            let alert = UIAlertController(title: "Discard profile changes?", message: "Your unsaved profile changes will be lost.", preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "Keep editing", style: .cancel) { [weak self] _ in self?.state.confirmingDiscard = false; self?.dismissChild() })
            alert.addAction(UIAlertAction(title: "Discard", style: .destructive) { [weak self] _ in self?.state.confirmingDiscard = false; self?.state.discard(); self?.dismissChild() })
            show(alert, kind: "discard")
        } else if let data = state.animationData {
            let alert = UIAlertController(title: "Animated picture", message: "Editing saves a still frame, not the animation. Original animation upload is available for GIFs only.", preferredStyle: .alert)
            let original = UIAlertAction(title: "Use original animation", style: .default) { [weak self] _ in self?.state.useAnimation(data); self?.dismissChild() }
            original.isEnabled = ProfilePicturePolicy.isGIF(data); alert.addAction(original)
            alert.addAction(UIAlertAction(title: "Edit a still picture", style: .default) { [weak self] _ in self?.state.editStill(data); self?.dismissChild() })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in self?.state.animationData = nil; self?.dismissChild() })
            show(alert, kind: "animation")
        } else if let request = state.editing {
            let editor = UIKitAttachmentEditor(request: request) { [weak self] in self?.state.closeCrop(); self?.dismissChild() }
            editor.modalPresentationStyle = .fullScreen; show(editor, kind: "crop")
        } else if let history = state.history {
            let controller = UIKitProfileHistory(state: history) { [weak self] in self?.state.history?.stop(); self?.state.history = nil; self?.dismissChild() }
            controller.isModalInPresentation = true; show(controller, kind: "history")
        }
    }
    private func choose() {
        guard state.current, !state.busy, child == nil, !closing else { return }
        let drawer = UIKitAttachDrawer(mode: .picture(title: "Choose a picture", pick: { [weak self] load in
            // The drawer closes before the new crop screen is presented.
            self?.state.choose(load)
        })) { [weak self] in self?.dismissChild() }
        show(drawer, kind: "picture")
    }
    private func show(_ controller: UIViewController, kind: String) {
        child = controller; childKind = kind; controller.overrideUserInterfaceStyle = .dark
        present(controller, animated: !CaperAccessibility.reduceMotion)
    }
    private func dismissChild() {
        guard let child, !closing else { return }; closing = true
        child.dismiss(animated: false) { [weak self] in
            guard let self else { return }
            self.child = nil; childKind = nil; closing = false; render()
        }
    }
    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool { session.hasItemsConforming(toTypeIdentifiers: [UTType.image.identifier]) }
    func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal {
        UIDropProposal(operation: state.current && !state.busy && child == nil ? .copy : .forbidden)
    }
    func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        guard state.current, !state.busy, child == nil, let provider = session.items.first?.itemProvider else { return }
        state.choose {
            let file: DroppedAttachment? = await withCheckedContinuation { done in
                _ = provider.loadTransferable(type: DroppedAttachment.self) { done.resume(returning: try? $0.get()) }
            }
            guard let file else { throw PictureError.unreadable }
            return try await Task.detached { defer { AttachmentStaging.remove(file.file.url) }; return try PictureFile.read(file.file.url) }.value
        }
    }
}
#endif
