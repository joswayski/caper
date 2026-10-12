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
}
