import XCTest
@testable import CaperCore

final class MessageGroupingTests: XCTestCase {
    private let maya = ChatAuthor(id: "member000001", name: "Maya", isGuest: false, avatarId: 31)
    private let alex = ChatAuthor(id: "member000002", name: "Alex", isGuest: false)
    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    private func message(_ id: String, _ author: ChatAuthor, at createdAt: String, threadRootId: String? = nil,
                         forwarded: Bool = false, pinned: Bool = false) -> ChatMessage {
        var message = ChatMessage(id: id, channelId: "chan00000001", seq: "1", author: author,
                                  content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: createdAt,
                                  clientMessageId: "c-\(id)")
        message.threadRootId = threadRootId
        if threadRootId != nil { message.broadcast = true }
        if forwarded { message.forward = MessageForward(message: nil, seq: "1") }
        if pinned { message.pin = MessagePin(author: alex, createdAt: createdAt) }
        return message
    }

    func testSameAuthorWithinFiveMinutesGroups() {
        let first = message("m1", maya, at: "2026-10-07T10:00:00Z")
        XCTAssertTrue(MessageGrouping.isGrouped(message("m2", maya, at: "2026-10-07T10:04:59Z"), after: first, inThread: false, calendar: utc))
        XCTAssertTrue(MessageGrouping.isGrouped(message("m2", maya, at: "2026-10-07T10:05:00.000Z"), after: first, inThread: false, calendar: utc),
                      "exactly five minutes still groups")
        XCTAssertFalse(MessageGrouping.isGrouped(message("m2", maya, at: "2026-10-07T10:05:01Z"), after: first, inThread: false, calendar: utc))
        // |Δ|: a slightly earlier timestamp (clock skew) still groups.
        XCTAssertTrue(MessageGrouping.isGrouped(message("m2", maya, at: "2026-10-07T09:58:00Z"), after: first, inThread: false, calendar: utc))
    }

    func testDifferentAuthorOrNoPreviousRowDoesNotGroup() {
        let first = message("m1", maya, at: "2026-10-07T10:00:00Z")
        XCTAssertFalse(MessageGrouping.isGrouped(message("m2", alex, at: "2026-10-07T10:01:00Z"), after: first, inThread: false, calendar: utc))
        XCTAssertFalse(MessageGrouping.isGrouped(message("m2", maya, at: "2026-10-07T10:01:00Z"), after: nil, inThread: false, calendar: utc))
        let nobody = ChatAuthor(id: "", name: "Guest", isGuest: true)
        XCTAssertFalse(MessageGrouping.isGrouped(message("m4", nobody, at: "2026-10-07T10:01:00Z"),
                                                 after: message("m3", nobody, at: "2026-10-07T10:00:00Z"), inThread: false, calendar: utc),
                       "rows without an author id never group")
    }

    func testDateDividerBreaksGrouping() throws {
        var pacific = Calendar(identifier: .gregorian)
        pacific.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        // 23:58 and 00:01 Pacific: three minutes apart, on different local days.
        let late = message("m1", maya, at: "2026-10-08T06:58:00Z")
        let early = message("m2", maya, at: "2026-10-08T07:01:00Z")
        XCTAssertFalse(MessageGrouping.isGrouped(early, after: late, inThread: false, calendar: pacific))
        XCTAssertTrue(MessageGrouping.isGrouped(early, after: late, inThread: false, calendar: utc))
    }

    func testThreadBroadcastKeepsItsHeaderInTheChannel() {
        let first = message("m1", maya, at: "2026-10-07T10:00:00Z")
        let broadcast = message("m2", maya, at: "2026-10-07T10:01:00Z", threadRootId: "root00000001")
        XCTAssertFalse(MessageGrouping.isGrouped(broadcast, after: first, inThread: false, calendar: utc))
        XCTAssertTrue(MessageGrouping.isGrouped(broadcast, after: first, inThread: true, calendar: utc), "replies group in their thread")
        // Only the current row matters: a row under a broadcast can group.
        XCTAssertTrue(MessageGrouping.isGrouped(message("m3", maya, at: "2026-10-07T10:02:00Z"), after: broadcast, inThread: false, calendar: utc))
    }

    func testForwardsAndPinsDoNotBreakGrouping() {
        let forwarded = message("m1", maya, at: "2026-10-07T10:00:00Z", forwarded: true, pinned: true)
        let next = message("m2", maya, at: "2026-10-07T10:01:00Z", forwarded: true, pinned: true)
        XCTAssertTrue(MessageGrouping.isGrouped(next, after: forwarded, inThread: false, calendar: utc))
    }

