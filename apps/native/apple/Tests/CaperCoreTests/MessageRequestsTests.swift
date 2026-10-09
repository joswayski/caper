import XCTest
@testable import CaperCore

final class MessageRequestsTests: XCTestCase {
    private let me = "Self00000001"
    private let maya = ChatAuthor(id: "member000001", name: "Maya", isGuest: false, avatarId: 31)
    private let alex = ChatAuthor(id: "member000002", name: "Alex", isGuest: false)

    private func conversation(_ id: String, peer: String = "peer00000001", status: DirectMessageStatus = .accepted,
                              blocked: Bool = false, lastSeq: String = "2", readSeq: String = "1") -> DirectMessageConversation {
        DirectMessageConversation(id: id, peer: DirectMessagePeer(id: peer, username: "jordan", displayName: "Jordan"),
                                  lastSeq: lastSeq, readSeq: readSeq, status: status, blocked: blocked)
    }
    private func message(_ id: String, _ author: ChatAuthor) -> ChatMessage {
        ChatMessage(id: id, channelId: "chan00000001", seq: "1", author: author,
                    content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: "2026-10-07T10:00:00Z", clientMessageId: "c-\(id)")
    }
    private func shape(_ entries: [TimelineEntry]) -> [String] {
        entries.map { entry in
            switch entry {
            case let .message(message): return message.id
            case let .blocked(run): return "\(run.revealed ? "shown" : "hidden"):\(run.messages.map(\.id).joined(separator: ","))"
            }
        }
    }

    // MARK: Decoding

