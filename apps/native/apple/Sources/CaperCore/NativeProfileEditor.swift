import SwiftUI
import UniformTypeIdentifiers
import Observation

/// Presentation glue for hosts still using sheets. All editor controls are
/// native; the existing photo/camera drawer keeps its original-byte contract.
struct NativeProfileEditorSheet: View {
    @State private var state: NativeProfileEditing
    @State private var choosing = false
    @State private var dismissID = UUID()
    #if os(iOS)
    @State private var pick: (() async throws -> Data)?
    #endif
    init(model: AppModel, close: @escaping () -> Void) { _state = State(initialValue: NativeProfileEditing(model: model, close: close)) }
    var body: some View {
        NativeProfileForm(state: state, choose: { choosing = true })
            .preference(key: ProfileDismissPreference.self, value: ProfileDismissAction(id: dismissID, action: state.requestClose))
            .interactiveDismissDisabled(state.busy || state.draft?.hasChanges == true)
            .onChange(of: state.model.accountEpoch) { _, _ in state.stop(); state.close() }
            .alert("Discard profile changes?", isPresented: $state.confirmingDiscard) {
                Button("Keep editing", role: .cancel) {}
                Button("Discard", role: .destructive, action: state.discard)
            } message: { Text("Your unsaved profile changes will be lost.") }
            .confirmationDialog("Animated picture", isPresented: Binding(get: { state.animationData != nil }, set: { if !$0 { state.animationData = nil } }), titleVisibility: .visible, presenting: state.animationData) { data in
                Button("Use original animation") { state.useAnimation(data) }.disabled(!ProfilePicturePolicy.isGIF(data))
                Button("Edit a still picture") { state.editStill(data) }
                Button("Cancel", role: .cancel) { state.animationData = nil }
            } message: { _ in Text("Editing saves a still frame, not the animation. Original animation upload is available for GIFs only.") }
            .sheet(item: $state.editing, onDismiss: state.closeCrop) { request in
                NativeAttachmentEditorBridge(request: request, close: state.closeCrop)
                    .preferredColorScheme(.dark)
                    #if os(macOS)
                    .frame(minWidth: 720, minHeight: 600)
                    #endif
            }
            .sheet(item: $state.history) { history in
                NativeProfileHistoryForm(state: history) { state.history?.stop(); state.history = nil }
                    .preferredColorScheme(.dark)
                    #if os(macOS)
                    .frame(width: 360, height: 560)
                    #endif
            }
            #if os(macOS)
            .fileImporter(isPresented: $choosing, allowedContentTypes: [.image]) { result in
                if case .success(let url) = result { state.choose { try await Task.detached { try PictureFile.read(url) }.value } }
            }
            .onExitCommand(perform: state.requestClose)
            #else
            .background(ProfileEditDismissObserver(preventDismissal: state.busy || state.draft?.hasChanges == true, attempt: state.requestClose))
            .background(ProfileSheetOutsideTap(close: state.requestClose))
            .dropDestination(for: DroppedAttachment.self) { items, _ in
                guard state.current, !state.busy, state.editing == nil, state.animationData == nil,
                      state.history == nil, !choosing, let first = items.first else {
                    items.forEach { AttachmentStaging.remove($0.file.url) }; return false
                }
                items.dropFirst().forEach { AttachmentStaging.remove($0.file.url) }
                state.choose {
                    defer { AttachmentStaging.remove(first.file.url) }
                    return try await Task.detached { try PictureFile.read(first.file.url) }.value
                }
                return true
            }
            .fullScreenCover(isPresented: $choosing, onDismiss: {
                guard let load = pick else { return }; pick = nil; state.choose(load)
            }) { AttachDrawer(mode: .picture(title: "Choose a picture", pick: { pick = $0 })) }
            #endif
    }
}

#if os(macOS)
import AppKit

struct NativeProfileForm: NSViewRepresentable {
    let state: NativeProfileEditing
    let choose: () -> Void
    func makeNSView(context: Context) -> AppKitProfileEditor { AppKitProfileEditor(state: state, choose: choose) }
    func updateNSView(_ view: AppKitProfileEditor, context: Context) { view.render() }
    static func dismantleNSView(_ view: AppKitProfileEditor, coordinator: ()) { view.stop() }
}

