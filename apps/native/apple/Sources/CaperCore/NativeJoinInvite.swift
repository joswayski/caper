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
    /// The inviter's sentence; an admin's name carries " · ADMIN" as message
    /// headers do.
    static func runs(_ preview: InvitePreview) -> [(text: String, strong: Bool)] {
        var runs = InviteLinks.invitedLine(preview)
        if preview.inviter.admin == true { runs.insert((" · ADMIN", false), at: 1) }
        return runs
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
    private let preview = AppKitKit.stack([], spacing: 10)
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
        // A previewed link is titled with its space, as on web's invite page.
        header.titleLabel.stringValue = state.preview?.space.name ?? "Join with invite link"
        header.detailLabel?.isHidden = state.preview != nil
        guard let value = state.preview, previewID != value.code else { return }; previewID = value.code
        let avatar = AppKitAvatar(); avatar.update(name: value.inviter.displayName, avatarId: value.inviter.avatarId, picture: value.inviter.avatarPicture)
        avatar.widthAnchor.constraint(equalToConstant: 28).isActive = true; avatar.heightAnchor.constraint(equalToConstant: 28).isActive = true
        let invited = AppKitKit.label("", size: 13, color: AppKitKit.muted)
        invited.attributedStringValue = NativeJoinInvite.runs(value).reduce(into: NSMutableAttributedString()) { line, run in
            line.append(NSAttributedString(string: run.text, attributes: [
                .font: AppKitKit.font(13, weight: run.strong ? .bold : .regular),
                .foregroundColor: run.strong ? AppKitKit.text : AppKitKit.muted,
            ]))
        }
        let by = NSStackView(views: [avatar, invited]); by.spacing = 10; by.alignment = .centerY
        preview.addArrangedSubview(by); AppKitKit.fillWidth(by, of: preview)
        let members = AppKitKit.label(InviteLinks.membersLine(value), size: 12, color: AppKitKit.muted)
        preview.addArrangedSubview(members); AppKitKit.fillWidth(members, of: preview)
    }
}
#elseif os(iOS)
import UIKit

@MainActor final class UIKitJoinInvite: UIKitMessageDialog, UITextFieldDelegate {
    private let state: NativeJoinInvite
    private let opened: Bool
    private let field = UITextField()
    private let failure = UIKitMessageDialog.label("", error: true)
    private let hint = UIKitMessageDialog.label("Paste it below to join!")
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
        field.font = UIFont(name: "Figtree-Regular", size: 14); field.borderStyle = .roundedRect
        field.autocapitalizationType = .none; field.autocorrectionType = .no; field.keyboardType = .URL; field.returnKeyType = .go
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        field.accessibilityIdentifier = "invite-link-field"; field.accessibilityLabel = "Invite link"; field.delegate = self
        field.addAction(UIAction { [weak self] _ in self?.state.link = self?.field.text ?? ""; self?.state.error = nil }, for: .editingChanged)
        preview.axis = .vertical; preview.spacing = 10; preview.accessibilityIdentifier = "invite-preview"
        failure.accessibilityIdentifier = "invite-link-error"
        submit = Self.button("Continue", primary: true) { [state] in state.submit() }
        [hint, field, preview, failure, submit!].forEach(body.addArrangedSubview)
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
        preview.isHidden = state.preview == nil; hint.isHidden = state.preview != nil
        // A previewed link is titled with its space, as on web's invite page.
        setTitle(state.preview?.space.name ?? "Join with invite link")
        guard let value = state.preview, previewID != value.code else { return }; previewID = value.code; field.resignFirstResponder()
        let avatar = UIKitAvatar(side: 28); avatar.show(name: value.inviter.displayName, avatarID: value.inviter.avatarId, picture: value.inviter.avatarPicture)
        let invited = Self.label("", size: 13)
        let font = { (bold: Bool) in UIFontMetrics(forTextStyle: .body).scaledFont(for:
            UIFont(name: bold ? "Figtree-Bold" : "Figtree-Regular", size: 13) ?? .systemFont(ofSize: 13)) }
        invited.attributedText = NativeJoinInvite.runs(value).reduce(into: NSMutableAttributedString()) { line, run in
            line.append(NSAttributedString(string: run.text, attributes: [
                .font: font(run.strong), .foregroundColor: UIColor(run.strong ? CaperTheme.text : CaperTheme.muted),
            ]))
        }
        let by = UIStackView(arrangedSubviews: [avatar, invited]); by.spacing = 10; by.alignment = .center
        preview.addArrangedSubview(by)
        let members = Self.label(InviteLinks.membersLine(value)); members.textColor = UIColor(CaperTheme.muted)
        preview.addArrangedSubview(members)
    }
}
#endif
