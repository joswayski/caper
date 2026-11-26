import Foundation
import Observation

/// Native forms retain their drafts while membership and invite lists refresh.
/// AppModel remains the authority for validation, permissions and mutations.
@MainActor @Observable final class NativeSpaceEditing {
    let model: AppModel
    let managing: Bool
    let inviteOnly: Bool
    let close: () -> Void
    let spaceID: String?
    private let epoch: Int
    var name: String
    var username = "" { didSet { if username != oldValue { memberStatus = nil; memberError = nil } } }
    var pending = false
    var error: String?
    var memberStatus: String?
    var memberError: String?
    var refocusMember = false
    init(model: AppModel, managing: Bool, inviteOnly: Bool = false, close: @escaping () -> Void) {
        self.model = model; self.managing = managing; self.inviteOnly = inviteOnly; self.close = close
        spaceID = model.detail?.space.id; epoch = model.accountEpoch; name = managing ? model.detail?.space.name ?? "" : ""
    }
    var current: Bool { epoch == model.accountEpoch && (!managing || (model.detail?.space.id == spaceID && model.isOwner)) }
    var title: String { inviteOnly ? "Invite people" : managing ? "Manage space" : "Create a space" }
    var creating: Bool { !managing && pending }
    var candidates: [MentionCandidate] { PeopleSuggestions.spaceInvite(people: model.people, members: model.detail?.members ?? [], invited: model.pendingMembers, accountID: model.account?.id) }
    var canSave: Bool { current && !pending && (managing ? name.trimmingCharacters(in: .whitespacesAndNewlines) != model.detail?.space.name : model.limits == nil || model.canCreateSpace) }
    func load() async {
        guard current, managing else { return }
        do { try await model.loadSpaceInvitations() } catch { if current { self.error = FriendlyError.message(for: error) } }
        await model.refreshPeople()
    }
    func save() {
        guard canSave else { return }; let name = name
        run { [self] in
            if managing { try await model.renameSpace(name) }
            else { try await model.createSpace(name: name); pending = false; close() }
        }
    }
    func invite() {
        guard current, !pending, WorkspaceValidation.usernameError(username) == nil else { return }
        let submitted = username; memberStatus = nil; memberError = nil
        if let error = WorkspaceValidation.inviteError(username: submitted, members: model.detail?.members ?? [], invited: model.pendingMembers, channel: false) { memberError = error; return }
        pending = true; error = nil; refocusMember = true
        Task {
            defer { pending = false }
            do { try await model.addSpaceMember(username: submitted); if current { if username == submitted { username = "" }; memberStatus = WorkspaceValidation.invitationSent } }
            catch { if current { memberError = WorkspaceValidation.memberMessage(error) } }
        }
    }
    func run(_ perform: @escaping () async throws -> Void) {
        guard current, !pending else { return }; pending = true; error = nil
        Task { defer { pending = false }; do { try await perform() } catch { if current { self.error = FriendlyError.message(for: error) } } }
    }
}

