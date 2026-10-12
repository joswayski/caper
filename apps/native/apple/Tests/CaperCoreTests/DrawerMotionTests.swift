import XCTest
import SwiftUI
@testable import CaperCore

final class DrawerMotionTests: XCTestCase {
    func testHeldDragPastHalfwaySwitchesWithTheFullFinish() {
        // Conversation dragged 60% open and held still.
        let release = DrawerRelease(open: false, position: 0.6, velocity: 0, distance: 240, travel: 400)
        XCTAssertTrue(release.commit)
        XCTAssertEqual(release.target, 1)
        XCTAssertEqual(release.duration, 0.32, accuracy: 0.0001)
        XCTAssertEqual(release.slope, 0)
    }

    func testShortHeldDragSettlesBack() {
        let release = DrawerRelease(open: false, position: 0.3, velocity: 0, distance: 120, travel: 400)
        XCTAssertFalse(release.commit)
        XCTAssertEqual(release.target, 0)
    }

    func testFlingSwitchesEarlyAndCarriesItsSpeed() {
        // 50 points toward open at 1200 pt/s.
        let release = DrawerRelease(open: false, position: 0.125, velocity: 1200, distance: 50, travel: 400)
        XCTAssertTrue(release.commit)
        XCTAssertEqual(release.target, 1)
        // 3 travels/s over 0.875 left: 3 × 0.875 / 3 = 0.875 s, capped at 0.32.
        XCTAssertEqual(release.duration, 0.32, accuracy: 0.0001)
        // The curve starts at the finger's speed, never beyond three times the average.
        XCTAssertEqual(release.slope, min(3, 3 * 0.32 / 0.875), accuracy: 0.0001)
    }

    func testFlingBackCancelsEvenPastHalfway() {
        let release = DrawerRelease(open: false, position: 0.7, velocity: -800, distance: 280, travel: 400)
        XCTAssertFalse(release.commit)
        XCTAssertEqual(release.target, 0)
    }

    func testFlingTooShortToCountStaysPut() {
        let release = DrawerRelease(open: false, position: 0.05, velocity: 900, distance: 20, travel: 400)
        XCTAssertFalse(release.commit)
    }

    func testClosingAnOpenDrawerMirrorsOpening() {
        // Browse open, dragged 60% closed (toward the conversation).
        let release = DrawerRelease(open: true, position: 0.4, velocity: 0, distance: -240, travel: 400)
        XCTAssertTrue(release.commit)
        XCTAssertEqual(release.target, 0)
        let fling = DrawerRelease(open: true, position: 0.9, velocity: -1000, distance: -40, travel: 400)
        XCTAssertTrue(fling.commit)
        XCTAssertEqual(fling.target, 0)
        XCTAssertGreaterThan(fling.slope, 0)
    }

    func testFastFlingHasAShortBoundedFinish() {
        let release = DrawerRelease(open: false, position: 0.5, velocity: 8000, distance: 200, travel: 400)
        XCTAssertEqual(release.duration, 0.16, accuracy: 0.0001)
        XCTAssertEqual(release.slope, 3, accuracy: 0.0001)
    }

    func testSettleIsWhereTheViewIsDrawn() {
        let start = Date(timeIntervalSinceReferenceDate: 1000)
        let settle = DrawerSettle(from: 0.2, to: 1, start: start, timing: .settle)
        XCTAssertEqual(settle.value(at: start), 0.2, accuracy: 0.0001)
        XCTAssertEqual(settle.value(at: start.addingTimeInterval(1)), 1, accuracy: 0.0001)
        // Between, it follows the same curve SwiftUI animates, moving forward only.
        let curve = DrawerSettle.Timing.easing
        let midway = settle.value(at: start.addingTimeInterval(0.16))
        XCTAssertEqual(midway, 0.2 + 0.8 * CGFloat(curve.value(at: 0.5)), accuracy: 0.0001)
        XCTAssertGreaterThan(midway, 0.2)
        XCTAssertLessThan(midway, 1)
        XCTAssertFalse(settle.finished(at: start.addingTimeInterval(0.3)))
        XCTAssertTrue(settle.finished(at: start.addingTimeInterval(0.33)))
    }

    @MainActor func testSwipeKeepsControlsOffThroughItsLift() {
        SwipeTapGuard.follow()
        XCTAssertTrue(SwipeTapGuard.swiping)
        SwipeTapGuard.release()
        // The lift's own tap arrives with it.
        XCTAssertTrue(SwipeTapGuard.swiping)
        let skipped = expectation(description: "guarded")
        skipped.isInverted = true
        SwipeTapGuard.unlessSwiping { skipped.fulfill() }()
        wait(for: [skipped], timeout: 0.05)
        // Only a time window: nothing a lost gesture leaves can keep controls off.
        let ran = expectation(description: "runs once quiet")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
            XCTAssertFalse(SwipeTapGuard.swiping)
            SwipeTapGuard.unlessSwiping { ran.fulfill() }()
        }
        wait(for: [ran], timeout: 2)
    }
}
