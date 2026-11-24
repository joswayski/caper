#if os(macOS)
import AppKit

@MainActor final class AppKitSpaceEditor: AppKitSettingsDialog, NSTextFieldDelegate {
    private let state: NativeSpaceEditing
    private let name = AppKitKitField()
    private let people = AppKitPeopleField(placeholder: "Exact username", identifier: "space-invite-username", listIdentifier: "space-invite-suggestions", normalize: WorkspaceValidation.normalizeUsername)
    private let membership = AppKitKit.stack([], spacing: 12)
    private let results = AppKitKit.label("", size: 12), failure = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let limit = AppKitSettingsDialog.note("You have reached your space limit.")
    private var save: AppKitKitButton!, invite: AppKitKitButton!, join: AppKitKitButton?
    private var shownMembers: [Member] = [], shownInvitations: [Member] = []
    private var memberPending: Bool?
    private var started = false
    private var icon: AppKitSpaceIcon?
    override var dismissDisabled: Bool { state.creating }
    init(model: AppModel, managing: Bool, inviteOnly: Bool = false, channelID: String? = nil, close: @escaping () -> Void, joinInvite: @escaping () -> Void) {
        state = NativeSpaceEditing(model: model, managing: managing, inviteOnly: inviteOnly, close: close)
        super.init(title: state.title, detail: inviteOnly ? model.detail?.space.name ?? "" : managing ? "Only the owner can change this space and its membership." : "",
            closeLabel: "Close", identifier: "space-editor", close: close)
        subviews.compactMap { $0 as? NSScrollView }.first?.setAccessibilityIdentifier("space-settings-scroll")
        name.stringValue = state.name; name.placeholderString = "Gaming nights"; name.delegate = self
        name.setAccessibilityLabel("Space name")
        save = AppKitKitButton(managing ? "Save name" : "Create space", kind: managing ? .secondary : .primary) { [state] in state.save() }
        invite = AppKitKitButton("Invite") { [state] in state.invite() }
        people.changed = { [state] in state.username = $0 }; people.submit = { [state] in state.invite() }
        var rows: [NSView] = []
        if !inviteOnly { rows += [Self.heading("Space name"), name, save] }
        if managing {
            if !inviteOnly, model.detail?.space.demo != true {
                let icon = AppKitSpaceIcon(model: model); self.icon = icon; rows += [AppKitKit.divider(), icon]
            }
            rows += [AppKitInviteLinks(state: NativeInviteLinks(model: model, showsDefault: inviteOnly, channelID: channelID)), AppKitKit.divider(), membership,
                Self.heading("Invite by username"), people, invite, results]
            if !inviteOnly {
                rows += [AppKitKit.divider(), Self.heading("Delete space"), Self.note("Delete this space and all its channels for every member."),
                    AppKitKitButton("Delete space", kind: .destructive, icon: "trash") { [weak self] in
                        self?.confirm("Delete space", detail: "Delete \(model.detail?.space.name ?? "space") for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", action: "Delete space") {
                            try await model.deleteCurrentSpace(); close()
                        }
                    }]
            }
        } else {
            rows.append(limit)
            let join = AppKitKitButton("Join with invite link", action: joinInvite); join.setAccessibilityIdentifier("join-invite-link")
            self.join = join
            rows += [AppKitKit.divider(), Self.note("Have an invite link?"), join]
        }
        rows.append(failure); setRows(rows); if managing { preferHeight(600) }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, !started {
            started = true; observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }); Task { [state] in await state.load() }
            if !state.managing { window?.makeFirstResponder(name) }
        } else if window == nil, started { observation.stop() }
    }
    func controlTextDidChange(_ notification: Notification) { state.name = name.stringValue; state.error = nil }
    private func render() {
        guard state.current else { state.close(); return }
        header.close.isEnabled = !state.creating; save.isEnabled = state.canSave
        save.title = state.pending ? (state.managing ? "Saving…" : "Creating…") : state.managing ? "Save name" : "Create space"
        join?.isEnabled = !state.pending; limit.isHidden = state.model.limits == nil || state.model.canCreateSpace
        people.candidates = state.candidates
        if people.text != state.username { people.setText(state.username) }
        invite.isEnabled = !state.pending && WorkspaceValidation.usernameError(state.username) == nil
        results.stringValue = state.memberError ?? state.memberStatus ?? ""; results.textColor = state.memberError == nil ? AppKitKit.muted : AppKitKit.error
        results.isHidden = results.stringValue.isEmpty
        failure.stringValue = state.error ?? ""; failure.isHidden = state.error == nil
        if !state.pending, state.refocusMember { state.refocusMember = false; window?.makeFirstResponder(people.field) }
        let members = state.model.detail?.members ?? [], invitations = state.model.pendingMembers
        guard shownMembers != members || shownInvitations != invitations || memberPending != state.pending else { return }
        shownMembers = members; shownInvitations = invitations; memberPending = state.pending
        membership.arrangedSubviews.forEach { $0.removeFromSuperview() }
        add(Self.heading("Pending invitations  \(invitations.count)"))
        for member in invitations { add(memberRow(member, action: "Cancel") { [state] in state.run { try await state.model.cancelSpaceInvitation(member) } }) }
        if !state.inviteOnly {
            let heading = Self.heading("Members  \(members.count)"); heading.setAccessibilityLabel("Members \(members.count)"); heading.setAccessibilityIdentifier("space-members-heading"); add(heading)
            for member in members {
                add(memberRow(member, action: member.owner ? nil : "Remove") { [weak self] in
                    self?.confirm("Remove \(member.displayName)?", detail: "They’ll lose access to \(self?.state.name ?? "this space") and its channels. You can invite them again later.", action: "Remove") { [weak self] in
                        guard let self, self.state.current else { throw CancellationError() }; try await self.state.model.removeSpaceMember(member)
                    }
                })
            }
        }
    }
    private func add(_ view: NSView) { membership.addArrangedSubview(view); AppKitKit.fillWidth(view, of: membership) }
    private func memberRow(_ member: Member, action: String?, perform: @escaping () -> Void) -> NSView {
        appKitWorkspaceMember(member, action: action, enabled: !state.pending, perform: perform)
    }
    private func confirm(_ title: String, detail: String, action: String, perform: @escaping () async throws -> Void) {
        guard !state.pending else { return }
        appKitConfirmWorkspaceAction(in: self, title: title, detail: detail, action: action) { [state] in
            guard state.current else { throw CancellationError() }; try await perform()
        }
    }
}

