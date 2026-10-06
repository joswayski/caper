import XCTest
@testable import CaperCore

final class MentionAutocompleteTests: XCTestCase {
    private let selfID = "Self00000001"
    private var members: [Member] {
        [
            Member(id: selfID, username: "fixture_self", displayName: "Alice Myself", owner: true),
            Member(id: "Alice0000001", username: "alice", displayName: "Alice Smith", owner: false, avatarId: 3),
            Member(id: "Alicia000001", username: "alicia", displayName: "Alicia Jones", owner: false),
            Member(id: "Bob000000001", username: "bob", displayName: "Bob Alder", owner: false),
            Member(id: "Malice000001", username: "malice", displayName: "Mal Ice", owner: false),
            Member(id: "Zed000000001", username: "zed", displayName: "Alfie Zed", owner: false),
        ]
    }
    private var space: MentionSource { .space(members: members, excluding: selfID) }

    private func caret(_ text: String) -> NSRange { NSRange(location: (text as NSString).length, length: 0) }
    private func query(_ text: String, selection: NSRange? = nil, markedText: Bool = false) -> MentionQuery? {
        MentionAutocomplete.activeQuery(text: text, selection: selection ?? caret(text), markedText: markedText)
    }
    private func names(_ text: String, _ source: MentionSource? = nil) -> [String] {
        MentionAutocomplete.match(text: text, selection: caret(text), markedText: false, source: source ?? space)?
            .choices.map(\.username) ?? []
    }
    private func message(author: String, _ mentions: [MessageMention], text: String = "hi") -> ChatMessage {
        ChatMessage(id: "m", channelId: "Channel12345", seq: "1", author: ChatAuthor(id: author, name: "Author", isGuest: false),
                    content: ChatContent(version: 1, type: "text", text: text, mentions: mentions),
                    createdAt: "now", clientMessageId: "c")
    }

    // MARK: Token detection

    func testStartRuleBoundariesAndCaret() {
        XCTAssertEqual(query("@"), MentionQuery(range: NSRange(location: 0, length: 1), query: ""))
        XCTAssertEqual(query("hi @Al"), MentionQuery(range: NSRange(location: 3, length: 3), query: "al"))
        for text in ["(@al", "[@al", "{@al", "line\n@al", "tab\t@al"] { XCTAssertNotNil(query(text), text) }
        for text in ["bob@alice", "x/@al", "@@al", "a,@al", "https://x.com/@al", "@al.", "@al-"] {
            XCTAssertNil(query(text), text)
        }
        // The caret must end the name: not inside it and not before another `@`.
        XCTAssertNil(query("@alice", selection: NSRange(location: 3, length: 0)))
        XCTAssertNil(query("@al@", selection: NSRange(location: 3, length: 0)))
        XCTAssertEqual(query("@al there", selection: NSRange(location: 3, length: 0))?.query, "al")
        XCTAssertEqual(query("@al!", selection: NSRange(location: 3, length: 0))?.query, "al")
        // Unicode before the token keeps UTF-16 ranges.
        let prefix = "👩‍💻 é "
        XCTAssertEqual(query("\(prefix)@al")?.range, NSRange(location: (prefix as NSString).length, length: 3))
    }

    func testNameRunOver32CharactersIsNotActive() {
        let longest = String(repeating: "a", count: 32)
        XCTAssertEqual(query("@\(longest)")?.query, longest)
        XCTAssertNil(query("@\(longest)b"))
    }

    func testIMECompositionAndSelectionGuards() {
        XCTAssertNil(query("@al", markedText: true))
        XCTAssertNil(query("@al", selection: NSRange(location: 1, length: 2)))
        XCTAssertNil(MentionAutocomplete.match(text: "@al", selection: NSRange(location: 3, length: 0), markedText: true, source: space))
    }

    func testEmojiAndMentionTokensAreNeverBothActive() {
        for text in ["@al", ":al", "@:al", ":@al", " @a:b", "x :a@b"] {
            let selection = caret(text)
            let mention = MentionAutocomplete.activeQuery(text: text, selection: selection, markedText: false) != nil
            let emoji = EmojiAutocomplete.match(text: text, selection: selection, markedText: false) != nil
            XCTAssertFalse(mention && emoji, text)
        }
        guard case .mention? = ComposerAutocomplete.match(text: "@al", selection: caret("@al"), markedText: false, mentions: space) else {
            return XCTFail("@ must open member suggestions")
        }
    }

    // MARK: Candidates and ranking

