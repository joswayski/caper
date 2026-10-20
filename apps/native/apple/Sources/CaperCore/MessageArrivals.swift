import Foundation

/// Remembers which messages arrived while the reader watched the live end of a
/// list, so only those animate in (web chat/arrivals.ts). The first render of a
/// channel or thread, older pages, jumps into history and newer pages loaded
/// towards the live end all appear in place. A pending send keeps its client
/// message ID once confirmed, so it animates once.
final class MessageArrivals {
    /// How long a row counts as arriving: long enough for its entrance, short
    /// enough that a row the lazy stack rebuilds later never replays it.
    static let window: TimeInterval = 1

    private struct List { let key: String; let ids: Set<String>; let last: String?; let live: Bool }
    private struct Arrival { let at: TimeInterval; var started = false }
    private var lists: [String: List] = [:]
    private var arrived: [String: Arrival] = [:]
    private var revisions: [String: Int] = [:]
    private let now: () -> TimeInterval

    init(now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) { self.now = now }

    /// A layout trigger, independent of row claims (including pending confirmation).
    func revision(_ scope: String) -> Int { revisions[scope, default: 0] }

    /// Call on every render with the list's client message IDs, oldest first.
    /// `live` is true when the list ends at the newest message. Both this and the
    /// previous render must be live, so a newer page loaded on the way back to the
    /// live end does not count.
    func observe(_ scope: String, key: String, ids: [String], live: Bool) {
        let previous = lists[scope]
        lists[scope] = List(key: key, ids: Set(ids), last: ids.last, live: live)
        guard let previous, previous.key == key, let last = previous.last, previous.live, live,
              // Only rows after the previous newest row are arrivals; prepended history is not.
              let tail = ids.lastIndex(of: last) else { return }
        let at = now()
        var appended = false
        for id in ids[(tail + 1)...] where !previous.ids.contains(id) && arrived[id] == nil {
            arrived[id] = Arrival(at: at)
            appended = true
        }
        if appended { revisions[scope, default: 0] += 1 }
        arrived = arrived.filter { at - $0.value.at <= Self.window }
    }

    func has(_ id: String) -> Bool {
        guard let arrival = arrived[id] else { return false }
        return now() - arrival.at <= Self.window
    }

    /// True once per arrival: the first row to show it animates, and a later one
    /// (the confirmed row replacing a pending send, or a rebuilt row) does not.
    func claim(_ id: String) -> Bool {
        guard has(id), arrived[id]?.started == false else { return false }
        arrived[id]?.started = true
        return true
    }
}
