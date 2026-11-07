import AppKit
import XCTest
@testable import CaperCore

@MainActor final class MessageTimelineTests: XCTestCase {
    private func message(_ index: Int, author: String = "peer", at date: String = "2026-11-05T09:00:00Z") -> ChatMessage {
        ChatMessage(id: "message-\(index)", channelId: "channel", seq: "\(index)",
            author: ChatAuthor(id: author, name: author, isGuest: false),
            content: ChatContent(version: 1, type: "text", text: "Row \(index) — Hello 👩🏽‍💻 مرحبا 中文"),
            createdAt: date, clientMessageId: "message-\(index)")
    }

    private func bodies(in timeline: MessageTimeline) -> [NSTextView] {
        timeline.documentView?.subviews.flatMap { $0.subviews.compactMap { $0 as? NSTextView } } ?? []
    }

    func testReplyQuoteAndThreadSummaryKeepTextSelectionAndFitAtBothWidths() throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let parent = NSViewController()
        let media = AppKitMediaPresenter(model: model, parent: parent)
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 500))
        timeline.makeReply = { media.replyContext($0) }
        var original = message(0, author: "maya")
        original.content = ChatContent(version: 1, type: "text",
            text: String(repeating: "An asymmetric quote — 👩🏽‍💻 café 中文. ", count: 15))
        var reply = message(1, author: "alex")
        reply.replyTo = MessageReply(message: original)
        reply.thread = ThreadSummary(replyCount: 2, participants: [original.author], seq: "7")
        timeline.update([reply], conversationID: "channel")
        let body = try XCTUnwrap(bodies(in: timeline).first)
        let row = try XCTUnwrap(body.superview)
        let selection = (body.string as NSString).range(of: "👩🏽‍💻")
        body.setSelectedRange(selection)
        for width in [CGFloat(640), CGFloat(300)] {
            timeline.setFrameSize(NSSize(width: width, height: 500))
            timeline.layoutSubtreeIfNeeded()
            let quote = try XCTUnwrap(row.subviews.compactMap { $0 as? AppKitAttachmentSurface }.first)
            let summary = try XCTUnwrap(row.subviews.compactMap { $0 as? NSButton }
                .first { $0.accessibilityIdentifier() == "thread-summary-message-1" })
            XCTAssertGreaterThan(quote.frame.height, 40, "Quotes must reserve visible space, not only an accessibility element")
            XCTAssertTrue(row.bounds.contains(quote.frame))
            XCTAssertGreaterThanOrEqual(body.frame.minY, quote.frame.maxY)
            XCTAssertGreaterThanOrEqual(summary.frame.minY, body.frame.maxY)
            XCTAssertTrue(row.bounds.contains(summary.frame))
            XCTAssertEqual(summary.accessibilityLabel(), "2 replies")
            XCTAssertEqual(body.accessibilitySelectedText(), "👩🏽‍💻")
        }
        original.content = ChatContent(version: 1, type: "text", text: "A replacement quote")
        reply.replyTo = MessageReply(message: original)
        reply.thread = ThreadSummary(replyCount: 3, participants: [original.author, reply.author], seq: "11")
        timeline.update([reply], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(body.selectedRange(), selection)
        XCTAssertEqual(row.subviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityIdentifier() == "thread-summary-message-1" }?.accessibilityLabel(), "3 replies")
        reply.replyTo = nil
        timeline.update([reply], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertFalse(row.subviews.contains { $0 is AppKitAttachmentSurface })
        XCTAssertEqual(body.accessibilitySelectedText(), "👩🏽‍💻")
        reply.deletedAt = "2026-11-05T11:00:00Z"
        timeline.update([reply], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(body.string, MessageDeletion.placeholder)
        let summary = try XCTUnwrap(row.subviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityIdentifier() == "thread-summary-message-1" })
        XCTAssertFalse(summary.isHidden, "Deleting the root must not strand its surviving replies")
        XCTAssertEqual(summary.accessibilityLabel(), "3 replies")
    }

    func testOriginalMessageJumpRevealsBlockedTargetWithoutRealizingInterveningHistory() throws {
        _ = NSApplication.shared
        let timeline = MessageTimeline(frame: .zero)
        let history = (0..<3000).map { message($0, author: $0 == 147 ? "blocked" : "peer") }
        var visible: Set<String> = []
        timeline.visibleMessagesChanged = { visible = $0 }
        timeline.update(history, conversationID: "channel", blocked: ["blocked"])
        XCTAssertTrue(timeline.focusMessage("message-147"), "A deep link may arrive before the window lays out")
        timeline.setFrameSize(NSSize(width: 420, height: 360))
        timeline.layoutSubtreeIfNeeded()
        let target = try XCTUnwrap(bodies(in: timeline).first { $0.string == history[147].content.text })
        let row = try XCTUnwrap(target.superview)
        XCTAssertTrue(timeline.contentView.bounds.intersects(row.frame))
        XCTAssertTrue(visible.contains("message-147"))
        XCTAssertLessThan(timeline.documentView?.accessibilityChildren()?.count ?? 3000, 40)
        XCTAssertFalse(bodies(in: timeline).contains { $0.string == history.last?.content.text })
        XCTAssertFalse(timeline.focusMessage("missing-message"))
        XCTAssertTrue(timeline.contentView.bounds.intersects(row.frame), "An unknown target must not move the reader")
        timeline.showLatest()
        timeline.layoutSubtreeIfNeeded()
        XCTAssertTrue(visible.contains("message-2999"))
        XCTAssertTrue(bodies(in: timeline).contains { $0.string == history.last?.content.text })
        XCTAssertLessThan(timeline.documentView?.accessibilityChildren()?.count ?? 3000, 40)
    }

    func testAttachmentLeavesHaveHeightAndStayInsideRealizedHistory() throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let parent = NSViewController()
        let media = AppKitMediaPresenter(model: model, parent: parent)
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        timeline.makeAttachments = { media.messageAttachments($0) }
        let history = (0..<3000).map { index -> ChatMessage in
            var value = message(index)
            value.content.attachments = [ChatAttachment(id: "image-\(index)", kind: .image, contentType: "image/png",
                name: "photo.png", size: 640, width: 240, height: 160, url: "file:///nonexistent/caper-test.png")]
            return value
        }
        timeline.update(history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let surfaces = try XCTUnwrap(timeline.documentView).subviews.flatMap { $0.subviews.compactMap { $0 as? AppKitAttachmentSurface } }
        XCTAssertFalse(surfaces.isEmpty)
        XCTAssertLessThan(surfaces.count, 10, "Media hosts exist only in the realized viewport, not all 3,000 messages")
        for surface in surfaces {
            XCTAssertGreaterThan(surface.frame.height, 100, "A media leaf must not exist only in accessibility while visually clipped to zero")
            XCTAssertTrue(try XCTUnwrap(surface.superview).bounds.contains(surface.frame))
        }
        timeline.update([message(1)], conversationID: "other")
        XCTAssertTrue(timeline.documentView?.subviews.flatMap(\.subviews).allSatisfy { !($0 is AppKitAttachmentSurface) } == true)
    }

    func testGifOnlyChangesRefreshMediaAndKeepTheCaptionSelection() throws {
        _ = NSApplication.shared
        let model = AppModel(api: APIClient(baseURL: URL(string: "https://caper.invalid")!))
        let parent = NSViewController()
        let media = AppKitMediaPresenter(model: model, parent: parent)
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        timeline.makeAttachments = { media.messageAttachments($0) }
        var value = message(0)
        timeline.update([value], conversationID: "channel")
        let body = try XCTUnwrap(bodies(in: timeline).first)
        body.setSelectedRange(NSRange(location: 0, length: 3))
        func surfaces() -> [AppKitAttachmentSurface] {
            timeline.documentView?.subviews.flatMap { $0.subviews.compactMap { $0 as? AppKitAttachmentSurface } } ?? []
        }
        XCTAssertTrue(surfaces().isEmpty)
        value.content.gif = ChatGif(kind: .sticker, slug: "fixture", title: "TEST FIXTURE sticker", width: 96, height: 64,
            gif: "https://static.klipy.com/caper-fixture/sticker.gif")
        timeline.update([value], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(surfaces().count, 1, "A GIF-only update must create its visible media, even with no attachment array")
        XCTAssertGreaterThan(try XCTUnwrap(surfaces().first).frame.height, 0)
        XCTAssertEqual(body.selectedRange(), NSRange(location: 0, length: 3))
        value.content.gif = nil
        timeline.update([value], conversationID: "channel")
        XCTAssertTrue(surfaces().isEmpty, "Removing only content.gif must tear down its animation host")
        XCTAssertEqual(body.string, value.content.text)
    }

    func testReactionUpdatesKeepFocusSelectionAndFailureControlsInsideTheRow() throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 400),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 420, height: 400))
        window.contentView = timeline
        var value = message(1)
        value.reactions = [MessageReaction(emoji: "👍", authorIds: ["peer"])]
        func update(allowed: Bool = true, failed: Bool = false) {
            timeline.update([value], conversationID: "channel", viewerID: "self", canInteract: allowed,
                reactionErrors: failed ? [value.id: "Couldn’t save reaction. Retry."] : [:])
            timeline.layoutSubtreeIfNeeded()
        }
        func visible(_ view: NSView) -> [NSView] {
            view.isHiddenOrHasHiddenAncestor ? [] : [view] + view.subviews.flatMap(visible)
        }
        func button(_ id: String) throws -> NSButton {
            try XCTUnwrap(visible(timeline).compactMap { $0 as? NSButton }.first { $0.accessibilityIdentifier() == id })
        }
        update()
        let body = try XCTUnwrap(bodies(in: timeline).first)
        let row = try XCTUnwrap(body.superview)
        let emoji = (body.string as NSString).range(of: "👩🏽‍💻")
        body.setSelectedRange(emoji)
        let quick = try button("quick-reaction-message-1-👍")
        XCTAssertTrue(window.makeFirstResponder(quick))
        value.reactions = [MessageReaction(emoji: "👍", authorIds: ["peer", "self"])]
        update()
        XCTAssertTrue(window.firstResponder === quick, "An optimistic reaction must not remove the focused quick action")
        XCTAssertEqual(quick.state, .on)
        XCTAssertEqual(body.accessibilitySelectedText(), "👩🏽‍💻")
        let chip = try button("reaction-message-1-👍")
        XCTAssertEqual(chip.title, "👍 2")
        XCTAssertTrue(window.makeFirstResponder(chip))
        value.reactions = [MessageReaction(emoji: "👍", authorIds: ["peer", "self", "other"])]
        update()
        XCTAssertTrue(window.firstResponder === chip, "Incoming counts must not replace a focused reaction chip")
        XCTAssertEqual(chip.title, "👍 3")
        let height = row.frame.height
        update(failed: true)
        let retry = try button("reaction-retry-message-1")
        let dismiss = try button("reaction-dismiss-message-1")
        XCTAssertGreaterThan(row.frame.height, height, "Error-only changes must invalidate the row measurement")
        XCTAssertGreaterThan(retry.frame.minY, chip.frame.maxY)
        for control in [retry, dismiss] { XCTAssertTrue(row.bounds.contains(control.frame)) }
        var actions: [String] = []
        timeline.messageAction = { message, action, _ in
            XCTAssertEqual(message.id, value.id)
            switch action {
            case .retryReaction: actions.append("retry")
            case .dismissReaction: actions.append("dismiss")
            default: XCTFail("Wrong action for failure recovery")
            }
        }
        retry.performClick(nil); dismiss.performClick(nil)
        XCTAssertEqual(actions, ["retry", "dismiss"])
        update(allowed: false, failed: true)
        XCTAssertFalse(retry.isEnabled)
        XCTAssertTrue(dismiss.isEnabled, "Read-only users can put an error away without mutating the server")
        XCTAssertFalse(try button("add-reaction-message-1").isEnabled)
        XCTAssertFalse(chip.isEnabled)
        update()
        XCTAssertTrue(retry.isHidden); XCTAssertTrue(dismiss.isHidden)
        XCTAssertEqual(row.frame.height, height, accuracy: 0.5)
        XCTAssertEqual(body.accessibilitySelectedText(), "👩🏽‍💻")
    }

    func testBlockedRunsRevealIndependentlyWithoutRealizingTheirWholeHistory() throws {
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        let history = [message(0, author: "blocked"), message(1, author: "blocked"),
            message(2, author: "self"), message(3, author: "blocked")]
        func visible() -> [String] { bodies(in: timeline).filter { !$0.isHidden }.map(\.string) }
        func toggle(_ label: String) throws -> NSButton {
            try XCTUnwrap(timeline.documentView?.subviews.flatMap { $0.subviews.compactMap { $0 as? NSButton } }
                .first { $0.accessibilityLabel() == label })
        }
        timeline.update(history, conversationID: "channel", viewerID: "self", blocked: ["blocked", "self"], canInteract: true)
        timeline.layoutSubtreeIfNeeded()
        XCTAssertEqual(visible(), [history[2].content.text], "Blocked content is absent; your own message never collapses")
        try toggle("Show 2 blocked messages").performClick(nil)
        XCTAssertTrue(visible().contains(history[0].content.text))
        XCTAssertTrue(visible().contains(history[1].content.text))
        XCTAssertFalse(visible().contains(history[3].content.text), "Showing one run must not reveal another")
        let revealed = try XCTUnwrap(bodies(in: timeline).first { $0.string == history[0].content.text })
        let addReaction = try XCTUnwrap(revealed.superview?.subviews.flatMap(\.subviews).compactMap { $0 as? NSButton }
            .first { $0.accessibilityIdentifier() == "add-reaction-message-0" })
        XCTAssertTrue(addReaction.isEnabled, "Revealing a blocked run must preserve the conversation's interaction permission")
        try toggle("Hide 2 blocked messages").performClick(nil)
        XCTAssertEqual(visible(), [history[2].content.text])
        timeline.update(history, conversationID: "channel", viewerID: "self")
        XCTAssertEqual(Set(visible()), Set(history.map(\.content.text)), "Unblocking restores all visible messages")

        let longRun = (0..<3000).map { message($0, author: "blocked") }
        timeline.update(longRun, conversationID: "long", viewerID: "self", blocked: ["blocked"])
        XCTAssertTrue(visible().isEmpty)
        try toggle("Show 3000 blocked messages").performClick(nil)
        XCTAssertFalse(visible().isEmpty)
        XCTAssertLessThan(visible().count, 40, "Revealing a long run still measures and exposes only its viewport")
        XCTAssertLessThan(timeline.documentView?.accessibilityChildren()?.count ?? 3000, 40)
        timeline.update(longRun + [message(3000, author: "blocked")], conversationID: "long", viewerID: "self", blocked: ["blocked"])
        XCTAssertFalse(visible().isEmpty, "An arriving message must not collapse a revealed run")
        try toggle("Hide 3001 blocked messages").performClick(nil)
        XCTAssertTrue(visible().isEmpty)
        try toggle("Show 3001 blocked messages").performClick(nil)
        timeline.update(history, conversationID: "other", viewerID: "self", blocked: ["blocked"])
        XCTAssertEqual(visible(), [history[2].content.text], "Reveal choices must not leak across conversations")
    }

    func testDateDividersFollowLocalMidnightWithoutSplittingSameDayGroups() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        let midnight = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 9, day: 24)))
        let iso = ISO8601DateFormatter()
        let history = [-60.0, 60, 180].enumerated().map { index, offset in
            message(index, at: iso.string(from: midnight.addingTimeInterval(offset)))
        } + [message(3, at: "invalid date")]
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 500))
        timeline.update(history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let rows = try XCTUnwrap(timeline.documentView).subviews
        let dates = rows.flatMap { $0.subviews.compactMap { $0 as? NSTextField } }
            .filter { !$0.isHidden && $0.accessibilityIdentifier().hasPrefix("date-") }
            .sorted { $0.convert(.zero, to: timeline.documentView).y < $1.convert(.zero, to: timeline.documentView).y }
        XCTAssertEqual(dates.map { $0.accessibilityIdentifier() }, ["date-message-0", "date-message-1"],
            "First loaded day and local midnight get dividers; same-day followups and malformed dates do not")
        let formatter = DateFormatter()
        formatter.dateStyle = .full
        XCTAssertEqual(dates.map(\.stringValue), [formatter.string(from: midnight.addingTimeInterval(-60)), formatter.string(from: midnight)])
        for label in dates {
            XCTAssertGreaterThanOrEqual(label.frame.width, try XCTUnwrap(label.cell).cellSize.width)
            let body = try XCTUnwrap(label.superview?.subviews.compactMap { $0 as? NSTextView }.first)
            XCTAssertGreaterThan(body.frame.minY, label.frame.maxY, "The divider must not overlap the message")
        }
        for (index, hidden) in [(1, false), (2, true)] {
            let body = try XCTUnwrap(bodies(in: timeline).first { $0.string == history[index].content.text })
            let author = try XCTUnwrap(body.superview?.subviews.compactMap { $0 as? NSTextField }.first { $0.stringValue == "peer" })
            XCTAssertEqual(author.isHidden, hidden, "Grouping stops at local midnight, not at every message")
        }
    }

    func testResolvedMentionsStaySelectableAndViewerChangesRefreshTheRow() throws {
        let timeline = MessageTimeline(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        var mentioned = message(0)
        mentioned.content = ChatContent(version: 1, type: "text", text: "👩🏽‍💻 **@ali**ce @nobody https://example.com", mentions: [.user(id: "alice", username: "alice")])
        timeline.update([mentioned], conversationID: "channel", viewerID: "alice")
        timeline.layoutSubtreeIfNeeded()
        let body = try XCTUnwrap(bodies(in: timeline).first)
        let row = try XCTUnwrap(body.superview)
        XCTAssertEqual(row.accessibilityHelp(), "Mentions you")
        XCTAssertEqual(body.string, "👩🏽‍💻 @alice @nobody https://example.com")
        let alice = (body.string as NSString).range(of: "@alice")
        let nobody = (body.string as NSString).range(of: "@nobody")
        let link = (body.string as NSString).range(of: "https://example.com")
        let text = body.attributedString()
        XCTAssertEqual(text.attribute(NativePresentation.mentionAttribute, at: alice.location, effectiveRange: nil) as? String, "alice")
        XCTAssertEqual((text.attribute(.backgroundColor, at: alice.location, effectiveRange: nil) as? NSColor)?.alphaComponent ?? 0, 0.24, accuracy: 0.001)
        XCTAssertNil(text.attribute(.backgroundColor, at: nobody.location, effectiveRange: nil), "Unresolved tokens remain ordinary text")
        XCTAssertEqual(text.attribute(.link, at: link.location, effectiveRange: nil) as? URL, URL(string: "https://example.com"))
        body.setSelectedRange(alice)
        XCTAssertEqual(body.accessibilitySelectedText(), "@alice")
        timeline.update([mentioned], conversationID: "channel", viewerID: "bob")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertNil(row.accessibilityHelp(), "Changing viewer must update unchanged rows too")
        XCTAssertEqual(body.accessibilitySelectedText(), "@alice")
        timeline.update([mentioned], conversationID: "channel", viewerID: nil)
        XCTAssertNil(row.accessibilityHelp())

        var everyone = message(1, author: "alice")
        everyone.content = ChatContent(version: 1, type: "text", text: "@everyone", mentions: [.everyone])
        timeline.update([everyone], conversationID: "channel", viewerID: "alice")
        let special = try XCTUnwrap(bodies(in: timeline).first)
        XCTAssertNil(special.superview?.accessibilityHelp(), "Your own everyone mention does not notify you")
        timeline.update([everyone], conversationID: "channel", viewerID: "bob")
        XCTAssertEqual(special.superview?.accessibilityHelp(), "Mentions you")
        everyone.deletedAt = "2026-11-05T10:00:00Z"
        timeline.update([everyone], conversationID: "channel", viewerID: "bob")
        XCTAssertEqual(special.string, "This message was deleted.")
        XCTAssertNil(special.superview?.accessibilityHelp())
        XCTAssertNil(special.attributedString().attribute(NativePresentation.mentionAttribute, at: 0, effectiveRange: nil))
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
        let firstDate = try XCTUnwrap(first.superview?.subviews.compactMap { $0 as? NSTextField }
            .first { $0.accessibilityIdentifier() == "date-message-50" })
        XCTAssertFalse(firstDate.isHidden)
        let before = first.convert(.zero, to: timeline.contentView).y - timeline.contentView.bounds.minY
        timeline.update((0..<50).map { message($0) } + history, conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        XCTAssertTrue(firstDate.isHidden, "Prepending the same local day moves its divider to the new first row")
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
        last.revision = 2
        timeline.update([first, previous, last], conversationID: "channel")
        timeline.layoutSubtreeIfNeeded()
        let history = try XCTUnwrap(row.subviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityIdentifier() == "message-history-message-2" })
        XCTAssertFalse(history.isHidden)
        XCTAssertGreaterThanOrEqual(body.frame.minY, history.frame.maxY, "A grouped edited marker must not cover the message text")
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
