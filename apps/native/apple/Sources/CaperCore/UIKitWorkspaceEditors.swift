#if os(iOS)
import UIKit

@MainActor final class UIKitSpaceEditor: UIKitMessageDialog {
    private let state: NativeSpaceEditing
    private let channelID: String?
    private let joinInvite: () -> Void
    private let name = UIKitMessageDialog.workspaceField("Gaming nights", label: "Space name")
    private let people = UIKitPeopleField(identifier: "space-invite-username", listIdentifier: "space-invite-suggestions", normalize: WorkspaceValidation.normalizeUsername)
    private let membership = UIStackView()
    private let results = UIKitMessageDialog.label(""), failure = UIKitMessageDialog.label("", error: true), limit = UIKitMessageDialog.label("You have reached your space limit.")
    private var save: UIButton!, invite: UIButton!, join: UIButton!
    private var shownMembers: [Member] = [], shownInvitations: [Member] = []
    private var memberPending: Bool?
    override var dismissDisabled: Bool { state.creating }
    init(model: AppModel, managing: Bool, inviteOnly: Bool = false, channelID: String? = nil, close: @escaping () -> Void, joinInvite: @escaping () -> Void) {
        state = NativeSpaceEditing(model: model, managing: managing, inviteOnly: inviteOnly, close: close)
        self.channelID = channelID; self.joinInvite = joinInvite; super.init(title: state.title, close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); body.superview?.accessibilityIdentifier = "space-settings-scroll"
        name.text = state.name; name.addAction(UIAction { [weak self] _ in guard let self else { return }; self.state.name = self.name.text ?? ""; self.state.error = nil }, for: .editingChanged)
        save = Self.button(state.managing ? "Save name" : "Create space", primary: !state.managing) { [state] in state.save() }
        invite = Self.button("Invite") { [state] in state.invite() }
        people.changed = { [state] in state.username = $0 }; people.submit = { [state] in state.invite() }
        people.heightChanged = { [weak self] in self?.parent?.view.setNeedsLayout() }
        if !state.inviteOnly { [Self.label("Space name", bold: true), name, save!].forEach(body.addArrangedSubview) }
        if state.managing {
            if !state.inviteOnly, state.model.detail?.space.demo != true {
                body.addArrangedSubview(UIKitSpaceIcon(model: state.model, present: { [weak self] in self?.present($0, animated: false) }))
            }
            let links = UIKitInviteLinks(state: NativeInviteLinks(model: state.model, showsDefault: state.inviteOnly, channelID: channelID), present: { [weak self] in self?.present($0, animated: false) })
            body.addArrangedSubview(links); membership.axis = .vertical; membership.spacing = 12
            [membership, Self.label("Invite by username", size: 14, bold: true), people, invite!, results].forEach(body.addArrangedSubview)
            if !state.inviteOnly {
                body.addArrangedSubview(Self.label("Delete this space and all its channels for every member."))
                body.addArrangedSubview(Self.button("Delete space", icon: "trash") { [weak self] in
                    guard let self, !self.state.pending else { return }
                    self.confirm("Delete space", detail: "Delete \(self.state.model.detail?.space.name ?? "space") for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", action: "Delete space") { [state] in
                        try await state.model.deleteCurrentSpace(); state.close()
                    }
                })
            }
        } else {
            join = Self.button("Join with invite link", action: joinInvite); join.accessibilityIdentifier = "join-invite-link"
            [limit, Self.label("Have an invite link?"), join!].forEach(body.addArrangedSubview)
        }
        body.addArrangedSubview(failure); observe(); Task { [state] in await state.load() }
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); if !state.managing { name.becomeFirstResponder() } }
    override func render() {
        guard state.current else { close(); return }
        save.isEnabled = state.canSave; save.configuration?.title = state.pending ? (state.managing ? "Saving…" : "Creating…") : state.managing ? "Save name" : "Create space"
        join?.isEnabled = !state.pending; limit.isHidden = state.model.limits == nil || state.model.canCreateSpace
        people.candidates = state.candidates; if people.text != state.username { people.setText(state.username) }
        invite.isEnabled = !state.pending && WorkspaceValidation.usernameError(state.username) == nil
        results.text = state.memberError ?? state.memberStatus; results.textColor = UIColor(state.memberError == nil ? CaperTheme.muted : CaperTheme.error); results.isHidden = results.text == nil
        failure.text = state.error; failure.isHidden = state.error == nil
        if !state.pending, state.refocusMember { state.refocusMember = false; people.field.becomeFirstResponder() }
        let members = state.model.detail?.members ?? [], invitations = state.model.pendingMembers
        guard memberPending == nil || shownMembers != members || shownInvitations != invitations || memberPending != state.pending else { return }
        shownMembers = members; shownInvitations = invitations; memberPending = state.pending
        membership.arrangedSubviews.forEach { membership.removeArrangedSubview($0); $0.removeFromSuperview() }
        membership.addArrangedSubview(Self.label("Pending invitations  \(invitations.count)", size: 14, bold: true))
        for member in invitations {
            membership.addArrangedSubview(Self.workspaceMember(member, action: "Cancel", enabled: !state.pending) { [state] in state.run { try await state.model.cancelSpaceInvitation(member) } })
        }
        if !state.inviteOnly {
            let heading = Self.label("Members  \(members.count)", size: 14, bold: true); heading.accessibilityLabel = "Members \(members.count)"; heading.accessibilityIdentifier = "space-members-heading"
            membership.addArrangedSubview(heading)
            for member in members {
                membership.addArrangedSubview(Self.workspaceMember(member, action: member.owner ? nil : "Remove", enabled: !state.pending) { [weak self, state] in
                    self?.confirm("Remove \(member.displayName)?", detail: "They’ll lose access to \(state.name ?? "this space") and its channels. You can invite them again later.", action: "Remove") { try await state.model.removeSpaceMember(member) }
                })
            }
        }
    }
    private func confirm(_ title: String, detail: String, action: String, perform: @escaping () async throws -> Void) {
        guard !state.pending else { return }
        confirmWorkspaceAction(title, detail: detail, action: action) { [state] in
            guard state.current else { throw CancellationError() }; try await perform()
        }
    }
}

