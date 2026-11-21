import Foundation

/// "Jump to latest": a floating button over a timeline's bottom trailing
/// corner while the reader is more than half a screen above the newest
/// loaded message of a history that reaches the live end. While it shows,
/// other people's messages arriving at the live end are counted on it.
public enum JumpToLatest {
    /// Shows when the newest loaded message is more than half a viewport
    /// below the bottom of what the reader sees, and the history is live
    /// (no "Load newer messages"/"Back to latest" window).
    public static func shows(distanceFromEnd: CGFloat, viewport: CGFloat, live: Bool) -> Bool {
        live && viewport > 0 && distanceFromEnd > viewport / 2
    }

    /// "1 new message", "3 new messages", capped at "99+ new messages"; nil without new messages.
    public static func countText(_ count: Int) -> String? {
        guard count > 0 else { return nil }
        if count > 99 { return "99+ new messages" }
        return count == 1 ? "1 new message" : "\(count) new messages"
    }

    public static func accessibilityLabel(_ count: Int) -> String {
        countText(count).map { "Jump to latest, \($0)" } ?? "Jump to latest"
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
