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

    func testFullDateIsLocalized() throws {
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertEqual(
            ChatDateDivider.fullDateLabel("2026-09-29T06:30:00Z", locale: Locale(identifier: "en_US"), timeZone: zone),
            "Monday, September 28, 2026"
        )
    }
}
