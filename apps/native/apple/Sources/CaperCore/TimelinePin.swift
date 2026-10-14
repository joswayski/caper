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
    var atBottom: Bool { updatedAtBottom || (last?.atBottom ?? true) }

    /// SwiftUI can report new reactions before their chips have been measured.
    /// Retain the reader's bottom position across those separate layout passes,
    /// including wrapped chips and the keyboard returning after a picker closes.
    func needsReveal(to pin: TimelinePin) -> Bool {
        defer { last = pin }
        guard let last, last.firstID == pin.firstID else {
            updatedAtBottom = false
            return false
        }
        let rowsChanged = pin.recent != last.recent
        let sizeChanged = pin.height != last.height || pin.contentHeight != last.contentHeight
        let scrolledAway = pin.height == last.height && pin.offset < last.offset - 1 && !pin.atBottom
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
        return !scrolledAway && (last.atBottom || updatedAtBottom) &&
            (pin.height < last.height ||
             (pin.contentHeight > last.contentHeight && (pin.atBottom || updatedAtBottom)))
    }
}