@MainActor final class AppKitProfileEditor: AppKitSettingsDialog, NSTextFieldDelegate {
    let state: NativeProfileEditing
    private let choose: () -> Void
    private let username = NSTextField(), displayName = NSTextField()
    private let hint = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let unsaved = AppKitKit.label("Unsaved changes", size: 12, color: AppKitKit.muted)
    private let avatar = AppKitAvatar()
    private let prepared = NSImageView()
    private let sign = AppKitKit.image("caper-change-me", size: 112)
    private var picture: AppKitKitButton!
    private var regenerate: AppKitKitButton!
    private var save: AppKitKitButton!
    private var data: Data?
    override var dismissDisabled: Bool { state.saving }
    init(state: NativeProfileEditing, choose: @escaping () -> Void) {
        self.state = state; self.choose = choose
        super.init(title: "Edit profile", detail: "", closeLabel: "Close profile", identifier: "profile-editor", close: state.requestClose)
        let art = NSView(); art.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([art.widthAnchor.constraint(equalToConstant: 112), art.heightAnchor.constraint(equalToConstant: 112)])
        for child in [avatar, prepared, sign] { child.frame = CGRect(x: 0, y: 0, width: 112, height: 112); art.addSubview(child) }
        prepared.imageScaling = .scaleProportionallyUpOrDown; prepared.wantsLayer = true; prepared.layer?.masksToBounds = true
        prepared.layer?.cornerRadius = ProfilePicturePolicy.cornerRadius(side: 112)
        picture = AppKitKitButton("Change profile picture") { [weak self] in self?.showPictureMenu() }
        picture.setAccessibilityIdentifier("change-profile-picture")
        regenerate = AppKitKitButton("Regenerate avatar") { [state] in if !state.busy { state.draft?.regenerate() } }
        regenerate.setAccessibilityIdentifier("regenerate-avatar"); regenerate.toolTip = "Generate a new random avatar"
        let controls = AppKitKit.stack([picture, regenerate], spacing: 8)
        let pictures = NSStackView(views: [art, controls]); pictures.spacing = 16
        for (field, title) in [(username, "Username"), (displayName, "Display name")] {
            field.font = AppKitKit.font(14); field.delegate = self; field.setAccessibilityLabel(title)
            field.heightAnchor.constraint(greaterThanOrEqualToConstant: 36).isActive = true
        }
        username.nextKeyView = displayName
        save = AppKitKitButton("Save profile", kind: .primary) { [state] in state.save() }
        save.setAccessibilityIdentifier("profile-save"); unsaved.setAccessibilityIdentifier("profile-unsaved")
        let actions = NSStackView(views: [unsaved, NSView(), save]); actions.spacing = 12
        setRows([pictures, AppKitKit.divider(), Self.heading("Username"), username,
            Self.note("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores."),
            Self.heading("Display name"), displayName, hint, error, actions])
        preferHeight(620); registerForDraggedTypes([.fileURL]); render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) }
        else { observation.stop() }
    }
    func stop() { observation.stop(); state.stop() }
    func controlTextDidChange(_ obj: Notification) {
        guard !state.saving else { return }
        state.draft?.username = username.stringValue; state.draft?.displayName = displayName.stringValue
    }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.insertNewline(_:)) {
            if control === username { window?.makeFirstResponder(displayName) } else { state.save() }; return true
        }; return false
    }
    override func cancelOperation(_ sender: Any?) { state.requestClose() }
    func render() {
        guard let draft = state.draft else { return }
        if username.stringValue != draft.username { username.stringValue = draft.username }
        if displayName.stringValue != draft.displayName { displayName.stringValue = draft.displayName }
        username.isEnabled = !state.saving; displayName.isEnabled = !state.saving
        let nameError = ProfileValidation.hiddenCharacterError(displayName: draft.displayName, saved: draft.baseline.displayName)
        hint.stringValue = nameError ?? "This is what people see in conversations."
        hint.textColor = nameError == nil ? AppKitKit.muted : AppKitKit.error
        error.stringValue = state.error ?? (state.pictureBusy ? "Preparing picture…" : ""); error.isHidden = error.stringValue.isEmpty
        unsaved.isHidden = !draft.hasChanges; save.isEnabled = state.canSave; save.title = state.saving ? "Saving…" : "Save profile"
        header.close.isEnabled = !state.saving; picture.isEnabled = !state.busy; regenerate.isEnabled = !state.busy; regenerate.isHidden = state.hasPicture
        avatar.update(name: draft.displayName.isEmpty ? draft.username : draft.displayName, avatarId: draft.avatarId, picture: draft.avatarPicture)
        sign.isHidden = state.hasPicture || CaperAvatar.index(for: draft.avatarId) == nil
        if data != draft.preparedPicture?.data { data = draft.preparedPicture?.data; prepared.image = data.flatMap(NSImage.init(data:)) }
        prepared.animates = !CaperAccessibility.reduceMotion; prepared.isHidden = data == nil
    }
    private func showPictureMenu() {
        guard !state.busy else { return }
        let menu = NSMenu()
        var actions: [(String, Selector)] = [("Choose a new picture…", #selector(pick)), ("Previous pictures", #selector(history))]
        if state.hasPicture { actions.insert(("Edit current picture", #selector(edit)), at: 0); actions.append(("Remove picture", #selector(remove))) }
        for (title, selector) in actions { let item = NSMenuItem(title: title, action: selector, keyEquivalent: ""); item.target = self; menu.addItem(item) }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: picture.bounds.maxY), in: picture)
    }
    @objc private func pick() { choose() }
    @objc private func edit() { state.editCurrent() }
    @objc private func history() { state.openHistory() }
    @objc private func remove() { state.draft?.removePicture() }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { state.busy ? [] : .copy }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard !state.busy, let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], let url = urls.first else { return false }
        state.choose { try await Task.detached { try PictureFile.read(url) }.value }; return true
    }
}

