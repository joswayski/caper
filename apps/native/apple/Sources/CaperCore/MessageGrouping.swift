import Foundation

/// Which message rows show compactly, without avatar and name, under the
/// previous visible row in the same list (the timeline, or a thread's
/// replies). Presentation only: ordering, paging, unread state and actions
/// are unchanged. Mirrors apps/web/src/chat/grouping.ts; see docs/media.md
/// "Message text: links and grouping".
enum MessageGrouping {
    /// At most five minutes apart, in either direction.
    static let window: TimeInterval = 5 * 60

    /// One list's grouping.
    struct Layout: Equatable {
        /// Rows shown compactly under the row above them.
        var grouped: Set<String> = []
        /// Rows with a grouped row right below: they give up their bottom
        /// padding so the run reads as one block.
        var continued: Set<String> = []
        /// The pending (still sending) message groups under the last row.
        var pendingGrouped = false
    }

    /// `message` groups under `previous`, the visible row above it, when the
    /// same account wrote both within five minutes on the same local day (no
    /// date divider between them). A "Replied to a thread" broadcast in the
    /// channel keeps its full header; in its thread it is an ordinary reply.
    static func isGrouped(_ message: ChatMessage, after previous: ChatMessage?, inThread: Bool,
                          calendar: Calendar = .current) -> Bool {
        guard let previous, inThread || message.threadRootId == nil else { return false }
        return follows(authorID: message.author.id, at: ChatDateDivider.date(message.createdAt),
                       previousAuthorID: previous.author.id, previousDate: ChatDateDivider.date(previous.createdAt),
                       calendar: calendar)
    }

    /// One list's rows, then the pending message shown after them, if any
    /// (it counts with its author's id). A collapsed blocked run is a
    /// placeholder row: nothing groups across it. A revealed run's messages
    /// sit under its Hide bar, so its first message keeps its header.
    static func layout(_ entries: [TimelineEntry], inThread: Bool, pending: PendingMessage? = nil,
                       author: ChatAuthor? = nil, calendar: Calendar = .current) -> Layout {
        var result = Layout()
        var previous: (id: String, authorID: String, date: Date?)?
        func visit(_ message: ChatMessage) {
            let date = ChatDateDivider.date(message.createdAt)
            if let previous, inThread || message.threadRootId == nil,
               follows(authorID: message.author.id, at: date, previousAuthorID: previous.authorID,
                       previousDate: previous.date, calendar: calendar) {
                result.grouped.insert(message.id)
                result.continued.insert(previous.id)
            }
            previous = (message.id, message.author.id, date)
        }
        for entry in entries {
            switch entry {
            case let .message(message): visit(message)
            case let .blocked(run):
                previous = nil
                guard run.revealed else { continue }
                run.messages.forEach(visit)
            }
        }
        if let pending, let previous,
           follows(authorID: author?.id, at: ChatDateDivider.date(pending.createdAt), previousAuthorID: previous.authorID,
                   previousDate: previous.date, calendar: calendar) {
            result.pendingGrouped = true
            result.continued.insert(previous.id)
        }
        return result
    }

    private static func follows(authorID: String?, at date: Date?, previousAuthorID: String, previousDate: Date?,
                                calendar: Calendar) -> Bool {
        guard let authorID, !authorID.isEmpty, authorID == previousAuthorID,
              let date, let previousDate else { return false }
        return abs(date.timeIntervalSince(previousDate)) <= window && calendar.isDate(date, inSameDayAs: previousDate)
    }
}
