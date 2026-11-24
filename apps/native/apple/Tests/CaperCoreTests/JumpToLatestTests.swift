@testable import CaperCore
import XCTest

final class JumpToLatestTests: XCTestCase {
    private func message(_ id: String, author: String) -> ChatMessage {
        ChatMessage(id: id, channelId: "Aaaaaaaaaaaa", seq: id, author: ChatAuthor(id: author, name: author, isGuest: false),
                    content: ChatContent(version: 1, type: "text", text: id), createdAt: "2026-01-01T00:00:00Z", clientMessageId: "c-\(id)")
    }

    func testShowsFromTenMessagesBelowALiveHistoryAndAlwaysInADetachedWindow() {
        XCTAssertFalse(JumpToLatest.shows(below: 9, live: true))
        XCTAssertTrue(JumpToLatest.shows(below: 10, live: true))
        XCTAssertTrue(JumpToLatest.shows(below: 50, live: false))
        XCTAssertTrue(JumpToLatest.shows(below: 0, live: false), "a detached window shows it even at its end")
    }

    func testLabelsCountAndCapAtNinetyNine() {
        XCTAssertNil(JumpToLatest.countText(0))
        XCTAssertEqual(JumpToLatest.countText(1), "1 new message")
        XCTAssertEqual(JumpToLatest.countText(3), "3 new messages")
        XCTAssertEqual(JumpToLatest.countText(99), "99 new messages")
        XCTAssertEqual(JumpToLatest.countText(100), "99+ new messages")
        XCTAssertEqual(JumpToLatest.label(0), "You’re viewing older messages")
        XCTAssertEqual(JumpToLatest.label(3), "3 new messages")
        XCTAssertEqual(JumpToLatest.accessibilityLabel(0), "Jump to latest")
        XCTAssertEqual(JumpToLatest.accessibilityLabel(3), "Jump to latest, 3 new messages")
    }

    func testCountsOnlyOthersMessagesAppendedAtTheLiveEnd() {
        let known = [message("b", author: "peer"), message("c", author: "me")]
        let next = [message("a", author: "peer")] + known + [message("d", author: "peer"), message("e", author: "me"), message("f", author: "other")]
        // "a" was prepended history and "e" is the reader's own.
        XCTAssertEqual(JumpToLatest.arrivals(next, previousIDs: Set(known.map(\.id)), viewerID: "me"), 2)
        XCTAssertEqual(JumpToLatest.arrivals(next, previousIDs: [], viewerID: "me"), 0, "a new page is not an arrival")
    }

    func testAnimatesUnlessReducedMotionOrManyScreensAway() {
        XCTAssertTrue(JumpToLatest.animates(distanceFromEnd: 1200, viewport: 600, reduceMotion: false))
        XCTAssertFalse(JumpToLatest.animates(distanceFromEnd: 1200, viewport: 600, reduceMotion: true))
        XCTAssertFalse(JumpToLatest.animates(distanceFromEnd: 6000, viewport: 600, reduceMotion: false))
    }
}