struct NativeProfileHistoryForm: NSViewRepresentable {
    let state: NativeProfileHistory
    let close: () -> Void
    func makeNSView(context: Context) -> AppKitProfileHistory { AppKitProfileHistory(state: state, close: close) }
    func updateNSView(_ view: AppKitProfileHistory, context: Context) {}
    static func dismantleNSView(_ view: AppKitProfileHistory, coordinator: ()) { view.stop() }
}

@MainActor final class AppKitProfileHistory: AppKitSettingsDialog {
    private let state: NativeProfileHistory
    private let pictures = AppKitKit.stack([], spacing: 8)
    private let status = AppKitKit.label("", size: 12)
    private var more: AppKitKitButton!
    private var use: AppKitKitButton!
    private var rows: [AppKitKitButton] = []
    init(state: NativeProfileHistory, close: @escaping () -> Void) {
        self.state = state
        super.init(title: "Previous pictures", detail: "", closeLabel: "Cancel", identifier: "profile-picture-history", close: close)
        more = AppKitKitButton("Load more") { state.load() }; use = AppKitKitButton("Use picture", kind: .primary) { state.use() }
        setRows([pictures, status, more, use]); preferHeight(560); state.load()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) } else { observation.stop() }
    }
    func stop() { observation.stop(); state.stop() }
    private func render() {
        if rows.count != state.pictures.count {
            pictures.arrangedSubviews.forEach { $0.removeFromSuperview() }; rows = []
            for item in state.pictures {
                let avatar = AppKitAvatar(); avatar.translatesAutoresizingMaskIntoConstraints = false
                NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 76), avatar.heightAnchor.constraint(equalToConstant: 76)])
                avatar.update(name: "", avatarId: nil, picture: item.avatarPicture)
                let button = AppKitKitButton(item.avatarAsset == state.expected ? "Current picture" : "Previous picture") { [state] in state.selected = item }
                rows.append(button)
                let row = NSStackView(views: [avatar, button]); row.spacing = 12; pictures.addArrangedSubview(row)
            }
        }
        for (button, item) in zip(rows, state.pictures) { button.showsSelection = item == state.selected; button.setAccessibilityValue(item == state.selected ? "Selected" : "Not selected") }
        status.stringValue = state.error ?? (state.loading ? "Loading…" : state.loaded && state.pictures.isEmpty ? "No previous pictures yet." : "")
        status.isHidden = status.stringValue.isEmpty
        more.title = state.error == nil ? "Load more" : "Retry"; more.isEnabled = !state.loading
        more.isHidden = state.error == nil && state.cursor == nil; use.isEnabled = state.canUse
    }
}
#elseif os(iOS)
import UIKit

struct NativeProfileForm: UIViewControllerRepresentable {
    let state: NativeProfileEditing
    let choose: () -> Void
    func makeUIViewController(context: Context) -> UIKitProfileEditor { UIKitProfileEditor(state: state, choose: choose) }
    func updateUIViewController(_ controller: UIKitProfileEditor, context: Context) { controller.render() }
    static func dismantleUIViewController(_ controller: UIKitProfileEditor, coordinator: ()) { controller.stop() }
}

