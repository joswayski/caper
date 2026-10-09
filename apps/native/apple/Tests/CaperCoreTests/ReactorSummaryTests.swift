import XCTest
@testable import CaperCore

final class ReactorSummaryTests: XCTestCase {
    private let alice = ReactorPerson(id: "alice", username: "alice", displayName: "Alice A", avatarId: 100)
    private let bob = ReactorPerson(id: "bob", username: "bob", displayName: "Bob B", avatarId: 101)
    private let carol = ReactorPerson(id: "carol", username: "carol", displayName: "Carol C")
    private let dave = ReactorPerson(id: "dave", username: "dave", displayName: "Dave D")
    private let me = ReactorPerson(id: "me", username: "me", displayName: "Me Myself")

    func testSharedSpecExamples() {
        XCTAssertEqual(ReactionSummary.text(authors: [me], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "You reacted with :thumbs-up:")
        XCTAssertEqual(ReactionSummary.text(authors: [bob, me], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "You and Bob B reacted with :thumbs-up:")
        XCTAssertEqual(ReactionSummary.text(authors: [alice, bob, carol], selfID: "me", emojiName: "party-popper", emoji: "🎉"),
                       "Alice A, Bob B and Carol C reacted with :party-popper:")
        XCTAssertEqual(ReactionSummary.text(authors: [alice, bob, me, carol, dave], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "You, Alice A, Bob B and 2 others reacted with :thumbs-up:")
    }

    func testOthersKeepReactionOrderAndOneOtherIsSingular() {
        XCTAssertEqual(ReactionSummary.text(authors: [bob, alice], selfID: nil, emojiName: "thumbs-up", emoji: "👍"),
                       "Bob B and Alice A reacted with :thumbs-up:")
        XCTAssertEqual(ReactionSummary.text(authors: [alice, bob, carol, dave], selfID: "someone-else", emojiName: "thumbs-up", emoji: "👍"),
                       "Alice A, Bob B, Carol C and 1 other reacted with :thumbs-up:")
    }

    func testNamesFallBackToUsernameThenSomeoneAndUnnamedEmojiShowTheGlyph() {
        let usernameOnly = ReactorPerson(id: "u", username: "user_only")
        let blankName = ReactorPerson(id: "b", username: "blank_name", displayName: "  ")
        let anonymous = ReactorPerson(id: "x")
        XCTAssertEqual(blankName.name, "blank_name")
        XCTAssertEqual(ReactionSummary.text(authors: [usernameOnly, anonymous], selfID: nil, emojiName: nil, emoji: "🫠"),
                       "user_only and Someone reacted with 🫠")
        XCTAssertEqual(ReactionSummary.emojiLabel(emojiName: "", emoji: "🫠"), "🫠")
        XCTAssertEqual(ReactionSummary.emojiLabel(emojiName: "thumbs-up", emoji: "👍"), ":thumbs-up:")
    }

    func testSnapshotFallbackBeforeNamesLoad() {
        XCTAssertEqual(ReactionSummary.fallback(authorIDs: ["me"], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "You reacted with :thumbs-up:")
        XCTAssertEqual(ReactionSummary.fallback(authorIDs: ["alice"], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "1 person reacted with :thumbs-up:")
        XCTAssertEqual(ReactionSummary.fallback(authorIDs: ["me", "alice", "bob"], selfID: "me", emojiName: "red-heart", emoji: "\u{2764}\u{FE0F}"),
                       "3 people reacted with :red-heart:")
        XCTAssertEqual(ReactionSummary.fallback(authorIDs: ["me"], selfID: nil, emojiName: nil, emoji: "🫠"),
                       "1 person reacted with 🫠")
        XCTAssertEqual(ReactionSummary.text(authors: [], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "0 people reacted with :thumbs-up:")
    }

    func testCatalogNamesIgnorePresentationSelectors() {
        XCTAssertEqual(EmojiArtwork.name(for: "\u{1F44D}"), "thumbs-up")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{1F602}"), "face-with-tears-of-joy")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{1F389}"), "party-popper")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{1F440}"), "eyes")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{1F525}"), "fire", "CLDR names, not the picker package's longest alias")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{2764}\u{FE0F}"), "red-heart", "stored fully qualified, catalogued without U+FE0F")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{2764}"), "red-heart")
        XCTAssertEqual(EmojiArtwork.name(for: "\u{2764}\u{FE0F}\u{200D}\u{1F525}"), "heart-on-fire")
        XCTAssertNil(EmojiArtwork.name(for: "\u{1F44D}\u{1F3FD}"), "unnamed variants show their glyph")
        XCTAssertNil(EmojiArtwork.name(for: "not an emoji"))
    }

