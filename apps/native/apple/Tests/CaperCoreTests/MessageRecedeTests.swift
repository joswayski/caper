@testable import CaperCore
import CoreGraphics
import XCTest

final class MessageRecedeTests: XCTestCase {
    func testRowsAtTheLiveEndStayAtRest() {
        // The newest row, at least 16 points tall, ends at or above restBottom.
        let progress = MessageRecede.progress(rowTop: 484, restBottom: 500)
        XCTAssertEqual(progress, 1)
        XCTAssertEqual(MessageRecede.alpha(progress), 1)
        XCTAssertEqual(MessageRecede.transform(progress, size: CGSize(width: 320, height: 16), anchorPoint: CGPoint(x: 0.5, y: 0.5)), .identity)
    }

    func testRecedesOverFortyPointsFromTwentyFourBelowRest() {
        XCTAssertEqual(MessageRecede.progress(rowTop: 524, restBottom: 500), 0)
        XCTAssertEqual(MessageRecede.progress(rowTop: 600, restBottom: 500), 0)
        XCTAssertEqual(MessageRecede.progress(rowTop: 504, restBottom: 500), 0.5, accuracy: 0.0001)
        XCTAssertEqual(MessageRecede.alpha(0), 0.7, accuracy: 0.0001)
    }

    func testShrinksAboutTheBottomCentreWhateverTheAnchor() {
        let size = CGSize(width: 200, height: 50)
        for anchor in [CGPoint(x: 0.5, y: 0.5), CGPoint(x: 0, y: 0)] {
            let transform = MessageRecede.transform(0, size: size, anchorPoint: anchor)
            let pivot = CGPoint(x: size.width * anchor.x, y: size.height * anchor.y)
            // The bottom centre, relative to the anchor, lands 4 points lower.
            let bottom = CGPoint(x: size.width / 2 - pivot.x, y: size.height - pivot.y).applying(transform)
            XCTAssertEqual(bottom.x, size.width / 2 - pivot.x, accuracy: 0.0001)
            XCTAssertEqual(bottom.y, size.height - pivot.y + 4, accuracy: 0.0001)
            XCTAssertEqual(transform.a, 0.98, accuracy: 0.0001)
        }
    }
}
