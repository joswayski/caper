import XCTest
@testable import CaperCore

final class WhatsNewTests: XCTestCase {
    private func revision(_ sequence: Int) -> String { String(repeating: "0", count: 39) + String(sequence) }
    private func snapshot(_ last: Int, silent: Set<Int> = []) -> WhatsNewSnapshot {
        WhatsNewSnapshot(schema: 1, platform: "ios", revision: revision(last), sequence: Int64(last), releases: (1...last).map {
            PlatformRelease(sequence: Int64($0), revision: revision($0), publishedAt: "2026-10-11T19:00:00Z",
                            highlights: silent.contains($0) ? [] : [ReleaseHighlight(title: "Update \($0)", body: "Useful change \($0).")])
        })
    }

    func testBaselineCatchUpDismissalAndRetainedSettingsHistory() throws {
        var ack: Int64?
        let history = WhatsNewHistory(read: { ack }, write: { ack = $0 })
        try history.receive(snapshot(1), platform: "ios", revision: revision(1))
        XCTAssertEqual(ack, 1)
        XCTAssertTrue(history.unread.isEmpty)
        try history.receive(snapshot(4, silent: [3]), platform: "ios", revision: revision(4))
        XCTAssertEqual(history.unread.map(\.sequence), [4, 2])
        XCTAssertEqual(ack, 1)
        history.acknowledge(try XCTUnwrap(history.snapshot).sequence)
        XCTAssertEqual(ack, 4)
        XCTAssertTrue(history.unread.isEmpty)
        XCTAssertEqual(history.retained.map(\.sequence), [4, 2, 1])
        let relaunched = WhatsNewHistory(read: { ack }, write: { ack = $0 })
        try relaunched.receive(snapshot(4, silent: [3]), platform: "ios", revision: revision(4))
        XCTAssertTrue(relaunched.unread.isEmpty)
    }

    func testSilentUpdatesAndRollbackNeverLowerAcknowledgement() throws {
        var ack: Int64? = 1
        let history = WhatsNewHistory(read: { ack }, write: { ack = $0 })
        try history.receive(snapshot(3, silent: [2, 3]), platform: "ios", revision: revision(3))
        XCTAssertEqual(ack, 3)
        XCTAssertTrue(history.unread.isEmpty)
        try history.receive(snapshot(1), platform: "ios", revision: revision(1))
        history.acknowledge(1)
        XCTAssertEqual(ack, 3)
        try history.receive(snapshot(4), platform: "ios", revision: revision(4))
        XCTAssertEqual(history.unread.map(\.sequence), [4])
    }

    func testInvalidRevisionPlatformOrderingAndSequenceNeverAcknowledge() throws {
        var ack: Int64?
        let history = WhatsNewHistory(read: { ack }, write: { ack = $0 })
        let value = snapshot(3)
        XCTAssertThrowsError(try history.receive(value, platform: "ios", revision: revision(2)))
        XCTAssertThrowsError(try history.receive(value, platform: "macos", revision: revision(3)))
        let malformedRelease = PlatformRelease(sequence: 1, revision: revision(1) + "\n", publishedAt: "2026-10-11T19:00:00Z", highlights: [])
        for releases in [Array(value.releases.reversed()), value.releases + [value.releases.last!], [malformedRelease] + value.releases.dropFirst()] {
            let wrong = WhatsNewSnapshot(schema: 1, platform: "ios", revision: revision(3), sequence: 3, releases: releases)
            XCTAssertThrowsError(try history.receive(wrong, platform: "ios", revision: revision(3)))
        }
        for sequence: Int64 in [0, 4, 9_007_199_254_740_992] {
            let wrong = WhatsNewSnapshot(schema: 1, platform: "ios", revision: revision(3), sequence: sequence, releases: value.releases)
            XCTAssertThrowsError(try history.receive(wrong, platform: "ios", revision: revision(3)))
        }
        XCTAssertNil(ack)
        XCTAssertNil(history.snapshot)
        try history.receive(value, platform: "ios", revision: revision(3))
        XCTAssertThrowsError(try history.receive(snapshot(4), platform: "ios", revision: revision(3)))
        XCTAssertEqual(history.snapshot, value)
        XCTAssertEqual(ack, 3)
    }

    func testTextBoundsCountUnicodeScalarsNotGraphemes() throws {
        let highlight = ReleaseHighlight(title: String(repeating: "🙂", count: 100), body: String(repeating: "e\u{301}", count: 250))
        func value(_ highlights: [ReleaseHighlight]) -> WhatsNewSnapshot {
            WhatsNewSnapshot(schema: 1, platform: "ios", revision: revision(1), sequence: 1,
                             releases: [PlatformRelease(sequence: 1, revision: revision(1), publishedAt: "2026-10-11T19:00:00Z", highlights: highlights)])
        }
        _ = try value([highlight]).validated(platform: "ios", revision: revision(1))
        for invalid in [ReleaseHighlight(title: highlight.title + "x", body: highlight.body),
                        ReleaseHighlight(title: highlight.title, body: highlight.body + "x"),
                        ReleaseHighlight(title: "Title", body: "\u{0}")] {
            XCTAssertThrowsError(try value([invalid]).validated(platform: "ios", revision: revision(1)))
        }
        XCTAssertThrowsError(try value(Array(repeating: highlight, count: 6)).validated(platform: "ios", revision: revision(1)))
    }
}
