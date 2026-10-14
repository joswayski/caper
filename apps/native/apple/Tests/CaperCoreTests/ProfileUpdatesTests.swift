import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest
@testable import CaperCore

private let oldURL = "https://cdn.caper.chat/assets/Pic0000000000000old1?exp=1&sig=old"
private let newURL = "https://cdn.caper.chat/assets/Pic0000000000000new1?exp=2&sig=new"

final class ProfileUpdatesTests: XCTestCase {
    /// Maya renamed herself and changed her picture.
    private let maya = Person(id: "Maya00000001", username: "maya_r", displayName: "Maya R", avatarId: 31,
                              avatarAsset: "Pic0000000000000new1", avatarUrl: newURL)

    private var directory: ProfileDirectory { ProfileDirectory([maya]) }

    // MARK: Events

    func testFeedEventsParseAndUnknownOrMalformedOnesAreIgnored() throws {
        let person = ProfileFeedEvent.parse(["type": "profile.updated", "person": [
            "id": "Maya00000001", "username": "maya_r", "displayName": "Maya R", "avatarId": 31,
            "avatarAsset": "Pic0000000000000new1", "avatarUrl": newURL,
        ]])
        XCTAssertEqual(person, .person(maya))
        let removed = ProfileFeedEvent.parse(["type": "profile.updated", "person": ["id": "Maya00000001", "username": "maya", "displayName": "Maya"]])
        guard case .person(let plain)? = removed else { return XCTFail("expected a person") }
        XCTAssertNil(plain.avatarPicture, "absent picture fields mean the picture was removed")

        let space = ProfileFeedEvent.parse(["type": "space.updated", "space": [
            "id": "Space1234567", "name": "Friends", "ownerId": "Owner0000001", "iconAsset": "Pic0000000000000new1", "iconUrl": newURL,
        ]])
        guard case .space(let friends)? = space else { return XCTFail("expected a space") }
        XCTAssertEqual(friends.name, "Friends")
        XCTAssertEqual(friends.iconPicture?.asset, "Pic0000000000000new1")

        XCTAssertNil(ProfileFeedEvent.parse(["type": "profile.deleted", "person": ["id": "x", "username": "x", "displayName": "X"]]))
        XCTAssertNil(ProfileFeedEvent.parse(["type": "profile.updated", "person": "Maya"]), "a non-object never reaches the decoder")
        XCTAssertNil(ProfileFeedEvent.parse(["type": "profile.updated", "person": ["id": "x"]]))
        XCTAssertNil(ProfileFeedEvent.parse(["type": "space.updated"]))
        XCTAssertNil(ProfileFeedEvent.parse(["type": "subscription.error", "status": 400]))
    }

    // MARK: People

    func testAnUpdateReachesEveryRetainedCopyOfThatPersonOnly() {
        let profiles = directory
        let member = Member(id: maya.id, username: "maya", displayName: "Maya", owner: true, avatarId: 31,
                            avatarAsset: "Pic0000000000000old1", avatarUrl: oldURL)
        let updated = profiles.apply(member)
        XCTAssertEqual(updated.displayName, "Maya R")
        XCTAssertEqual(updated.username, "maya_r")
        XCTAssertTrue(updated.owner, "membership facts stay")
        XCTAssertEqual(updated.avatarPicture?.asset, "Pic0000000000000new1")
        let other = Member(id: "Other0000001", username: "other", displayName: "Other", owner: false)
        XCTAssertEqual(profiles.apply(other), other)

        XCTAssertEqual(profiles.apply(Person(id: maya.id, username: "maya", displayName: "Maya")).displayName, "Maya R")
        let peer = DirectMessagePeer(id: maya.id, username: "maya", displayName: "Maya", avatarId: 31, team: true)
        let conversation = profiles.apply(DirectMessageConversation(id: "Dm0000000001", peer: peer, lastSeq: "4", readSeq: "2", status: .incoming, blocked: true))
        XCTAssertEqual(conversation.peer.displayName, "Maya R")
        XCTAssertEqual(conversation.peer.avatarPicture?.asset, "Pic0000000000000new1")
        XCTAssertTrue(conversation.peer.isTeam)
        XCTAssertEqual([conversation.lastSeq, conversation.readSeq], ["4", "2"], "cursors never change")
        XCTAssertEqual(conversation.status, .incoming)
        XCTAssertTrue(conversation.blocked)

        XCTAssertEqual(profiles.apply(ReactorPerson(id: maya.id, username: nil, displayName: nil)).name, "Maya R")
        XCTAssertEqual(profiles.apply(BlockedAccount(id: maya.id, username: "maya", displayName: "Maya")).username, "maya_r")
        let report = FeedbackReport(id: "Report000000001", messageId: "Report000000001", threadRootId: "Report000000001", createdAt: "now", text: "Hi",
                                    reporter: FeedbackReport.Reporter(id: maya.id, username: "maya", displayName: "Maya"),
                                    channel: FeedbackReport.Conversation(id: "Chan12345678", name: "general", direct: false), space: nil)
        XCTAssertEqual(profiles.apply(report).reporter.displayName, "Maya R")
        XCTAssertEqual(profiles.apply(report).text, "Hi")

        let detail = profiles.apply(SpaceDetail(space: Space(id: "Space1234567", name: "Studio", ownerId: maya.id), channels: [], members: [other, member],
                                                channelMembers: ["Priv12345678": [maya.id]]))
        XCTAssertEqual(detail.members.map(\.displayName), ["Other", "Maya R"])
        // A private channel's readers keep leading its member list.
        XCTAssertEqual(detail.channelMembers, ["Priv12345678": [maya.id]])
        XCTAssertEqual(detail.memberOrder(channelID: "Priv12345678").map(\.displayName), ["Maya R", "Other"])
        XCTAssertEqual(MentionCandidate(detail.members[1]).picture?.asset, "Pic0000000000000new1", "suggestions follow the members")
    }

