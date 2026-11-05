import AppKit
import XCTest
@testable import CaperCore

@MainActor final class MessageTimelineTests: XCTestCase {
    private func message(_ index: Int, author: String = "peer") -> ChatMessage {
        ChatMessage(id: "message-\(index)", channelId: "channel", seq: "\(index)",
            author: ChatAuthor(id: author, name: author, isGuest: false),
            content: ChatContent(version: 1, type: "text", text: "Row \(index) — Hello 👩🏽‍💻 مرحبا 中文"),
            createdAt: "2026-11-05T09:00:00Z", clientMessageId: "message-\(index)")
    }

    private func bodies(in timeline: MessageTimeline) -> [NSTextView] {
        timeline.documentView?.subviews.flatMap { $0.subviews.compactMap { $0 as? NSTextView } } ?? []
    }

    func testScrollingIntoHistoryLoadsAPageWithoutMovingTheReader() throws {
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 360))
        let history = (50..<100).map { message($0) }
        var requested = false
        timeline.canLoadOlder = true
        timeline.loadOlder = { requested = true }
        timeline.update(history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertFalse(requested, "Opening the latest page must not fetch the entire history")
        timeline.contentView.scroll(to: NSPoint(x: 0, y: 0))
        timeline.layoutSubtreeIfNeeded()
        XCTAssertTrue(requested, "Scrolling to the start requests older messages without a button")
        XCTAssertFalse(timeline.canLoadOlder, "Do not issue another request before the model updates")
        let first = try XCTUnwrap(bodies(in: timeline).first { $0.string == history.first?.content.text })
        let before = first.convert(.zero, to: timeline.contentView).y - timeline.contentView.bounds.minY
        timeline.update((0..<50).map { message($0) } + history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let retained = try XCTUnwrap(bodies(in: timeline).first { $0.string == first.string })
        XCTAssertEqual(retained.convert(.zero, to: timeline.contentView).y - timeline.contentView.bounds.minY, before, accuracy: 0.5,
            "Text stays put when grouping hides its header: body \(retained.frame), row \(String(describing: retained.superview?.frame)), clip \(timeline.contentView.bounds)")

        // A live arrival while reading history must not steal the viewport.
        timeline.update((0..<101).map { message($0) }, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(retained.convert(.zero, to: timeline.contentView).y - timeline.contentView.bounds.minY, before, accuracy: 0.5)
        timeline.update([message(101)], conversationID: "other-channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(bodies(in: timeline).map(\.string), [message(101).content.text])
    }

    func testEmojiKeepsLinePitchAndTimestampsFollowAuthorNames() throws {
        CaperFontLoader.register()
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        var plain = message(0, author: "Li")
        plain.content = ChatContent(version: 1, type: "text", text: "First line\nSecond line\nThird line")
        var emoji = message(1, author: "A much longer author name")
        emoji.content = ChatContent(version: 1, type: "text", text: "First 🔥 line\nSecond 🚀 line\nThird 🎉 line")
        timeline.update([plain, emoji], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let plainBody = try XCTUnwrap(bodies(in: timeline).first { $0.string == plain.content.text })
        let emojiBody = try XCTUnwrap(bodies(in: timeline).first { $0.string == emoji.content.text })
        XCTAssertEqual(plainBody.frame.height, emojiBody.frame.height, accuracy: 0.5)
        for body in [plainBody, emojiBody] {
            let layout = try XCTUnwrap(body.layoutManager)
            var origins: [CGFloat] = []
            layout.enumerateLineFragments(forGlyphRange: NSRange(location: 0, length: layout.numberOfGlyphs)) { rect, _, _, _, _ in
                origins.append(rect.minY)
            }
            XCTAssertEqual(origins.count, 3)
            for index in 1..<origins.count { XCTAssertEqual(origins[index] - origins[index - 1], 20, accuracy: 0.5) }
            let row = try XCTUnwrap(body.superview)
            let labels = row.subviews.compactMap { $0 as? NSTextField }
            let name = try XCTUnwrap(labels.first { [plain.author.name, emoji.author.name].contains($0.stringValue) })
            let time = try XCTUnwrap(labels.first { $0.stringValue == NativePresentation.time(plain.createdAt) })
            XCTAssertGreaterThanOrEqual(name.frame.width, try XCTUnwrap(name.cell).cellSize.width, "The full author name must fit when space is available")
            XCTAssertGreaterThanOrEqual(time.frame.width, try XCTUnwrap(time.cell).cellSize.width, "The time must not be ellipsized")
            XCTAssertGreaterThanOrEqual(time.frame.minX, name.frame.maxX)
            XCTAssertLessThanOrEqual(time.frame.minX - name.frame.maxX, 10)
            XCTAssertLessThanOrEqual(time.frame.maxX, row.bounds.maxX)
        }
        let rocket = (emojiBody.string as NSString).range(of: "🚀")
        emojiBody.setSelectedRange(rocket)
        XCTAssertEqual(emojiBody.accessibilitySelectedText(), "🚀")
    }

    func testThreeThousandRowsExposeOnlyRealizedSelectableTextAndKeepHistoryAnchor() throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 360),
            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 360))
        window.contentView = timeline
        let history = (0..<3000).map { message($0, author: "peer-\(($0 / 3) % 7)") }
        timeline.update(history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let document = try XCTUnwrap(timeline.documentView)
        XCTAssertGreaterThan(document.accessibilityChildren()?.count ?? 0, 0)
        XCTAssertLessThan(document.accessibilityChildren()?.count ?? 3000, 40,
            "Accessibility must not manufacture an element for every loaded message")
        let latest = try XCTUnwrap(bodies(in: timeline).first { $0.string == history.last?.content.text })
        window.makeFirstResponder(latest)
        let emoji = (latest.string as NSString).range(of: "👩🏽‍💻")
        latest.setSelectedRange(emoji)
        XCTAssertEqual(latest.accessibilitySelectedText(), "👩🏽‍💻")
        XCTAssertEqual(latest.accessibilitySelectedTextRange(), emoji)
        XCTAssertEqual(latest.accessibilityValue() as? String, latest.string)

        timeline.contentView.scroll(to: NSPoint(x: 0, y: 460))
        timeline.layoutSubtreeIfNeeded()
        XCTAssertFalse(bodies(in: timeline).contains { $0.string == history.last?.content.text })
        let anchor = try XCTUnwrap(bodies(in: timeline).first {
            guard let row = $0.superview else { return false }
            return row.frame.contains(NSPoint(x: 1, y: timeline.contentView.bounds.minY))
        })
        let anchorText = anchor.string
        let offset = try XCTUnwrap(anchor.superview).frame.minY - timeline.contentView.bounds.minY
        timeline.update((-50..<0).map { message($0) } + history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let afterPrepend = try XCTUnwrap(bodies(in: timeline).first { $0.string == anchorText })
        XCTAssertEqual(try XCTUnwrap(afterPrepend.superview).frame.minY - timeline.contentView.bounds.minY, offset, accuracy: 0.5)
        window.setContentSize(NSSize(width: 420, height: 360))
        timeline.layoutSubtreeIfNeeded()
        XCTAssertTrue(bodies(in: timeline).contains { $0.string == anchorText }, "Resize must not snap a history reader to the end")
        XCTAssertLessThan(document.accessibilityChildren()?.count ?? 3000, 40)
        timeline.contentView.scroll(to: NSPoint(x: 0, y: document.bounds.height - timeline.contentSize.height))
        timeline.layoutSubtreeIfNeeded()
        let returned = try XCTUnwrap(bodies(in: timeline).first { $0.string == history.last?.content.text })
        XCTAssertEqual(returned.accessibilitySelectedText(), "👩🏽‍💻", "Recycling preserves selection of unchanged text")
    }

    func testDeletingAPredecessorAndEditingTextRemeasuresTheVisibleRow() throws {
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        let first = message(0, author: "alex")
        let previous = message(1, author: "maya")
        var last = message(2, author: "maya")
        timeline.update([first, previous, last], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let body = try XCTUnwrap(bodies(in: timeline).first { $0.string == last.content.text })
        let row = try XCTUnwrap(body.superview)
        let author = try XCTUnwrap(row.subviews.compactMap { $0 as? NSTextField }.first { $0.stringValue == "maya" })
        XCTAssertTrue(author.isHidden, "Consecutive messages from the same author are grouped")
        timeline.update([first, last], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertFalse(author.isHidden, "Deleting the preceding message reveals the author again")
        XCTAssertGreaterThanOrEqual(body.frame.minY, author.frame.maxY)
        last.content = ChatContent(version: 1, type: "text",
            text: Array(repeating: "A longer edited message that wraps at a narrow width.", count: 12).joined(separator: "\n"))
        timeline.update([first, last], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let edited = try XCTUnwrap(bodies(in: timeline).first { $0.string == last.content.text })
        let container = try XCTUnwrap(edited.textContainer)
        let layout = try XCTUnwrap(edited.layoutManager)
        layout.ensureLayout(for: container)
        XCTAssertGreaterThanOrEqual(edited.frame.height, ceil(layout.usedRect(for: container).height), "Updated text must not clip")
    }
}
