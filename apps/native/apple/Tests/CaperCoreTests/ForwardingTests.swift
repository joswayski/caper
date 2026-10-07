import XCTest
@testable import CaperCore

final class ForwardingTests: XCTestCase {
    private func wrapper(_ sourceSeq: String, _ destinationSeq: String) throws -> ChatMessage {
        let json = """
        {"id":"wrapper00000001","channelId":"channel00001","seq":"1","clientMessageId":"wrapper-key",
         "author":{"id":"bob","name":"Bob","isGuest":false},"content":{"version":1,"type":"text","text":"note"},"createdAt":"2026-10-06T12:01:00Z",
         "forwardSeq":"\(destinationSeq)","forward":{"seq":"\(sourceSeq)","message":{
           "id":"source000000001","channelId":"private00001","seq":"89","clientMessageId":"source-key",
           "author":{"id":"alice","name":"Alice","isGuest":false},"content":{"version":1,"type":"text","text":"source \(sourceSeq)"},"createdAt":"2026-10-06T12:00:00Z"}}}
        """
        return try JSONDecoder().decode(ChatMessage.self, from: Data(json.utf8))
    }

    func testSourceAndDestinationRevisionsAdvanceIndependently() throws {
        let merged = mergeForward(try wrapper("9007199254740995", "1"), try wrapper("9007199254740993", "2"))
        XCTAssertEqual(merged.forward?.seq, "9007199254740995")
        XCTAssertEqual(merged.forward?.message?.content.text, "source 9007199254740995")
        XCTAssertEqual(merged.forwardSeq, "2")
        XCTAssertEqual(merged.seq, "1")
        XCTAssertEqual(merged.content.text, "note")
    }

    func testUnloadedOverlayAndUnavailableOriginalSurviveStaleHistory() throws {
        var snapshots = ForwardSnapshots()
        let old = try wrapper("9007199254740993", "1")
        snapshots.seed([try wrapper("9007199254740995", "2")])
        XCTAssertEqual(snapshots.overlay(old).forward?.seq, "9007199254740995")
        var removed = try wrapper("0", "3")
        removed.forward = MessageForward(message: nil, seq: "0")
        snapshots.seed([removed, old])
        XCTAssertNil(snapshots.overlay(old).forward?.message)
        snapshots.reset()
        XCTAssertNotNil(snapshots.overlay(old).forward?.message)
    }
}
