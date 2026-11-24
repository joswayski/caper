import Foundation

/// "You’re viewing older messages": a toast floating over a timeline just
/// above the composer, as in Discord, with a Jump to latest button. A history
/// that reaches the live end shows it once `after` or more loaded messages
/// lie below the viewport, and counts other people's messages
/// arriving at the live end meanwhile. A detached window (one that stops short
/// of the live end) shows it wherever the reader is, without a count.
public enum JumpToLatest {
    /// The toast's label until others have written.
    public static let viewingOlder = "You’re viewing older messages"
    /// The toast's button.
    public static let button = "Jump to latest"

    /// How many loaded messages below the viewport show the toast: reading the
    /// last few is still reading the newest messages.
    public static let after = 10

    /// In a live history, shows once `below` (messages wholly below the
    /// viewport) reaches `after`; in a detached window (`live` false), always.
    public static func shows(below: Int, live: Bool) -> Bool {
        !live || below >= after
    }

    /// "1 new message", "3 new messages", capped at "99+ new messages"; nil without new messages.
    public static func countText(_ count: Int) -> String? {
        guard count > 0 else { return nil }
        if count > 99 { return "99+ new messages" }
        return count == 1 ? "1 new message" : "\(count) new messages"
    }

    /// The toast's visible label: the count once others have written, else "You’re viewing older messages".
    public static func label(_ count: Int) -> String {
        countText(count) ?? viewingOlder
    }

    /// The button's accessible name.
    public static func accessibilityLabel(_ count: Int) -> String {
        countText(count).map { "\(button), \($0)" } ?? button
    }

    /// Animated unless Reduce Motion is on or the newest message is many screens away.
    public static func animates(distanceFromEnd: CGFloat, viewport: CGFloat, reduceMotion: Bool) -> Bool {
        !reduceMotion && distanceFromEnd <= viewport * 4
    }

    /// Other people's messages in `next` that arrived at the live end since
    /// `previousIDs`: after the last previously loaded message, not deleted.
    public static func arrivals(_ next: [ChatMessage], previousIDs: Set<String>, viewerID: String?) -> Int {
        guard let lastKnown = next.lastIndex(where: { previousIDs.contains($0.id) }) else { return 0 }
        return next[(lastKnown + 1)...].filter { !previousIDs.contains($0.id) && !$0.isDeleted && $0.author.id != viewerID }.count
    }
}
