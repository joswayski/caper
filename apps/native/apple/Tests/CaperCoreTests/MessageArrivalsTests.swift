import XCTest
@testable import CaperCore

/// Mirrors apps/web/src/chat/arrivals.test.ts.
final class MessageArrivalsTests: XCTestCase {
    private var time: TimeInterval = 0
    private lazy var arrivals = MessageArrivals { [unowned self] in self.time }

    func testChannelFirstRenderAndSwitchDoNotAnimate() {
        arrivals.observe("channel", key: "a", ids: ["1", "2"], live: true)
        XCTAssertFalse(arrivals.has("2"))
        arrivals.observe("channel", key: "b", ids: ["3", "4"], live: true)
        XCTAssertFalse(arrivals.has("4"))
    }

    func testRowsAppendedAtTheLiveEndAnimate() {
        arrivals.observe("channel", key: "a", ids: ["1", "2"], live: true)
        XCTAssertEqual(arrivals.revision("channel"), 0)
        arrivals.observe("channel", key: "a", ids: ["1", "2", "3"], live: true)
        XCTAssertTrue(arrivals.has("3"))
        XCTAssertFalse(arrivals.has("2"))
        XCTAssertEqual(arrivals.revision("channel"), 1)
        arrivals.observe("channel", key: "a", ids: ["0", "1", "2", "3"], live: true)
        XCTAssertEqual(arrivals.revision("channel"), 1)
    }

    func testEmptyListsFirstMessagesDoNotAnimate() {
        arrivals.observe("channel", key: "a", ids: [], live: true)
        arrivals.observe("channel", key: "a", ids: ["1", "2"], live: true)
        XCTAssertFalse(arrivals.has("1"))
    }

    func testOlderPagesAndNewerPagesTowardsTheLiveEndDoNotAnimate() {
        arrivals.observe("channel", key: "a", ids: ["5", "6"], live: false)
        arrivals.observe("channel", key: "a", ids: ["3", "4", "5", "6"], live: false)
        arrivals.observe("channel", key: "a", ids: ["3", "4", "5", "6", "7", "8"], live: true)
        XCTAssertFalse(["3", "4", "7", "8"].contains(where: arrivals.has))
        arrivals.observe("channel", key: "a", ids: ["1", "2", "3", "4", "5", "6", "7", "8"], live: true)
        XCTAssertFalse(arrivals.has("1"))
    }

    func testConfirmedSendAnimatesOnceKeepingItsClientMessageID() {
        arrivals.observe("channel", key: "a", ids: ["1"], live: true)
        arrivals.observe("channel", key: "a", ids: ["1", "sent"], live: true)
        XCTAssertTrue(arrivals.has("sent"))
        time += MessageArrivals.window + 0.001
        arrivals.observe("channel", key: "a", ids: ["1", "sent"], live: true)
        XCTAssertFalse(arrivals.has("sent"))
    }

    func testChannelAndThreadListsStayApart() {
        arrivals.observe("channel", key: "a", ids: ["1"], live: true)
        arrivals.observe("thread", key: "root", ids: ["r1"], live: true)
        arrivals.observe("thread", key: "root", ids: ["r1", "r2"], live: true)
        arrivals.observe("channel", key: "a", ids: ["1"], live: true)
        XCTAssertTrue(arrivals.has("r2"))
    }

    /// Native rows are separate views for a pending send and its confirmed
    /// message, and lazy stacks rebuild rows: only the first view animates.
    func testOnlyTheFirstRowShowingAnArrivalClaimsIt() {
        arrivals.observe("channel", key: "a", ids: ["1"], live: true)
        XCTAssertFalse(arrivals.claim("1"))
        arrivals.observe("channel", key: "a", ids: ["1", "sent"], live: true)
        XCTAssertTrue(arrivals.claim("sent"))
        // The pending row gives way to the confirmed message with the same ID.
        arrivals.observe("channel", key: "a", ids: ["1"], live: true)
        arrivals.observe("channel", key: "a", ids: ["1", "sent"], live: true)
        XCTAssertFalse(arrivals.claim("sent"))
        XCTAssertTrue(arrivals.has("sent"))
        XCTAssertEqual(arrivals.revision("channel"), 1)
    }
}
