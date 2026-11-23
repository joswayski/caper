import SwiftUI
import UniformTypeIdentifiers
import Observation

/// Presentation glue for hosts still using sheets. All editor controls are
/// native; the existing photo/camera drawer keeps its original-byte contract.
#if os(iOS)
struct NativeProfileEditorSheet: View {
    @State private var state: NativeProfileEditing
    @State private var choosing = false
    @State private var dismissID = UUID()
    @State private var pick: (() async throws -> Data)?
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
                Button("Use original animation") { state.useAnimation(data) }.disabled(!ProfilePicturePolicy.keepsAnimation(data))
                Button("Edit a still picture") { state.editStill(data) }
                Button("Cancel", role: .cancel) { state.animationData = nil }
            } message: { data in Text(ProfilePicturePolicy.animationMessage(data)) }
            .sheet(item: $state.editing, onDismiss: state.closeCrop) { request in
                NativeAttachmentEditorBridge(request: request, close: state.closeCrop)
                    .preferredColorScheme(.dark)
            }
            .sheet(item: $state.history) { history in
                NativeProfileHistoryForm(state: history) { state.history?.stop(); state.history = nil }
                    .preferredColorScheme(.dark)
            }
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
    }
}
#endif

#if os(macOS)
import AppKit

private final class NonInteractiveLabel: NSTextField {
    override var acceptsFirstResponder: Bool { false }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

@MainActor final class AppKitProfileEditor: AppKitSettingsDialog, NSTextFieldDelegate {
    let state: NativeProfileEditing
    private let choose: () -> Void
    private let username = AppKitKitField(), displayName = AppKitKitField()
    private let hint = AppKitKit.label("", size: 12, color: AppKitKit.muted)
    private let error = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let unsaved: NSTextField = {
        let label = NonInteractiveLabel(labelWithString: "Unsaved changes")
        label.font = AppKitKit.font(12)
        label.textColor = AppKitKit.muted
        label.isEditable = false
        label.isSelectable = false
        label.isBordered = false
        label.drawsBackground = false
        label.usesSingleLineMode = true
        label.maximumNumberOfLines = 1
        label.refusesFirstResponder = true
        return label
    }()
    private let avatar = AppKitAvatar()
    private let prepared = NSImageView()
    private let sign = AppKitKit.image("caper-change-me", size: 112)
    private var picture: AppKitPictureButton!
    private let badge = NSView()
    private let pencil: NSImageView = {
        let view = AppKitKit.image("caper-pencil", size: 15)
        view.image?.isTemplate = true
        return view
    }()
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
        prepared.layer?.backgroundColor = AppKitKit.raised.cgColor
        // The whole picture is the control, with a pencil badge overhanging its
        // corner, as on web; the badge sits inside the control's 118pt square.
        picture = AppKitPictureButton { [weak self] in self?.showPictureMenu() }
        picture.setAccessibilityLabel("Change profile picture"); picture.toolTip = "Change profile picture"
        picture.setAccessibilityIdentifier("change-profile-picture")
        badge.wantsLayer = true; badge.layer?.cornerRadius = 8; badge.layer?.borderWidth = 3
        badge.translatesAutoresizingMaskIntoConstraints = false; pencil.translatesAutoresizingMaskIntoConstraints = false
        badge.addSubview(pencil); picture.addSubview(art); picture.addSubview(badge)
        NSLayoutConstraint.activate([
            picture.widthAnchor.constraint(equalToConstant: 118), picture.heightAnchor.constraint(equalToConstant: 118),
            art.leadingAnchor.constraint(equalTo: picture.leadingAnchor), art.topAnchor.constraint(equalTo: picture.topAnchor),
            badge.widthAnchor.constraint(equalToConstant: 34), badge.heightAnchor.constraint(equalToConstant: 34),
            badge.trailingAnchor.constraint(equalTo: picture.trailingAnchor), badge.bottomAnchor.constraint(equalTo: picture.bottomAnchor),
            pencil.centerXAnchor.constraint(equalTo: badge.centerXAnchor), pencil.centerYAnchor.constraint(equalTo: badge.centerYAnchor),
        ])
        regenerate = AppKitKitButton("Regenerate avatar") { [state] in if !state.busy { state.draft?.regenerate() } }
        regenerate.setAccessibilityIdentifier("regenerate-avatar"); regenerate.toolTip = "Generate a new random avatar"
        let pictures = AppKitKit.stack([picture, regenerate], spacing: 12)
        for (field, title) in [(username, "Username"), (displayName, "Display name")] {
            field.delegate = self; field.setAccessibilityLabel(title)
        }
        username.nextKeyView = displayName
        save = AppKitKitButton("Save profile", kind: .primary) { [state] in state.save() }
        save.setAccessibilityIdentifier("profile-save"); unsaved.setAccessibilityIdentifier("profile-unsaved")
        let spacer = NSView()
        spacer.translatesAutoresizingMaskIntoConstraints = false
        spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        spacer.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        unsaved.setContentHuggingPriority(.required, for: .horizontal)
        unsaved.setContentCompressionResistancePriority(.required, for: .horizontal)
        save.setContentHuggingPriority(.required, for: .horizontal)
        save.setContentCompressionResistancePriority(.required, for: .horizontal)
        let actions = NSStackView(views: [spacer, unsaved, save])
        actions.orientation = .horizontal
        actions.alignment = .centerY
        actions.spacing = 12
        // Each field reads label, field, then its muted hint, tight together; a wider gap separates the groups.
        let usernameHeading = Self.heading("Username"), displayHeading = Self.heading("Display name")
        let usernameHint = Self.note("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores.")
        setRows([pictures, usernameHeading, username, usernameHint, displayHeading, displayName, hint, error, actions])
        body.setCustomSpacing(24, after: pictures)
        for view in [usernameHeading, username, displayHeading, displayName] { body.setCustomSpacing(8, after: view) }
        body.setCustomSpacing(22, after: usernameHint)
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
        badge.layer?.borderColor = AppKitKit.surface.cgColor
        badge.layer?.backgroundColor = (state.busy ? AppKitKit.border : AppKitKit.action).cgColor
        pencil.contentTintColor = state.busy ? AppKitKit.muted : AppKitKit.onAction
        avatar.update(name: draft.displayName.isEmpty ? draft.username : draft.displayName, avatarId: draft.avatarId, picture: draft.avatarPicture)
        sign.isHidden = state.hasPicture || CaperAvatar.index(for: draft.avatarId) == nil
        if data != draft.preparedPicture?.data { data = draft.preparedPicture?.data; prepared.image = data.flatMap(NSImage.init(data:)) }
        prepared.layer?.backgroundColor = AppKitKit.raised.cgColor
        prepared.animates = !CaperAccessibility.reduceMotion; prepared.isHidden = data == nil; avatar.isHidden = data != nil
    }
    private func showPictureMenu() {
        guard !state.busy else { return }
        let menu = NSMenu()
        var actions: [(String, Selector)] = [("Choose a new picture…", #selector(pick)), ("Previous pictures", #selector(history))]
        if state.hasPicture { actions.insert(("Edit current picture", #selector(edit)), at: 0); actions.append(("Remove picture", #selector(remove))) }
        for (title, selector) in actions {
            let item = NSMenuItem(title: title, action: selector, keyEquivalent: ""); item.target = self
            if selector == #selector(remove) { item.caperRole = .destructive }
            menu.addItem(item)
        }
        CaperMenu.show(menu, below: picture)
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

/// Edit profile's picture control: the picture with its pencil badge, which
/// opens the picture menu on click, Space or Return, and VoiceOver's press.
@MainActor final class AppKitPictureButton: NSControl {
    private let handler: () -> Void
    init(action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false; wantsLayer = true
        focusRingType = .exterior
        setAccessibilityElement(true); setAccessibilityRole(.button)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var isEnabled: Bool { didSet { window?.invalidateCursorRects(for: self) } }
    override var acceptsFirstResponder: Bool { isEnabled && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override var focusRingMaskBounds: NSRect { bounds }
    override func drawFocusRingMask() {
        // The picture fills the top-leading 112pt of this unflipped 118pt square.
        let radius = ProfilePicturePolicy.cornerRadius(side: 112)
        NSBezierPath(roundedRect: NSRect(x: 0, y: 6, width: 112, height: 112), xRadius: radius, yRadius: radius).fill()
    }
    private func fire() { if isEnabled { handler() } }
    /// Clicks on the picture and badge land here, not on the views drawing them.
    override func hitTest(_ point: NSPoint) -> NSView? { frame.contains(point) ? self : nil }
    override func accessibilityChildren() -> [Any]? { [] }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { fire() }
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 48 { KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self) }
        else if [" ", "\r"].contains(event.charactersIgnoringModifiers) { fire() } else { super.keyDown(with: event) }
    }
    override func accessibilityPerformPress() -> Bool { fire(); return isEnabled }
    override func resetCursorRects() { if isEnabled { addCursorRect(bounds, cursor: .pointingHand) } }
}

/// One picture in the Previous pictures grid: the whole thumbnail selects it,
/// with a ring and check on the selected one. The current picture is labelled
/// and can't be chosen again.
@MainActor private final class AppKitPictureTile: NSControl {
    static let side: CGFloat = 92
    private let handler: () -> Void
    private let avatar = AppKitAvatar(corner: ProfilePicturePolicy.cornerRadius(side: AppKitPictureTile.side - 10))
    private let ring = CALayer()
    private let check = NSImageView()
    private let current: Bool
    private var hovered = false
    private var tracking: NSTrackingArea?

    init(picture: AvatarHistoryPicture, number: Int, current: Bool, action: @escaping () -> Void) {
        handler = action; self.current = current
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false; wantsLayer = true
        ring.cornerRadius = ProfilePicturePolicy.cornerRadius(side: Self.side - 10) + 5; ring.borderWidth = 2
        layer?.addSublayer(ring)
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatar.update(name: "", avatarId: nil, picture: picture.avatarPicture)
        check.image = NSImage(systemSymbolName: "checkmark.circle.fill", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 20, weight: .semibold))
        check.translatesAutoresizingMaskIntoConstraints = false; check.setAccessibilityElement(false)
        check.wantsLayer = true; check.layer?.cornerRadius = 10
        addSubview(avatar); addSubview(check)
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: Self.side), heightAnchor.constraint(equalToConstant: Self.side),
            avatar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 5), avatar.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -5),
            avatar.topAnchor.constraint(equalTo: topAnchor, constant: 5), avatar.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -5),
            check.topAnchor.constraint(equalTo: topAnchor, constant: 1), check.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -1),
        ])
        if current {
            let badge = AppKitKit.label("Current", size: 11, weight: .bold, wraps: false)
            badge.textColor = .white; badge.alignment = .center; badge.setAccessibilityElement(false)
            let pill = NSView(); pill.wantsLayer = true; pill.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.68).cgColor
            pill.layer?.cornerRadius = 9; pill.translatesAutoresizingMaskIntoConstraints = false; badge.translatesAutoresizingMaskIntoConstraints = false
            pill.addSubview(badge); addSubview(pill)
            NSLayoutConstraint.activate([
                badge.leadingAnchor.constraint(equalTo: pill.leadingAnchor, constant: 8), badge.trailingAnchor.constraint(equalTo: pill.trailingAnchor, constant: -8),
                badge.centerYAnchor.constraint(equalTo: pill.centerYAnchor), pill.heightAnchor.constraint(equalToConstant: 18),
                pill.centerXAnchor.constraint(equalTo: centerXAnchor), pill.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -11),
            ])
        }
        setAccessibilityElement(true); setAccessibilityRole(.radioButton)
        setAccessibilityLabel("\(current ? "Current picture" : "Previous picture") \(number)")
        if current { setAccessibilityHelp("This is your picture now.") }
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    var isSelected = false { didSet { if isSelected != oldValue { refresh() } } }
    override var isEnabled: Bool { didSet { refresh() } }
    private var choosable: Bool { isEnabled && !current }
    override var acceptsFirstResponder: Bool { choosable && KeyboardTabOrder.keyboardFocusAllowed }
    override var canBecomeKeyView: Bool { acceptsFirstResponder && !isHiddenOrHasHiddenAncestor && window != nil }
    override func becomeFirstResponder() -> Bool { let became = super.becomeFirstResponder(); refresh(); return became }
    override func resignFirstResponder() -> Bool { let resigned = super.resignFirstResponder(); refresh(); return resigned }

    private func refresh() {
        let focused = window?.firstResponder === self
        let accent = AppKitKit.terracottaBright
        CATransaction.begin(); CATransaction.setDisableActions(true)
        ring.frame = bounds
        ring.borderColor = (isSelected || focused ? accent : hovered && choosable ? AppKitKit.muted.withAlphaComponent(0.6) : .clear).cgColor
        CATransaction.commit()
        check.isHidden = !isSelected; check.contentTintColor = accent; check.layer?.backgroundColor = AppKitKit.surface.cgColor
        setAccessibilityValue(isSelected ? 1 : 0)
        window?.invalidateCursorRects(for: self)
    }
    override func layout() { super.layout(); refresh() }

    private func fire() { if choosable { handler() } }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { fire() }
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 48 { KeyboardTabOrder.step(in: window, forward: !event.modifierFlags.contains(.shift), from: self) }
        else if [" ", "\r"].contains(event.charactersIgnoringModifiers) { fire() } else { super.keyDown(with: event) }
    }
    override func accessibilityPerformPress() -> Bool { fire(); return choosable }
    override func resetCursorRects() { if choosable { addCursorRect(bounds, cursor: .pointingHand) } }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; refresh() }
    override func mouseExited(with event: NSEvent) { hovered = false; refresh() }
}

