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

    func testSlowReleaseSwitchesByTheFingersTravelWhenTheViewLags() {
        // The finger moved 220 of 400 points; the view, picking up 21 points
        // late, is drawn only 0.4975 open. Past halfway by the finger switches.
        let opening = DrawerRelease(open: false, position: 0.4975, velocity: 250, distance: 220, travel: 400)
        XCTAssertTrue(opening.commit)
        XCTAssertEqual(opening.target, 1)
        let closing = DrawerRelease(open: true, position: 0.5025, velocity: -250, distance: -220, travel: 400)
        XCTAssertTrue(closing.commit)
        XCTAssertEqual(closing.target, 0)
        // Short of halfway by both measures, it still settles back.
        XCTAssertFalse(DrawerRelease(open: false, position: 0.44, velocity: 250, distance: 190, travel: 400).commit)
        // A finger that went back past its start counts for nothing.
        XCTAssertFalse(DrawerRelease(open: true, position: 0.9, velocity: 0, distance: 220, travel: 400).commit)
    }

    func testHalfwayIsAcrossWhatIsSeenNotTheShadowsRoom() {
        // A 402-point phone: Browse travels 450 points with the shadow's room.
        // A slow 221-point swipe back is past the screen's middle.
        let back = DrawerRelease(open: true, position: 0.5548, velocity: -213, distance: -221, travel: 450, shadow: 48)
        XCTAssertTrue(back.commit)
        XCTAssertEqual(back.target, 0)
        // Without the shadow's room it would have counted as short of halfway.
        XCTAssertFalse(DrawerRelease(open: true, position: 0.5548, velocity: -213, distance: -221, travel: 450).commit)
        // Short of the screen's middle still settles back.
        XCTAssertFalse(DrawerRelease(open: true, position: 0.6, velocity: -213, distance: -180, travel: 450, shadow: 48).commit)
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

    func testTapSlideLeavesAtOnceAndSlowsSteadily() {
        // Web `TAP`: 300 ms, cubic-bezier(0.33, 1, 0.68, 1).
        let start = Date(timeIntervalSinceReferenceDate: 1500)
        let slide = DrawerSettle(from: 0, to: 1, start: start, timing: .tap)
        XCTAssertEqual(slide.timing.duration, 0.30, accuracy: 0.0001)
        // Moving in the first frame, but not most of the way after two of them.
        let first = slide.value(at: start.addingTimeInterval(1.0 / 60))
        let second = slide.value(at: start.addingTimeInterval(2.0 / 60))
        XCTAssertGreaterThan(first, 0.1)
        XCTAssertLessThan(second, 0.35)
        XCTAssertFalse(slide.finished(at: start.addingTimeInterval(0.29)))
        XCTAssertTrue(slide.finished(at: start.addingTimeInterval(0.31)))
        XCTAssertEqual(slide.value(at: start.addingTimeInterval(0.3)), 1, accuracy: 0.0001)
    }

    private final class TestDrawer: SlideDrawer {
        var drag: CGFloat?
        var travel: CGFloat = 400
        var settle: DrawerSettle?
        var rest: CGFloat?
        static let opening: CGFloat = 1
    }

    /// Where the drawer is drawn every 1/120 s of `seconds` from `start`.
    private func frames(_ drawer: TestDrawer, rest: CGFloat, from start: Date, seconds: Double = 0.4) -> [CGFloat] {
        stride(from: 0.0, through: seconds, by: 1.0 / 120).map {
            drawer.position(rest: rest, fallback: .tap, reduceMotion: false, at: start.addingTimeInterval($0))
        }
    }

    func testReleasedSwipeCarriesOnFromTheFingerAndNeverPassesItsRest() {
        // Browse open, dragged 30% of the way closed, then released and switched.
        let start = Date(timeIntervalSinceReferenceDate: 2000)
        let drawer = TestDrawer()
        XCTAssertEqual(drawer.position(rest: 1, fallback: .tap, reduceMotion: false, at: start), 1)
        drawer.drag = 0.7
        XCTAssertEqual(drawer.position(rest: 1, fallback: .tap, reduceMotion: false, at: start), 0.7)
        let release = DrawerRelease(open: true, position: 0.7, velocity: -600, distance: -120, travel: drawer.travel)
        XCTAssertTrue(release.commit)
        drawer.settle = DrawerSettle(from: 0.7, to: release.target, start: start, timing: .release(release))
        drawer.drag = nil
        let drawn = frames(drawer, rest: release.target, from: start)
        XCTAssertEqual(drawn.first ?? -1, 0.7, accuracy: 0.0001)
        XCTAssertEqual(drawn.last ?? -1, 0, accuracy: 0.0001)
        for (earlier, later) in zip(drawn, drawn.dropFirst()) {
            XCTAssertLessThanOrEqual(later, earlier + 0.0001, "Only moves toward the conversation")
        }
        XCTAssertGreaterThanOrEqual(drawn.min() ?? -1, 0, "Never drawn past the conversation's rest, on Browse's side")
    }

    func testModelChangeSlidesFromWhereTheDrawerWas() {
        let start = Date(timeIntervalSinceReferenceDate: 3000)
        let drawer = TestDrawer()
        _ = drawer.position(rest: 0, fallback: .tap, reduceMotion: false, at: start)
        let drawn = frames(drawer, rest: 1, from: start)
        XCTAssertEqual(drawn.first ?? -1, 0, accuracy: 0.0001, "Starts where it was rather than jumping to its new rest")
        XCTAssertEqual(drawn.last ?? -1, 1, accuracy: 0.0001)
        XCTAssertTrue(drawn.allSatisfy { (0...1).contains($0) })
    }

    func testModelChangeMidSlideTurnsAroundWhereItIsDrawn() {
        let start = Date(timeIntervalSinceReferenceDate: 4000)
        let drawer = TestDrawer()
        _ = drawer.position(rest: 0, fallback: .tap, reduceMotion: false, at: start)
        let midway = start.addingTimeInterval(0.1)
        let opening = drawer.position(rest: 1, fallback: .tap, reduceMotion: false, at: start)
        XCTAssertEqual(opening, 0, accuracy: 0.0001)
        let there = drawer.position(rest: 1, fallback: .tap, reduceMotion: false, at: midway)
        XCTAssertGreaterThan(there, 0)
        XCTAssertLessThan(there, 1)
        // Sent back before it arrives: it turns around from there, not from 1.
        XCTAssertEqual(drawer.position(rest: 0, fallback: .tap, reduceMotion: false, at: midway), there, accuracy: 0.0001)
        XCTAssertEqual(drawer.position(rest: 0, fallback: .tap, reduceMotion: false, at: midway.addingTimeInterval(1)), 0)
    }

    func testReducedMotionGoesStraightToRest() {
        let start = Date(timeIntervalSinceReferenceDate: 5000)
        let drawer = TestDrawer()
        _ = drawer.position(rest: 0, fallback: .tap, reduceMotion: true, at: start)
        XCTAssertEqual(drawer.position(rest: 1, fallback: .tap, reduceMotion: true, at: start), 1)
        XCTAssertNil(drawer.settle)
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