    func testReactorListDecodesTheServerShapeAndNullNames() throws {
        let json = """
        {"messageId":"Message00000001","reactionSeq":"12","reactions":[
          {"emoji":"👍","authors":[{"id":"bob","username":"bob","displayName":"Bob B","avatarId":101},
                                  {"id":"alice","username":"alice","displayName":"Alice A","avatarId":100}]},
          {"emoji":"🎉","authors":[{"id":"ghost","username":null,"displayName":null,"avatarId":null}]}
        ]}
        """
        let list = try JSONDecoder().decode(ReactorList.self, from: Data(json.utf8))
        XCTAssertEqual(list.messageId, "Message00000001")
        XCTAssertEqual(list.reactionSeq, "12")
        XCTAssertEqual(list.reactions.map(\.emoji), ["👍", "🎉"])
        XCTAssertEqual(list.reactions[0].authors, [bob, alice], "first person to react comes first")
        XCTAssertEqual(list.reactions[1].authors[0], ReactorPerson(id: "ghost"))
        XCTAssertEqual(list.reactions[1].authors[0].name, "Someone")
        XCTAssertTrue(list.isValid(messageID: "Message00000001"))
        XCTAssertFalse(list.isValid(messageID: "Message00000002"))
    }

    func testReactorListValidationRejectsMalformedLists() {
        let duplicateEmoji = ReactorList(messageId: "m", reactionSeq: "3", reactions: [
            ReactorGroup(emoji: "👍", authors: [alice]), ReactorGroup(emoji: "👍", authors: [bob]),
        ])
        let duplicatePerson = ReactorList(messageId: "m", reactionSeq: "3", reactions: [ReactorGroup(emoji: "👍", authors: [alice, alice])])
        let badSequence = ReactorList(messageId: "m", reactionSeq: "03", reactions: [])
        XCTAssertFalse(duplicateEmoji.isValid(messageID: "m"))
        XCTAssertFalse(duplicatePerson.isValid(messageID: "m"))
        XCTAssertFalse(badSequence.isValid(messageID: "m"))
        XCTAssertTrue(ReactorList(messageId: "m", reactionSeq: "0", reactions: []).isValid(messageID: "m"))
    }

    func testPeopleFollowTheCurrentSnapshot() {
        let list = ReactorList(messageId: "m", reactionSeq: "4", reactions: [
            ReactorGroup(emoji: "👍", authors: [bob, alice]),
            ReactorGroup(emoji: "🎉", authors: [me]),
        ])
        XCTAssertEqual(list.people(for: MessageReaction(emoji: "👍", authorIds: ["alice", "bob"]), viewer: nil), [bob, alice],
                       "reaction order comes from the list")
        XCTAssertEqual(list.people(for: MessageReaction(emoji: "👍", authorIds: ["alice"]), viewer: nil), [alice],
                       "a removed reaction is no longer named")
        XCTAssertEqual(list.people(for: MessageReaction(emoji: "👍", authorIds: ["bob", "alice", "me"]), viewer: ReactorPerson(id: "me")),
                       [bob, alice, me], "the viewer's newer reaction is named from elsewhere in the list")
        let newViewer = ReactorPerson(id: "new", displayName: "New Viewer")
        XCTAssertEqual(list.people(for: MessageReaction(emoji: "🎉", authorIds: ["me", "new"]), viewer: newViewer), [me, newViewer])
        XCTAssertNil(list.people(for: MessageReaction(emoji: "👍", authorIds: ["bob", "carol"]), viewer: ReactorPerson(id: "me")),
                     "anyone else unnamed needs a fresh list")
        XCTAssertNil(list.people(for: MessageReaction(emoji: "🚀", authorIds: ["carol"]), viewer: nil))
        XCTAssertEqual(ReactionSummary.text(authors: [bob, alice, me], selfID: "me", emojiName: "thumbs-up", emoji: "👍"),
                       "You, Bob B and Alice A reacted with :thumbs-up:")
    }
}