@MainActor final class AppKitChannelEditor: AppKitSettingsDialog, NSTextFieldDelegate {
    private let state: NativeChannelEditing
    private let name = AppKitKitField(), failure = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let membership = AppKitKit.stack([], spacing: 12), members = AppKitKit.stack([], spacing: 8), invitations = AppKitKit.stack([], spacing: 8)
    private let people = AppKitPeopleField(placeholder: "Exact username", identifier: "channel-invite-username", listIdentifier: "channel-member-suggestions", normalize: WorkspaceValidation.normalizeUsername)
    private let memberStatus = AppKitKit.label("", size: 12), memberFailure = AppKitKit.label("", size: 12, color: AppKitKit.error)
    private let memberHeading = AppKitKit.label("Members", size: 14, weight: .bold), pendingHeading = AppKitKit.label("", size: 14, weight: .bold)
    private let privacyDetail = AppKitSettingsDialog.note("")
    private var saveBar: NSStackView?
    private var privacy: AppKitKitSwitchRow!, invite: AppKitKitButton!, retry: AppKitKitButton!, save: AppKitKitButton!, reset: AppKitKitButton!, delete: AppKitKitButton!
    private var started = false
    private var shownMembers: [Member] = [], shownInvitations: [Member] = []
    private var memberEnabled: Bool?
    override var dismissDisabled: Bool { state.creating }
    init(model: AppModel, channel: Channel?, close: @escaping () -> Void) {
        state = NativeChannelEditing(model: model, channel: channel, close: close)
        super.init(title: channel == nil ? "Create a channel" : "Channel settings", detail: "", closeLabel: "Close", identifier: "channel-editor", close: close)
        subviews.compactMap { $0 as? NSScrollView }.first?.setAccessibilityIdentifier("channel-settings-scroll")
        name.stringValue = state.name; name.placeholderString = "project-updates"; name.delegate = self
        name.setAccessibilityLabel("Channel name")
        privacy = AppKitKitSwitchRow(title: "Private channel", detail: nil, identifier: "channel-private") { [state] in state.privateChannel = $0 }
        invite = AppKitKitButton("Invite") { [state] in state.invite() }; people.changed = { [state] in state.username = $0 }; people.submit = { [state] in state.invite() }
        retry = AppKitKitButton("Retry loading members") { [state] in Task { await state.loadMembers() } }
        save = AppKitKitButton("Create channel", kind: .primary) { [state] in state.save() }
        reset = AppKitKitButton("Reset") { [state] in state.reset() }
        delete = AppKitKitButton("Delete channel", kind: .destructive, icon: "trash") { [weak self] in
            guard let self, let channel = self.state.channel else { return }
            self.confirm("Delete channel", detail: "Delete #\(channel.name) for everyone? This channel and its messages will disappear from the space. This cannot be undone.", action: "Delete channel") {
                try await model.deleteChannel(channel); close()
            }
        }
        for row in [memberHeading, memberFailure, retry!, people, invite!, memberStatus, members, pendingHeading, invitations] as [NSView] {
            membership.addArrangedSubview(row); AppKitKit.fillWidth(row, of: membership)
        }
        setRows([Self.heading("Channel name"), name, Self.note("Channels are where conversations happen around a topic. Use a name that is easy to find and understand."), privacy, privacyDetail, failure, membership,
            delete, save])
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, !started {
            started = true; observation.start(reading: { [weak self] in self?.render() }, apply: { _ in }); Task { [state] in await state.loadMembers() }
            if state.channel == nil { window?.makeFirstResponder(name) }
        } else if window == nil, started { observation.stop() }
    }
    func controlTextDidChange(_ notification: Notification) {
        state.name = WorkspaceValidation.normalizeChannelName(name.stringValue); if name.stringValue != state.name { name.stringValue = state.name }; state.error = nil
    }
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.insertNewline(_:)) { state.save(); return true }; return false
    }
    private func render() {
        guard state.current else { state.close(); return }
        if name.stringValue != state.name { name.stringValue = state.name }
        privacy.isOn = state.privateChannel; header.close.isEnabled = !state.creating
        privacyDetail.stringValue = state.privateChannel ? "Only you and the people you add can view or join." : "Anyone in \(state.model.detail?.space.name ?? "this space") can view or join this channel."
        header.titleLabel.stringValue = state.channel == nil ? "Create a channel" : "Channel settings"
        if state.channel != nil, saveBar == nil {
            save.removeFromSuperview()
            let actions = NSStackView(views: [reset, save]); actions.spacing = 8
            let bar = AppKitKit.stack([Self.note("You have unsaved changes."), actions], spacing: 8)
            bar.edgeInsets = NSEdgeInsets(top: 12, left: 22, bottom: 12, right: 22)
            bar.wantsLayer = true; bar.layer?.backgroundColor = AppKitKit.raised.cgColor
            bar.setAccessibilityIdentifier("channel-save-bar"); setFooter(bar); saveBar = bar; preferHeight(600)
        }
        saveBar?.isHidden = !state.dirty
        save.title = state.pending ? (state.channel == nil ? "Creating…" : "Saving…") : state.channel == nil ? "Create channel" : "Save changes"
        save.isEnabled = state.canSave; reset.isEnabled = !state.pending
        delete.isHidden = state.channel == nil; delete.isEnabled = !state.pending
        failure.stringValue = state.error ?? ""; failure.isHidden = state.error == nil
        membership.isHidden = state.channel?.private != true
        people.candidates = state.candidates; people.editable = !state.loadingMembers && state.membersError == nil
        if people.text != state.username { people.setText(state.username) }
        invite.isEnabled = people.editable && !state.pending && WorkspaceValidation.usernameError(state.username) == nil
        memberHeading.stringValue = state.loadingMembers ? "Loading members…" : "Members \(state.members.count)"
        memberFailure.stringValue = state.membersError ?? state.memberError ?? ""; memberFailure.isHidden = memberFailure.stringValue.isEmpty
        retry.isHidden = state.membersError == nil; retry.isEnabled = !state.loadingMembers
        memberStatus.stringValue = state.memberStatus ?? ""; memberStatus.isHidden = state.memberStatus == nil
        pendingHeading.stringValue = "Pending invitations  \(state.invitations.count)"
        if !state.pending, state.refocusMember { state.refocusMember = false; window?.makeFirstResponder(people.field) }
        let profiled = state.members.map(state.model.profiled), pending = state.invitations.map(state.model.profiled)
        let enabled = !state.pending && !state.loadingMembers && state.membersError == nil
        guard shownMembers != profiled || shownInvitations != pending || memberEnabled != enabled else { return }
        shownMembers = profiled; shownInvitations = pending; memberEnabled = enabled
        for stack in [members, invitations] { stack.arrangedSubviews.forEach { $0.removeFromSuperview() } }
        for member in profiled {
            let row = appKitWorkspaceMember(member, action: member.owner ? nil : "Remove", enabled: enabled) { [weak self, state] in
                self?.confirm("Remove \(member.displayName) from #\(state.channel?.name ?? "channel")?", detail: "They’ll lose access to this private channel. You can add them again later.", action: "Remove") { try await state.remove(member) }
            }
            members.addArrangedSubview(row); AppKitKit.fillWidth(row, of: members)
        }
        for member in pending {
            let row = appKitWorkspaceMember(member, action: "Cancel", enabled: enabled) { [state] in state.run { try await state.remove(member) } }
            invitations.addArrangedSubview(row); AppKitKit.fillWidth(row, of: invitations)
        }
    }
    private func confirm(_ title: String, detail: String, action: String, perform: @escaping () async throws -> Void) {
        guard !state.pending else { return }
        appKitConfirmWorkspaceAction(in: self, title: title, detail: detail, action: action) { [state] in
            guard state.current else { throw CancellationError() }; try await perform()
        }
    }
}