    func testRemovingAPictureClearsItAndTheSavedMascotStays() {
        let removed = ProfileDirectory([Person(id: maya.id, username: "maya", displayName: "Maya")])
        let member = Member(id: maya.id, username: "maya", displayName: "Maya", owner: false, avatarId: 31,
                            avatarAsset: "Pic0000000000000old1", avatarUrl: oldURL)
        let updated = removed.apply(member)
        XCTAssertNil(updated.avatarPicture)
        XCTAssertEqual(updated.avatarId, 31, "an event without avatarId keeps the saved mascot")
        // The same picture keeps a snapshot's (possibly fresher) signature.
        let same = Member(id: maya.id, username: "maya", displayName: "Maya", owner: false, avatarAsset: "Pic0000000000000new1", avatarUrl: oldURL)
        XCTAssertEqual(directory.apply(same).avatarUrl, oldURL)
    }

    func testMessageAuthorsUpdateIncludingNestedCopiesButNeverText() {
        let old = ChatAuthor(id: maya.id, name: "Maya", isGuest: false, avatarId: 31, admin: true)
        let other = ChatAuthor(id: "Other0000001", name: "Other", isGuest: false)
        let original = ChatMessage(id: "Original0000001", channelId: "Chan12345678", seq: "3", author: old,
                                   content: ChatContent(version: 1, type: "text", text: "Hello from Maya"), createdAt: "now", clientMessageId: "c3")
        var message = ChatMessage(id: "Message00000001", channelId: "Chan12345678", seq: "9", author: other,
                                  content: ChatContent(version: 1, type: "text", text: "@maya hi"), createdAt: "now", clientMessageId: "c9")
        message.replyTo = MessageReply(message: original)
        message.pin = MessagePin(author: old, createdAt: "now")
        message.thread = ThreadSummary(replyCount: 2, participants: [other, old], seq: "10")
        message.forward = MessageForward(message: original, seq: "11")

        let updated = directory.apply(message)
        XCTAssertEqual(updated.author, other)
        XCTAssertEqual(updated.replyTo?.author.name, "Maya R")
        XCTAssertEqual(updated.replyTo?.text, "Hello from Maya")
        XCTAssertEqual(updated.pin?.author.name, "Maya R")
        XCTAssertEqual(updated.thread?.participants.map(\.name), ["Other", "Maya R"])
        XCTAssertEqual(updated.forward?.message?.author.name, "Maya R")
        XCTAssertEqual(updated.forward?.message?.author.avatarPicture?.asset, "Pic0000000000000new1")
        XCTAssertEqual(updated.forward?.message?.content.text, "Hello from Maya")
        XCTAssertEqual(updated.content.text, "@maya hi", "message text, including mentions, never changes")
        XCTAssertEqual(updated.seq, "9")
        XCTAssertEqual(directory.apply(original).author.admin, true, "the admin badge is the server's")

        let guest = ChatAuthor(id: maya.id, name: "Guest", isGuest: true)
        XCTAssertEqual(directory.apply(guest), guest)
        XCTAssertEqual(ProfileDirectory().apply(message), message)
    }