    func testRankingExactPrefixDisplayNameWordAndContains() {
        XCTAssertEqual(names("@alice"), ["alice", "malice"], "exact username, then contains")
        XCTAssertEqual(names("@ALI"), ["alice", "alicia", "malice"], "queries compare lowercased")
        XCTAssertEqual(names("@al"), ["alice", "alicia", "bob", "zed", "malice"],
                       "prefix, then display-name words (ties by username), then contains")
        XCTAssertEqual(names("@jones"), ["alicia"], "a later display-name word matches")
        XCTAssertEqual(names("@ice"), ["malice", "alice"], "display-name word ranks above username contains")
        XCTAssertEqual(names("@nobody"), [])
        XCTAssertFalse(names("@fixture").contains("fixture_self"), "the signed-in account is never suggested")
        XCTAssertFalse(names("@myself").contains("fixture_self"))
    }

    func testSpecialsFollowMembersAndKeepTheirSlotsInTheCapOfSix() {
        XCTAssertEqual(names("@"), ["alice", "alicia", "bob", "malice", "everyone", "here"],
                       "an empty query lists members alphabetically, capped to leave room for both specials")
        XCTAssertEqual(names("@e"), ["alice", "malice", "zed", "everyone"])
        XCTAssertEqual(names("@HER"), ["here"])
        let many = (0..<10).map { Member(id: "User0000000\($0)", username: "user\($0)", displayName: "User \($0)", owner: false) }
        XCTAssertEqual(names("@user", .space(members: many, excluding: nil)).count, 6)
        XCTAssertEqual(names("@", .space(members: many, excluding: nil)),
                       ["user0", "user1", "user2", "user3", "everyone", "here"])
        XCTAssertEqual(names("@", .space(members: [], excluding: selfID)), ["everyone", "here"],
                       "before members load a space channel offers only the specials")
    }

    func testDirectMessagesFallBackToTheOtherParticipantUntilPeopleLoad() {
        let peer = DirectMessagePeer(id: "Peer00000001", username: "pat", displayName: "Pat Doe")
        let direct = MentionSource.direct(peer: peer, people: nil, accountID: selfID)
        XCTAssertEqual(names("@", direct), ["pat"])
        XCTAssertEqual(names("@doe", direct), ["pat"])
        XCTAssertEqual(names("@every", direct), [], "specials are plain text in DMs")
        let notes = MentionSource.direct(peer: DirectMessagePeer(id: selfID, username: "fixture_self", displayName: "Me"), people: nil, accountID: selfID)
        XCTAssertEqual(notes, .empty, "nobody to mention in self-notes before people load")
        XCTAssertNil(MentionAutocomplete.match(text: "@", selection: caret("@"), markedText: false, source: notes))
        XCTAssertEqual(MentionSource.direct(peer: nil, people: nil, accountID: selfID), .empty)
    }

    func testDirectMessagesSuggestEveryoneFromPeopleWithoutSpecials() {
        let people = [
            Person(id: "Alex00000001", username: "alex", displayName: "Alex Stone", avatarId: 7),
            Person(id: "Peer00000001", username: "pat", displayName: "Pat Doe"),
            Person(id: selfID, username: "fixture_self", displayName: "Me"),
            Person(id: "Sam000000001", username: "sam", displayName: "Sam Lee"),
        ]
        let peer = DirectMessagePeer(id: "Peer00000001", username: "pat", displayName: "Pat Doe")
        let direct = MentionSource.direct(peer: peer, people: people, accountID: selfID)
        XCTAssertFalse(direct.specials)
        XCTAssertEqual(names("@", direct), ["alex", "pat", "sam"], "people beyond the peer, never yourself, no specials")
        XCTAssertEqual(names("@stone", direct), ["alex"], "same display-name ranking as space channels")
        XCTAssertEqual(names("@every", direct), [], "@everyone/@here stay space-only")
        XCTAssertEqual(names("@her", direct), [])
        XCTAssertEqual(direct.members.first?.avatarId, 7)
        XCTAssertEqual(direct.members.filter { $0.id == peer.id }.count, 1, "the peer is not duplicated")

        let notes = MentionSource.direct(peer: DirectMessagePeer(id: selfID, username: "fixture_self", displayName: "Me"),
                                         people: people, accountID: selfID)
        XCTAssertEqual(names("@", notes), ["alex", "pat", "sam"], "self-notes suggest people too")
        let newer = MentionSource.direct(peer: DirectMessagePeer(id: "New000000001", username: "newcomer", displayName: "New"),
                                         people: people, accountID: selfID)
        XCTAssertEqual(names("@new", newer), ["newcomer"], "a conversation newer than the list still offers its peer")
        XCTAssertEqual(MentionSource.direct(peer: nil, people: [], accountID: selfID), .empty)
        let many = (0..<10).map { Person(id: "User0000000\($0)", username: "user\($0)", displayName: "User \($0)") }
        XCTAssertEqual(names("@", .direct(peer: nil, people: many, accountID: selfID)).count, 6, "same cap of six")
    }

