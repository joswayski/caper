import XCTest
@testable import CaperCore

/// Quick switcher matching and conversation stepping: every case in
/// shared/shortcuts/switcher-cases.json, which every client runs.
final class KeyboardNavigationTests: XCTestCase {
    private struct Cases: Decodable {
        struct Match: Decodable { let query: String; let text: String; let score: Int? }
        struct Item: Decodable { let name: String; let alternates: [String]; let unread: Bool }
        struct Rank: Decodable { let note: String; let query: String; let items: [Item]; let order: [Int] }
        struct Adjacent: Decodable { let unread: [Bool]; let current: Int?; let forward: Bool; let unreadOnly: Bool; let result: Int? }
        let match: [Match]
        let rank: [Rank]
        let adjacent: [Adjacent]
    }

    private func cases() throws -> Cases {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/shortcuts/switcher-cases.json")
        return try JSONDecoder().decode(Cases.self, from: Data(contentsOf: url))
    }

    func testMatchingFollowsTheSharedCases() throws {
        let cases = try cases()
        for item in cases.match {
            XCTAssertEqual(KeyboardNavigation.score(item.query, item.text), item.score, "\(item.query) ~ \(item.text)")
        }
        for item in cases.rank {
            let ranked = KeyboardNavigation.rank(item.items.map { ([$0.name] + $0.alternates, $0.unread) }, query: item.query)
            XCTAssertEqual(ranked, item.order, item.note)
        }
    }

    func testSteppingFollowsTheSharedCases() throws {
        for item in try cases().adjacent {
            XCTAssertEqual(KeyboardNavigation.adjacent(unread: item.unread, current: item.current, forward: item.forward,
                                                       unreadOnly: item.unreadOnly), item.result, "\(item)")
        }
    }
}
