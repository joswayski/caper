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

    /// Every client types these same steps, generated from web; see
    /// shared/messages/composer-cases.json. A placed step is a tap: a new caret
    /// and a fresh ComposerTyping, so no style carries over.
    func testComposerTypingFollowsTheSharedCases() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("shared/messages/composer-cases.json"))) as! [String: Any]
        let cases = json["cases"] as! [[String: Any]]
        XCTAssertGreaterThan(cases.count, 20)
        for fixture in cases {
            var text = fixture["text"] as! String
            var caret = MessageFormatting.projection(text).original(fixture["caret"] as! Int)
            var typing = ComposerTyping()
            for step in fixture["steps"] as! [[String: Any]] {
                if step["placed"] as! Bool {
                    caret = MessageFormatting.normalizedSelection(text, selection: NSRange(location: caret, length: 0)).location
                    typing = ComposerTyping()
                }
                let key = step["key"] as! String, range = NSRange(location: caret, length: 0)
                let proposed = (text as NSString).replacingCharacters(in: range, with: key)
                let result = typing.repairEdit(text, text: proposed, selection: NSRange(location: caret + key.utf16.count, length: 0), replacing: range)
                text = result.text
                caret = MessageFormatting.normalizedSelection(text, selection: result.selection).location
                // The selection notification typing itself posts keeps the style.
                typing.observe(text, selection: NSRange(location: caret, length: 0))
                let note = "\(fixture["note"] as! String): \(key.debugDescription)"
                XCTAssertEqual(text, step["text"] as! String, note)
                XCTAssertEqual(MessageFormatting.projection(text).displayed(caret), step["caret"] as! Int, note)
            }
        }
    }

    /// The style continues only while the composer shows what typing left:
    /// moving the caret away and back, undo then redo, or another conversation
    /// with an identical draft all type plain again.
    func testComposerTypingEndsWhenTheComposerChangesUnderIt() {
        func type(_ typing: inout ComposerTyping, _ text: String, _ key: String) -> String {
            let caret = (text as NSString).length, range = NSRange(location: caret, length: 0)
            let result = typing.repairEdit(text, text: text + key, selection: NSRange(location: caret + key.utf16.count, length: 0), replacing: range)
            typing.observe(result.text, selection: result.selection)
            return result.text
        }
        var typing = ComposerTyping(), text = type(&typing, "**bold**", " ")
        XCTAssertEqual(type(&typing, text, "a"), "**bold** **a**", "continuous typing")
        typing = ComposerTyping(); text = type(&typing, "**bold**", " ")
        typing.observe(text, selection: NSRange(location: 0, length: 0))
        typing.observe(text, selection: NSRange(location: 9, length: 0))
        XCTAssertEqual(type(&typing, text, "a"), "**bold** a", "caret moved away and back")
        typing = ComposerTyping(); text = type(&typing, "**bold**", " ")
        typing.observe("**bold**", selection: NSRange(location: 8, length: 0))
        typing.observe(text, selection: NSRange(location: 9, length: 0))
        XCTAssertEqual(type(&typing, text, "a"), "**bold** a", "undo then redo")
        typing = ComposerTyping(); text = type(&typing, "**bold**", " ")
        typing.end()
        XCTAssertEqual(type(&typing, text, "a"), "**bold** a", "identical draft in another conversation")
    }

    /// The SwiftUI editors reuse one text view and coordinator across
    /// conversations and threads, handing the coordinator each update's
    /// configuration. Typing on in the same conversation keeps the style; a
    /// cached thread root or another conversation with the identical draft and
    /// caret types plain.
    @MainActor func testEditorCoordinatorEndsTypingInAnotherConversation() {
        func editor(_ conversation: String) -> NativeMessageTextEditor {
            NativeMessageTextEditor(text: .constant(""), placeholder: "Message", controller: ComposerAutocompleteController(),
                                    mentions: .empty, submit: {}, conversation: conversation)
        }
        for (next, expected) in [("1:channel", "**bold** **a**"), ("1:channel:root-b", "**bold** a"), ("2:channel", "**bold** a")] {
            let coordinator = NativeMessageTextEditor.Coordinator(editor("1:channel"))
            #if os(macOS)
            let view = ComposerTextView()
            view.delegate = coordinator; coordinator.view = view
            view.string = "**bold**"; view.setSelectedRange(NSRange(location: 8, length: 0))
            // A key as AppKit delivers it to the focused field: shouldChange (the
            // coordinator may apply its own repair and refuse), then the change and
            // its selection. A view that isn't first responder would instead wrap
            // each programmatic insert in begin/end editing, which ends the style.
            func type(_ key: String) {
                let range = view.selectedRange()
                guard coordinator.textView(view, shouldChangeTextIn: range, replacementString: key) else { return }
                view.string = (view.string as NSString).replacingCharacters(in: range, with: key)
                view.setSelectedRange(NSRange(location: range.location + key.utf16.count, length: 0))
            }
            func text() -> String { view.string }
            #else
            let view = ComposerTextView(frame: .zero, textContainer: nil)
            view.delegate = coordinator; coordinator.view = view
            view.text = "**bold**"; view.selectedRange = NSRange(location: 8, length: 0)
            // A key as UIKit delivers it to the focused field; a view that isn't
            // first responder skips shouldChange for insertText.
            func type(_ key: String) {
                let range = view.selectedRange
                guard coordinator.textView(view, shouldChangeTextIn: range, replacementText: key) else { return }
                view.text = (view.text as NSString).replacingCharacters(in: range, with: key)
                view.selectedRange = NSRange(location: range.location + key.utf16.count, length: 0)
            }
            func text() -> String { view.text }
            #endif
            type(" ")
            XCTAssertEqual(text(), "**bold** ", next)
            coordinator.adopt(editor(next))
            type("a")
            XCTAssertEqual(text(), expected, next)
        }
    }

    /// The bench's formatted draft, typed key by key through the editor's own
    /// delegate as the system delivers keys. When a key closes a style (the
    /// second `*` of `**Release check**`) the text is unchanged, so the system
    /// applies it and places the caret after the markers; the style typing
    /// carries must survive that, or the next space reopens bold on every word.
    @MainActor func testEditorCoordinatorTypesAFormattedDraftExactly() {
        let draft = "**Release check** for _the voice beta_: see https://example.com/docs/42?tab=voice#s-3 "
            + "and www.example.org/specs/17. @maya can you confirm **both _builds_** and *the narrow layout*? "
            + "**Done** when _history loading_ and **link previews** look right."
        let coordinator = NativeMessageTextEditor.Coordinator(
            NativeMessageTextEditor(text: .constant(""), placeholder: "Message", controller: ComposerAutocompleteController(),
                                    mentions: .empty, submit: {}, conversation: "1:channel"))
        #if os(macOS)
        let view = ComposerTextView()
        view.delegate = coordinator; coordinator.view = view
        for key in draft.map(String.init) {
            let range = view.selectedRange()
            guard coordinator.textView(view, shouldChangeTextIn: range, replacementString: key) else { continue }
            view.string = (view.string as NSString).replacingCharacters(in: range, with: key)
            view.setSelectedRange(NSRange(location: range.location + key.utf16.count, length: 0))
            coordinator.textDidChange(Notification(name: NSText.didChangeNotification, object: view))
        }
        XCTAssertEqual(view.string, draft)
        #else
        let view = ComposerTextView(frame: .zero, textContainer: nil)
        view.delegate = coordinator; coordinator.view = view
        for key in draft.map(String.init) {
            let range = view.selectedRange
            guard coordinator.textView(view, shouldChangeTextIn: range, replacementText: key) else { continue }
            view.text = (view.text as NSString).replacingCharacters(in: range, with: key)
            view.selectedRange = NSRange(location: range.location + key.utf16.count, length: 0)
            coordinator.textViewDidChangeSelection(view)
            coordinator.textViewDidChange(view)
        }
        XCTAssertEqual(view.text, draft)
        #endif
    }

    /// The same, for any composer: the caret repairEdit returns is the one the
    /// composer shows, so a key the system applies itself keeps the chain.
    func testComposerTypingKeepsTheChainWhenTheSystemAppliesAKey() {
        for draft in ["**hello** world", "**Release check** for _the voice beta_: see", "**both _builds_** and _soft_ too"] {
            var typing = ComposerTyping(), text = "", caret = 0
            for key in draft.map(String.init) {
                let range = NSRange(location: caret, length: 0)
                let proposed = (text as NSString).replacingCharacters(in: range, with: key)
                let typed = NSRange(location: caret + key.utf16.count, length: 0)
                let result = typing.repairEdit(text, text: proposed, selection: typed, replacing: range)
                text = result.text
                // An unchanged edit is the system's own: it reports the typed caret, then the composer normalizes it.
                let reported = text == proposed ? typed : result.selection
                typing.observe(text, selection: reported)
                caret = MessageFormatting.normalizedSelection(text, selection: reported).location
            }
            XCTAssertEqual(text, draft)
        }
    }

    /// Types one character at a time the way both native composers do: each
    /// edit goes through repairEdit, then the caret through normalizedSelection.
    /// Closing a style at the end of the draft leaves the caret after it, so
    /// typing on neither lands inside the style nor orphans its markers.
    func testTypingPastAClosingDelimiterKeepsTheStyle() {
        for (typed, expected) in [("**bold** and", "**bold** and"), ("_soft_ and **bold** too", "_soft_ and **bold** too")] {
            var text = "", caret = 0
            for character in typed {
                let insertion = String(character)
                let proposed = (text as NSString).replacingCharacters(in: NSRange(location: caret, length: 0), with: insertion)
                let repaired = MessageFormatting.repairEdit(text, text: proposed,
                                                            selection: NSRange(location: caret + insertion.utf16.count, length: 0))
                text = repaired.text
                caret = MessageFormatting.normalizedSelection(text, selection: repaired.selection).location
            }
            XCTAssertEqual(text, expected)
            XCTAssertEqual(caret, (expected as NSString).length)
        }
        // A caret at the very end of a styled draft sits after its closing marker.
        XCTAssertEqual(MessageFormatting.normalizedSelection("**bold**", selection: NSRange(location: 8, length: 0)), NSRange(location: 8, length: 0))
        XCTAssertEqual(MessageFormatting.normalizedSelection("**bold**", selection: NSRange(location: 6, length: 0)), NSRange(location: 8, length: 0))
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
        let result = MessageFormatting.toggle("**hello** rest", selection: NSRange(location: 2, length: 7), style: .bold)
        XCTAssertEqual(result?.text, "hello rest")
        XCTAssertEqual(result?.selection, NSRange(location: 0, length: 5))
        XCTAssertNil(MessageFormatting.toggle("identifier", selection: NSRange(location: 2, length: 3), style: .italic))
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
        for item in MessageFormattingAction.all {
            // An empty selection, or one splitting the link, takes no style.
            for (a, b) in [(start + 2, start + 2), (start + 2, end), (0, end - 2), (start, end - 1)] {
                XCTAssertNil(MessageFormatting.toggle(text, selection: NSRange(location: a, length: b - a), style: item.style), item.title)
            }
            let whole = MessageFormatting.toggle(text, selection: NSRange(location: start, length: end - start), style: item.style)
            if item.style == .bold { XCTAssertNotNil(whole) }
            guard let result = whole else { continue }
            XCTAssertEqual(MessageFormatting.runs(result.text).map(\.text).joined(), text, item.title)
            // Renderers find links in each run's visible text, as web's LinkedText;
            // `~~` and `||` around a URL hide it from a scan of the raw source.
            let rendered = MessageFormatting.runs(result.text).flatMap { MessageLinks.segments(in: $0.text).compactMap(\.href) }
            XCTAssertEqual(rendered, MessageLinks.segments(in: text).compactMap(\.href), item.title)
        }
    }

    /// Every toolbar toggle web makes, through the shared toggle; see
    /// shared/messages/toggle-cases.json. Cases give visible UTF-16 offsets,
    /// mapped to the source as shared/messages/composer.rs's test does.
    func testToolbarTogglesFollowTheSharedCases() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("shared/messages/toggle-cases.json"))) as! [String: Any]
        let cases = json["cases"] as! [[String: Any]]
        XCTAssertGreaterThan(cases.count, 300)
        let styles: [String: MessageStyle] = ["bold": .bold, "italic": .italic, "underline": .underline,
                                              "strikethrough": .strikethrough, "spoiler": .spoiler, "quote": .quote]
        for fixture in cases {
            let text = fixture["text"] as! String
            let style = try XCTUnwrap(styles[fixture["style"] as! String])
            let from = fixture["from"] as! Int, to = fixture["to"] as! Int
            let shown = MessageFormatting.projection(text)
            let start = from < shown.sourceIndices.count ? shown.sourceIndices[from] : shown.sourceLength
            let end = to > from ? shown.sourceIndices[to - 1] + 1 : start
            let result = MessageFormatting.toggle(text, selection: NSRange(location: start, length: end - start), style: style)
            let note = "\(fixture)"
            guard let expected = fixture["result"] as? String else {
                XCTAssertNil(result, note)
                continue
            }
            let toggled = try XCTUnwrap(result, note)
            XCTAssertEqual(toggled.text, expected, note)
            // The selection keeps its visible place.
            let after = MessageFormatting.projection(toggled.text)
            XCTAssertEqual(after.displayed(toggled.selection.location), from, note)
            XCTAssertEqual(after.displayed(NSMaxRange(toggled.selection)), to, note)
        }
    }

    func testToggleRefusesInvalidSelections() {
        XCTAssertNil(MessageFormatting.toggle("x", selection: NSRange(location: NSNotFound, length: 0), style: .italic))
        XCTAssertNil(MessageFormatting.toggle("x", selection: NSRange(location: 0, length: 2), style: .bold))
        XCTAssertNil(MessageFormatting.toggle("", selection: NSRange(location: 0, length: 0), style: .bold))
    }

    /// The pressed toolbar buttons: every style any visible selected character has.
    func testSelectionStylesAreThePressedButtons() {
        let text = "**a** __b__ ~~c~~ ||d||\n> e"
        let source = text as NSString
        func styles(_ visible: String) -> MessageStyle {
            MessageFormatting.selectionStyles(text, selection: source.range(of: visible))
        }
        XCTAssertEqual(styles("a"), .bold)
        XCTAssertEqual(styles("b"), .underline)
        XCTAssertEqual(styles("c"), .strikethrough)
        XCTAssertEqual(styles("d"), .spoiler)
        XCTAssertEqual(styles("e"), .quote)
        XCTAssertEqual(MessageFormatting.selectionStyles(text, selection: NSRange(location: 0, length: source.length)),
                       [.bold, .underline, .strikethrough, .spoiler, .quote])
        XCTAssertEqual(MessageFormatting.selectionStyles(text, selection: NSRange(location: 2, length: 0)), [], "a caret presses nothing")
    }

    func testRunsCarryEveryStyle() {
        let runs = MessageFormatting.runs("__u__ ~~s~~ ||p||\n> q")
        XCTAssertEqual(runs.map(\.text).joined(), "u s p\nq")
        XCTAssertTrue(runs.contains { $0.text == "u" && $0.underline })
        XCTAssertTrue(runs.contains { $0.text == "s" && $0.strikethrough })
        XCTAssertTrue(runs.contains { $0.text == "p" && $0.spoiler })
        XCTAssertTrue(runs.contains { $0.text == "q" && $0.quote })
        let blocks = MessageFormatting.quoteBlocks(runs)
        XCTAssertEqual(blocks.map { $0.quote }, [false, true])
        XCTAssertEqual(MessageFormatting.stackedBlocks(runs).map { $0.range }, [0..<5, 6..<7], "the newline before a quote block is its edge")
    }
}