    func testYourOwnMessagesTakeYourProfileAndKeepTheirAdminBadge() {
        let stamped = ChatAuthor(id: maya.id, name: "Maya", isGuest: false, avatarId: 31, admin: true)
        let own = ChatAuthor(id: maya.id, name: "Maya R", isGuest: false, avatarId: 31)
        var message = ChatMessage(id: "Message00000001", channelId: "Chan12345678", seq: "9", author: stamped,
                                  content: ChatContent(version: 1, type: "text", text: "Answered"), createdAt: "now", clientMessageId: "c9")
        message.pin = MessagePin(author: stamped, createdAt: "now")
        let updated = message.withProfile(own)
        XCTAssertEqual(updated.author.name, "Maya R")
        XCTAssertEqual(updated.author.admin, true, "the server's admin stamp survives your own profile")
        XCTAssertEqual(updated.pin?.author.admin, true)
        let plain = ChatMessage(id: "Message00000002", channelId: "Chan12345678", seq: "10", author: own,
                                content: ChatContent(version: 1, type: "text", text: "Hi"), createdAt: "now", clientMessageId: "c10")
        XCTAssertNil(plain.withProfile(own).author.admin)
    }

    func testYourOwnAccountTakesAnUpdateFromAnotherDevice() {
        let account = Account(id: maya.id, username: "maya", displayName: "Maya", avatarId: 31, avatarAsset: "Pic0000000000000old1", avatarUrl: oldURL)
        let updated = account.applying(maya)
        XCTAssertEqual(updated.username, "maya_r")
        XCTAssertEqual(updated.displayName, "Maya R")
        XCTAssertEqual(updated.avatarPicture?.asset, "Pic0000000000000new1")
        XCTAssertEqual(updated.avatarId, 31)
        XCTAssertNil(account.applying(Person(id: maya.id, username: "maya", displayName: "Maya")).avatarAsset)
        XCTAssertEqual(account.applying(Person(id: "Other0000001", username: "x", displayName: "X")), account)
    }

    // MARK: Self-healing

    func testAMissedEventIsRepairedByTheNextRefresh() {
        let p1 = Person(id: maya.id, username: "maya", displayName: "Maya P1", avatarId: 31, avatarAsset: "Pic0000000000000old1", avatarUrl: oldURL)
        let p2 = Person(id: maya.id, username: "maya_p2", displayName: "Maya P2", avatarId: 31, avatarAsset: "Pic0000000000000new1", avatarUrl: newURL)
        // An author loaded long before either change.
        let author = ChatAuthor(id: maya.id, name: "Maya", isGuest: false, avatarId: 31)
        var profiles = ProfileDirectory()
        profiles.record(p1)
        XCTAssertEqual(profiles.apply(author).name, "Maya P1")
        // P2 is missed (no replay after a reconnect); the next refresh carries it.
        let mark = profiles.mark
        let refreshed = [Member(id: maya.id, username: "maya_p2", displayName: "Maya P2", owner: false, avatarId: 31,
                                avatarAsset: "Pic0000000000000new1", avatarUrl: newURL).profile]
        profiles.recordFetched(refreshed, since: mark)
        let shown = profiles.apply(author)
        XCTAssertEqual(shown.name, "Maya P2")
        XCTAssertEqual(shown.avatarPicture?.asset, "Pic0000000000000new1")
        XCTAssertEqual(profiles[maya.id], p2)
        // A refresh without the picture clears it: absent means none.
        profiles.recordFetched([Person(id: maya.id, username: "maya_p2", displayName: "Maya P2", avatarId: 31)], since: profiles.mark)
        XCTAssertNil(profiles.apply(author).avatarPicture)
    }