    func testConversationsTolerateMissingStatusAndBlocked() throws {
        let json = #"""
        {"conversations":[
          {"id":"dm0000000001","peer":{"id":"a","username":"alex","displayName":"Alex"},"lastSeq":"3","readSeq":"1"},
          {"id":"dm0000000002","peer":{"id":"b","username":"jordan","displayName":"Jordan","avatarId":412},"lastSeq":"1","readSeq":"0","status":"incoming","blocked":false},
          {"id":"dm0000000003","peer":{"id":"c","username":"sam","displayName":"Sam","avatarId":null},"lastSeq":"0","readSeq":"0","status":"outgoing","blocked":true},
          {"id":"dm0000000004","peer":{"id":"d","username":"lee","displayName":"Lee"},"lastSeq":"0","readSeq":"0","status":"someday","blocked":null}
        ]}
        """#
        let conversations = try JSONDecoder().decode(DirectMessagesResponse.self, from: Data(json.utf8)).conversations
        XCTAssertEqual(conversations.map(\.status), [.accepted, .incoming, .outgoing, .accepted], "missing or unknown status means accepted")
        XCTAssertEqual(conversations.map(\.blocked), [false, false, true, false])
        XCTAssertEqual(conversations.map(\.peer.avatarId), [nil, 412, nil, nil])
        let roundTrip = try JSONDecoder().decode(DirectMessageConversation.self, from: JSONEncoder().encode(conversations[2]))
        XCTAssertEqual(roundTrip, conversations[2])
    }

    func testBlocksAndPrivacyPayloads() throws {
        let blocks = try JSONDecoder().decode(BlocksResponse.self, from: Data(#"""
        {"blocks":[{"id":"member000001","username":"maya","displayName":"Maya","avatarId":31},{"id":"stranger0001","username":"jordan","displayName":"Jordan","avatarId":null}]}
        """#.utf8)).blocks
        XCTAssertEqual(blocks.map(\.username), ["maya", "jordan"])
        XCTAssertEqual(blocks.map(\.avatarId), [31, nil])
        let privacy = try JSONDecoder().decode(PrivacySettings.self, from: Data(#"{"directMessages":"spaces"}"#.utf8))
        XCTAssertEqual(privacy.directMessages, .spaces)
        let body = try JSONSerialization.jsonObject(with: JSONEncoder().encode(PrivacySettings(directMessages: .nobody))) as? [String: String]
        XCTAssertEqual(body, ["directMessages": "nobody"], "PUT sends exactly one field")
        XCTAssertEqual(DirectMessagePrivacy.allCases.map(\.title), ["Anyone", "People in my spaces", "No one new"])
        XCTAssertEqual(DirectMessagePrivacy.anyone.detail, "People outside your spaces send a message request first.")
        XCTAssertEqual(DirectMessagePrivacy.nobody.detail, "Conversations you already have stay open.")
        XCTAssertNil(DirectMessagePrivacy.spaces.detail)
    }

    // MARK: Lists, unread and errors

    func testRequestsStayOutOfTheMainListAndUnreadCounts() {
        let list = [conversation("dm1", status: .accepted), conversation("dm2", status: .outgoing),
                    conversation("dm3", status: .incoming), conversation("dm4", status: .incoming, lastSeq: "1", readSeq: "1")]
        XCTAssertEqual(MessageRequests.visible(list).map(\.id), ["dm1", "dm2"])
        XCTAssertEqual(MessageRequests.incoming(list).map(\.id), ["dm3", "dm4"], "the row counts conversations, not messages")
        XCTAssertFalse(list[2].unread, "requests never show unread")
        XCTAssertTrue(list[0].unread)
        XCTAssertEqual(MessageRequests.unreadCount(list), 2)
        XCTAssertEqual(list[0].with(readSeq: "2").unread, false)
        XCTAssertEqual(list[1].with(readSeq: "2").status, .outgoing, "updating the read cursor keeps status")
    }

    func testBlockingDropsTheirRequestAndMarksConversations() {
        let list = [conversation("dm1", peer: "a"), conversation("dm2", peer: "b", status: .incoming), conversation("dm3", peer: "b", status: .outgoing)]
        let blocked = MessageRequests.applying(blocked: true, peerID: "b", to: list)
        XCTAssertEqual(blocked.map(\.id), ["dm1", "dm3"], "blocking a requester declines the request")
        XCTAssertEqual(blocked.map(\.blocked), [false, true])
        XCTAssertEqual(MessageRequests.applying(blocked: false, peerID: "b", to: blocked).map(\.blocked), [false, false])
    }

    func testCopyAndErrorMapping() {
        let peer = DirectMessagePeer(id: "stranger0001", username: "jordan", displayName: "Jordan", avatarId: 412)
        let prompt = MessageRequests.prompt(for: peer)
        XCTAssertEqual(prompt.name + prompt.detail, "Jordan (@jordan) wants to message you. You don't share a space.")
        XCTAssertEqual(MessageRequests.waitingNotice(for: peer), "Waiting for @jordan to accept. They'll see your messages when they do.")
        XCTAssertEqual(MessageRequests.blockedNotice(for: peer), "You blocked @jordan.")
        XCTAssertEqual(DirectMessageErrors.message(code: "dm_not_accepted"), "This person isn't accepting direct messages.")
        XCTAssertEqual(DirectMessageErrors.message(code: "dm_blocked"), "You blocked this person. Unblock them to message them.")
        XCTAssertNil(DirectMessageErrors.message(code: "rate_limited"))
        XCTAssertNil(DirectMessageErrors.message(code: nil))
        XCTAssertTrue(DirectMessageErrors.isRefusal(status: 403, code: "dm_blocked"))
        XCTAssertFalse(DirectMessageErrors.isRefusal(status: 403, code: nil), "other 403s keep their meaning")
        XCTAssertFalse(DirectMessageErrors.isRefusal(status: 429, code: "dm_not_accepted"))
        let target = BlockTarget(author: alex)
        XCTAssertEqual(target.confirmationTitle, "Block Alex?")
        XCTAssertNil(target.username, "a message author carries no username")
        XCTAssertEqual(BlockTarget(peer: peer).username, "jordan")
        XCTAssertEqual(BlockTarget.confirmationMessage,
                       "You won't see their messages unless you choose to, and they can't send you DMs or requests.")
    }

    // MARK: Blocked runs

    func testConsecutiveBlockedMessagesCollapseIntoRuns() {
        let ownAuthor = ChatAuthor(id: me, name: "Me", isGuest: false)
        let guest = ChatAuthor(id: maya.id, name: "Guest Maya", isGuest: true)
        let messages = [message("m1", alex), message("m2", maya), message("m3", maya), message("m4", alex),
                        message("m5", maya), message("m6", guest), message("m7", ownAuthor)]
        let blocked: Set<String> = [maya.id, me]
        XCTAssertEqual(shape(BlockedMessages.entries(messages, blocked: blocked, viewerID: me)),
                       ["m1", "hidden:m2,m3", "m4", "hidden:m5", "m6", "m7"], "guests and your own messages never collapse")
        XCTAssertEqual(shape(BlockedMessages.entries(messages, blocked: [], viewerID: me)), messages.map(\.id))
        guard case let .blocked(run)? = BlockedMessages.entries(messages, blocked: blocked, viewerID: me).dropFirst().first else {
            return XCTFail("Expected a blocked run")
        }
        XCTAssertEqual(run.label, "2 blocked messages")
        XCTAssertEqual(run.id, "blocked-m2")
        XCTAssertEqual(TimelineEntry.blocked(run).scrollID, "m3", "scrolling to the newest message lands on its run")
        XCTAssertEqual(TimelineEntry.blocked(run).id, "m3", "a run's list identity is its scroll anchor")
        // Blocks arrive after history: every entry keeps a message's own ID,
        // so folding messages into a run never adds a clashing identity.
        let unblocked = Set(BlockedMessages.entries(messages, blocked: [], viewerID: me).map(\.id))
        let collapsed = BlockedMessages.entries(messages, blocked: blocked, viewerID: me).map(\.id)
        XCTAssertEqual(Set(collapsed).count, collapsed.count)
        XCTAssertTrue(Set(collapsed).isSubset(of: unblocked))
        XCTAssertEqual(BlockedRun(messages: [messages[4]], revealed: false).label, "1 blocked message")
    }

    func testShowAndHideAreInMemoryPerRun() {
        let messages = [message("m1", maya), message("m2", maya), message("m3", alex), message("m4", maya)]
        let blocked: Set<String> = [maya.id]
        var revealed: Set<String> = []
        let first = BlockedMessages.entries(messages, blocked: blocked, viewerID: me, revealed: revealed)
        guard case let .blocked(run) = first[0] else { return XCTFail("Expected a run") }
        revealed = BlockedMessages.toggling(run, in: revealed)
        XCTAssertEqual(shape(BlockedMessages.entries(messages, blocked: blocked, viewerID: me, revealed: revealed)),
                       ["shown:m1,m2", "m3", "hidden:m4"], "Show reveals only that run")
        let grown = messages.prefix(2) + [message("m2b", maya)] + messages.suffix(2)
        XCTAssertEqual(shape(BlockedMessages.entries(Array(grown), blocked: blocked, viewerID: me, revealed: revealed)).first,
                       "shown:m1,m2,m2b", "a shown run stays shown as it grows")
        guard case let .blocked(shown) = BlockedMessages.entries(messages, blocked: blocked, viewerID: me, revealed: revealed)[0] else {
            return XCTFail("Expected a run")
        }
        XCTAssertTrue(BlockedMessages.toggling(shown, in: revealed).isEmpty, "Hide collapses it again")
    }

    func testWhoCanBeBlockedFromMessageActions() {
        XCTAssertTrue(BlockedMessages.canBlock(alex, viewerID: me, blocked: []))
        XCTAssertFalse(BlockedMessages.canBlock(alex, viewerID: me, blocked: [alex.id]), "already blocked")
        XCTAssertFalse(BlockedMessages.canBlock(ChatAuthor(id: me, name: "Me", isGuest: false), viewerID: me, blocked: []), "not yourself")
        XCTAssertFalse(BlockedMessages.canBlock(ChatAuthor(id: "guest", name: "Guest", isGuest: true), viewerID: me, blocked: []), "not guests")
        XCTAssertFalse(BlockedMessages.canBlock(alex, viewerID: nil, blocked: []), "only when signed in")
    }

    func testWhoCanBeUnblockedFromMessageActions() {
        XCTAssertTrue(BlockedMessages.canUnblock(alex, viewerID: me, blocked: [alex.id]))
        XCTAssertFalse(BlockedMessages.canUnblock(alex, viewerID: me, blocked: []), "not blocked")
        XCTAssertFalse(BlockedMessages.canUnblock(ChatAuthor(id: me, name: "Me", isGuest: false), viewerID: me, blocked: [me]), "not yourself")
        XCTAssertFalse(BlockedMessages.canUnblock(ChatAuthor(id: "guest", name: "Guest", isGuest: true), viewerID: me, blocked: ["guest"]), "not guests")
        XCTAssertFalse(BlockedMessages.canUnblock(alex, viewerID: nil, blocked: [alex.id]), "only when signed in")
    }
}
