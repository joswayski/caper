import Foundation

/// The timeline's content/viewport heights and whether its end is visible.
struct TimelinePin: Equatable {
    var height: CGFloat
    var contentHeight: CGFloat
    var offset: CGFloat
    var atBottom: Bool
    var firstID: String?
    /// The last rows, whose reactions, edits and files grow the end in place.
    var recent: [TimelineEntry]
    /// More than a screen of short messages.
    static let recentRows = 20
}

/// A reference box so scroll measurements never re-render the conversation.
final class TimelinePinBox {
    private var last: TimelinePin?
    private var updatedAtBottom = false
    /// Opened at the end: rows measured or loaded afterwards (lazy heights,
    /// link card images on a cold launch) keep the end in view until the reader scrolls.
    private var opening = false
    var atBottom: Bool { opening || updatedAtBottom || (last?.atBottom ?? true) }

    /// The conversation was just opened (or its history replaced) at the end.
    func opened() { opening = true }
    /// Older history or a jump takes the reader away from the end.
    func leftEnd() { opening = false; updatedAtBottom = false }

    /// SwiftUI can report new reactions before their chips have been measured.
    /// Retain the reader's bottom position across those separate layout passes,
    /// including wrapped chips and the keyboard returning after a picker closes.
    func needsReveal(to pin: TimelinePin, scrolling: Bool = false) -> Bool {
        defer { last = pin }
        // Interactive keyboard dismissal changes the viewport in both
        // directions. scrollTo would interrupt the gesture driving it. Keep
        // measuring, but discard pending follow so it cannot replay afterward.
        guard !scrolling else {
            updatedAtBottom = false
            opening = false
            return false
        }
        guard let last, last.firstID == pin.firstID else {
            updatedAtBottom = false
            return false
        }
        let rowsChanged = pin.recent != last.recent
        let sizeChanged = pin.height != last.height || pin.contentHeight != last.contentHeight
        let scrolledAway = pin.height == last.height && pin.offset < last.offset - 1 && !pin.atBottom
        if opening {
            // Rows above settling can move the offset too; only a still
            // extent moving up is the reader leaving the end.
            if scrolledAway && pin.contentHeight == last.contentHeight { opening = false }
            else { return sizeChanged }
        }
        if scrolledAway {
            // Lazy rows can be measured in the same pass as an upward scroll.
            // Their new height must not prolong a pending reaction reveal.
            updatedAtBottom = false
        } else if rowsChanged {
            updatedAtBottom = updatedAtBottom || last.atBottom
        } else if !sizeChanged {
            // Scrolling away cancels a pending reveal. Reaching the bottom
            // after a reveal also settles it, before any unrelated lazy layout.
            updatedAtBottom = false
        }
        // A changed first row above cancels follow for prepended/replaced
        // history. Growth with no row update while leaving the bottom is lazy
        // history being measured as the reader scrolls; it must not snap back.
        // Removing draft files expands the viewport: re-pin then too, or the
        // old scroll offset leaves the files' former space below the last row.
        return !scrolledAway && (last.atBottom || updatedAtBottom) &&
            (pin.height != last.height ||
             (pin.contentHeight > last.contentHeight && (pin.atBottom || updatedAtBottom)))
    }
}