@MainActor @Observable final class NativeChannelEditing {
    let model: AppModel
    let close: () -> Void
    private let spaceID: String?
    private let epoch: Int
    var channel: Channel?
    var name: String
    var privateChannel: Bool
    var username = "" { didSet { if username != oldValue { memberStatus = nil; memberError = nil } } }
    var pending = false
    var error: String?
    var members: [Member] = []
    var invitations: [Member] = []
    var loadingMembers = false
    var membersError: String?
    var memberError: String?
    var memberStatus: String?
    var refocusMember = false
    init(model: AppModel, channel: Channel?, close: @escaping () -> Void) {
        self.model = model; self.channel = channel; self.close = close
        spaceID = model.detail?.space.id; epoch = model.accountEpoch; name = channel?.name ?? ""; privateChannel = channel?.private ?? false
    }
    var current: Bool { model.accountEpoch == epoch && model.detail?.space.id == spaceID && model.isOwner }
    var creating: Bool { channel == nil && pending }
    var dirty: Bool { channel.map { name != $0.name || privateChannel != $0.private } ?? false }
    var candidates: [MentionCandidate] {
        guard !loadingMembers, membersError == nil else { return [] }
        return PeopleSuggestions.channelGrant(spaceMembers: model.detail?.members ?? [], channelMembers: members, invited: invitations, accountID: model.account?.id)
    }
    var canSave: Bool { current && !pending && (channel == nil ? !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty : dirty) }
    func reset() { guard !pending else { return }; name = channel?.name ?? ""; privateChannel = channel?.private ?? false; error = nil }
    func save() {
        guard canSave else { return }; let name = name, privateChannel = privateChannel
        run { [self] in
            let updated: Channel?
            if let existing = channel { updated = try await model.updateChannel(existing, name: name, privateChannel: privateChannel) }
            else { updated = try await model.createChannel(name: name, privateChannel: privateChannel) }
            guard current else { return }
            if channel != nil || updated?.private == true {
                let wasPrivate = channel?.private == true
                channel = updated; self.name = updated?.name ?? name; self.privateChannel = updated?.private ?? privateChannel
                if updated?.private == true, !wasPrivate { await loadMembers() }
            } else { pending = false; close() }
        }
    }
    func loadMembers() async {
        guard current, !loadingMembers, let channel, channel.private else { return }
        loadingMembers = true; membersError = nil; defer { loadingMembers = false }
        do {
            let members = try await model.channelMembers(channel), invitations = try await model.channelInvitations(channel)
            if current { self.members = members; self.invitations = invitations }
        } catch { if current { membersError = FriendlyError.message(for: error) } }
    }
    func invite() {
        guard current, !pending, !loadingMembers, membersError == nil, let channel else { return }
        guard WorkspaceValidation.usernameError(username) == nil else { memberError = "Enter an exact username."; return }
        let submitted = username; memberStatus = nil; memberError = nil
        if let error = WorkspaceValidation.inviteError(username: submitted, members: members, invited: invitations, channel: true) { memberError = error; return }
        pending = true; error = nil; refocusMember = true
        Task {
            defer { pending = false }
            do {
                let member = try await model.addChannelMember(channel, username: submitted)
                guard current else { return }
                invitations.removeAll { $0.id == member.id }; invitations.append(member)
                if username == submitted { username = "" }; memberStatus = WorkspaceValidation.invitationSent
            } catch { if current { memberError = WorkspaceValidation.memberMessage(error) } }
        }
    }
    func remove(_ member: Member) async throws {
        guard current, let channel else { throw CancellationError() }
        try await model.removeChannelMember(channel, member: member)
        if current { members.removeAll { $0.id == member.id }; invitations.removeAll { $0.id == member.id } }
    }
    func run(_ perform: @escaping () async throws -> Void) {
        guard current, !pending else { return }; pending = true; error = nil
        Task { defer { pending = false }; do { try await perform() } catch { if current { self.error = FriendlyError.message(for: error) } } }
    }
}

/// A retained invite section: opening prepares the default without copying it.
/// Every link owns its original settings; custom choices only affect new links.
@MainActor @Observable final class NativeInviteLinks {
    let model: AppModel
    let defaultChannel: String?
    let showsDefault: Bool
    private let spaceID: String?
    private let epoch: Int
    var defaultLink: InviteLink?
    var deactivated = false
    var expanded = false
    var links: [InviteLink] = []
    var loading = false
    var pending: String?
    var error: String?
    var copied: String?
    var expiryNumber = InviteLinks.defaultExpiryNumber
    var expiryUnit = InviteLinks.defaultExpiryUnit
    var maxUses = InviteLinks.defaultMaxUses
    var expiryError: String?
    var maxUsesError: String?
    var channelID: String?
    private var copiedReset: Task<Void, Never>?
    init(model: AppModel, showsDefault: Bool, channelID: String?) {
        self.model = model; self.showsDefault = showsDefault; defaultChannel = channelID
        spaceID = model.detail?.space.id; epoch = model.accountEpoch; self.channelID = model.detail?.channels.first { !$0.private }?.id
    }
    var current: Bool { model.accountEpoch == epoch && model.detail?.space.id == spaceID && model.isOwner }
    var busy: Bool { loading || pending != nil }
    var publicChannels: [Channel] { (model.detail?.channels ?? []).filter { !$0.private } }
    func load() async {
        guard current, !busy else { return }; loading = true; error = nil; defer { loading = false }
        do {
            if showsDefault, defaultLink == nil, !deactivated { defaultLink = try await model.defaultInviteLink(channelID: defaultChannel) }
            links = try await model.loadInviteLinks()
        } catch is CancellationError {} catch { if current { self.error = FriendlyError.message(for: error) } }
    }
    func prepare() { run("prepare") { [self] in defaultLink = try await model.defaultInviteLink(channelID: defaultChannel); deactivated = false; copied = nil } }
    func regenerate(_ link: InviteLink) {
        run(link.code) { [self] in
            let replacement = try await model.regenerateInviteLink(link)
            if defaultLink?.code == link.code { defaultLink = replacement; deactivated = false; copied = nil }
            links = try await model.loadInviteLinks()
        }
    }
    func revoke(_ link: InviteLink) {
        run(link.code) { [self] in
            do { try await model.revokeInviteLink(link) } catch let error as APIError where error.status == 404 {}
            links.removeAll { $0.code == link.code }
            if defaultLink?.code == link.code { defaultLink = nil; deactivated = true; copied = nil }
        }
    }
    func create() {
        guard current, !busy else { return }
        let expiry = InviteLinks.expirySeconds(expiryNumber, unit: expiryUnit), uses = InviteLinks.maxUses(maxUses)
        if case .failure(let error) = expiry { expiryError = error.message } else { expiryError = nil }
        if case .failure(let error) = uses { maxUsesError = error.message } else { maxUsesError = nil }
        guard case .success(let age) = expiry, case .success(let limit) = uses else { return }
        let channel = channelID.flatMap { id in publicChannels.contains { $0.id == id } ? id : nil } ?? publicChannels.first?.id
        run("create") { [self] in
            let link = try await model.createInviteLink(channelID: channel, maxAge: age, maxUses: limit)
            links.removeAll { $0.code == link.code }; links.insert(link, at: 0); copy(link, briefly: true)
        }
    }
    func copy(_ link: InviteLink, briefly: Bool = false) {
        guard current else { return }
        #if os(macOS)
        NSPasteboard.general.clearContents()
        guard NSPasteboard.general.setString(model.inviteLinkURL(link.code), forType: .string) else { error = "Copy failed; try again."; return }
        #else
        UIPasteboard.general.string = model.inviteLinkURL(link.code)
        #endif
        copied = link.code; copiedReset?.cancel()
        if briefly { copiedReset = Task { [weak self] in do { try await Task.sleep(for: .seconds(2)) } catch { return }; self?.copied = nil } }
    }
    func run(_ key: String, _ perform: @escaping () async throws -> Void) {
        guard current, !busy else { return }; pending = key; error = nil
        Task { defer { pending = nil }; do { try await perform() } catch is CancellationError {} catch { if current { self.error = FriendlyError.message(for: error) } } }
    }
    isolated deinit { copiedReset?.cancel() }
}