    func testLayoutAcrossTimelineEntries() {
        let rows = [
            message("m1", maya, at: "2026-10-07T10:00:00Z"),
            message("m2", maya, at: "2026-10-07T10:01:00Z"),
            message("m3", maya, at: "2026-10-07T10:02:00Z", threadRootId: "root00000001"),
            message("m4", maya, at: "2026-10-07T10:03:00Z"),
            message("m5", alex, at: "2026-10-07T10:04:00Z"),
            message("m6", maya, at: "2026-10-07T10:20:00Z"),
            message("m7", maya, at: "2026-10-07T10:21:00Z"),
        ]
        let entries = rows.map(TimelineEntry.message)
        let channel = MessageGrouping.layout(entries, inThread: false, calendar: utc)
        XCTAssertEqual(channel.grouped, ["m2", "m4", "m7"])
        XCTAssertEqual(channel.continued, ["m1", "m3", "m6"], "the row above each grouped row")
        XCTAssertFalse(channel.pendingGrouped)
        let thread = MessageGrouping.layout(entries, inThread: true, calendar: utc)
        XCTAssertEqual(thread.grouped, ["m2", "m3", "m4", "m7"])
        XCTAssertEqual(thread.continued, ["m1", "m2", "m3", "m6"])
        XCTAssertEqual(MessageGrouping.layout([], inThread: false, calendar: utc), MessageGrouping.Layout())
    }

    func testBlockedRunsAreSeparators() {
        let before = message("m1", maya, at: "2026-10-07T10:00:00Z")
        let hidden = [message("m2", alex, at: "2026-10-07T10:01:00Z"), message("m3", alex, at: "2026-10-07T10:02:00Z")]
        let after = message("m4", maya, at: "2026-10-07T10:03:00Z")
        let collapsed: [TimelineEntry] = [.message(before), .blocked(BlockedRun(messages: hidden, revealed: false)), .message(after)]
        XCTAssertEqual(MessageGrouping.layout(collapsed, inThread: false, calendar: utc), MessageGrouping.Layout(),
                       "nothing groups across the blocked-messages placeholder")
        let revealed: [TimelineEntry] = [.message(before), .blocked(BlockedRun(messages: hidden, revealed: true)), .message(after)]
        let shown = MessageGrouping.layout(revealed, inThread: false, calendar: utc)
        XCTAssertEqual(shown.grouped, ["m3"], "a shown run groups within itself, under its Hide bar")
        XCTAssertEqual(shown.continued, ["m2"])
        // A pending message follows the placeholder, not the hidden messages behind it.
        let pending = PendingMessage(id: "p1", text: "sending", createdAt: "2026-10-07T10:02:30Z")
        XCTAssertFalse(MessageGrouping.layout(Array(collapsed.dropLast()), inThread: false, pending: pending, author: alex,
                                              calendar: utc).pendingGrouped)
        XCTAssertTrue(MessageGrouping.layout(Array(revealed.dropLast()), inThread: false, pending: pending, author: alex,
                                             calendar: utc).pendingGrouped)
    }

    func testPendingMessageGroupsUnderTheAuthorsLastRow() {
        let entries = [TimelineEntry.message(message("m1", maya, at: "2026-10-07T10:00:00Z"))]
        let pending = PendingMessage(id: "p1", text: "sending", createdAt: "2026-10-07T10:02:00Z")
        let own = MessageGrouping.layout(entries, inThread: false, pending: pending, author: maya, calendar: utc)
        XCTAssertTrue(own.pendingGrouped)
        XCTAssertEqual(own.continued, ["m1"], "the row above the pending message gives up its bottom padding")
        XCTAssertFalse(MessageGrouping.layout(entries, inThread: false, pending: pending, author: alex, calendar: utc).pendingGrouped)
        XCTAssertFalse(MessageGrouping.layout(entries, inThread: false, pending: pending, author: nil, calendar: utc).pendingGrouped)
        XCTAssertFalse(MessageGrouping.layout([], inThread: true, pending: pending, author: maya, calendar: utc).pendingGrouped,
                       "a first reply never groups with the thread root")
        let later = PendingMessage(id: "p2", text: "sending", createdAt: "2026-10-07T10:06:00Z")
        XCTAssertFalse(MessageGrouping.layout(entries, inThread: false, pending: later, author: maya, calendar: utc).pendingGrouped)
    }
}
