import Foundation
import Observation

/// Both native dialogs share lookup/join guards and the dead-link response.
@MainActor @Observable final class NativeJoinInvite {
    let model: AppModel
    let close: () -> Void
    var link: String
    var preview: InvitePreview?
    var pending = false
    var error: String?
    init(model: AppModel, opened: OpenedInviteLink?, close: @escaping () -> Void) {
        self.model = model; self.close = close; link = opened?.link ?? ""
    }
    var label: String {
        if preview == nil { return pending ? "Checking…" : "Continue" }
        if preview?.alreadyMember == true { return pending ? "Opening…" : "Open space" }
        return pending ? "Joining…" : "Join space"
    }
    func submit() {
        guard !pending else { return }
        let code: String
        if let preview { code = preview.code }
        else {
            guard let parsed = InviteLinks.code(from: link) else { error = InviteLinks.invalid; return }
            code = parsed
        }
        let joining = preview != nil
        pending = true; error = nil
        Task {
            do {
                if joining { try await model.joinInvite(code: code); pending = false; close() }
                else { preview = try await model.invitePreview(code: code) }
            } catch {
                if let failure = error as? APIError, failure.status == 404 { self.error = InviteLinks.invalid }
                else { self.error = FriendlyError.message(for: error) }
            }
            pending = false
        }
    }
}

#if os(macOS)
import AppKit

@MainActor final class AppKitJoinInvite: AppKitSettingsDialog, NSTextFieldDelegate {
    private let state: NativeJoinInvite
    private let field = AppKitKitField()
    private let failure = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let preview = AppKitKit.stack([], spacing: 6)
    private var previewID: String?
    private var submit: AppKitKitButton!
    private var cancel: AppKitKitButton!
    override var dismissDisabled: Bool { state.pending }
    init(model: AppModel, opened: OpenedInviteLink?, close: @escaping () -> Void) {
        state = NativeJoinInvite(model: model, opened: opened, close: close)
        super.init(title: "Join with invite link", detail: "Paste it below to join!", closeLabel: "Close invite link", identifier: "join-invite", close: close)
        field.placeholderString = "caper.chat/invite/AbCdE12345"; field.stringValue = state.link; field.delegate = self
        field.setAccessibilityLabel("Invite link"); field.setAccessibilityIdentifier("invite-link-field")
        failure.setAccessibilityIdentifier("invite-link-error"); preview.setAccessibilityIdentifier("invite-preview")
        submit = AppKitKitButton("Continue", kind: .primary) { [state] in state.submit() }; submit.keyEquivalent = "\r"
        cancel = AppKitKitButton("Cancel") { [state] in if !state.pending { close() } }
        let actions = NSStackView(views: [NSView(), cancel, submit]); actions.spacing = 8
        setRows([field, preview, failure, actions])
        observation.start(reading: { [weak self] in self?.render() }, apply: { _ in })
        if opened != nil { state.submit() }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if state.preview == nil, !state.pending { window?.makeFirstResponder(field) }
    }
    func controlTextDidChange(_ notification: Notification) { state.link = field.stringValue; state.error = nil }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.insertNewline(_:)) { state.submit(); return true }; return false
    }
    private func render() {
        field.isEditable = !state.pending; field.isHidden = state.preview != nil
        header.close.isEnabled = !state.pending; cancel.isHidden = state.preview == nil; cancel.isEnabled = !state.pending
        submit.title = state.label; submit.isEnabled = !state.pending && !state.link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        submit.setAccessibilityIdentifier(state.preview == nil ? "invite-link-continue" : "invite-link-join")
        failure.stringValue = state.error ?? ""; failure.isHidden = state.error == nil; preview.isHidden = state.preview == nil
        guard let value = state.preview, previewID != value.code else { return }; previewID = value.code
        let icon = AppKitAvatar(); icon.update(name: value.space.name, avatarId: nil, picture: value.space.iconPicture)
        icon.widthAnchor.constraint(equalToConstant: 52).isActive = true; icon.heightAnchor.constraint(equalToConstant: 52).isActive = true
        preview.addArrangedSubview(icon)
        var lines = [value.space.name]
        if value.alreadyMember == true { lines.append("You’re already a member") }
        lines += ["Invited by \(value.inviter.displayName)\(value.inviter.admin == true ? " · Admin" : "")", InviteLinks.memberCount(value.memberCount)]
        if let channel = value.channel { lines.append("Opens in #\(channel.name)") }
        for (index, line) in lines.enumerated() {
            let label = AppKitKit.label(line, size: index == 0 ? 18 : 12, weight: index == 0 ? .bold : .regular)
            preview.addArrangedSubview(label); AppKitKit.fillWidth(label, of: preview)
        }
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitJoinInvite: UIKitMessageDialog, UITextFieldDelegate {
    private let state: NativeJoinInvite
    private let opened: Bool
    private let field = UITextField()
    private let failure = UIKitMessageDialog.label("", error: true)
    private let preview = UIStackView()
    private var previewID: String?
    private var submit: UIButton!
    override var dismissDisabled: Bool { state.pending }
    init(model: AppModel, opened: OpenedInviteLink?, close: @escaping () -> Void) {
        state = NativeJoinInvite(model: model, opened: opened, close: close); self.opened = opened != nil
        super.init(title: "Join with invite link", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad()
        field.placeholder = "caper.chat/invite/AbCdE12345"; field.text = state.link
        field.font = UIFont(name: "Inrond40-Regular", size: 14); field.borderStyle = .roundedRect
        field.autocapitalizationType = .none; field.autocorrectionType = .no; field.keyboardType = .URL; field.returnKeyType = .go
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        field.accessibilityIdentifier = "invite-link-field"; field.accessibilityLabel = "Invite link"; field.delegate = self
        field.addAction(UIAction { [weak self] _ in self?.state.link = self?.field.text ?? ""; self?.state.error = nil }, for: .editingChanged)
        preview.axis = .vertical; preview.spacing = 6; preview.accessibilityIdentifier = "invite-preview"
        failure.accessibilityIdentifier = "invite-link-error"
        submit = Self.button("Continue", primary: true) { [state] in state.submit() }
        [Self.label("Paste it below to join!"), field, preview, failure, submit!].forEach(body.addArrangedSubview)
        observe(); if opened { state.submit() }
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); if !opened { field.becomeFirstResponder() } }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool { state.submit(); return false }
    func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool { !state.pending }
    override func render() {
        field.isHidden = state.preview != nil; failure.text = state.error; failure.isHidden = state.error == nil
        submit.configuration?.title = state.label
        submit.isEnabled = !state.pending && !state.link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        submit.accessibilityIdentifier = state.preview == nil ? "invite-link-continue" : "invite-link-join"
        preview.isHidden = state.preview == nil
        guard let value = state.preview, previewID != value.code else { return }; previewID = value.code; field.resignFirstResponder()
        let icon = UIKitAvatar(side: 52); icon.show(name: value.space.name, avatarID: nil, picture: value.space.iconPicture)
        let header = UIStackView(arrangedSubviews: [icon, Self.label(value.space.name, size: 18, bold: true)]); header.spacing = 14; header.alignment = .center
        preview.addArrangedSubview(header)
        if value.alreadyMember == true { preview.addArrangedSubview(Self.label("You’re already a member")) }
        preview.addArrangedSubview(Self.label("Invited by \(value.inviter.displayName)\(value.inviter.admin == true ? " · Admin" : "")"))
        preview.addArrangedSubview(Self.label(InviteLinks.memberCount(value.memberCount)))
        if let channel = value.channel { preview.addArrangedSubview(Self.label("Opens in #\(channel.name)")) }
    }
}
#endif
