import XCTest
@testable import CaperCore

/// Runs every case in shared/messages/link-cases.json, as web and the other
/// native clients do (apps/web/src/server/links.test.ts).
final class MessageLinksTests: XCTestCase {
    private struct Cases: Decodable {
        struct Case: Decodable {
            let note: String
            let text: String
            let segments: [Segment]
        }
        struct Segment: Decodable {
            let text: String
            let href: String?
        }
        let cases: [Case]
    }

    /// The repository's shared cases, found from this file: Tests/CaperCoreTests
    /// sits five directories below the repository root.
    private func sharedCases() throws -> [Cases.Case] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/messages/link-cases.json")
        return try JSONDecoder().decode(Cases.self, from: Data(contentsOf: url)).cases
    }

    func testSharedLinkCases() throws {
        let cases = try sharedCases()
        XCTAssertGreaterThan(cases.count, 30, "link-cases.json lost its cases")
        for item in cases {
            let expected = item.segments.map { LinkSegment(text: $0.text, href: $0.href) }
            XCTAssertEqual(MessageLinks.segments(in: item.text), expected, item.note)
        }
    }

    func testSegmentsKeepTheWholeText() throws {
        for item in try sharedCases() {
            XCTAssertEqual(MessageLinks.segments(in: item.text).map(\.text).joined(), item.text, item.note)
        }
        XCTAssertEqual(MessageLinks.segments(in: ""), [])
    }

    func testOnlyWebLinksEverOpen() {
        let text = "javascript:alert(1) mailto:a@b.com ftp://example.com file:///etc/hosts data:text/html,x https://ok.example.com"
        let hrefs = MessageLinks.segments(in: text).compactMap(\.href)
        XCTAssertEqual(hrefs, ["https://ok.example.com"])
        XCTAssertTrue(hrefs.allSatisfy { $0.hasPrefix("https://") || $0.hasPrefix("http://") })
    }

    /// Links are found in the text between mention pills, never inside one.
    func testLinksRunOnTheTextBetweenMentions() {
        let content = ChatContent(version: 1, type: "text", text: "@maya see https://example.com/a?b=c, then @alex",
                                  mentions: [.user(id: "member000001", username: "maya"), .user(id: "member000002", username: "alex")])
        let segments = MentionAutocomplete.segments(in: content.text, mentions: content.mentions)
        XCTAssertEqual(segments.map(\.highlighted), [true, false, true])
        XCTAssertEqual(MessageLinks.segments(in: segments[1].text), [
            LinkSegment(text: " see "),
            LinkSegment(text: "https://example.com/a?b=c", href: "https://example.com/a?b=c"),
            LinkSegment(text: ", then "),
        ])
    }
}