    func testAnEventDuringAFetchWinsAndOldSessionsChangeNothing() {
        var profiles = ProfileDirectory()
        let mark = profiles.mark
        profiles.record(maya)  // arrives while the request is in flight
        profiles.recordFetched([Person(id: maya.id, username: "maya", displayName: "Maya (stale)")], since: mark)
        XCTAssertEqual(profiles[maya.id]?.displayName, "Maya R", "the in-flight result is older than the event")
        // Fresh lists also cover people with no event yet.
        profiles.recordFetched([DirectMessagePeer(id: "Sam000000001", username: "sam", displayName: "Sam").profile], since: profiles.mark)
        XCTAssertEqual(profiles.apply(ChatAuthor(id: "Sam000000001", name: "Old Sam", isGuest: false)).name, "Sam")
        // A fetch started before a logout cannot reach the next session's directory.
        var next = ProfileDirectory()
        next.recordFetched([maya], since: mark)
        XCTAssertTrue(next.isEmpty)
    }

    // MARK: Spaces

    func testSpaceUpdatesRenameAndChangeTheIconOfThatSpaceOnly() {
        var studio = Space(id: "Space1234567", name: "Studio", ownerId: "Owner0000001")
        studio.inviter = Inviter(username: "owner", displayName: "Owner")
        let other = Space(id: "Other1234567", name: "Other", ownerId: "Owner0000001")
        var update = Space(id: "Space1234567", name: "Friends", ownerId: "Owner0000001")
        update.iconAsset = "Pic0000000000000new1"; update.iconUrl = newURL

        let renamed = [studio, other].map { $0.applying(update) }
        XCTAssertEqual(renamed.map(\.name), ["Friends", "Other"])
        XCTAssertEqual(renamed[0].iconPicture?.asset, "Pic0000000000000new1")
        XCTAssertEqual(renamed[0].inviter, studio.inviter, "list-only fields stay")
        XCTAssertNil(renamed[1].iconPicture)
        let unknown = Space(id: "Unknown12345", name: "Elsewhere", ownerId: "x")
        XCTAssertEqual([studio, other].map { $0.applying(unknown) }, [studio, other], "spaces you don't have change nothing")

        let removed = renamed[0].applying(Space(id: "Space1234567", name: "Friends", ownerId: "Owner0000001"))
        XCTAssertNil(removed.iconPicture, "absent icon fields mean the icon was removed")
        let detail = SpaceDetail(space: studio, channels: [Channel(id: "Chan12345678", spaceId: studio.id, name: "general", private: false)], members: [],
                                 channelMembers: ["Priv12345678": ["Owner0000001"]])
        XCTAssertEqual(detail.applying(update).space.name, "Friends")
        XCTAssertEqual(detail.applying(update).channels.count, 1)
        XCTAssertEqual(detail.applying(update).channelMembers, ["Priv12345678": ["Owner0000001"]])
    }

    func testSpaceAndProfileUpdatesKeepPrivateChannelMembers() {
        let studio = Space(id: "Space1234567", name: "Studio", ownerId: maya.id)
        let secret = Channel(id: "Chan12345678", spaceId: studio.id, name: "secret", private: true)
        let member = Member(id: maya.id, username: "maya", displayName: "Maya", owner: true)
        let other = Member(id: "Other0000001", username: "other", displayName: "Other", owner: false)
        let invitation = ChannelInvitation(channel: secret, inviter: Inviter(username: "maya", displayName: "Maya"))
        let detail = SpaceDetail(space: studio, channels: [secret], members: [member, other], channelInvitations: [invitation],
                                 channelMembers: [secret.id: [maya.id]])
        let update = Space(id: studio.id, name: "Friends", ownerId: maya.id)

        for rebuilt in [detail.applying(update), directory.apply(detail), detail.with(space: update), detail.withProfile(nil)] {
            XCTAssertEqual(rebuilt.channelMembers, [secret.id: [maya.id]])
            XCTAssertEqual(rebuilt.channelInvitations, [invitation])
            XCTAssertEqual(rebuilt.memberGroups(channelID: secret.id)?.inChannel.map(\.id), [maya.id])
            XCTAssertEqual(rebuilt.memberGroups(channelID: secret.id)?.others.map(\.id), [other.id])
        }
        XCTAssertEqual(directory.apply(detail).members.first?.displayName, "Maya R")
        XCTAssertEqual(detail.with(members: [other]).channelMembers, detail.channelMembers)
    }

    func testCustomPicturesAreRoundedSquares() {
        XCTAssertEqual(ProfilePicturePolicy.cornerRadius(side: 40), 12, accuracy: 0.0001)
        XCTAssertEqual(ProfilePicturePolicy.cornerRadius(side: 72), 21.6, accuracy: 0.0001)
    }
}
