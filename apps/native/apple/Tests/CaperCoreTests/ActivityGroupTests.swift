import XCTest
@testable import CaperCore

final class ActivityGroupTests: XCTestCase {
    private func item(_ id: String, _ kind: String = "message", actor: String = "Alex", unread: Bool = true,
                      channel: String? = "channel00001", conversation: String? = nil, thread: String? = nil,
                      message: String? = "Message00000001", emoji: String? = nil) -> ActivityItem {
        ActivityItem(id: id, kind: kind, actorId: "id-\(actor)", actorName: actor, avatarId: nil, body: "Body \(id)",
            createdAt: "2026-11-04T12:00:00Z", unread: unread, spaceId: conversation == nil ? "space0000001" : nil,
            spaceName: conversation == nil ? "Studio" : nil, channelId: conversation == nil ? channel : "direct0000001",
            channelName: conversation == nil ? "general" : "direct", conversationId: conversation, messageId: message,
            threadRootId: thread, emoji: emoji)
    }

    private func ids(_ groups: [ActivityGroup]) -> [[String]] { groups.map(\.ids) }

    func testOnlyAdjacentMatchingItemsMerge() {
        let groups = ActivityGroup.grouping([item("a"), item("b", actor: "Maya"), item("c", channel: "channel00002"), item("d")])
        XCTAssertEqual(ids(groups), [["a", "b"], ["c"], ["d"]], "Same conversation separated by another stays apart; order never changes")
        XCTAssertEqual(groups[0].id, "a")
        XCTAssertEqual(groups[0].newest.id, "a")
        XCTAssertEqual(groups[0].oldest.id, "b", "Opening a group targets its oldest item")
    }

    func testReadAndUnreadNeighboursStayApart() {
        let groups = ActivityGroup.grouping([item("a"), item("b", unread: false), item("c", unread: false)])
        XCTAssertEqual(ids(groups), [["a"], ["b", "c"]])
        XCTAssertTrue(groups[0].unread); XCTAssertFalse(groups[1].unread)
    }

    func testThreadsKindsAndConversationsSeparate() {
        let groups = ActivityGroup.grouping([
            item("a", "reply", thread: "Thread00000001"), item("b", "reply", thread: "Thread00000002"),
            item("c", "reply", thread: "Thread00000002"), item("d", "reply"), item("e", "mention"),
            item("f", "message", conversation: "dm0000000001"), item("g", "message", conversation: "dm0000000001"),
            item("h", "message", conversation: "dm0000000002"),
        ])
        XCTAssertEqual(ids(groups), [["a"], ["b", "c"], ["d"], ["e"], ["f", "g"], ["h"]])
        XCTAssertEqual(groups[1].summary, "2 replies")
        XCTAssertEqual(groups[4].summary, "2 messages")
        XCTAssertEqual(groups[4].newest.location, "Direct message", "A DM shows no raw channel name")
        XCTAssertEqual(groups[0].newest.location, "Studio · general")
    }

    func testReactionsGroupByMessage() {
        let groups = ActivityGroup.grouping([
            item("a", "reaction", actor: "Alex", emoji: "🎉"), item("b", "reaction", actor: "Maya", emoji: "👍"),
            item("c", "reaction", actor: "Alex", emoji: "🎉"), item("d", "reaction", message: "Message00000002", emoji: "🎉"),
        ])
        XCTAssertEqual(ids(groups), [["a", "b", "c"], ["d"]])
        XCTAssertEqual(groups[0].summary, "Reacted 🎉 👍 to your message", "Distinct emojis, newest first")
        XCTAssertEqual(groups[0].names, "Alex and Maya")
        XCTAssertEqual(groups[1].summary, "Reacted 🎉 to your message", "A single reaction reads as before")
    }

    func testInvitationsAndRequestsNeverGroup() {
        for kind in ["space.invitation", "channel.invitation", "direct.request", "unknown"] {
            let groups = ActivityGroup.grouping([item("a", kind), item("b", kind)])
            XCTAssertEqual(ids(groups), [["a"], ["b"]], kind)
        }
    }

    func testNamesSummariesAndReadLabel() {
        let mentions = ActivityGroup.grouping([item("a", "mention", actor: "Alex"), item("b", "mention", actor: "Maya"),
                                                item("c", "mention", actor: "Alex"), item("d", "mention", actor: "Sam")])
        XCTAssertEqual(mentions.count, 1)
        XCTAssertEqual(mentions[0].names, "Alex and 2 others")
        XCTAssertEqual(mentions[0].summary, "Mentioned you 4 times")
        XCTAssertEqual(mentions[0].readLabel, "Mark 4 notifications from Alex and 2 others read")
        let single = ActivityGroup(items: [item("a", "mention")])
        XCTAssertEqual(single.names, "Alex")
        XCTAssertNil(single.summary)
        XCTAssertEqual(single.readLabel, "Mark notification from Alex read")
    }

    func testGroupsHoldAtMostOneHundredItems() {
        let groups = ActivityGroup.grouping((0..<205).map { item("n\($0)") })
        XCTAssertEqual(groups.map(\.items.count), [100, 100, 5], "One read request takes at most 100 IDs")
    }
}
