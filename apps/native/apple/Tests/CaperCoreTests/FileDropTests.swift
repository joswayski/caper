import XCTest
@testable import CaperCore

/// Runs every case in shared/messages/drop-cases.json, as web and the other
/// native clients do (apps/web/src/server/drop.test.ts).
final class FileDropTests: XCTestCase {
    private struct Cases: Decodable {
        struct Case: Decodable {
            let note: String
            let channel: String
            let direct: Bool
            let attached: Int
            let title: String
            let detail: String
        }
        let cases: [Case]
    }

    func testSharedDropCases() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/messages/drop-cases.json")
        let cases = try JSONDecoder().decode(Cases.self, from: Data(contentsOf: url)).cases
        XCTAssertGreaterThanOrEqual(cases.count, 5, "drop-cases.json lost its cases")
        for item in cases {
            let text = AttachmentPolicy.dropText(channel: item.channel, direct: item.direct, attached: item.attached)
            XCTAssertEqual(text.title, item.title, item.note)
            XCTAssertEqual(text.detail, item.detail, item.note)
        }
    }

    private struct Motion: Decodable {
        struct Card: Decodable { let x, y, turn, opacity: Double }
        struct Character: Decodable { let y, scaleX, scaleY, turn, opacity: Double }
        struct Case: Decodable {
            let elapsed: Double
            let still: Bool
            let cards: [Card]
            let character: Character
        }
        struct Random: Decodable {
            let seed: UInt32
            let values: [Double]
        }
        let random: Random
        let seed: UInt32
        let cases: [Case]
    }

    /// Every client derives the same motion from a seed (shared/messages/drop-motion-cases.json).
    func testSharedDropMotion() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/messages/drop-motion-cases.json")
        let motion = try JSONDecoder().decode(Motion.self, from: Data(contentsOf: url))
        var random = FileDropMotion.Random(seed: motion.random.seed)
        for value in motion.random.values { XCTAssertEqual(random.next(), value, accuracy: 1e-6) }
        let plan = FileDropMotion.Plan(seed: motion.seed)
        XCTAssertGreaterThanOrEqual(motion.cases.count, 5, "drop-motion-cases.json lost its cases")
        for item in motion.cases {
            let pose = FileDropMotion.pose(plan, item.elapsed, still: item.still)
            let at = "\(item.elapsed)"
            for (card, expected) in zip(pose.cards, item.cards) {
                XCTAssertEqual(card.x, expected.x, accuracy: 1e-4, at)
                XCTAssertEqual(card.y, expected.y, accuracy: 1e-4, at)
                XCTAssertEqual(card.turn, expected.turn, accuracy: 1e-4, at)
                XCTAssertEqual(card.opacity, expected.opacity, accuracy: 1e-4, at)
            }
            XCTAssertEqual(pose.character.y, item.character.y, accuracy: 1e-4, at)
            XCTAssertEqual(pose.character.scaleX, item.character.scaleX, accuracy: 1e-4, at)
            XCTAssertEqual(pose.character.scaleY, item.character.scaleY, accuracy: 1e-4, at)
            XCTAssertEqual(pose.character.turn, item.character.turn, accuracy: 1e-4, at)
            XCTAssertEqual(pose.character.opacity, item.character.opacity, accuracy: 1e-4, at)
        }
    }
}
