import Foundation
import Observation

/// Profile actions share the account-scoped draft with every entry point.
/// Opening a known incoming DM previews it; it must not implicitly accept it.
@MainActor @Observable final class NativeProfileActions {
    let model: AppModel
    let person: MentionCardPerson
    let draft: ProfileMessageDraft
    var choosingSpace = false
    private(set) var pending: String?
    private(set) var error: String?
    private(set) var status: String?
    private(set) var invited: [String: String] = [:]
    @ObservationIgnored private let close: () -> Void

    init(model: AppModel, person: MentionCardPerson, close: @escaping () -> Void) {
        self.model = model; self.person = person; self.close = close
        draft = model.profileMessageDraft(person: person)
    }
    var blocked: Bool { model.blockedIDs.contains(person.id) }
    var canMessage: Bool { !person.isSelf && !blocked && person.username != nil }
    var busy: Bool { pending != nil || draft.preventsImplicitDismissal }
    var ownedSpaces: [Space] {
        let open = model.detail.flatMap { detail in detail.members.contains { $0.id == person.id } ? detail.space.id : nil }
        return model.spaces.filter { $0.ownerId == model.account?.id && $0.demo != true && $0.id != open }
    }
    var canInvite: Bool { !person.isSelf && !blocked && person.username != nil && !ownedSpaces.isEmpty }
    var canBlock: Bool { !person.isSelf && (blocked || person.canBlock) }
    var blockTarget: BlockTarget {
        BlockTarget(id: person.id, username: person.username, displayName: person.title, avatarId: person.avatarId,
                    avatarAsset: person.picture?.asset, avatarUrl: person.picture?.url.absoluteString, admin: person.admin, team: person.team)
    }
    func openConversation() async {
        guard !busy, canMessage, let username = person.username else { return }
        pending = "message"; error = nil
        defer { pending = nil }
        await model.refreshDirectMessages()
        guard model.account != nil, !draft.discarded else { return }
        if let failure = model.directMessagesError { error = failure; return }
        do {
            if let conversation = model.directMessages.first(where: { $0.peer.id == person.id }) {
                await model.select(directMessage: conversation)
                close()
            } else if try await model.createDirectMessage(username: username) { close() }
            else { error = "Couldn’t open that conversation." }
        } catch { self.error = FriendlyError.message(for: error) }
    }
    func invite(_ space: Space) async {
        guard !busy, canInvite, invited[space.id] == nil, let username = person.username else { return }
        pending = space.id; error = nil; status = nil
        defer { pending = nil }
        do {
            try await model.addSpaceMember(username: username, spaceID: space.id)
            guard !draft.discarded else { return }
            invited[space.id] = "Invited"; status = "Invitation sent to \(space.name)."
        } catch {
            guard !draft.discarded else { return }
            let redundant = ["user already in space": "Member", "user already invited": "Invited"]
            if let already = (error as? APIError).flatMap({ redundant[$0.message] }) { invited[space.id] = already }
            self.error = FriendlyError.message(for: error)
        }
    }
    func unblock() async {
        guard !busy, blocked else { return }
        pending = "unblock"; error = nil
        defer { pending = nil }
        do {
            try await model.unblock(accountID: person.id)
            if !draft.discarded { status = "Unblocked." }
        } catch { if !draft.discarded { self.error = FriendlyError.message(for: error) } }
    }
}