@MainActor final class UIKitProfileEditor: UIKitMessageDialog, UITextFieldDelegate {
    private let state: NativeProfileEditing
    private let choose: () -> Void
    private let username = UITextField(), displayName = UITextField()
    private let hint = UIKitMessageDialog.label("")
    private let error = UIKitMessageDialog.label("", error: true)
    private let unsaved = UIKitMessageDialog.label("Unsaved changes")
    private let avatar = UIKitAvatar(side: 112)
    private let prepared = UIImageView()
    private let sign = UIImageView(image: UIImage(named: "caper-change-me", in: caperResourceBundle, compatibleWith: nil))
    private var picture: UIButton!
    private var regenerate: UIButton!
    private var save: UIButton!
    private var data: Data?
    private var decoding: Task<Void, Never>?
    override var dismissDisabled: Bool { state.saving }
    init(state: NativeProfileEditing, choose: @escaping () -> Void) {
        self.state = state; self.choose = choose; super.init(title: "Edit profile", close: state.requestClose)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); view.accessibilityIdentifier = "profile-editor"
        let art = UIView(); art.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([art.widthAnchor.constraint(equalToConstant: 112), art.heightAnchor.constraint(equalToConstant: 112)])
        art.addSubview(avatar)
        NSLayoutConstraint.activate([avatar.leadingAnchor.constraint(equalTo: art.leadingAnchor), avatar.topAnchor.constraint(equalTo: art.topAnchor)])
        for child in [prepared, sign] { child.frame = CGRect(x: 0, y: 0, width: 112, height: 112); child.contentMode = .scaleAspectFit; art.addSubview(child) }
        prepared.clipsToBounds = true; prepared.layer.cornerRadius = ProfilePicturePolicy.cornerRadius(side: 112)
        picture = Self.button("Change profile picture") {}; picture.showsMenuAsPrimaryAction = true; picture.accessibilityIdentifier = "change-profile-picture"
        regenerate = Self.button("Regenerate avatar") { [state] in if !state.busy { state.draft?.regenerate() } }; regenerate.accessibilityIdentifier = "regenerate-avatar"
        let controls = UIStackView(arrangedSubviews: [picture, regenerate]); controls.axis = .vertical; controls.spacing = 8
        let pictures = UIStackView(arrangedSubviews: [art, controls]); pictures.spacing = 12; pictures.alignment = .center
        for (field, label) in [(username, "Username"), (displayName, "Display name")] {
            field.borderStyle = .roundedRect; field.delegate = self; field.accessibilityLabel = label
            field.font = UIFont(name: "Satoshi-Regular", size: 14); field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
            field.addTarget(self, action: #selector(changed), for: .editingChanged)
        }
        username.autocapitalizationType = .none; username.autocorrectionType = .no; username.returnKeyType = .next; displayName.returnKeyType = .done
        save = Self.button("Save profile", primary: true) { [state] in state.save() }; save.accessibilityIdentifier = "profile-save"
        unsaved.accessibilityIdentifier = "profile-unsaved"
        let actions = UIStackView(arrangedSubviews: [unsaved, UIView(), save]); actions.spacing = 12
        [pictures, Self.label("Username", bold: true), username, Self.label("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores."),
         Self.label("Display name", bold: true), displayName, hint, error, actions].forEach { body.addArrangedSubview($0) }
        observe()
    }
    @objc private func changed() {
        guard !state.saving else { return }
        state.draft?.username = username.text ?? ""; state.draft?.displayName = displayName.text ?? ""
    }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if textField === username { displayName.becomeFirstResponder() } else { state.save() }; return true
    }
    override func render() {
        guard isViewLoaded, let draft = state.draft else { return }
        if username.markedTextRange == nil, username.text != draft.username { username.text = draft.username }
        if displayName.markedTextRange == nil, displayName.text != draft.displayName { displayName.text = draft.displayName }
        username.isEnabled = !state.saving; displayName.isEnabled = !state.saving
        let nameError = ProfileValidation.hiddenCharacterError(displayName: draft.displayName, saved: draft.baseline.displayName)
        hint.text = nameError ?? "This is what people see in conversations."; hint.textColor = UIColor(nameError == nil ? CaperTheme.muted : CaperTheme.error)
        error.text = state.error ?? (state.pictureBusy ? "Preparing picture…" : nil); error.isHidden = error.text == nil
        unsaved.isHidden = !draft.hasChanges; save.isEnabled = state.canSave; save.configuration?.title = state.saving ? "Saving…" : "Save profile"
        picture.isEnabled = !state.busy; regenerate.isEnabled = !state.busy; regenerate.isHidden = state.hasPicture
        avatar.show(name: draft.displayName.isEmpty ? draft.username : draft.displayName, avatarID: draft.avatarId, picture: draft.avatarPicture)
        sign.isHidden = state.hasPicture || CaperAvatar.index(for: draft.avatarId) == nil
        prepared.isHidden = draft.preparedPicture == nil
        if data != draft.preparedPicture?.data {
            data = draft.preparedPicture?.data; decoding?.cancel(); prepared.image = nil
            if let data { decoding = Task { [weak self] in
                let frames = await Task.detached { try? ProfilePictureDecoder.decode(data, pixelSize: 336) }.value
                guard let self, !Task.isCancelled, let frames, let first = frames.images.first else { return }
                self.prepared.image = frames.animated && !CaperAccessibility.reduceMotion
                    ? UIImage.animatedImage(with: frames.images.map { UIImage(cgImage: $0) }, duration: frames.durations.reduce(0, +)) : UIImage(cgImage: first)
            } }
        }
        var actions = [UIAction(title: "Choose a new picture…") { [choose] _ in choose() }, UIAction(title: "Previous pictures") { [state] _ in state.openHistory() }]
        if state.hasPicture {
            actions.insert(UIAction(title: "Edit current picture") { [state] _ in state.editCurrent() }, at: 0)
            actions.append(UIAction(title: "Remove picture", attributes: .destructive) { [state] _ in state.draft?.removePicture() })
        }
        picture.menu = UIMenu(children: actions)
    }
    override var keyCommands: [UIKeyCommand]? { [UIKeyCommand(input: UIKeyCommand.inputEscape, modifierFlags: [], action: #selector(requestClose))] }
    @objc private func requestClose() { state.requestClose() }
    override func stop() { super.stop(); decoding?.cancel(); state.stop() }
}

struct NativeProfileHistoryForm: UIViewControllerRepresentable {
    let state: NativeProfileHistory
    let close: () -> Void
    func makeUIViewController(context: Context) -> UIKitProfileHistory { UIKitProfileHistory(state: state, close: close) }
    func updateUIViewController(_ controller: UIKitProfileHistory, context: Context) {}
    static func dismantleUIViewController(_ controller: UIKitProfileHistory, coordinator: ()) { controller.stop() }
}

@MainActor final class UIKitProfileHistory: UIKitMessageDialog {
    private let state: NativeProfileHistory
    private let pictures = UIStackView()
    private let status = UIKitMessageDialog.label("")
    private var more: UIButton!
    private var use: UIButton!
    private var rows: [UIButton] = []
    init(state: NativeProfileHistory, close: @escaping () -> Void) { self.state = state; super.init(title: "Previous pictures", close: close) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); pictures.axis = .vertical; pictures.spacing = 8
        more = Self.button("Load more") { [state] in state.load() }; use = Self.button("Use picture", primary: true) { [state] in state.use() }
        [pictures, status, more!, use!].forEach { body.addArrangedSubview($0) }; observe(); state.load()
    }
    override func render() {
        if rows.count != state.pictures.count {
            pictures.arrangedSubviews.forEach { $0.removeFromSuperview() }; rows = []
            for item in state.pictures {
                let avatar = UIKitAvatar(side: 76); avatar.show(name: "", avatarID: nil, picture: item.avatarPicture)
                let button = Self.button(item.avatarAsset == state.expected ? "Current picture" : "Previous picture") { [state] in state.selected = item }
                rows.append(button)
                let row = UIStackView(arrangedSubviews: [avatar, button]); row.spacing = 12; row.alignment = .center; pictures.addArrangedSubview(row)
            }
        }
        for (button, item) in zip(rows, state.pictures) {
            button.isSelected = item == state.selected; button.accessibilityTraits = button.isSelected ? [.button, .selected] : .button
            button.layer.borderWidth = button.isSelected ? 2 : 0; button.layer.borderColor = UIColor(CaperTheme.terracotta).cgColor; button.layer.cornerRadius = 8
        }
        status.text = state.error ?? (state.loading ? "Loading…" : state.loaded && state.pictures.isEmpty ? "No previous pictures yet." : nil)
        status.isHidden = status.text == nil
        more.configuration?.title = state.error == nil ? "Load more" : "Retry"; more.isEnabled = !state.loading
        more.isHidden = state.error == nil && state.cursor == nil; use.isEnabled = state.canUse
    }
    override func stop() { super.stop(); state.stop() }
}
#endif
