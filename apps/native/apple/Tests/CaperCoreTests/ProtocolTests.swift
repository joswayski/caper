import XCTest
@testable import CaperCore

final class ProtocolTests: XCTestCase {
    func testSequenceComparisonDoesNotRoundThroughDouble() throws {
        XCTAssertEqual(try Sequence.compare("9007199254740993", "9007199254740992"), .orderedDescending)
        XCTAssertEqual(try Sequence.compare("184467440737095516160", "99999999999999999999"), .orderedDescending)
        XCTAssertThrowsError(try Sequence.compare("01", "1"))
        XCTAssertThrowsError(try Sequence.compare("-1", "0"))
        XCTAssertThrowsError(try Sequence.compare("١", "1"), "Arabic-Indic digits are not canonical server cursors")
    }

    func testSuccessorRejectsGapAndOverflow() {
        XCTAssertTrue(Sequence.isSuccessor("43", of: "42"))
        XCTAssertFalse(Sequence.isSuccessor("44", of: "42"))
        XCTAssertFalse(Sequence.isSuccessor("18446744073709551616", of: "18446744073709551615"))
    }

    func testHTTPConfirmationDoesNotAdvanceReplayCursor() {
        var delivery = ChatDeliveryState(cursor: "41")
        let command = delivery.begin(text: "hello", makeID: { "stable-id" })
        delivery.confirmHTTP(id: command.id)
        XCTAssertEqual(delivery.cursor, "41")
        XCTAssertTrue(delivery.receive(seq: "42"))
        XCTAssertEqual(delivery.cursor, "42")
    }

    func testUnknownOutcomeRetriesExactCommandAndResetPreventsResurrection() {
        var delivery = ChatDeliveryState(cursor: "8")
        let first = delivery.begin(text: "original", makeID: { "id-one" })
        let retry = delivery.begin(text: "edited draft", makeID: { "id-two" })
        XCTAssertEqual(first, retry)
        XCTAssertFalse(delivery.receive(seq: "10"), "a replay gap must resync instead of skipping seq 9")
        XCTAssertEqual(delivery.cursor, "8")
        delivery.reset()
        delivery.confirmHTTP(id: first.id)
        XCTAssertEqual(delivery.cursor, "0")
        XCTAssertNil(delivery.pending)
    }

    func testGatewayConfirmationClearsOnlyMatchingSenderAndValidationUnlocksEditing() {
        var delivery = ChatDeliveryState(cursor: "3")
        let pending = delivery.begin(text: "first", makeID: { "client-one" })
        XCTAssertFalse(delivery.confirmGateway(clientMessageID: pending.id, authorID: "other", ownAuthorID: "self"))
        XCTAssertEqual(delivery.pending, pending)
        XCTAssertTrue(delivery.confirmGateway(clientMessageID: pending.id, authorID: "self", ownAuthorID: "self"))
        XCTAssertNil(delivery.pending)
        XCTAssertFalse(
            delivery.confirmGateway(clientMessageID: pending.id, authorID: "self", ownAuthorID: "self"),
            "a later HTTP failure can recognize that the gateway already confirmed delivery"
        )

        let rejected = delivery.begin(text: "invalid", makeID: { "client-two" })
        delivery.reject(id: rejected.id)
        let edited = delivery.begin(text: "edited", makeID: { "client-three" })
        XCTAssertEqual(edited.id, "client-three")
        XCTAssertEqual(edited.text, "edited")
    }

    func testResyncResetPreservesUnknownOutcomeButChannelResetDoesNot() {
        var delivery = ChatDeliveryState(cursor: "12")
        let pending = delivery.begin(text: "possibly delivered", makeID: { "stable" })
        delivery.reset(cursor: "14", preservingPending: true)
        XCTAssertEqual(delivery.pending, pending)
        XCTAssertEqual(delivery.cursor, "14")
        delivery.reset(cursor: "0")
        XCTAssertNil(delivery.pending)
    }

    func testKeychainNamespaceIncludesCanonicalOrigin() {
        XCTAssertEqual(APIClient.sessionService(for: URL(string: "https://CAPER.chat")!), "chat.caper.session")
        XCTAssertNotEqual(
            APIClient.sessionService(for: URL(string: "https://staging.caper.chat")!),
            APIClient.sessionService(for: URL(string: "https://caper.chat")!)
        )
        XCTAssertNotEqual(
            APIClient.sessionService(for: URL(string: "https://caper.chat:444")!),
            APIClient.sessionService(for: URL(string: "https://caper.chat")!)
        )
    }

    func testMessageValidationCountsCharactersAndRejectsControls() {
        XCTAssertNil(MessageValidation.error(for: String(repeating: "🪐", count: 4_000)))
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "🪐", count: 4_001)), "Messages can be at most 4,000 characters.")
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "e\u{301}", count: 2_001)), "Messages can be at most 4,000 characters.", "Messages count Unicode scalars, not grapheme clusters.")
        XCTAssertEqual(MessageValidation.error(for: String(repeating: "👨‍👩‍👧‍👦", count: 572)), "Messages can be at most 4,000 characters.", "Messages count every scalar in a ZWJ sequence.")
        XCTAssertNil(MessageValidation.error(for: "line one\nline two\tindented"))
        XCTAssertEqual(MessageValidation.error(for: "hidden\u{0007}"), "Messages cannot contain control characters.")
    }

    func testHTTPConfirmationMustMatchTheExactCommandChannelAndAuthor() {
        let command = PendingMessage(id: "client", text: "expected")
        let author = ChatAuthor(id: "self", name: "Self", isGuest: false)
        let content = ChatContent(version: 1, type: "text", text: "expected")
        let valid = ChatMessage(id: "m", channelId: "Channel12345", seq: "1", author: author, content: content, createdAt: "now", clientMessageId: "client")
        XCTAssertTrue(MessageValidation.acceptsResponse(valid, channelID: "Channel12345", command: command, authorID: "self"))
        let wrongChannel = ChatMessage(id: "m", channelId: "Other1234567", seq: "1", author: author, content: content, createdAt: "now", clientMessageId: "client")
        XCTAssertFalse(MessageValidation.acceptsResponse(wrongChannel, channelID: "Channel12345", command: command, authorID: "self"))
        let wrongText = ChatMessage(id: "m", channelId: "Channel12345", seq: "1", author: author, content: ChatContent(version: 1, type: "text", text: "changed"), createdAt: "now", clientMessageId: "client")
        XCTAssertFalse(MessageValidation.acceptsResponse(wrongText, channelID: "Channel12345", command: command, authorID: "self"))
    }

    func testRedirectPolicyAllowsDefaultPortButBlocksCredentialExfiltration() {
        XCTAssertTrue(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://CAPER.chat:443/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://evil.example/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "http://caper.chat/b")!))
        XCTAssertFalse(RedirectPolicy.sameOrigin(URL(string: "https://caper.chat/a")!, URL(string: "https://caper.chat:444/b")!))
    }

    func testIceServerAcceptsSingleURLAndArray() throws {
        let decoder = JSONDecoder()
        XCTAssertEqual(try decoder.decode(IceServer.self, from: Data(#"{"urls":"stun:one"}"#.utf8)).urls, ["stun:one"])
        XCTAssertEqual(try decoder.decode(IceServer.self, from: Data(#"{"urls":["turn:one","turn:two"],"username":"u","credential":"p"}"#.utf8)).urls, ["turn:one", "turn:two"])
    }
}
