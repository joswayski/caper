import XCTest
@testable import CaperCore

final class EmojiAutocompleteTests: XCTestCase {
    private func entry(_ id: String, _ emoji: String, _ name: String, _ keywords: String = "") -> EmojiCatalogEntry {
        EmojiCatalogEntry(id: id, emoji: emoji, name: name, keywords: keywords, category: "test", selectable: true, sheet: 0, x: 0, y: 0)
    }
    private let catalog = [
        EmojiCatalogEntry(id: "1f44d", emoji: "😀", name: "grin", keywords: "happy face", category: "test", selectable: true, sheet: 0, x: 0, y: 0),
        EmojiCatalogEntry(id: "b", emoji: "👍", name: "grinning_face", keywords: "thumb good", category: "test", selectable: true, sheet: 0, x: 0, y: 0),
        EmojiCatalogEntry(id: "c", emoji: "🎉", name: "party", keywords: "grinning celebration", category: "test", selectable: true, sheet: 0, x: 0, y: 0)
    ]

    func testBoundariesAndCaret() {
        XCTAssertNotNil(EmojiAutocomplete.match(text: ":", selection: NSRange(location: 1, length: 0), markedText: false, catalog: catalog))
        XCTAssertNotNil(EmojiAutocomplete.match(text: "hi (:gr", selection: NSRange(location: 7, length: 0), markedText: false, catalog: catalog))
        for text in ["word:gr", "12:30", "https://x", ":grin:", ":D", "ok :P", "hi (:3"] {
            XCTAssertNil(EmojiAutocomplete.match(text: text, selection: NSRange(location: (text as NSString).length, length: 0), markedText: false, catalog: catalog), text)
        }
        XCTAssertNil(EmojiAutocomplete.match(text: ":grin", selection: NSRange(location: 2, length: 0), markedText: false, catalog: catalog))
        XCTAssertNil(EmojiAutocomplete.match(text: ":gr", selection: NSRange(location: 3, length: 0), markedText: true, catalog: catalog))
        XCTAssertNil(EmojiAutocomplete.match(text: ":gr", selection: NSRange(location: 1, length: 1), markedText: false, catalog: catalog))
    }

    func testRankingAndNormalization() {
        let match = EmojiAutocomplete.match(text: ":grin", selection: NSRange(location: 5, length: 0), markedText: false, catalog: catalog)
        XCTAssertEqual(match?.choices.map(\.id), ["1f44d", "b", "c"])
        XCTAssertEqual(EmojiAutocomplete.match(text: ":grinning-face", selection: NSRange(location: 14, length: 0), markedText: false, catalog: catalog)?.choices.first?.id, "b")
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
        let match = EmojiAutocomplete.match(text: text, selection: NSRange(location: caret, length: 0), markedText: false, catalog: catalog)!
        let result = EmojiAutocomplete.inserting(catalog[0], in: text, match: match)!
        XCTAssertEqual(result.text, "👩‍💻 😀 then 🚀")
        XCTAssertEqual(result.selection.location, ("👩‍💻 😀" as NSString).length)
        XCTAssertNil(EmojiAutocomplete.inserting(entry("z", "😀😀", "two"), in: "abc :gr", match: EmojiAutocomplete.match(text: "abc :gr", selection: NSRange(location: 7, length: 0), markedText: false, catalog: catalog)!, scalarLimit: 5))
    }
}