@MainActor final class AppKitProfileHistory: AppKitSettingsDialog {
    private static let columns = 3
    private let state: NativeProfileHistory
    private let pictures = AppKitKit.stack([], spacing: 12)
    private let status = AppKitKit.label("", size: 12)
    private let hint = AppKitKit.label("Select a picture to use it again.", size: 12, color: AppKitKit.muted, wraps: false)
    private var more: AppKitKitButton!
    private var use: AppKitKitButton!
    private var tiles: [AppKitPictureTile] = []
    init(state: NativeProfileHistory, close: @escaping () -> Void) {
        self.state = state
        super.init(title: "Previous pictures", detail: "", closeLabel: "Cancel", identifier: "profile-picture-history", close: close)
        pictures.setAccessibilityElement(true); pictures.setAccessibilityRole(.group); pictures.setAccessibilityLabel("Previous pictures")
        more = AppKitKitButton("Load more") { state.load() }
        use = AppKitKitButton("Use previous picture", kind: .primary) { state.use() }
        use.setAccessibilityIdentifier("use-previous-picture")
        // The action row stays put below the grid; the button appears once a
        // previous picture is selected, in place of the hint.
        let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let bar = NSStackView(views: [hint, spacer, use]); bar.orientation = .horizontal; bar.alignment = .centerY; bar.spacing = 12
        bar.edgeInsets = NSEdgeInsets(top: 12, left: 22, bottom: 16, right: 22)
        bar.heightAnchor.constraint(greaterThanOrEqualToConstant: 70).isActive = true
        setRows([pictures, status, more]); setFooter(bar); preferHeight(560); state.load()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }) } else { observation.stop() }
    }
    func stop() { observation.stop(); state.stop() }
    private func render() {
        if tiles.count != state.pictures.count {
            pictures.arrangedSubviews.forEach { $0.removeFromSuperview() }
            tiles = state.pictures.enumerated().map { index, item in
                AppKitPictureTile(picture: item, number: index + 1, current: item.avatarAsset == state.expected) { [state] in state.selected = item }
            }
            for start in stride(from: 0, to: tiles.count, by: Self.columns) {
                let row = NSStackView(views: Array(tiles[start..<min(start + Self.columns, tiles.count)])); row.spacing = 12
                pictures.addArrangedSubview(row)
            }
        }
        for (tile, item) in zip(tiles, state.pictures) { tile.isSelected = item == state.selected }
        status.stringValue = state.error ?? (state.loading ? "Loading…" : state.loaded && state.pictures.isEmpty ? "No previous pictures yet." : "")
        status.isHidden = status.stringValue.isEmpty
        more.title = state.error == nil ? "Load more" : "Retry"; more.isEnabled = !state.loading
        more.isHidden = state.error == nil && state.cursor == nil
        use.isHidden = !state.canUse; hint.isHidden = state.canUse || state.pictures.isEmpty
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
    // The workspace dialog sizes to its content and proposes no height.
    func sizeThatFits(_ proposal: ProposedViewSize, uiViewController: UIKitProfileEditor, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return CGSize(width: width, height: proposal.height ?? uiViewController.fittingHeight(width: width))
    }
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
    /// The whole picture is the control, with a pencil badge on its corner, as on web.
    private let picture = UIButton(type: .custom)
    private let badge = UIView()
    private let pencil = UIImageView(image: UIKitVoiceControls.icon("pencil", size: 15))
    private var regenerate: UIButton!
    private var save: UIButton!
    private var data: Data?
    private var decoding: Task<Void, Never>?
    override var dismissDisabled: Bool { state.saving }
    init(state: NativeProfileEditing, choose: @escaping () -> Void) {
        self.state = state; self.choose = choose; super.init(title: "Edit profile", closeLabel: "Close profile", close: state.requestClose)
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
        art.isUserInteractionEnabled = false
        badge.layer.cornerRadius = 8; badge.layer.cornerCurve = .continuous
        badge.layer.borderWidth = 3; badge.layer.borderColor = UIColor(CaperTheme.raised).cgColor
        badge.isUserInteractionEnabled = false
        pencil.contentMode = .center
        for child in [art, badge, pencil] { child.translatesAutoresizingMaskIntoConstraints = false }
        badge.addSubview(pencil); picture.addSubview(art); picture.addSubview(badge)
        picture.translatesAutoresizingMaskIntoConstraints = false
        picture.showsMenuAsPrimaryAction = true
        picture.accessibilityLabel = "Change profile picture"; picture.accessibilityIdentifier = "change-profile-picture"
        // The badge overhangs the picture's corner by 6pt, inside the button so it stays tappable.
        NSLayoutConstraint.activate([
            art.leadingAnchor.constraint(equalTo: picture.leadingAnchor), art.topAnchor.constraint(equalTo: picture.topAnchor),
            picture.trailingAnchor.constraint(equalTo: art.trailingAnchor, constant: 6),
            picture.bottomAnchor.constraint(equalTo: art.bottomAnchor, constant: 6),
            badge.widthAnchor.constraint(equalToConstant: 34), badge.heightAnchor.constraint(equalToConstant: 34),
            badge.trailingAnchor.constraint(equalTo: picture.trailingAnchor), badge.bottomAnchor.constraint(equalTo: picture.bottomAnchor),
            pencil.centerXAnchor.constraint(equalTo: badge.centerXAnchor), pencil.centerYAnchor.constraint(equalTo: badge.centerYAnchor),
        ])
        regenerate = Self.button("Regenerate avatar") { [state] in if !state.busy { state.draft?.regenerate() } }; regenerate.accessibilityIdentifier = "regenerate-avatar"
        regenerate.configuration?.image = UIKitVoiceControls.icon("rotate-ccw", size: 16); regenerate.configuration?.imagePadding = 8
        let pictures = UIStackView(arrangedSubviews: [picture, regenerate]); pictures.axis = .vertical; pictures.spacing = 12; pictures.alignment = .leading
        for (field, label) in [(username, "Username"), (displayName, "Display name")] {
            field.borderStyle = .roundedRect; field.delegate = self; field.accessibilityLabel = label
            field.font = UIFont(name: "Figtree-Regular", size: 14); field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
            field.addTarget(self, action: #selector(changed), for: .editingChanged)
        }
        username.autocapitalizationType = .none; username.autocorrectionType = .no; username.returnKeyType = .next; displayName.returnKeyType = .done
        save = Self.button("Save profile", primary: true) { [state] in state.save() }; save.accessibilityIdentifier = "profile-save"
        unsaved.accessibilityIdentifier = "profile-unsaved"
        unsaved.isUserInteractionEnabled = false
        let spacer = UIView()
        spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        unsaved.setContentHuggingPriority(.required, for: .horizontal)
        save.setContentHuggingPriority(.required, for: .horizontal)
        let actions = UIStackView(arrangedSubviews: [spacer, unsaved, save])
        actions.alignment = .center
        actions.spacing = 12
        // Each field reads label, field, then its muted hint, tight together; a wider gap separates the groups.
        let usernameLabel = Self.label("Username", bold: true), displayLabel = Self.label("Display name", bold: true)
        let usernameHint = Self.label("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores.")
        usernameHint.textColor = UIColor(CaperTheme.muted)
        [pictures, usernameLabel, username, usernameHint, displayLabel, displayName, hint, error, actions].forEach { body.addArrangedSubview($0) }
        body.setCustomSpacing(24, after: pictures)
        for view in [usernameLabel, username, displayLabel, displayName] { body.setCustomSpacing(8, after: view) }
        body.setCustomSpacing(22, after: usernameHint)
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
        badge.backgroundColor = UIColor(state.busy ? CaperTheme.border : CaperTheme.action)
        pencil.tintColor = UIColor(state.busy ? CaperTheme.muted : CaperTheme.onAction)
        avatar.show(name: draft.displayName.isEmpty ? draft.username : draft.displayName, avatarID: draft.avatarId, picture: draft.avatarPicture)
        sign.isHidden = state.hasPicture || CaperAvatar.index(for: draft.avatarId) == nil
        prepared.backgroundColor = UIColor(CaperTheme.raised)
        prepared.isHidden = draft.preparedPicture == nil; avatar.isHidden = draft.preparedPicture != nil
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

/// One picture in the Previous pictures grid: tapping the thumbnail selects
/// it, with a ring and check on the selected one. The current picture is
/// labelled and can't be chosen again.
@MainActor private final class UIKitPictureTile: UIControl {
    static let side: CGFloat = 92
    private let ring = UIView()
    private let check = UIImageView(image: UIImage(systemName: "checkmark.circle.fill"))
    private let current: Bool

    init(picture: AvatarHistoryPicture, number: Int, current: Bool, action: @escaping () -> Void) {
        self.current = current
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let inner = Self.side - 10
        let avatar = UIKitAvatar(side: inner, corner: ProfilePicturePolicy.cornerRadius(side: inner))
        avatar.show(name: "", avatarID: nil, picture: picture.avatarPicture); avatar.isUserInteractionEnabled = false
        ring.layer.cornerRadius = ProfilePicturePolicy.cornerRadius(side: inner) + 5; ring.layer.cornerCurve = .continuous
        ring.layer.borderWidth = 2; ring.isUserInteractionEnabled = false
        check.preferredSymbolConfiguration = .init(pointSize: 20, weight: .semibold)
        check.backgroundColor = UIColor(CaperTheme.surface); check.layer.cornerRadius = 10; check.clipsToBounds = true
        check.tintColor = UIColor(CaperTheme.terracottaBright); check.isUserInteractionEnabled = false
        for child in [ring, avatar, check] as [UIView] { child.translatesAutoresizingMaskIntoConstraints = false; addSubview(child) }
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: Self.side), heightAnchor.constraint(equalToConstant: Self.side),
            ring.leadingAnchor.constraint(equalTo: leadingAnchor), ring.trailingAnchor.constraint(equalTo: trailingAnchor),
            ring.topAnchor.constraint(equalTo: topAnchor), ring.bottomAnchor.constraint(equalTo: bottomAnchor),
            avatar.centerXAnchor.constraint(equalTo: centerXAnchor), avatar.centerYAnchor.constraint(equalTo: centerYAnchor),
            check.topAnchor.constraint(equalTo: topAnchor, constant: 1), check.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -1),
        ])
        if current {
            let badge = UIKitMessageDialog.label("Current", size: 11, bold: true)
            badge.textColor = .white; badge.textAlignment = .center; badge.numberOfLines = 1
            let pill = UIView(); pill.backgroundColor = UIColor.black.withAlphaComponent(0.68); pill.layer.cornerRadius = 9
            pill.isUserInteractionEnabled = false
            for child in [pill, badge] { child.translatesAutoresizingMaskIntoConstraints = false }
            pill.addSubview(badge); addSubview(pill)
            NSLayoutConstraint.activate([
                badge.leadingAnchor.constraint(equalTo: pill.leadingAnchor, constant: 8), badge.trailingAnchor.constraint(equalTo: pill.trailingAnchor, constant: -8),
                badge.topAnchor.constraint(equalTo: pill.topAnchor, constant: 2), badge.bottomAnchor.constraint(equalTo: pill.bottomAnchor, constant: -2),
                pill.centerXAnchor.constraint(equalTo: centerXAnchor), pill.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -11),
            ])
        }
        isAccessibilityElement = true
        accessibilityLabel = "\(current ? "Current picture" : "Previous picture") \(number)"
        if current { accessibilityHint = "This is your picture now." }
        addAction(UIAction { [weak self] _ in if let self, !self.current { action() } }, for: .touchUpInside)
        refresh()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isSelected: Bool { didSet { if isSelected != oldValue { refresh() } } }
    override var isEnabled: Bool { didSet { refresh() } }
    private func refresh() {
        ring.layer.borderColor = (isSelected ? UIColor(CaperTheme.terracottaBright) : .clear).cgColor
        check.isHidden = !isSelected
        var traits: UIAccessibilityTraits = .button
        if isSelected { traits.insert(.selected) }
        if current || !isEnabled { traits.insert(.notEnabled) }
        accessibilityTraits = traits
    }
}

