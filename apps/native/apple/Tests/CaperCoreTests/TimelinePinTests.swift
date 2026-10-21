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

    func testUpwardScrollWithLazyGrowthCancelsUnmeasuredRowUpdate() {
        let follow = TimelinePinBox()
        XCTAssertFalse(follow.needsReveal(to: pin()))
        XCTAssertFalse(follow.needsReveal(to: pin(reacted: true)))
        XCTAssertFalse(follow.needsReveal(to: pin(contentHeight: 944, offset: 400, atBottom: false, reacted: true)))
        XCTAssertFalse(follow.atBottom)
    }
}
