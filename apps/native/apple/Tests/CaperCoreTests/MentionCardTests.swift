import XCTest
@testable import CaperCore

final class MentionCardTests: XCTestCase {
    private let me = Account(id: "Self00000001", username: "fixture_owner", displayName: "Fixture Owner", avatarId: 4)
    private let maya = MentionPill(id: "Maya00000001", username: "maya")

    private func resolve(_ pill: MentionPill, members: [Member] = [], people: [Person]? = nil,
                         peers: [DirectMessagePeer] = [], viewerID: String? = "Self00000001") -> MentionCardPerson {
        MentionCard.resolve(pill, members: members, people: people, peers: peers, account: me, viewerID: viewerID)
    }

    func testResolvesMemberThenPeopleThenDMPeerByID() {
        let member = Member(id: maya.id, username: "maya", displayName: "Maya (space)", owner: false, avatarId: 31)
        let person = Person(id: maya.id, username: "maya", displayName: "Maya (people)", avatarId: 32)
        let peer = DirectMessagePeer(id: maya.id, username: "maya", displayName: "Maya (DM)")
        XCTAssertEqual(resolve(maya, members: [member], people: [person], peers: [peer]),
                       MentionCardPerson(id: maya.id, username: "maya", displayName: "Maya (space)", avatarId: 31, isSelf: false))
        XCTAssertEqual(resolve(maya, people: [person], peers: [peer]).displayName, "Maya (people)")
        XCTAssertEqual(resolve(maya, people: [person], peers: [peer]).avatarId, 32)
        XCTAssertEqual(resolve(maya, people: nil, peers: [peer]).displayName, "Maya (DM)", "people not loaded yet")
        XCTAssertNil(resolve(maya, peers: [peer]).avatarId)
        // Matching is by id, never by the username typed in the message.
        let other = Member(id: "Other0000001", username: "maya", displayName: "Someone Else", owner: false)
        XCTAssertNil(resolve(maya, members: [other]).displayName)
        // The canonical username of the match wins over the entry's.
        let renamed = Person(id: maya.id, username: "maya_r", displayName: "Maya")
        XCTAssertEqual(resolve(maya, people: [renamed]).username, "maya_r")
    }

    func testUnknownPersonFallsBackToUsername() {
        let unknown = resolve(MentionPill(id: "Stranger0001", username: "stranger"),
                              members: [Member(id: maya.id, username: "maya", displayName: "Maya", owner: false)], people: [])
        XCTAssertEqual(unknown, MentionCardPerson(id: "Stranger0001", username: "stranger", displayName: nil, avatarId: nil, isSelf: false))
        XCTAssertEqual(unknown.title, "@stranger")
        XCTAssertNil(unknown.subtitle, "no second line")
        XCTAssertEqual(unknown.avatarName, "stranger", "the generic initial avatar comes from the username")
        let blank = resolve(maya, peers: [DirectMessagePeer(id: maya.id, username: "maya", displayName: "  ")])
        XCTAssertNil(blank.displayName, "a blank name is treated as unknown")
    }

    func testSelfDetection() {
        let pill = MentionPill(id: me.id, username: "fixture_owner")
        let fromMembers = resolve(pill, members: [Member(id: me.id, username: "fixture_owner", displayName: "Fixture Owner", owner: true)])
        XCTAssertTrue(fromMembers.isSelf)
        XCTAssertEqual(fromMembers.title, "Fixture Owner")
        XCTAssertEqual(fromMembers.subtitle, "@fixture_owner")
        let fromAccount = resolve(pill, people: [])
        XCTAssertTrue(fromAccount.isSelf, "people never lists you; your account still names the card")
        XCTAssertEqual(fromAccount.displayName, "Fixture Owner")
        XCTAssertEqual(fromAccount.avatarId, 4)
        XCTAssertTrue(resolve(pill, viewerID: nil).isSelf, "falls back to the account id")
        XCTAssertFalse(resolve(maya).isSelf)
    }

    func testPillLinksRoundTripAndIgnoreOtherURLs() throws {
        let pill = MentionPill(id: "AbC123xyz_-9", username: "alice")
        let url = try XCTUnwrap(MentionCard.url(for: pill))
        XCTAssertEqual(url.scheme, "caper-mention")
        XCTAssertEqual(MentionCard.userID(from: url), "AbC123xyz_-9", "ids keep their case")
        XCTAssertNil(MentionCard.userID(from: try XCTUnwrap(URL(string: "https://caper.chat/AbC123xyz_-9"))))
        XCTAssertNil(MentionCard.userID(from: try XCTUnwrap(URL(string: "mailto:alice"))))
    }

    func testPillsAreResolvedPeopleOnlyInOrderWithoutDuplicates() {
        let mentions: [MessageMention] = [.user(id: "Alice0000001", username: "alice"), .everyone, .user(id: "Bob000000001", username: "bob")]
        let text = "@bob and @Alice, @everyone, @alice again, @carol"
        XCTAssertEqual(MentionAutocomplete.pills(in: text, mentions: mentions), [
            MentionPill(id: "Bob000000001", username: "bob"),
            MentionPill(id: "Alice0000001", username: "alice"),
        ], "@everyone stays inert and unresolved names are not pills")
        XCTAssertEqual(MentionAutocomplete.segments(in: "@everyone", mentions: [.everyone]),
                       [MentionSegment(text: "@everyone", highlighted: true, user: nil)])
        XCTAssertEqual(MentionAutocomplete.pills(in: "@alice", mentions: []), [])
    }
}
