import XCTest
@testable import CaperCore

final class LastEditableMessageTests: XCTestCase {
    private func message(_ id: String, author: String = "me", thread: String? = nil, broadcast: Bool = false,
                         deleted: Bool = false, forwarded: Bool = false) throws -> ChatMessage {
        var fields = [
            "\"id\":\"\(id)\"", "\"channelId\":\"channel00001\"", "\"seq\":\"1\"", "\"clientMessageId\":\"key-\(id)\"",
            "\"author\":{\"id\":\"\(author)\",\"name\":\"\(author)\",\"isGuest\":false}",
            "\"content\":{\"version\":1,\"type\":\"text\",\"text\":\"\(id)\"}", "\"createdAt\":\"2026-11-24T12:00:00Z\"",
        ]
        if let thread { fields.append("\"threadRootId\":\"\(thread)\"") }
        if broadcast { fields.append("\"broadcast\":true") }
        if deleted { fields.append("\"deletedAt\":\"2026-11-24T12:01:00Z\"") }
        if forwarded { fields.append("\"forwardSeq\":\"1\",\"forward\":{\"seq\":\"1\"}") }
        return try JSONDecoder().decode(ChatMessage.self, from: Data("{\(fields.joined(separator: ","))}".utf8))
    }

    func testPicksNewestOwnMessageShownInThatConversation() throws {
        let messages = [
            try message("root"),
            try message("timeline"),
            try message("broadcast", thread: "root", broadcast: true),
            try message("hidden", thread: "root"),
            try message("deleted", deleted: true),
            try message("forwarded", forwarded: true),
            try message("someone", author: "them"),
            try message("other-thread", thread: "elsewhere"),
        ]
        // The hidden reply is newer, but only the broadcast one shows in the timeline.
        XCTAssertEqual(LastEditableMessage.find(in: messages, viewerID: "me", threadRootID: nil)?.id, "broadcast")
        // In the thread, hidden replies count; other threads don't.
        XCTAssertEqual(LastEditableMessage.find(in: messages, viewerID: "me", threadRootID: "root")?.id, "hidden")
        XCTAssertEqual(LastEditableMessage.find(in: [try message("root"), try message("reply", author: "them", thread: "root")],
                                                viewerID: "me", threadRootID: "root")?.id, "root")
        XCTAssertEqual(LastEditableMessage.find(in: messages, viewerID: "them", threadRootID: nil)?.id, "someone")
        XCTAssertNil(LastEditableMessage.find(in: messages, viewerID: nil, threadRootID: nil))
        XCTAssertNil(LastEditableMessage.find(in: [try message("deleted", deleted: true), try message("forwarded", forwarded: true)],
                                              viewerID: "me", threadRootID: nil))
    }
}
