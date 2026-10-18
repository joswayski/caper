import CoreGraphics
import XCTest
@testable import CaperCore

final class SentGalleryTests: XCTestCase {
    func testTwoAndThreeUseGapAwareLandscapeGeometry() {
        let two = SentGalleryGeometry.frames(count: 2, width: 480)
        XCTAssertEqual(two, [CGRect(x: 0, y: 0, width: 238, height: 320),
                             CGRect(x: 242, y: 0, width: 238, height: 320)])
        let three = SentGalleryGeometry.frames(count: 3, width: 480)
        XCTAssertEqual(three[0].width, 476 * 2 / 3, accuracy: 0.001)
        XCTAssertEqual(three[1].width, 476 / 3, accuracy: 0.001)
        XCTAssertEqual(three[1].height, 158)
        XCTAssertEqual(three[2].minY, 162)
        XCTAssertEqual(three[2].maxX, 480, accuracy: 0.001)
        XCTAssertEqual(three[2].maxY, 320)
    }

    func testShortRowsComeFirstAndEveryRowFillsAvailableWidth() {
        for width in [CGFloat(280), 375, 480, 700] {
            for count in 4...10 {
                let frames = SentGalleryGeometry.frames(count: count, width: width)
                let actualWidth = min(480, width)
                let rows = (count + 2) / 3
                let shortRows = rows * 3 - count
                let rowHeight = count == 4 ? (actualWidth - 4) / 2 : (actualWidth - 8) / 3
                XCTAssertEqual(frames.count, count)
                var index = 0
                for row in 0..<rows {
                    let columns = row < shortRows ? 2 : 3
                    let cells = Array(frames[index..<(index + columns)])
                    XCTAssertEqual(cells[0].minX, 0)
                    XCTAssertEqual(cells.last!.maxX, actualWidth, accuracy: 0.001)
                    for cell in cells {
                        XCTAssertEqual(cell.minY, CGFloat(row) * (rowHeight + 4), accuracy: 0.001)
                        XCTAssertEqual(cell.height, rowHeight, accuracy: 0.001)
                    }
                    for column in 1..<columns {
                        XCTAssertEqual(cells[column].minX - cells[column - 1].maxX, 4, accuracy: 0.001)
                    }
                    index += columns
                }
            }
        }
    }
}
