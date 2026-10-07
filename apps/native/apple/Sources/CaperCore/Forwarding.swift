import Foundation

/// Source content revisions and destination delivery sequences are independent.
func mergeForward(_ current: ChatMessage, _ incoming: ChatMessage) -> ChatMessage {
    guard current.id == incoming.id, current.channelId == incoming.channelId, let next = incoming.forward else { return current }
    var result = current
    if let old = current.forward, next.message != nil,
       old.message == nil || (try? Sequence.compare(old.seq, next.seq)) == .orderedDescending {
        result.forward = old
    } else { result.forward = next }
    if (try? Sequence.compare(current.forwardSeq ?? "0", incoming.forwardSeq ?? "0")) != .orderedDescending {
        result.forwardSeq = incoming.forwardSeq
    }
    return result
}

struct ForwardSnapshots {
    private var byID: [String: ChatMessage] = [:]
    mutating func seed(_ messages: [ChatMessage]) {
        for message in messages where message.forward != nil {
            byID[message.id] = byID[message.id].map { mergeForward($0, message) } ?? message
            if byID.count > 256, let id = byID.keys.first(where: { $0 != message.id }) { byID.removeValue(forKey: id) }
        }
    }
    func overlay(_ message: ChatMessage) -> ChatMessage { byID[message.id].map { mergeForward(message, $0) } ?? message }
    mutating func reset() { byID = [:] }
}