/// Web's VanityLinkSettings: one readable link per space that never expires.
/// A name the space used stays reserved for it, so a shared link can't later
/// lead to someone else's space.
@MainActor @Observable final class NativeVanityLink {
    let model: AppModel
    private let spaceID: String?
    private let epoch: Int
    var current: String?
    var name = ""
    var loaded = false
    var pending = false
    var copied = false
    var error: String?
    private var copiedReset: Task<Void, Never>?
    init(model: AppModel) { self.model = model; spaceID = model.detail?.space.id; epoch = model.accountEpoch }
    var live: Bool { model.accountEpoch == epoch && model.detail?.space.id == spaceID && model.isOwner }
    var changed: Bool { name != (current ?? "") }
    /// What a confirmation shows before replacing or turning off a live name.
    var currentURL: String { current.map(model.inviteLinkURL) ?? "" }
    var prefix: String { model.inviteLinkURL("").replacingOccurrences(of: "https://", with: "").replacingOccurrences(of: "http://", with: "") }
    func load() async {
        guard live, !loaded else { return }
        do { current = try await model.vanityLink(); name = current ?? "" } catch is CancellationError {} catch { if live { self.error = FriendlyError.message(for: error) } }
        loaded = true
    }
    func setName(_ text: String) { name = InviteLinks.vanityInput(text); error = nil }
    /// Nil when it is ready to save; "confirm" when it replaces a live name.
    func check() -> String? {
        guard changed, !name.isEmpty else { return "none" }
        if let invalid = InviteLinks.vanityNameError(name) { error = invalid; return invalid }
        return current == nil ? nil : "confirm"
    }
    func apply(_ code: String?) {
        guard live, !pending else { return }; pending = true; error = nil
        Task {
            defer { pending = false }
            do { current = try await model.vanityLink(change: .some(code)); name = current ?? "" }
            catch is CancellationError {} catch { if live { self.error = FriendlyError.message(for: error) } }
        }
    }
    func copy() {
        guard let current else { return }
        #if os(macOS)
        NSPasteboard.general.clearContents()
        guard NSPasteboard.general.setString(model.inviteLinkURL(current), forType: .string) else { error = "Copy failed; try again."; return }
        #else
        UIPasteboard.general.string = model.inviteLinkURL(current)
        #endif
        copied = true; copiedReset?.cancel()
        copiedReset = Task { [weak self] in do { try await Task.sleep(for: .seconds(2)) } catch { return }; self?.copied = false }
    }
    isolated deinit { copiedReset?.cancel() }
}

#if os(macOS)
import AppKit
#else
import UIKit
#endif