@MainActor final class UIKitChannelEditor: UIKitMessageDialog, UITextFieldDelegate {
    private let state: NativeChannelEditing
    private let name = UIKitMessageDialog.workspaceField("project-updates", label: "Channel name")
    private let privacy = UISwitch(), privacyDetail = UIKitMessageDialog.label("")
    private let membership = UIStackView(), members = UIStackView(), invitations = UIStackView()
    private let people = UIKitPeopleField(identifier: "channel-invite-username", listIdentifier: "channel-member-suggestions", normalize: WorkspaceValidation.normalizeUsername)
    private let failure = UIKitMessageDialog.label("", error: true), memberFailure = UIKitMessageDialog.label("", error: true), memberStatus = UIKitMessageDialog.label("")
    private let memberHeading = UIKitMessageDialog.label("Members", size: 14, bold: true), pendingHeading = UIKitMessageDialog.label("", size: 14, bold: true)
    private var saveBar: UIStackView?
    private var save: UIButton!, reset: UIButton!, delete: UIButton!, invite: UIButton!, retry: UIButton!
    private var shownMembers: [Member] = [], shownInvitations: [Member] = []
    private var memberEnabled: Bool?
    override var dismissDisabled: Bool { state.creating }
    init(model: AppModel, channel: Channel?, close: @escaping () -> Void) {
        state = NativeChannelEditing(model: model, channel: channel, close: close); super.init(title: channel == nil ? "Create a channel" : "Channel settings", close: close)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidLoad() {
        super.viewDidLoad(); body.superview?.accessibilityIdentifier = "channel-settings-scroll"
        name.text = state.name; name.delegate = self; name.autocapitalizationType = .none; name.autocorrectionType = .no
        name.addAction(UIAction { [weak self] _ in self?.nameChanged() }, for: .editingChanged)
        privacy.accessibilityLabel = "Private channel"; privacy.onTintColor = UIColor(CaperTheme.terracotta)
        privacy.addAction(UIAction { [weak self] _ in guard let self else { return }; self.state.privateChannel = self.privacy.isOn }, for: .valueChanged)
        let toggle = UIStackView(arrangedSubviews: [Self.label("Private channel", size: 13, bold: true), privacy]); toggle.spacing = 10
        save = Self.button("Create channel", primary: true) { [state] in state.save() }; reset = Self.button("Reset") { [state] in state.reset() }
        delete = Self.button("Delete channel", icon: "trash") { [weak self] in
            guard let self, let channel = self.state.channel else { return }
            self.confirm("Delete channel", detail: "Delete #\(channel.name) for everyone? This channel and its messages will disappear from the space. This cannot be undone.", action: "Delete channel") { [state] in
                try await state.model.deleteChannel(channel); state.close()
            }
        }
        invite = Self.button("Invite") { [state] in state.invite() }; retry = Self.button("Retry loading members") { [state] in Task { await state.loadMembers() } }
        people.changed = { [state] in state.username = $0 }; people.submit = { [state] in state.invite() }; people.heightChanged = { [weak self] in self?.parent?.view.setNeedsLayout() }
        for stack in [membership, members, invitations] { stack.axis = .vertical; stack.spacing = 12 }
        [memberHeading, memberFailure, retry!, people, invite!, memberStatus, members, pendingHeading, invitations].forEach(membership.addArrangedSubview)
        [Self.label("Channel name", bold: true), name, Self.label("Channels are where conversations happen around a topic. Use a name that is easy to find and understand."), toggle, privacyDetail, failure, membership, delete!, save!].forEach(body.addArrangedSubview)
        observe(); Task { [state] in await state.loadMembers() }
    }
    override func viewDidAppear(_ animated: Bool) { super.viewDidAppear(animated); if state.channel == nil { name.becomeFirstResponder() } }
    func textFieldShouldReturn(_ textField: UITextField) -> Bool { state.save(); return false }
    private func nameChanged() {
        guard name.markedTextRange == nil else { return }
        state.name = WorkspaceValidation.normalizeChannelName(name.text ?? ""); if name.text != state.name { name.text = state.name }; state.error = nil
    }
    override func render() {
        guard state.current else { close(); return }
        if name.text != state.name, name.markedTextRange == nil { name.text = state.name }
        privacy.isOn = state.privateChannel
        privacyDetail.text = state.privateChannel ? "Only you and the people you add can view or join." : "Anyone in \(state.model.detail?.space.name ?? "this space") can view or join this channel."
        setTitle(state.channel == nil ? "Create a channel" : "Channel settings")
        if state.channel != nil, saveBar == nil {
            body.removeArrangedSubview(save); save.removeFromSuperview()
            let actions = UIStackView(arrangedSubviews: [reset, save]); actions.spacing = 8
            let bar = UIStackView(arrangedSubviews: [Self.label("You have unsaved changes."), actions]); bar.axis = .vertical; bar.spacing = 8
            bar.isLayoutMarginsRelativeArrangement = true; bar.layoutMargins = UIEdgeInsets(top: 12, left: 16, bottom: 12, right: 16)
            bar.backgroundColor = UIColor(CaperTheme.raised); bar.accessibilityIdentifier = "channel-save-bar"
            setFooter(bar); saveBar = bar; idealHeight = 600
        }
        saveBar?.isHidden = !state.dirty
        save.configuration?.title = state.pending ? (state.channel == nil ? "Creating…" : "Saving…") : state.channel == nil ? "Create channel" : "Save changes"
        save.isEnabled = state.canSave; reset.isEnabled = !state.pending
        delete.isHidden = state.channel == nil; delete.isEnabled = !state.pending
        failure.text = state.error; failure.isHidden = state.error == nil; membership.isHidden = state.channel?.private != true
        people.candidates = state.candidates; people.editable = !state.loadingMembers && state.membersError == nil
        if people.text != state.username { people.setText(state.username) }
        invite.isEnabled = people.editable && !state.pending && WorkspaceValidation.usernameError(state.username) == nil
        memberHeading.text = state.loadingMembers ? "Loading members…" : "Members \(state.members.count)"
        memberFailure.text = state.membersError ?? state.memberError; memberFailure.isHidden = memberFailure.text == nil
        retry.isHidden = state.membersError == nil; retry.isEnabled = !state.loadingMembers
        memberStatus.text = state.memberStatus; memberStatus.isHidden = state.memberStatus == nil; pendingHeading.text = "Pending invitations  \(state.invitations.count)"
        if !state.pending, state.refocusMember { state.refocusMember = false; people.field.becomeFirstResponder() }
        let next = state.members.map { state.model.profiled($0) }, pending = state.invitations.map { state.model.profiled($0) }
        let enabled = !state.pending && !state.loadingMembers && state.membersError == nil
        guard shownMembers != next || shownInvitations != pending || memberEnabled != enabled else { return }
        shownMembers = next; shownInvitations = pending; memberEnabled = enabled
        for stack in [members, invitations] { stack.arrangedSubviews.forEach { stack.removeArrangedSubview($0); $0.removeFromSuperview() } }
        for member in next {
            members.addArrangedSubview(Self.workspaceMember(member, action: member.owner ? nil : "Remove", enabled: enabled) { [weak self, state] in
                self?.confirm("Remove \(member.displayName) from #\(state.channel?.name ?? "channel")?", detail: "They’ll lose access to this private channel. You can add them again later.", action: "Remove") { try await state.remove(member) }
            })
        }
        for member in pending { invitations.addArrangedSubview(Self.workspaceMember(member, action: "Cancel", enabled: enabled) { [state] in state.run { try await state.remove(member) } }) }
    }
    private func confirm(_ title: String, detail: String, action: String, perform: @escaping () async throws -> Void) {
        guard !state.pending else { return }
        confirmWorkspaceAction(title, detail: detail, action: action) { [state] in
            guard state.current else { throw CancellationError() }; try await perform()
        }
    }
}

extension UIKitMessageDialog {
    fileprivate func confirmWorkspaceAction(_ title: String, detail: String, action: String, perform: @escaping () async throws -> Void) {
        guard presentedViewController == nil else { return }
        let confirmation = UIKitConfirmationDialog(title: title, detail: detail, action: action,
            pendingLabel: action.hasPrefix("Delete") ? "Deleting…" : "Removing…",
            close: { [weak self] in self?.presentedViewController?.dismiss(animated: false) }, perform: perform)
        UIKitHosting.sheet(confirmation, background: CaperTheme.surface)
        present(confirmation, animated: false)
    }
    static func workspaceField(_ placeholder: String, label: String) -> UITextField {
        let field = UITextField(); field.placeholder = placeholder; field.accessibilityLabel = label; field.borderStyle = .roundedRect
        field.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "Figtree-Regular", size: 14) ?? .systemFont(ofSize: 14)); field.adjustsFontForContentSizeCategory = true
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true; return field
    }
    static func workspaceMember(_ member: Member, action: String?, enabled: Bool, perform: @escaping () -> Void) -> UIView {
        let avatar = UIKitAvatar(side: 30); avatar.show(name: member.displayName, avatarID: member.avatarId, picture: member.avatarPicture)
        let names = UIStackView(arrangedSubviews: [label(member.displayName, size: 12, bold: true), label("@\(member.username)\(member.owner ? " · Owner" : "")", size: 11)])
        names.axis = .vertical; names.spacing = 1
        let row = UIStackView(arrangedSubviews: [avatar, names]); row.spacing = 10; row.alignment = .center
        if let action { let button = button(action, action: perform); button.isEnabled = enabled; button.setContentHuggingPriority(.required, for: .horizontal); row.addArrangedSubview(button) }
        return row
    }
}
#endif
