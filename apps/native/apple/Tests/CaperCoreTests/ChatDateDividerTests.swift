import XCTest
@testable import CaperCore

final class ChatDateDividerTests: XCTestCase {
    func testLocalDayBoundariesUseCalendarTimeZone() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertTrue(ChatDateDivider.sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T06:45:00Z", calendar: calendar))
        XCTAssertFalse(ChatDateDivider.sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T07:30:00Z", calendar: calendar))
    }

    func testFullDateIsLocalized() throws {
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        XCTAssertEqual(
            ChatDateDivider.fullDateLabel("2026-09-29T06:30:00Z", locale: Locale(identifier: "en_US"), timeZone: zone),
            "Monday, September 28, 2026"
        )
    }
}
