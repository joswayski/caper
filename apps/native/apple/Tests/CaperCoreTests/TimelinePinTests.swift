import XCTest
@testable import CaperCore

final class TimelinePinTests: XCTestCase {
    private func pin(height: CGFloat = 300, contentHeight: CGFloat = 900, offset: CGFloat = 600,
                     atBottom: Bool = true, firstID: String = "first", reacted: Bool = false) -> TimelinePin {
        var message = ChatMessage(id: "latest", channelId: "channel", seq: "1",
                                  author: ChatAuthor(id: "peer", name: "Peer", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "Latest message"),
                                  createdAt: "2026-10-14T07:00:00Z", clientMessageId: "latest")
        if reacted { message.reactions = [MessageReaction(emoji: "👍", authorIds: ["peer"])] }
        return TimelinePin(height: height, contentHeight: contentHeight, offset: offset, atBottom: atBottom,
                           firstID: firstID, recent: [.message(message)])
    }

    func testReactionMeasuredAfterModelChangeStaysAboveKeyboard() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        // The model changes while the keyboard is open, before chip layout.
        XCTAssertFalse(follow.needsReveal(to: pin(reacted: true)))
        // A chip taller than the 24pt bottom tolerance is now clipped.
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false, reacted: true)))
        XCTAssertTrue(follow.atBottom, "Keep follow until the requested scroll is measured")
        // A second layout pass wraps the chips before scrollTo has settled.
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 988, atBottom: false, reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 988, reacted: true)))
    }

    func testReactionAndChipHeightChangingTogetherFollow() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false, reacted: true)))
    }

    func testKeyboardReturningDuringReactionLayoutKeepsFollow() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin(height: 600)))
        XCTAssertFalse(follow.needsReveal(to: pin(height: 600, reacted: true)))
        XCTAssertTrue(follow.needsReveal(to: pin(height: 300, atBottom: false, reacted: true)))
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false, reacted: true)))
    }

    func testInteractiveKeyboardDismissalAndReversalDoNotRepin() {
        for reacted in [false, true] {
            for finalHeight: CGFloat in [300, 600] {
                let follow = TimelinePinBox()
                XCTAssertFalse(follow.needsReveal(to: pin()))
                XCTAssertFalse(follow.needsReveal(to: pin(reacted: reacted)))
                // The keyboard follows the finger down, then back up. The
                // reader may still be within the bottom tolerance at first.
                for height: CGFloat in [420, 460, 360] {
                    XCTAssertFalse(follow.needsReveal(to: pin(height: height, offset: 890 - height,
                                                             reacted: reacted), scrolling: true),
                                   "A viewport resize during a drag must not issue scrollTo")
                }
                // Either cancel dismissal or finish it while leaving the end.
                let settled = pin(height: finalHeight, offset: 800 - finalHeight, atBottom: false, reacted: reacted)
                XCTAssertFalse(follow.needsReveal(to: settled, scrolling: true))
                XCTAssertFalse(follow.atBottom, "The drag must cancel pending reaction follow")
                // Even a delayed layout arriving as scrolling ends must not
                // replay follow from before the drag.
                XCTAssertFalse(follow.needsReveal(to: pin(height: finalHeight, contentHeight: 944,
                                                         offset: 800 - finalHeight, atBottom: false, reacted: reacted)))
                // Returning to the end restores ordinary keyboard avoidance.
                XCTAssertFalse(follow.needsReveal(to: pin(height: 600, offset: 300, reacted: reacted)))
                XCTAssertTrue(follow.needsReveal(to: pin(height: 300, offset: 300, atBottom: false, reacted: reacted)))
            }
        }
    }

    func testRemovingAttachmentRowsKeepsLatestAtComposerWithoutMovingHistoryReader() {
        for readingHistory in [false, true] {
            let follow = TimelinePinBox()
            var offset: CGFloat = readingHistory ? 200 : 600
            XCTAssertFalse(follow.needsReveal(to: pin(offset: offset, atBottom: !readingHistory)))
            // Removing one wrapped row, then the final file, expands the
            // viewport before SwiftUI corrects its old bottom scroll offset.
            for height: CGFloat in [412, 524] {
                XCTAssertEqual(follow.needsReveal(to: pin(height: height, offset: offset, atBottom: !readingHistory)), !readingHistory,
                               "Only a reader already at the bottom should be re-pinned after removing files")
                if !readingHistory { offset = 900 - height }
                XCTAssertFalse(follow.needsReveal(to: pin(height: height, offset: offset, atBottom: !readingHistory)),
                               "The correction must settle without repeated scrolling")
            }
        }
    }

    func testHistoryReaderDoesNotFollowReactionsOrKeyboard() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin(atBottom: false)))
        XCTAssertFalse(follow.needsReveal(to: pin(atBottom: false, reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(height: 200, contentHeight: 944, atBottom: false, reacted: true)))
    }

    func testScrollingAwayCancelsUnmeasuredRowUpdate() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertFalse(follow.needsReveal(to: pin(reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(atBottom: false, reacted: true)))
        XCTAssertFalse(follow.atBottom)
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false, reacted: true)))
    }

    func testPrependingHistoryCancelsPendingReveal() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertFalse(follow.needsReveal(to: pin(reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 1800, atBottom: false, firstID: "older", reacted: true)))
        XCTAssertFalse(follow.atBottom)
    }

    func testLazyMeasurementWithoutChangedRowsDoesNotSnapBack() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 988, atBottom: false)))
    }

    func testOpenedConversationFollowsRowsSettlingUntilTheReaderScrolls() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin(firstID: "previous")))
        follow.opened()
        // The new conversation's first measurement.
        XCTAssertFalse(follow.needsReveal(to: pin()))
        // Rows near the end are measured, then a link card image loads.
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 944, atBottom: false)))
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 990, offset: 400, atBottom: false)),
                      "Rows above settling can move the offset without the reader scrolling")
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 990, offset: 690)))
        XCTAssertTrue(follow.needsReveal(to: pin(contentHeight: 1100, offset: 690, atBottom: false)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 1100, offset: 800)))
        // The reader scrolls up; later growth leaves them where they are.
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 1100, offset: 500, atBottom: false), scrolling: true))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 1200, offset: 500, atBottom: false)))
    }

    func testOpenedFollowEndsWhenTheReaderMovesUpWithoutDrag() {
        let follow = TimelinePinBox()
        follow.opened()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        // Momentum after a drag ends, with nothing growing.
        XCTAssertFalse(follow.needsReveal(to: pin(offset: 400, atBottom: false)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 944, offset: 400, atBottom: false)))
    }

    func testLoadingOlderEndsOpenedFollow() {
        let follow = TimelinePinBox()
        follow.opened()
        XCTAssertFalse(follow.needsReveal(to: pin(height: 900, contentHeight: 600, offset: 0)))
        follow.leftEnd()
        XCTAssertFalse(follow.needsReveal(to: pin(height: 900, contentHeight: 1500, offset: 0, atBottom: false, firstID: "older")))
        XCTAssertFalse(follow.needsReveal(to: pin(height: 900, contentHeight: 1600, offset: 0, atBottom: false, firstID: "older")))
    }

    func testUpwardScrollWithLazyGrowthCancelsUnmeasuredRowUpdate() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertFalse(follow.needsReveal(to: pin(reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 944, offset: 400, atBottom: false, reacted: true)))
        XCTAssertFalse(follow.atBottom)
    }

    func testNativeTimelineMeasuresOnlyTheVisibleWindowAndHonorsBoundaries() {
        var layout = NativeTimelineLayout(estimatedHeight: 50)
        layout.replace(ids: (0..<3000).map(String.init), width: 600)
        XCTAssertEqual(layout.measuredCount, 0)
        XCTAssertEqual(layout.contentHeight, 150_000)
        XCTAssertEqual(layout.visibleRange(offset: 50, height: 100), 1..<3)
        XCTAssertEqual(layout.visibleRange(offset: 49, height: 102), 0..<4)
        let visible = layout.visibleRange(offset: 149_500, height: 500)
        XCTAssertEqual(visible, 2990..<3000)
        for index in visible { layout.measure(index, height: index.isMultiple(of: 2) ? 31 : 89) }
        XCTAssertEqual(layout.measuredCount, 10, "History outside the viewport stays estimated")
        XCTAssertEqual(layout.contentHeight, 150_100)
        XCTAssertEqual(layout.index(at: 149_530), 2990)
        XCTAssertEqual(layout.index(at: 149_531), 2991)
        XCTAssertEqual(layout.origin(of: 2992), 149_620)
        XCTAssertEqual(layout.visibleRange(offset: 150_100, height: 500), 0..<0)
        layout.replace(ids: [], width: 600)
        XCTAssertEqual(layout.contentHeight, 0)
        XCTAssertEqual(layout.visibleRange(offset: 0, height: 500), 0..<0)
        XCTAssertNil(layout.anchor(at: 0))
    }

    func testNativeTimelinePreservesAnAnchorAcrossPrependEditAndResize() throws {
        var layout = NativeTimelineLayout(estimatedHeight: 50)
        layout.replace(ids: ["a", "b", "c", "d"], width: 400)
        layout.measure(0, height: 70)
        layout.measure(1, height: 30)
        let anchor = try XCTUnwrap(layout.anchor(at: 108))
        XCTAssertEqual(anchor.id, "c")
        XCTAssertEqual(anchor.offset, 8)
        layout.replace(ids: ["older", "a", "b", "c", "d", "newer"], width: 400)
        XCTAssertEqual(layout.offset(for: anchor), 158)
        XCTAssertEqual(layout.measuredCount, 2)
        layout.measure(0, height: 120)
        XCTAssertEqual(layout.offset(for: anchor), 228)
        layout.replace(ids: layout.ids, width: 400, invalidating: ["b"])
        XCTAssertEqual(layout.offset(for: anchor), 248)
        XCTAssertEqual(layout.measuredCount, 2)
        layout.replace(ids: layout.ids, width: 200)
        XCTAssertEqual(layout.measuredCount, 0, "Old-width heights cannot clip reflowed text")
        XCTAssertEqual(layout.offset(for: anchor), 158)
        layout.replace(ids: ["a", "b"], width: 200)
        XCTAssertNil(layout.offset(for: anchor), "A removed row cannot anchor a different message")
    }
}
