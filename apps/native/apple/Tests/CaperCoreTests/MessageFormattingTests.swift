import XCTest
@testable import CaperCore
#if os(macOS)
import AppKit
#else
import UIKit
#endif

final class MessageFormattingTests: XCTestCase {
    @MainActor func testNativeTextKeepsResolvedMentionBoundariesAcrossFormatting() throws {
        let source = "👩‍💻 **@ali**ce bob**@alice** @bob @everyone @here @feedback **https://example.com/a_b**"
        let text = NativePresentation.messageText(source, mentions: [
            .user(id: "alice-id", username: "alice"), .everyone, .feedback,
        ])
        XCTAssertEqual(text.string, "👩‍💻 @alice bob@alice @bob @everyone @here @feedback https://example.com/a_b")
        let visible = text.string as NSString
        var pills: [(String, String)] = []
        text.enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: text.length)) { value, range, _ in
            if let id = value as? String { pills.append((visible.substring(with: range), id)) }
        }
        XCTAssertEqual(pills.map { $0.0 }, ["@alice", "@everyone", "@feedback"])
        XCTAssertEqual(pills.map { $0.1 }, ["alice-id", "", ""])
        let alice = visible.range(of: "@alice")
        #if os(macOS)
        let color = try XCTUnwrap(text.attribute(.backgroundColor, at: alice.location, effectiveRange: nil) as? NSColor)
        #else
        let color = try XCTUnwrap(text.attribute(.backgroundColor, at: alice.location, effectiveRange: nil) as? UIColor)
        #endif
        XCTAssertEqual(color.cgColor.alpha, 0.24, accuracy: 0.001)
        XCTAssertNil(text.attribute(.link, at: alice.location, effectiveRange: nil), "Hosts handle person pills, not the URL opener")
        let link = visible.range(of: "https://example.com/a_b")
        XCTAssertEqual(text.attribute(.link, at: link.location, effectiveRange: nil) as? URL,
            URL(string: "https://example.com/a_b"))
        let unresolved = NativePresentation.messageText(source)
        XCTAssertEqual(unresolved.string, text.string)
        unresolved.enumerateAttribute(NativePresentation.mentionAttribute, in: NSRange(location: 0, length: unresolved.length)) { value, _, _ in
            XCTAssertNil(value, "A plain @name without server-resolved metadata is not a pill")
        }
    }

    func testVisibleNavigationDeletionAndClipboard() {
        let text = "**a👩‍💻b**"
        XCTAssertEqual(MessageFormatting.normalizedSelection(text, selection: NSRange(location: 1, length: 0)), NSRange(location: 2, length: 0))
        let afterEmoji = MessageFormatting.movedSelection(text, selection: NSRange(location: 3, length: 0), backward: false, extending: false)
        XCTAssertEqual(afterEmoji, NSRange(location: 8, length: 0))
        XCTAssertEqual(MessageFormatting.deletionRange(text, selection: afterEmoji, backward: true), NSRange(location: 3, length: 5))
        XCTAssertEqual(MessageFormatting.movedSelection(text, selection: NSRange(location: 11, length: 0), backward: true, extending: false), NSRange(location: 8, length: 0))
        XCTAssertEqual(MessageFormatting.copiedText(text, selection: NSRange(location: 0, length: 11)), "a👩‍💻b")
        XCTAssertEqual(MessageFormatting.copiedText("**hello**", selection: NSRange(location: 3, length: 3)), "ell")
        XCTAssertEqual(MessageFormatting.movedSelection("**a**b", selection: NSRange(location: 2, length: 0), backward: false, extending: false), NSRange(location: 5, length: 0))
    }

    func testReplacementAndStyledLinkEdgeEdits() {
        XCTAssertEqual(MessageFormatting.repairEdit("**hello**", text: "**new**", selection: NSRange(location: 5, length: 0)).text, "**new**")
        let url = "https://example.com/a_b"
        for edited in ["** \(url)**", "**\(url) **"] {
            let result = MessageFormatting.repairEdit("**\(url)**", text: edited, selection: NSRange(location: 3, length: 0))
            XCTAssertEqual(MessageLinks.segments(in: result.text).compactMap(\.href), [url])
            XCTAssertEqual(MessageFormatting.projection(result.text).text, edited.replacingOccurrences(of: "**", with: ""))
        }
    }

    func testFormattedCompletionKeepsWireRanges() throws {
        let text = "**:rocket**"
        let match = try XCTUnwrap(ComposerAutocomplete.match(text: text, selection: NSRange(location: 9, length: 0), markedText: false, mentions: .empty))
        guard case let .emoji(emoji) = match else { return XCTFail("Expected emoji completion") }
        XCTAssertEqual(emoji.replacementRange, NSRange(location: 2, length: 7))
        let result = try XCTUnwrap(ComposerAutocomplete.inserting(choice: 0, in: text, match: match))
        XCTAssertEqual(result.text, "**🚀**")
        XCTAssertEqual(result.selection, NSRange(location: 4, length: 0))
        let person = MentionCandidate.member(id: "alice", username: "alice", displayName: "Alice")
        let mention = MentionAutocompleteMatch(replacementRange: NSRange(location: 2, length: 3), choices: [person])
        XCTAssertEqual(ComposerAutocomplete.inserting(choice: 0, in: "**@al**", match: .mention(mention))?.text, "**@alice** ")
        XCTAssertEqual(EmojiAutocomplete.inserting("🚀", in: "**hello** tail", selection: NSRange(location: 0, length: 7))?.text, "🚀 tail")
    }

    func testEditsDiscardOnlyOrphanedDelimiters() {
        for (before, after, caret, expected, expectedCaret) in [
            ("**hello**", "****", 2, "", 0),
            ("**_🙂_**", "**__**", 3, "", 0),
            ("a **bold** z", "a ** z", 4, "a  z", 2),
            ("**hello** tail", "**llo** tail", 2, "**llo** tail", 2),
            ("a **", "a *", 3, "a *", 3),
        ] {
            let result = MessageFormatting.repairEdit(before, text: after, selection: NSRange(location: caret, length: 0))
            XCTAssertEqual(result.text, expected)
            XCTAssertEqual(result.selection, NSRange(location: expectedCaret, length: 0))
        }
        let result = MessageFormatting.toggle("**hello** rest", selection: NSRange(location: 2, length: 7), marker: "**")
        XCTAssertEqual(result?.text, "hello rest")
        XCTAssertEqual(result?.selection, NSRange(location: 0, length: 5))
        XCTAssertNil(MessageFormatting.toggle("identifier", selection: NSRange(location: 2, length: 3), marker: "_"))
    }

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
            let projection = MessageFormatting.projection(fixture.text)
            let units = Array(fixture.text.utf16)
            XCTAssertEqual(String(decoding: projection.sourceIndices.map { units[$0] }, as: UTF16.self), fixture.runs.map(\.text).joined())
        }
    }

    func testURLBoundariesCannotBeSplit() throws {
        let text = "🙂 https://example.com/a_b?q=one&x=two tail"
        let source = text as NSString
        let start = source.range(of: "https:").location, end = source.range(of: " tail").location
        for marker in ["**", "_"] {
            for (a, b) in [(start + 2, start + 2), (start + 2, end), (0, end - 2), (start, end - 1)] {
                let selection = NSRange(location: a, length: b - a)
                let result = try XCTUnwrap(MessageFormatting.toggle(text, selection: selection, marker: marker))
                XCTAssertEqual(result.text, text)
                XCTAssertEqual(result.selection, selection)
            }
            let result = try XCTUnwrap(MessageFormatting.toggle(text, selection: NSRange(location: start, length: end - start), marker: marker))
            XCTAssertEqual(MessageFormatting.runs(result.text).map(\.text).joined(), text)
            XCTAssertEqual(MessageLinks.segments(in: result.text).compactMap(\.href), MessageLinks.segments(in: text).compactMap(\.href))
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
            ("", NSRange(location: 0, length: 0), "**", "", NSRange(location: 0, length: 0)),
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