/// Keep the confirmation open through the request and any retryable failure.
@MainActor private func appKitConfirmWorkspaceAction(in parent: NSView, title: String, detail: String, action: String,
                                                    perform: @escaping () async throws -> Void) {
    guard let window = parent.window, window.attachedSheet == nil else { return }
    let sheet = NSWindow()
    let content = AppKitConfirmationDialog(title: title, detail: detail, action: action, close: { [weak sheet] in
        guard let sheet else { return }; sheet.sheetParent?.endSheet(sheet); sheet.orderOut(nil)
    }, perform: perform)
    let width = min(500, max(1, window.contentLayoutRect.width - 32))
    content.widthAnchor.constraint(equalToConstant: width).isActive = true
    sheet.contentView = content; content.layoutSubtreeIfNeeded()
    sheet.setContentSize(NSSize(width: width, height: ceil(content.fittingSize.height)))
    sheet.initialFirstResponder = content.initialFocus
    window.beginSheet(sheet)
    // A sheet doesn't apply initialFirstResponder; start on Cancel explicitly.
    if let focus = content.initialFocus { sheet.makeFirstResponder(focus) }
}

@MainActor private func appKitWorkspaceMember(_ member: Member, action: String?, enabled: Bool, perform: @escaping () -> Void) -> NSView {
    let avatar = AppKitAvatar(); avatar.update(name: member.displayName, avatarId: member.avatarId, picture: member.avatarPicture)
    avatar.translatesAutoresizingMaskIntoConstraints = false; avatar.widthAnchor.constraint(equalToConstant: 30).isActive = true; avatar.heightAnchor.constraint(equalToConstant: 30).isActive = true
    let names = AppKitKit.stack([AppKitKit.label(member.displayName, size: 12, weight: .bold), AppKitKit.label("@\(member.username)\(member.owner ? " · Owner" : "")", size: 11, color: AppKitKit.muted)], spacing: 1)
    let spacer = NSView(); spacer.setContentHuggingPriority(.init(1), for: .horizontal)
    let row = NSStackView(views: [avatar, names, spacer]); row.spacing = 10
    if let action { let button = AppKitKitButton(action, action: perform); button.isEnabled = enabled; row.addArrangedSubview(button) }
    return row
}
#endif