@MainActor final class UIKitProfileHistory: UIKitMessageDialog {
    private static let columns = 3
    private let state: NativeProfileHistory
    private let pictures = UIStackView()
    private let status = UIKitMessageDialog.label("")
    private let hint = UIKitMessageDialog.label("Select a picture to use it again.")
    private var more: UIButton!
    private var use: UIButton!
    private var tiles: [UIKitPictureTile] = []
    init(state: NativeProfileHistory, close: @escaping () -> Void) { self.state = state; super.init(title: "Previous pictures", close: close) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); pictures.axis = .vertical; pictures.spacing = 12; pictures.alignment = .center
        pictures.accessibilityLabel = "Previous pictures"
        hint.textColor = UIColor(CaperTheme.muted)
        more = Self.button("Load more") { [state] in state.load() }
        use = Self.button("Use previous picture", primary: true) { [state] in state.use() }
        use.accessibilityIdentifier = "use-previous-picture"
        // The action row stays put below the grid; the button appears once a
        // previous picture is selected, in place of the hint.
        let bar = UIStackView(arrangedSubviews: [hint, use]); bar.axis = .vertical; bar.spacing = 8
        bar.isLayoutMarginsRelativeArrangement = true; bar.layoutMargins = UIEdgeInsets(top: 12, left: 16, bottom: 16, right: 16)
        bar.heightAnchor.constraint(greaterThanOrEqualToConstant: 72).isActive = true
        [pictures, status, more!].forEach { body.addArrangedSubview($0) }; setFooter(bar); observe(); state.load()
    }
    override func render() {
        if tiles.count != state.pictures.count {
            pictures.arrangedSubviews.forEach { $0.removeFromSuperview() }
            tiles = state.pictures.enumerated().map { index, item in
                UIKitPictureTile(picture: item, number: index + 1, current: item.avatarAsset == state.expected) { [state] in state.selected = item }
            }
            for start in stride(from: 0, to: tiles.count, by: Self.columns) {
                let row = UIStackView(arrangedSubviews: Array(tiles[start..<min(start + Self.columns, tiles.count)])); row.spacing = 12
                pictures.addArrangedSubview(row)
            }
        }
        for (tile, item) in zip(tiles, state.pictures) { tile.isSelected = item == state.selected }
        status.text = state.error ?? (state.loading ? "Loading…" : state.loaded && state.pictures.isEmpty ? "No previous pictures yet." : nil)
        status.isHidden = status.text == nil
        more.configuration?.title = state.error == nil ? "Load more" : "Retry"; more.isEnabled = !state.loading
        more.isHidden = state.error == nil && state.cursor == nil
        use.isHidden = !state.canUse; hint.isHidden = state.canUse || state.pictures.isEmpty
    }
    override func stop() { super.stop(); state.stop() }
}
#endif