    func testPeopleResponseDecodesNullAndMissingAvatars() throws {
        let json = #"""
        {"people":[
          {"id":"Alex00000001","username":"alex","displayName":"Alex Stone","avatarId":12},
          {"id":"Pat000000001","username":"pat","displayName":"Pat Doe","avatarId":null},
          {"id":"Sam000000001","username":"sam","displayName":"Sam Lee"}
        ]}
        """#
        let response = try JSONDecoder().decode(PeopleResponse.self, from: Data(json.utf8))
        XCTAssertEqual(response.people, [
            Person(id: "Alex00000001", username: "alex", displayName: "Alex Stone", avatarId: 12),
            Person(id: "Pat000000001", username: "pat", displayName: "Pat Doe", avatarId: nil),
            Person(id: "Sam000000001", username: "sam", displayName: "Sam Lee"),
        ])
        XCTAssertEqual(try JSONDecoder().decode(PeopleResponse.self, from: Data(#"{"people":[]}"#.utf8)).people, [])
        XCTAssertThrowsError(try JSONDecoder().decode(PeopleResponse.self, from: Data(#"{"people":[{"id":"x","username":"x"}]}"#.utf8)))
    }

    // MARK: Insertion

    func testInsertionAddsTrailingSpacePreservesUnicodeAndCaret() throws {
        let text = "👩‍💻 hi @al🚀 later"
        let selection = NSRange(location: ("👩‍💻 hi @al" as NSString).length, length: 0)
        let match = try XCTUnwrap(MentionAutocomplete.match(text: text, selection: selection, markedText: false, source: space))
        let result = try XCTUnwrap(MentionAutocomplete.inserting(match.choices[0], in: text, match: match))
        XCTAssertEqual(result.text, "👩‍💻 hi @alice 🚀 later")
        XCTAssertEqual(result.selection, NSRange(location: ("👩‍💻 hi @alice " as NSString).length, length: 0))

        let special = try XCTUnwrap(MentionAutocomplete.match(text: "@EV", selection: caret("@EV"), markedText: false, source: space))
        XCTAssertEqual(MentionAutocomplete.inserting(special.choices[0], in: "@EV", match: special)?.text, "@everyone ")

        let shared = try XCTUnwrap(ComposerAutocomplete.match(text: "x @bo", selection: caret("x @bo"), markedText: false, mentions: space))
        XCTAssertEqual(ComposerAutocomplete.inserting(choice: 0, in: "x @bo", match: shared)?.text, "x @bob ")
        XCTAssertNil(ComposerAutocomplete.inserting(choice: 9, in: "x @bo", match: shared))
    }

    func testInsertionRespectsScalarLimit() throws {
        let text = "abc @al"
        let match = try XCTUnwrap(MentionAutocomplete.match(text: text, selection: caret(text), markedText: false, source: space))
        XCTAssertNil(MentionAutocomplete.inserting(match.choices[0], in: text, match: match, scalarLimit: 10),
                     "\"abc @alice \" is 11 scalars")
        XCTAssertEqual(MentionAutocomplete.inserting(match.choices[0], in: text, match: match, scalarLimit: 11)?.text, "abc @alice ")
        let full = String(repeating: "x", count: 3_996) + " @al"
        let fullMatch = try XCTUnwrap(MentionAutocomplete.match(text: full, selection: caret(full), markedText: false, source: space))
        XCTAssertNil(MentionAutocomplete.inserting(fullMatch.choices[0], in: full, match: fullMatch), "4,000-scalar draft limit")
    }

    // MARK: Messages

    func testMessageTokenizationMatchesServerGrammar() {
        let text = "@Alice, hi @bob! bob@alice.com x/@carol @@dave (@erin) [@f] @ @al.\u{E9} @alic\u{E9}"
        XCTAssertEqual(MentionAutocomplete.tokens(in: text).map(\.name), ["alice", "bob", "erin", "f", "al", "alic"])
        let longest = String(repeating: "a", count: 32)
        XCTAssertEqual(MentionAutocomplete.tokens(in: "@\(longest)").map(\.name), [longest])
        XCTAssertEqual(MentionAutocomplete.tokens(in: "@\(longest)b @ok").map(\.name), ["ok"], "a 33-character run is not a mention")
        XCTAssertEqual(MentionAutocomplete.tokens(in: "🙂 @bob").map(\.name), ["bob"])
    }

    func testHighlightsOnlyResolvedNames() {
        let mentions: [MessageMention] = [.user(id: "Alice0000001", username: "alice"), .everyone]
        let text = "@Alice @bob @everyone @here bob@alice"
        XCTAssertEqual(MentionAutocomplete.segments(in: text, mentions: mentions), [
            MentionSegment(text: "@Alice", highlighted: true),
            MentionSegment(text: " @bob ", highlighted: false),
            MentionSegment(text: "@everyone", highlighted: true),
            MentionSegment(text: " @here bob@alice", highlighted: false),
        ])
        XCTAssertTrue(MentionAutocomplete.isHighlighted(name: "ALICE", mentions: mentions), "case-insensitive")
        XCTAssertFalse(MentionAutocomplete.isHighlighted(name: "here", mentions: mentions), "@here needs its own entry")
        XCTAssertTrue(MentionAutocomplete.isHighlighted(name: "here", mentions: [.here]))
        XCTAssertEqual(MentionAutocomplete.segments(in: "@everyone in a DM", mentions: []),
                       [MentionSegment(text: "@everyone in a DM", highlighted: false)], "no entries, no pills")
        let unicode = "👩‍💻 @alice 🚀"
        XCTAssertEqual(MentionAutocomplete.segments(in: unicode, mentions: mentions).map(\.text).joined(), unicode)
    }

    func testContentDecodesMentionsLenientlyAndOmitsEmptyOnEncode() throws {
        let json = #"""
        {"version":1,"type":"text","text":"hey @alice @here","mentions":[
          {"type":"user","id":"Alice0000001","username":"alice"},
          {"type":"channel","id":"x"},
          {"type":"user"},
          "garbage",
          {"type":"here"},
          {"type":"everyone"}
        ]}
        """#
        let content = try JSONDecoder().decode(ChatContent.self, from: Data(json.utf8))
        XCTAssertEqual(content.mentions, [.user(id: "Alice0000001", username: "alice"), .here, .everyone],
                       "unknown or malformed entries are ignored")
        let older = try JSONDecoder().decode(ChatContent.self, from: Data(#"{"version":1,"type":"text","text":"hi"}"#.utf8))
        XCTAssertEqual(older, ChatContent(version: 1, type: "text", text: "hi"))
        let null = try JSONDecoder().decode(ChatContent.self, from: Data(#"{"version":1,"type":"text","text":"hi","mentions":null}"#.utf8))
        XCTAssertEqual(null.mentions, [])
        let object = try JSONDecoder().decode(ChatContent.self, from: Data(#"{"version":1,"type":"text","text":"hi","mentions":{}}"#.utf8))
        XCTAssertEqual(object.mentions, [], "a malformed list never drops the message")

        let plain = try JSONSerialization.jsonObject(with: JSONEncoder().encode(older)) as? [String: Any]
        XCTAssertNil(plain?["mentions"])
        XCTAssertEqual(try JSONDecoder().decode(ChatContent.self, from: JSONEncoder().encode(content)), content)
    }

    func testMentionsCurrentUserRule() {
        let other = "Other0000001"
        XCTAssertTrue(MentionAutocomplete.mentionsCurrentUser(message(author: other, [.user(id: selfID, username: "fixture_self")]), currentUserID: selfID))
        XCTAssertTrue(MentionAutocomplete.mentionsCurrentUser(message(author: other, [.everyone]), currentUserID: selfID))
        XCTAssertTrue(MentionAutocomplete.mentionsCurrentUser(message(author: other, [.here]), currentUserID: selfID))
        XCTAssertFalse(MentionAutocomplete.mentionsCurrentUser(message(author: selfID, [.everyone, .here]), currentUserID: selfID),
                       "your own @everyone does not mention you")
        XCTAssertTrue(MentionAutocomplete.mentionsCurrentUser(message(author: selfID, [.user(id: selfID, username: "fixture_self")]), currentUserID: selfID))
        XCTAssertFalse(MentionAutocomplete.mentionsCurrentUser(message(author: other, [.user(id: other, username: "other")]), currentUserID: selfID))
        XCTAssertFalse(MentionAutocomplete.mentionsCurrentUser(message(author: other, []), currentUserID: selfID))
        XCTAssertFalse(MentionAutocomplete.mentionsCurrentUser(message(author: other, [.everyone]), currentUserID: nil))
    }
}
