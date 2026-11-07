import XCTest
@testable import CaperCore

final class EmojiAutocompleteTests: XCTestCase {
    /// Every client replays these, generated from web; see shared/messages/emoji-cases.json.
    func testAutocompleteFollowsTheSharedCases() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("shared/messages/emoji-cases.json"))) as! [String: Any]
        let tokens = json["tokens"] as! [[String: Any]]
        XCTAssertGreaterThan(tokens.count, 400)
        for fixture in tokens {
            let text = fixture["text"] as! String, caret = fixture["caret"] as! Int
            let token = EmojiAutocomplete.token(text, caret: caret)
            let expected = (fixture["token"] as? [String: Any]).map {
                (NSRange(location: $0["start"] as! Int, length: ($0["end"] as! Int) - ($0["start"] as! Int)), $0["query"] as! String)
            }
            XCTAssertEqual(token?.range, expected?.0, "\(text.debugDescription) at \(caret)")
            XCTAssertEqual(token?.query, expected?.1, "\(text.debugDescription) at \(caret)")
        }
        for fixture in json["suggestions"] as! [[String: Any]] {
            let query = fixture["query"] as! String
            XCTAssertEqual(EmojiAutocomplete.suggestions(query).map(\.id), fixture["ids"] as! [String], query)
        }
    }

    func testBundledCatalogDefaultsAliasesAndPresentation() {
        func choices(_ text: String) -> [EmojiCatalogEntry] {
            EmojiAutocomplete.match(text: text, selection: NSRange(location: (text as NSString).length, length: 0), markedText: false)?.choices ?? []
        }
        XCTAssertEqual(choices(":").map(\.id), ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"])
        XCTAssertEqual(choices(":thumbs_up").first?.emoji, "👍")
        XCTAssertEqual(choices(":+1").first?.emoji, "👍")
        XCTAssertEqual(choices(":red_heart").first?.emoji, "❤️")
        XCTAssertEqual(choices(":face").count, 6)
        XCTAssertEqual(choices(":notanemojiname").count, 0)
    }

    func testInsertionPreservesUnicodeSuffixAndScalarLimit() {
        let text = "👩‍💻 :gr then 🚀"
        let caret = ("👩‍💻 :gr" as NSString).length
        let match = EmojiAutocomplete.match(text: text, selection: NSRange(location: caret, length: 0), markedText: false)!
        let choice = match.choices[0]
        let result = EmojiAutocomplete.inserting(choice, in: text, match: match)!
        XCTAssertEqual(result.text, "👩‍💻 \(choice.emoji) then 🚀")
        XCTAssertEqual(result.selection.location, ("👩‍💻 \(choice.emoji)" as NSString).length)
        XCTAssertNil(EmojiAutocomplete.inserting(choice.emoji + choice.emoji, in: "abc :gr", selection: NSRange(location: 4, length: 3), scalarLimit: 5))
    }

    func testPickerInsertionReplacesSelectionAndEnforcesScalarLimit() throws {
        let text = "👩‍💻 replace then 🚀"
        let result = try XCTUnwrap(EmojiAutocomplete.inserting("🎉", in: text, selection: NSRange(location: 6, length: 7)))
        XCTAssertEqual(result.text, "👩‍💻 🎉 then 🚀")
        XCTAssertEqual(result.selection, NSRange(location: 8, length: 0), "native offsets are UTF-16, not scalar counts")
        let caret = try XCTUnwrap(EmojiAutocomplete.inserting("👍", in: "a🚀z", selection: NSRange(location: 3, length: 0)))
        XCTAssertEqual(caret.text, "a🚀👍z")
        XCTAssertEqual(caret.selection, NSRange(location: 5, length: 0))
        let limit = String(repeating: "a", count: 3_999) + "X"
        XCTAssertEqual(EmojiAutocomplete.inserting("👍", in: limit, selection: NSRange(location: 3_999, length: 1))?.text,
                       String(repeating: "a", count: 3_999) + "👍", "replacement at the scalar limit is allowed")
        XCTAssertNil(EmojiAutocomplete.inserting("👍", in: limit, selection: NSRange(location: 4_000, length: 0)))
        XCTAssertNil(EmojiAutocomplete.inserting("👩‍💻", in: limit, selection: NSRange(location: 3_999, length: 1)))
        XCTAssertNil(EmojiAutocomplete.inserting("👍", in: text, selection: NSRange(location: 100, length: 1)))
    }
}
