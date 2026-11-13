import XCTest
@testable import CaperCore

final class ChatDateDividerTests: XCTestCase {
    func testLocalDayBoundariesUseCalendarTimeZone() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertTrue(ChatDateDivider.sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T06:45:00Z", calendar: calendar))
        XCTAssertFalse(ChatDateDivider.sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T07:30:00Z", calendar: calendar))
    }

    /// Parsed dates are memoized: a repeat, or a parse from another thread,
    /// must return the same instant, and a bad value stays unparsed.
    func testRepeatedParsesReturnTheSameDate() {
        let fractional = "2026-09-29T06:30:00.250Z", whole = "2026-09-29T06:30:00Z"
        let expected = Date(timeIntervalSince1970: 1_790_663_400)
        XCTAssertEqual(ChatTimestamp.date(whole), expected)
        XCTAssertEqual(ChatTimestamp.date(whole), expected)
        XCTAssertEqual(ChatTimestamp.date(fractional), expected.addingTimeInterval(0.25))
        XCTAssertEqual(ChatTimestamp.date(fractional), expected.addingTimeInterval(0.25))
        XCTAssertNil(ChatTimestamp.date("not a date"))
        XCTAssertNil(ChatTimestamp.date("not a date"))
        let results = UnsafeMutableBufferPointer<Date?>.allocate(capacity: 200)
        defer { results.deallocate() }
        DispatchQueue.concurrentPerform(iterations: 200) { index in
            results[index] = ChatTimestamp.date(String(format: "2026-09-29T06:30:%02dZ", index % 60))
        }
        for index in 0..<200 { XCTAssertEqual(results[index], expected.addingTimeInterval(Double(index % 60))) }
    }

    /// The thread applies the channel's rule to its own rows: the root, then
    /// each reply on another local day than the row directly above it.
    func testThreadRowsAreDividedByLocalDay() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        let author = ChatAuthor(id: "member000001", name: "Maya", isGuest: false)
        func message(_ id: String, at createdAt: String) -> ChatMessage {
            ChatMessage(id: id, channelId: "chan00000001", seq: id, author: author,
                        content: ChatContent(version: 1, type: "text", text: "hi"), createdAt: createdAt,
                        clientMessageId: "c-\(id)")
        }
        // Local days: the 28th for the root and "1", the 29th for "2" and "3", the 30th for "4".
        let root = "2026-09-29T06:30:00Z"
        let replies = [message("1", at: "2026-09-29T06:45:00Z"), message("2", at: "2026-09-29T07:30:00Z"),
                       message("3", at: "2026-09-29T20:00:00Z"), message("4", at: "2026-09-30T07:00:00Z")]
        let rows = replies.map(TimelineEntry.message)

        XCTAssertEqual(ChatDateDivider.divided([.message(message("0", at: root))], calendar: calendar), ["0"])
        XCTAssertEqual(ChatDateDivider.divided(rows, after: root, calendar: calendar), ["2", "4"])
        // Without a shown root, the first shown reply carries the divider.
        XCTAssertEqual(ChatDateDivider.divided(rows, calendar: calendar), ["1", "2", "4"])
        XCTAssertEqual(ChatDateDivider.divided([], after: root, calendar: calendar), [])

        // A collapsed run is one row: it compares by its first message above and its last below.
        let run = TimelineEntry.blocked(BlockedRun(messages: [replies[0], replies[1]], revealed: false))
        XCTAssertEqual(ChatDateDivider.divided([run, rows[2], rows[3]], after: root, calendar: calendar), ["4"])
        XCTAssertEqual(ChatDateDivider.divided([rows[0], .blocked(BlockedRun(messages: [replies[1], replies[3]], revealed: false))],
                                               after: root, calendar: calendar), ["4"])
    }

    func testFullDateIsLocalized() throws {
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertEqual(
            ChatDateDivider.fullDateLabel("2026-09-29T06:30:00Z", locale: Locale(identifier: "en_US"), timeZone: zone),
            "Monday, September 28, 2026"
        )
    }
}
