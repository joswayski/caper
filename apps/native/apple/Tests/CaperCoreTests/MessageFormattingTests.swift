import XCTest
@testable import CaperCore

final class MessageFormattingTests: XCTestCase {
    private struct Fixtures: Decodable {
        struct Case: Decodable {
            let text: String
            let runs: [FormattedMessageRun]
        }
        let cases: [Case]
    }

    func testSharedParserFixtures() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("shared/messages/formatting-cases.json"))
        let fixtures = try JSONDecoder().decode(Fixtures.self, from: data)
        XCTAssertFalse(fixtures.cases.isEmpty)
        for fixture in fixtures.cases {
            XCTAssertEqual(MessageFormatting.runs(fixture.text), fixture.runs, fixture.text)
        }
    }

    func testToggleSelectionFixtures() throws {
        let fixtures: [(String, NSRange, String, String, NSRange)] = [
            ("hello", NSRange(location: 0, length: 5), "**", "**hello**", NSRange(location: 2, length: 5)),
            ("**hello**", NSRange(location: 2, length: 5), "**", "hello", NSRange(location: 0, length: 5)),
            ("**hello**", NSRange(location: 0, length: 9), "**", "hello", NSRange(location: 0, length: 5)),
            ("_hello_", NSRange(location: 1, length: 5), "_", "hello", NSRange(location: 0, length: 5)),
            ("_hello_", NSRange(location: 0, length: 7), "_", "hello", NSRange(location: 0, length: 5)),
            ("🙂 hi", NSRange(location: 3, length: 2), "_", "🙂 _hi_", NSRange(location: 4, length: 2)),
            ("", NSRange(location: 0, length: 0), "**", "****", NSRange(location: 2, length: 0)),
            ("_x_", NSRange(location: 0, length: 3), "**", "**_x_**", NSRange(location: 2, length: 3)),
            (" word \n", NSRange(location: 0, length: 7), "_", " _word_ \n", NSRange(location: 2, length: 4)),
            (" \t", NSRange(location: 0, length: 2), "**", " \t", NSRange(location: 2, length: 0)),
            ("**bold** plus normal", NSRange(location: 0, length: 20), "**", "**bold plus normal**", NSRange(location: 2, length: 16)),
            ("**one** and **two**", NSRange(location: 0, length: 19), "**", "**one and two**", NSRange(location: 2, length: 11)),
            ("**bold _both_**", NSRange(location: 0, length: 15), "_", "_**bold both**_", NSRange(location: 1, length: 13)),
        ]
        for (text, selection, marker, expected, expectedSelection) in fixtures {
            let result = try XCTUnwrap(MessageFormatting.toggle(text, selection: selection, marker: marker))
            XCTAssertEqual(result.text, expected)
            XCTAssertEqual(result.selection, expectedSelection)
        }
        XCTAssertNil(MessageFormatting.toggle("x", selection: NSRange(location: NSNotFound, length: 0), marker: "_"))
    }
}
