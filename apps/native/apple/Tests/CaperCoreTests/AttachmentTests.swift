import CoreGraphics
import CoreMedia
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import CaperCore

private final class AttachmentTokenStore: TokenStore, @unchecked Sendable {
    var token: String?
    init(_ token: String?) { self.token = token }
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

/// Records every request (method, URL, headers, JSON body) and answers from `handler`.
private final class AttachmentURLProtocol: URLProtocol, @unchecked Sendable {
    struct Recorded { let method: String; let url: URL; let headers: [String: String]; let body: Data? }
    private static let lock = NSLock()
    nonisolated(unsafe) static var handler: ((URLRequest, Data?) throws -> (Int, Data))?
    nonisolated(unsafe) private static var recorded: [Recorded] = []

    static var requests: [Recorded] { lock.withLock { recorded } }
    static func reset() { lock.withLock { recorded = []; handler = nil } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let body = Self.body(of: request)
        Self.lock.withLock {
            Self.recorded.append(Recorded(method: request.httpMethod ?? "GET", url: request.url!,
                                          headers: request.allHTTPHeaderFields ?? [:], body: body))
        }
        do {
            guard let handler = Self.handler else { throw URLError(.badURL) }
            let (status, data) = try handler(request, body)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                                                                  headerFields: ["content-type": "application/json"])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}

    private static func body(of request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}

private func json(_ data: Data?) -> [String: Any]? {
    data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
}

private let futureExp = 4_102_444_800 // 2100-01-01
private let channelID = "Chan12345678"

final class AttachmentTests: XCTestCase {
    override func tearDown() {
        AttachmentURLProtocol.reset()
        super.tearDown()
    }

    private func client(token: String? = "account-secret") -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [AttachmentURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: AttachmentTokenStore(token))
    }

    @MainActor
    private func waitUntil(_ message: String = "condition", _ predicate: @escaping @MainActor () -> Bool) async {
        for _ in 0..<300 where !predicate() { try? await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(predicate(), "timed out waiting for \(message)")
    }

    // MARK: Tolerant decoding

    func testMalformedAttachmentsAreSkippedWithoutRejectingMessagesOrHistory() async throws {
        let history = """
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"3","hasMore":false,"messages":[
          {"id":"m1","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"image","contentType":"image/png","name":"shot.png","size":2048,"width":800,"height":600,"preview":{},
              "url":"https://cdn.caper.chat/original/Asset0000001?exp=\(futureExp)&sig=a","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\(futureExp)&sig=b"},
             {"id":"Asset0000002","kind":"image","contentType":"image/png","name":"bad-size.png","size":"big"},
             {"id":"Asset0000003","kind":"sticker","contentType":"image/png","name":"unknown.png","size":1},
             {"id":"Asset0000004","kind":"file","contentType":"text/html","name":"x.html","size":1,"url":"javascript:alert(1)"},
             {"id":"Asset0000005","kind":"file","contentType":"application/pdf","name":"gone.pdf","size":10,"unavailable":true},
             {"id":"Asset0000006","kind":"video","contentType":"video/mp4","name":"neg.mp4","size":10,"width":-4},
             "not an object", null
           ]}},
          {"id":"m2","channelId":"\(channelID)","seq":"2","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c2",
           "content":{"version":1,"type":"text","text":"wrong shape","attachments":"oops"}},
          {"id":"m3","channelId":"\(channelID)","seq":"3","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c3",
           "content":{"version":1,"type":"text","text":"plain"}}
        ]}
        """
        AttachmentURLProtocol.handler = { _, _ in (200, Data(history.utf8)) }
        let page = try await client().history(channelID: channelID)
        XCTAssertEqual(page.messages.map(\.id), ["m1", "m2", "m3"], "no message or page is rejected for bad attachments")
        let attachments = try XCTUnwrap(page.messages[0].content.attachments)
        XCTAssertEqual(attachments.map(\.id), ["Asset0000001", "Asset0000005"])
        XCTAssertEqual(page.messages[0].content.text, "")
        XCTAssertTrue(attachments[0].hasPreview)
        XCTAssertEqual(attachments[0].kind, .image)
        XCTAssertEqual(attachments[0].width, 800)
        XCTAssertTrue(attachments[1].unavailable)
        XCTAssertNil(attachments[1].url)
        XCTAssertNil(page.messages[1].content.attachments, "a non-array field is ignored")
        XCTAssertNil(page.messages[2].content.attachments)

        // Gateway frames decode through the same model.
        let frame = Data(#"{"id":"m4","channelId":"c","seq":"4","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c4","content":{"version":1,"type":"text","text":"hi","attachments":[{"id":"A","kind":"audio","contentType":"audio/mpeg","name":"a.mp3","size":5,"durationMs":1200},{"id":7}]}}"#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: frame)
        XCTAssertEqual(message.content.attachments?.map(\.durationMs), [1200])
    }

    func testAttachmentRoundTripsThroughEncoding() throws {
        let attachment = ChatAttachment(id: "A", kind: .video, contentType: "video/mp4", name: "v.mp4", size: 9, width: 4, height: 3,
                                        durationMs: 10, hasPreview: true, url: "https://cdn.invalid/o", previewUrl: "https://cdn.invalid/p")
        let content = ChatContent(version: 1, type: "text", text: "", attachments: [attachment])
        let decoded = try JSONDecoder().decode(ChatContent.self, from: JSONEncoder().encode(content))
        XCTAssertEqual(decoded, content)
    }

    func testAttachmentKindMatchesInlineAllowlist() {
        XCTAssertEqual(AttachmentKind(contentType: "image/webp"), .image)
        XCTAssertEqual(AttachmentKind(contentType: "IMAGE/PNG"), .image)
        XCTAssertEqual(AttachmentKind(contentType: "image/heic"), .file, "HEIC is not inline-renderable in browsers")
        XCTAssertEqual(AttachmentKind(contentType: "image/svg+xml"), .file)
        XCTAssertEqual(AttachmentKind(contentType: "video/quicktime"), .video)
        XCTAssertEqual(AttachmentKind(contentType: "audio/x-m4a"), .audio)
        XCTAssertEqual(AttachmentKind(contentType: "application/pdf"), .file)
    }

    // MARK: URL expiry and refresh

    func testSignedURLExpiryDecisions() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        func url(_ exp: Int) -> String { "https://cdn.caper.chat/original/A?exp=\(exp)&sig=x" }
        XCTAssertEqual(AttachmentURLPolicy.expiry(of: url(1_800_000_123)), Date(timeIntervalSince1970: 1_800_000_123))
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(url(1_799_999_000), now: now), "past expiry")
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(url(1_800_001_800), now: now), "expires within the hour")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh(url(1_800_007_200), now: now), "two hours left")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("https://cdn.caper.chat/original/A?sig=x", now: now), "no exp")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("https://cdn.caper.chat/original/A?exp=soon", now: now))
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh("file:///tmp/local.png", now: now), "pending local copies never refresh")
        XCTAssertFalse(AttachmentURLPolicy.needsRefresh(nil, now: now))
        XCTAssertTrue(AttachmentURLPolicy.refreshesAfterFailure(status: 403))
        XCTAssertTrue(AttachmentURLPolicy.refreshesAfterFailure(status: 404))
        XCTAssertFalse(AttachmentURLPolicy.refreshesAfterFailure(status: 500))
    }

    private func historyWithAttachment(exp: Int) -> Data {
        Data("""
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"1","hasMore":false,"messages":[
          {"id":"m1","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"image","contentType":"image/png","name":"shot.png","size":2048,"preview":{},
              "url":"https://cdn.caper.chat/original/Asset0000001?exp=\(exp)&sig=old","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\(exp)&sig=old"}
           ]}}
        ]}
        """.utf8)
    }

    @MainActor
    func testExpiredURLsRefreshInOneBatchAndPatchTheTimeline() async throws {
        let history = historyWithAttachment(exp: 1_000)
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            case ("POST", "/api/assets/urls"):
                return (200, Data(#"{"urls":{"Asset0000001":{"url":"https://cdn.caper.chat/original/Asset0000001?exp=\#(futureExp)&sig=new","previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\#(futureExp)&sig=new"}}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        XCTAssertTrue(AttachmentURLPolicy.needsRefresh(chat.messages.first?.content.attachments?.first?.url))
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        await waitUntil("fresh URL") { chat.messages.first?.content.attachments?.first?.url?.contains("sig=new") == true }
        let refreshes = AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }
        XCTAssertEqual(refreshes.count, 1, "duplicate requests are batched")
        XCTAssertEqual(json(refreshes.first?.body)?["ids"] as? [String], ["Asset0000001"])
        XCTAssertEqual(refreshes.first?.headers["Authorization"] ?? refreshes.first?.headers["authorization"], "Bearer account-secret")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.previewUrl?.contains("sig=new"), true)
        let opened = await chat.currentURL(for: try XCTUnwrap(chat.messages.first?.content.attachments?.first))
        XCTAssertEqual(opened?.query?.contains("sig=new"), true)
        XCTAssertEqual(AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }.count, 1, "fresh URLs are reused")
        await chat.stop()
    }

    @MainActor
    func testRefreshIsAttemptedOnceWhenAttachmentIsNoLongerVisible() async throws {
        let history = historyWithAttachment(exp: 1_000)
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            case ("POST", "/api/assets/urls"): return (200, Data(#"{"urls":{}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        await waitUntil("first refresh") { AttachmentURLProtocol.requests.contains { $0.url.path == "/api/assets/urls" } }
        try await Task.sleep(for: .milliseconds(100))
        // A repeated load failure for the same stale URL must not loop.
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(AttachmentURLProtocol.requests.filter { $0.url.path == "/api/assets/urls" }.count, 1)
        XCTAssertTrue(chat.messages.first?.content.attachments?.first?.url?.contains("sig=old") == true)
        await chat.stop()
    }

    // MARK: Processing status and live updates

    func testStatusAndAnimatedDecodeTolerantly() throws {
        func decode(_ json: String) throws -> ChatAttachment {
            try JSONDecoder().decode(ChatAttachment.self, from: Data(json.utf8))
        }
        let base = #""id":"Asset0000001","kind":"image","contentType":"image/heic","name":"IMG.heic","size":10"#
        XCTAssertEqual(try decode("{\(base)}").status, .ready, "absent status means ready (older payloads)")
        XCTAssertFalse(try decode("{\(base)}").animated)
        let processing = try decode(#"{\#(base),"status":"processing","width":4032,"height":3024,"preview":{},"previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=1&sig=p"}"#)
        XCTAssertEqual(processing.status, .processing)
        XCTAssertNil(processing.url)
        XCTAssertNotNil(processing.previewUrl)
        XCTAssertEqual(try decode(#"{\#(base),"status":"failed"}"#).status, .failed)
        XCTAssertEqual(try decode(#"{\#(base),"status":"queued"}"#).status, .processing, "unknown states without a URL wait")
        XCTAssertEqual(try decode(#"{\#(base),"status":"later","url":"https://cdn.caper.chat/original/A?exp=1&sig=o"}"#).status, .ready)
        XCTAssertEqual(try decode(#"{\#(base),"status":7}"#).status, .ready, "a malformed status never drops the file")
        let animated = try decode(#"{"id":"Asset0000002","kind":"video","contentType":"video/mp4","name":"party.mp4","size":10,"status":"ready","animated":true,"url":"https://cdn.caper.chat/original/Asset0000002?exp=1&sig=o"}"#)
        XCTAssertTrue(animated.animated)
        XCTAssertFalse(try decode(#"{\#(base),"animated":"yes"}"#).animated)

        let roundTrip = ChatAttachment(id: "A", kind: .video, contentType: "video/mp4", name: "a.mp4", size: 1, status: .processing, animated: true)
        XCTAssertEqual(try JSONDecoder().decode(ChatAttachment.self, from: JSONEncoder().encode(roundTrip)), roundTrip)
    }

    func testAttachmentsEventDecodingSkipsMalformedEntries() throws {
        let event = try JSONDecoder().decode(MessageAttachmentsEvent.self, from: Data(#"""
        {"type":"message.attachments","schemaVersion":1,"channelId":"Chan12345678","seq":"9","messageId":"Message00000001",
         "attachments":[{"id":"Asset0000001","kind":"image","contentType":"image/avif","name":"a.avif","size":5,"status":"ready",
                         "url":"https://cdn.caper.chat/original/Asset0000001?exp=1&sig=o"},{"id":7},null]}
        """#.utf8))
        XCTAssertTrue(event.isValid)
        XCTAssertEqual(event.attachments.map(\.id), ["Asset0000001"])
        XCTAssertThrowsError(try JSONDecoder().decode(MessageAttachmentsEvent.self, from: Data(#"""
        {"type":"message.attachments","schemaVersion":1,"channelId":"c","seq":"9","messageId":"m","attachments":"oops"}
        """#.utf8)))
        XCTAssertFalse(MessageAttachmentsEvent(channelId: "c", seq: "09", messageId: "m", attachments: []).isValid)
        XCTAssertFalse(MessageAttachmentsEvent(schemaVersion: 2, channelId: "c", seq: "9", messageId: "m", attachments: []).isValid)
    }

    func testAttachmentSnapshotsKeepTheNewestSequence() {
        let author = ChatAuthor(id: "u", name: "U", isGuest: false)
        let processing = ChatAttachment(id: "A", kind: .image, contentType: "image/png", name: "a.png", size: 1, status: .processing)
        var ready = processing
        ready.status = .ready
        ready.url = "https://cdn.caper.chat/original/A?exp=1&sig=o"
        func message(_ attachment: ChatAttachment, seq: String?) -> ChatMessage {
            ChatMessage(id: "Message00000001", channelId: channelID, seq: "1", author: author,
                        content: ChatContent(version: 1, type: "text", text: "", attachments: [attachment]),
                        createdAt: "now", clientMessageId: "c", attachmentsSeq: seq)
        }
        var snapshots = AttachmentSnapshots()
        XCTAssertTrue(snapshots.apply(messageID: "Message00000001", seq: "5", attachments: [ready]))
        XCTAssertFalse(snapshots.apply(messageID: "Message00000001", seq: "4", attachments: [processing]), "a replayed older event is ignored")
        XCTAssertFalse(snapshots.apply(messageID: "Message00000001", seq: "5", attachments: [processing]), "a duplicate is ignored")

        // A late send response (never updated by the worker) or an older page.
        snapshots.seed([message(processing, seq: nil)])
        XCTAssertEqual(snapshots.overlay(message(processing, seq: nil)).content.attachments?.first?.status, .ready)
        XCTAssertEqual(snapshots.overlay(message(processing, seq: "3")).attachmentsSeq, "5")
        // An equal or newer fetched snapshot wins (it may carry fresher URLs).
        var refreshed = ready
        refreshed.url = "https://cdn.caper.chat/original/A?exp=2&sig=new"
        XCTAssertEqual(snapshots.overlay(message(refreshed, seq: "5")).content.attachments?.first?.url, refreshed.url)
        snapshots.seed([message(processing, seq: "6")])
        XCTAssertEqual(snapshots.overlay(message(ready, seq: nil)).content.attachments?.first?.status, .processing)

        snapshots.reset()
        for index in 0..<AttachmentSnapshots.maximumUnseen {
            XCTAssertTrue(snapshots.apply(messageID: "unseen-\(index)", seq: "\(index + 1)", attachments: []))
        }
        XCTAssertFalse(snapshots.apply(messageID: "one-too-many", seq: "999", attachments: []))
        XCTAssertTrue(snapshots.unseenOverflowed)
    }

    private func processingHistory() -> Data {
        Data("""
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"1","hasMore":false,"messages":[
          {"id":"Message00000001","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"video","contentType":"video/quicktime","name":"clip.mov","size":9000,"width":1920,"height":1080,"status":"processing"}
           ]}}
        ]}
        """.utf8)
    }

    private func attachmentsEvent(seq: String, status: String, url: Bool, messageID: String = "Message00000001") -> [String: Any] {
        var attachment: [String: Any] = ["id": "Asset0000001", "kind": "video", "contentType": "video/mp4", "name": "clip.mp4", "size": 3000,
                                         "width": 1920, "height": 1080, "status": status, "preview": [String: Any](),
                                         "previewUrl": "https://cdn.caper.chat/preview/Asset0000001?exp=\(futureExp)&sig=p"]
        if url { attachment["url"] = "https://cdn.caper.chat/original/Asset0000001?exp=\(futureExp)&sig=o" }
        return ["type": "message.attachments", "schemaVersion": 1, "channelId": channelID, "seq": seq,
                "messageId": messageID, "attachments": [attachment]]
    }

    @MainActor
    func testAttachmentEventsAndProgressUpdateTheTimeline() async throws {
        let history = processingHistory()
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        let generation = chat.eventGeneration
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.status, .processing)

        // Ephemeral progress, like typing: no sequence, applied to processing files only.
        chat.receive(["type": "attachment.progress", "channelId": channelID, "messageId": "Message00000001",
                      "attachmentId": "Asset0000001", "percent": 42], generation: generation, channelID: channelID)
        XCTAssertEqual(chat.attachmentProgress["Asset0000001"], 42)
        chat.receive(["type": "attachment.progress", "channelId": channelID, "messageId": "Message00000001",
                      "attachmentId": "Asset0000001", "percent": 180.4], generation: generation, channelID: channelID)
        XCTAssertEqual(chat.attachmentProgress["Asset0000001"], 100, "clamped")
        chat.receive(["type": "attachment.progress", "channelId": "Other1234567", "messageId": "Message00000001",
                      "attachmentId": "Asset0000001", "percent": 5], generation: generation, channelID: channelID)
        XCTAssertEqual(chat.attachmentProgress["Asset0000001"], 100, "other channels are ignored")
        XCTAssertNil(chat.error, "progress never resyncs")

        // A poster arrives first, then the result replaces the attachments.
        chat.receive(attachmentsEvent(seq: "2", status: "processing", url: false), generation: generation, channelID: channelID)
        var attachment = try XCTUnwrap(chat.messages.first?.content.attachments?.first)
        XCTAssertEqual(attachment.status, .processing)
        XCTAssertNotNil(attachment.previewUrl)
        XCTAssertEqual(chat.messages.first?.attachmentsSeq, "2")
        chat.receive(attachmentsEvent(seq: "3", status: "ready", url: true), generation: generation, channelID: channelID)
        attachment = try XCTUnwrap(chat.messages.first?.content.attachments?.first)
        XCTAssertEqual(attachment.status, .ready)
        XCTAssertEqual(attachment.contentType, "video/mp4")
        XCTAssertEqual(attachment.name, "clip.mp4")
        XCTAssertNotNil(attachment.url)
        XCTAssertEqual(chat.messages.first?.attachmentsSeq, "3")
        XCTAssertNil(chat.attachmentProgress["Asset0000001"], "finished files drop their progress")
        XCTAssertNil(chat.error)

        // A duplicate replay of an older state never reverts the result.
        chat.receive(attachmentsEvent(seq: "3", status: "processing", url: false), generation: generation, channelID: channelID)
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.status, .ready)
        chat.receive(["type": "attachment.progress", "channelId": channelID, "messageId": "Message00000001",
                      "attachmentId": "Asset0000001", "percent": 10], generation: generation, channelID: channelID)
        XCTAssertNil(chat.attachmentProgress["Asset0000001"], "late progress after ready is dropped")
        await chat.stop()
    }

    @MainActor
    func testFailedAttachmentEventAndSequenceGapResync() async throws {
        let history = processingHistory()
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        let generation = chat.eventGeneration
        chat.receive(attachmentsEvent(seq: "2", status: "failed", url: false), generation: generation, channelID: channelID)
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.status, .failed)
        XCTAssertNil(chat.messages.first?.content.attachments?.first?.url)
        XCTAssertNil(chat.error)
        // A gap in the channel sequence asks for a refresh, like reactions.
        chat.receive(attachmentsEvent(seq: "5", status: "ready", url: true), generation: generation, channelID: channelID)
        XCTAssertEqual(chat.error, "Messages changed while reconnecting. Refreshing…")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.status, .failed)
        await chat.stop()
    }

    // MARK: Composer decisions

    func testLocalKindsAndContentTypes() {
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "IMG_0001.HEIC"), "image/heic")
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "clip.mov"), "video/quicktime")
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "x.unknownext"), "application/octet-stream")
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "noext"), "application/octet-stream")
        XCTAssertEqual(AttachmentKind.local(contentType: "image/heic"), .image, "the device previews its own HEIC original")
        XCTAssertEqual(AttachmentKind.local(contentType: "video/quicktime"), .video)
        XCTAssertEqual(AttachmentKind.local(contentType: "audio/x-m4a"), .audio)
        XCTAssertEqual(AttachmentKind.local(contentType: "image/svg+xml"), .file)
        XCTAssertEqual(AttachmentKind.local(contentType: "application/pdf"), .file)
        XCTAssertEqual(AttachmentKind.local(contentType: "application/octet-stream"), .file)
    }

    func testDisplaySize() {
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 1200, height: 600)! == (360, 180))
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 600, height: 1200)! == (150, 300))
        XCTAssertNil(AttachmentPolicy.displaySize(width: nil, height: 10))
    }

    func testFormatBytesAndDraftLabels() {
        XCTAssertEqual(AttachmentPolicy.formatBytes(512), "512 B")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1536), "1.5 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(146_432), "143 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1_677_722), "1.6 MB")
        var draft = AttachmentDraft(id: "d", name: "shot.png", kind: .image, localURL: URL(fileURLWithPath: "/tmp/x"), sourceSize: 1_677_722)
        XCTAssertEqual(draft.statusLabel, "Compressing…")
        draft.preparing = false
        draft.progress = 0.42
        XCTAssertEqual(draft.statusLabel, "Uploading… 42%")
        draft.storedSize = 146_432
        draft.attachment = ChatAttachment(id: "A", kind: .image, contentType: "image/webp", name: "shot.webp", size: 146_432)
        XCTAssertEqual(draft.statusLabel, "1.6 MB → 143 KB")
        draft.storedSize = 1_677_722
        XCTAssertEqual(draft.statusLabel, "1.6 MB", "no arrow when nothing was saved")
        draft.error = "You’ve used all of your file storage."
        XCTAssertEqual(draft.statusLabel, "You’ve used all of your file storage.")
    }

    func testFileOnlyMessagesValidate() {
        XCTAssertEqual(MessageValidation.error(for: "  "), "Write a message first.")
        XCTAssertNil(MessageValidation.error(for: "", attachmentCount: 1))
        XCTAssertNotNil(MessageValidation.error(for: "", attachmentCount: 11))
    }

    // MARK: Compression decisions

    func testCompressionSettingsDecodeWithDefaults() throws {
        let decoded = try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":5,"limit":10,"compression":{"imageQuality":80,"paletteColors":999,"previewEdge":320,"videoBitrateKbps":8000}}"#.utf8))
        XCTAssertEqual(decoded.used, 5)
        XCTAssertEqual(decoded.compression.imageQuality, 80)
        XCTAssertEqual(decoded.compression.paletteColors, 256, "out-of-range values fall back to defaults")
        XCTAssertEqual(decoded.compression.previewEdge, 320)
        XCTAssertEqual(decoded.compression.videoBitrateKbps, 8000)
        XCTAssertEqual(decoded.compression.imageMaxEdge, 4096)
        XCTAssertEqual(decoded.compression.videoMaxHeight, 1080)
        XCTAssertEqual(decoded.compression.audioBitrateKbps, 128)
        let legacy = try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":0,"limit":10,"maxUploadBytes":5}"#.utf8))
        XCTAssertEqual(legacy.compression, AttachmentCompression())
        XCTAssertEqual(AttachmentCompression().videoBitrateKbps, 6000, "matches the API default")
    }

    func testStillSourcesKeepLosslessFilesLossless() {
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/png"), .lossless)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/bmp"), .lossless)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/tiff"), .lossless)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/jpeg"), .photo)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/heic"), .photo)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/heif"), .photo)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/webp", webP: .lossy), .photo)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/webp", webP: .lossless), .lossless)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/webp", webP: nil), .lossless, "unknown WebP is never encoded lossily")
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/webp", webP: .animated), .unchanged)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/gif"), .unchanged)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/svg+xml"), .unchanged)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/avif"), .unchanged)
        XCTAssertEqual(AttachmentPolicy.stillSource(contentType: "image/x-icon"), .unchanged)

        let settings = AttachmentCompression()
        func candidates(_ type: String, _ source: AttachmentPolicy.StillSource, _ colors: Int?, _ settings: AttachmentCompression = AttachmentCompression())
            -> [AttachmentPolicy.StillCandidate] {
            AttachmentPolicy.stillCandidates(contentType: type, source: source, colorCount: colors, settings: settings)
        }
        // A real screenshot has thousands of colours: lossless WebP, never lossy.
        XCTAssertEqual(candidates("image/png", .lossless, nil), [.losslessWebP])
        XCTAssertEqual(candidates("image/png", .lossless, 12), [.indexedPNG, .losslessWebP])
        XCTAssertEqual(candidates("image/png", .lossless, 256), [.indexedPNG, .losslessWebP])
        XCTAssertEqual(candidates("image/jpeg", .photo, nil), [.lossy(quality: 0.92)])
        XCTAssertEqual(candidates("image/jpeg", .photo, 12), [.indexedPNG, .lossy(quality: 0.92)])
        XCTAssertEqual(candidates("image/gif", .unchanged, 2), [])
        var lossyOff = settings
        lossyOff.imageQuality = 100
        XCTAssertEqual(candidates("image/jpeg", .photo, nil, lossyOff), [], "100 disables lossy re-encoding")
        XCTAssertEqual(candidates("image/heic", .photo, nil, lossyOff), [.lossy(quality: 1)], "HEIC always converts")
        var paletteOff = settings
        paletteOff.paletteColors = 0
        XCTAssertEqual(candidates("image/png", .lossless, 2, paletteOff), [.losslessWebP], "0 disables the palette path")
        XCTAssertEqual(candidates("image/jpeg", .photo, 2, paletteOff), [.lossy(quality: 0.92)])
        var smallPalette = settings
        smallPalette.paletteColors = 16
        XCTAssertEqual(candidates("image/png", .lossless, 17, smallPalette), [.losslessWebP])

        for quality in [1, 50, 92, 100] {
            for colors in [nil, 2, 300] as [Int?] {
                var custom = settings
                custom.imageQuality = quality
                let lossy = candidates("image/png", .lossless, colors, custom).contains { candidate in
                    if case .lossy = candidate { return true }
                    return false
                }
                XCTAssertFalse(lossy, "quality \(quality), colours \(String(describing: colors))")
            }
        }
    }

    private func riffWebP(_ chunks: [[UInt8]]) -> Data {
        var bytes: [UInt8] = Array("RIFF".utf8)
        bytes += [0, 0, 0, 0] as [UInt8]
        bytes += Array("WEBP".utf8)
        for chunk in chunks { bytes += chunk }
        return Data(bytes)
    }

    private func webPChunk(_ tag: String, _ payload: [UInt8]) -> [UInt8] {
        var bytes: [UInt8] = Array(tag.utf8)
        bytes += [UInt8(payload.count), 0, 0, 0]
        bytes += payload
        if payload.count % 2 == 1 { bytes.append(0) }
        return bytes
    }

    func testWebPBitstreamSniffing() {
        XCTAssertEqual(AttachmentPolicy.webPFormat(riffWebP([webPChunk("VP8 ", [1, 2, 3, 4])])), .lossy)
        XCTAssertEqual(AttachmentPolicy.webPFormat(riffWebP([webPChunk("VP8L", [1, 2, 3, 4])])), .lossless)
        let alpha = webPChunk("VP8X", [0x10, 0, 0, 0, 9, 0, 0, 9, 0, 0])
        XCTAssertEqual(AttachmentPolicy.webPFormat(riffWebP([alpha, webPChunk("ALPH", [1, 2, 3]), webPChunk("VP8 ", [1, 2])])), .lossy,
                       "odd chunks are padded")
        XCTAssertEqual(AttachmentPolicy.webPFormat(riffWebP([webPChunk("VP8X", [0x02, 0, 0, 0, 9, 0, 0, 9, 0, 0])])), .animated)
        let icc = webPChunk("VP8X", [0x20, 0, 0, 0, 9, 0, 0, 9, 0, 0])
        XCTAssertEqual(AttachmentPolicy.webPFormat(riffWebP([icc, webPChunk("ICCP", [1, 2, 3]), webPChunk("VP8L", [1])])), .lossless)
        XCTAssertNil(AttachmentPolicy.webPFormat(Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 0, 0, 0, 0])))
        XCTAssertNil(AttachmentPolicy.webPFormat(riffWebP([])))
    }

    func testWebPICCWrapperLayout() throws {
        let simple = riffWebP([webPChunk("VP8L", [9, 8, 7, 6, 5])])
        let profile = Data([1, 2, 3])
        let wrapped = [UInt8](try XCTUnwrap(AttachmentPolicy.webPAddingICCProfile(simple, profile: profile, width: 300, height: 2, hasAlpha: true)))
        XCTAssertEqual(Array(wrapped[0..<4]), Array("RIFF".utf8))
        let riffSize = Int(wrapped[4]) | Int(wrapped[5]) << 8 | Int(wrapped[6]) << 16 | Int(wrapped[7]) << 24
        XCTAssertEqual(riffSize, wrapped.count - 8)
        XCTAssertEqual(Array(wrapped[12..<16]), Array("VP8X".utf8))
        XCTAssertEqual(wrapped[20], 0x30, "ICC and alpha flags")
        XCTAssertEqual(Array(wrapped[24..<27]), [43, 1, 0], "canvas width - 1 = 299")
        XCTAssertEqual(Array(wrapped[27..<30]), [1, 0, 0], "canvas height - 1 = 1")
        XCTAssertEqual(Array(wrapped[30..<34]), Array("ICCP".utf8))
        XCTAssertEqual(Array(wrapped[38..<42]), [1, 2, 3, 0], "odd profile padded")
        XCTAssertEqual(Array(wrapped[42...]), Array([UInt8](simple)[12...]), "the bitstream chunk is unchanged")
        XCTAssertEqual(AttachmentPolicy.webPFormat(Data(wrapped)), .lossless)
        XCTAssertNil(AttachmentPolicy.webPAddingICCProfile(Data(wrapped), profile: profile, width: 300, height: 2, hasAlpha: true),
                     "only simple-format input")
    }

    func testKeepReencodedThresholds() {
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/jpeg", originalSize: 1000, encodedSize: 900, lossless: false))
        XCTAssertFalse(AttachmentPolicy.keepReencoded(originalType: "image/jpeg", originalSize: 1000, encodedSize: 901, lossless: false))
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/png", originalSize: 1000, encodedSize: 999, lossless: true))
        XCTAssertFalse(AttachmentPolicy.keepReencoded(originalType: "image/png", originalSize: 1000, encodedSize: 1000, lossless: true))
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/heic", originalSize: 1000, encodedSize: 5000, lossless: false))
        XCTAssertTrue(AttachmentPolicy.keepReencoded(originalType: "image/bmp", originalSize: 1000, encodedSize: 5000, lossless: true))
        XCTAssertFalse(AttachmentPolicy.keepReencoded(originalType: "image/heic", originalSize: 1000, encodedSize: 0, lossless: false))
        XCTAssertEqual(AttachmentPolicy.renamed("IMG_0001.HEIC", contentType: "image/jpeg"), "IMG_0001.jpg")
        XCTAssertEqual(AttachmentPolicy.renamed("Screen Shot.tiff", contentType: "image/png"), "Screen Shot.png")
        XCTAssertEqual(AttachmentPolicy.renamed("shot.png", contentType: "image/webp"), "shot.webp")
        XCTAssertEqual(AttachmentPolicy.renamed("clip.mov", contentType: "video/mp4"), "clip.mp4")
        XCTAssertEqual(AttachmentPolicy.renamed("noext", contentType: "image/webp"), "noext.webp")
        XCTAssertEqual(AttachmentPolicy.renamed("notes.txt", contentType: "text/plain"), "notes.txt")
    }

    func testPreviewSizing() {
        let settings = AttachmentCompression()
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .image, width: 641, height: 10, byteSize: 100, settings: settings))
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .image, width: 640, height: 480, byteSize: 524_288, settings: settings))
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .image, width: 100, height: 100, byteSize: 524_289, settings: settings))
        XCTAssertTrue(AttachmentPolicy.needsPreview(kind: .video, width: 100, height: 100, byteSize: 1, settings: settings), "videos get a poster")
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .audio, width: nil, height: nil, byteSize: 9_999_999, settings: settings))
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .file, width: 4000, height: 4000, byteSize: 9_999_999, settings: settings))
        var off = settings
        off.previewEdge = 0
        XCTAssertFalse(AttachmentPolicy.needsPreview(kind: .image, width: 4000, height: 4000, byteSize: 1, settings: off))
        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 4000, height: 3000, edge: 640) == (640, 480))
        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 300, height: 9000, edge: 640) == (21, 640))
        XCTAssertTrue(AttachmentPolicy.fitWithin(width: 200, height: 100, edge: 640) == (200, 100), "never enlarged")
    }

    func testVideoPresetKeepsShortEdgeWithinLimit() {
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 3840, height: 2160, maxHeight: 1080), "AVAssetExportPreset1920x1080")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 720), "AVAssetExportPreset1280x720")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 1080, height: 1920, maxHeight: 1080), "AVAssetExportPreset3840x2160",
                       "the limit bounds the short edge, so portrait 1080p is re-encoded without scaling")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 2160, height: 3840, maxHeight: 1080), "AVAssetExportPreset1920x1080")
        XCTAssertEqual(AttachmentPolicy.videoPreset(width: 640, height: 360, maxHeight: 1080), "AVAssetExportPreset3840x2160",
                       "small videos are never enlarged, only re-encoded")
        XCTAssertNil(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 0), "0 disables transcoding")
        XCTAssertNil(AttachmentPolicy.videoPreset(width: 1920, height: 1080, maxHeight: 200))
    }

    func testVideoIsTranscodedOnlyWhenNeeded() {
        let settings = AttachmentCompression()
        func facts(_ width: Int, _ height: Int, h264: Bool = true, hdr: Bool = false, kbps: Double? = 4000, playable: Bool = true) -> AttachmentPolicy.VideoFacts {
            AttachmentPolicy.VideoFacts(width: width, height: height, isH264: h264, isHDR: hdr, bitrateKbps: kbps, playableContainer: playable)
        }
        func plan(_ facts: AttachmentPolicy.VideoFacts, _ settings: AttachmentCompression = AttachmentCompression()) -> AttachmentPolicy.VideoPlan {
            AttachmentPolicy.videoPlan(facts, settings: settings)
        }
        let full = "AVAssetExportPreset3840x2160", hd = "AVAssetExportPreset1920x1080"
        XCTAssertEqual(plan(facts(1920, 1080)), .keep, "efficient 1080p H.264 is never re-compressed")
        XCTAssertEqual(plan(facts(1920, 1080, kbps: 7500)), .keep, "up to 1.25 × the 6000 kbps target")
        XCTAssertEqual(plan(facts(1920, 1080, kbps: 7600)), .transcode(preset: full, required: false), "bitrate only: size-only transcode")
        XCTAssertEqual(plan(facts(3840, 2160, kbps: 20_000)), .transcode(preset: hd, required: false), "short edge above 1080")
        XCTAssertEqual(plan(facts(2160, 3840, kbps: 20_000)), .transcode(preset: hd, required: false), "portrait 4K")
        XCTAssertEqual(plan(facts(1080, 1920, h264: false, kbps: 8000)), .transcode(preset: full, required: true), "HEVC portrait 1080p keeps its size")
        XCTAssertEqual(plan(facts(1920, 1080, h264: false, hdr: true, kbps: 10_000)), .transcode(preset: full, required: true), "iPhone HDR (HEVC HLG)")
        XCTAssertEqual(plan(facts(1920, 1080, hdr: true)), .transcode(preset: full, required: true), "HDR H.264 is tone mapped too")
        XCTAssertEqual(plan(facts(640, 360, kbps: 1875)), .keep, "small videos get the 1500 kbps floor")
        XCTAssertEqual(plan(facts(640, 360, kbps: 1900)), .transcode(preset: full, required: false))
        XCTAssertEqual(plan(facts(1920, 1080, kbps: nil)), .keep, "unknown bitrate is not a reason")
        XCTAssertEqual(plan(facts(1920, 1080, playable: false)), .remux, "H.264 in an unplayable container is only rewrapped")
        XCTAssertEqual(plan(facts(1920, 1080, h264: false, playable: false)), .transcode(preset: full, required: true))
        var off = settings
        off.videoMaxHeight = 0
        XCTAssertEqual(plan(facts(3840, 2160, h264: false, hdr: true), off), .keep, "0 uploads videos unchanged")
        var tiny = settings
        tiny.videoMaxHeight = 200
        XCTAssertEqual(plan(facts(1920, 1080, h264: false), tiny), .keep, "no preset fits: keep the original")

        XCTAssertEqual(AttachmentPolicy.videoTargetKbps(width: 1920, height: 1080, settings: settings), 6000)
        XCTAssertEqual(AttachmentPolicy.videoTargetKbps(width: 1280, height: 720, settings: settings), 6000 * 921_600 / 2_073_600, accuracy: 0.001)
        XCTAssertEqual(AttachmentPolicy.videoTargetKbps(width: 320, height: 240, settings: settings), 1500)
    }

    func testTranscodedVideoMustBeSmallerSDRH264WithAudio() {
        func keep(required: Bool = false, size: Int = 800, h264: Bool = true, hdr: Bool = false, sourceAudio: Bool = true, outputAudio: Bool = true) -> Bool {
            AttachmentPolicy.keepTranscoded(required: required, originalSize: 1000, outputSize: size, outputIsH264: h264, outputIsHDR: hdr,
                                            sourceHasAudio: sourceAudio, outputHasAudio: outputAudio)
        }
        XCTAssertTrue(keep(size: 900))
        XCTAssertFalse(keep(size: 901), "a size-only transcode must save 10%")
        XCTAssertTrue(keep(required: true, size: 5000), "codec and HDR transcodes are kept at any size")
        XCTAssertFalse(keep(required: true, hdr: true), "never a washed-out or still-HDR file")
        XCTAssertFalse(keep(required: true, h264: false))
        XCTAssertFalse(keep(required: true, outputAudio: false), "audio is never dropped")
        XCTAssertTrue(keep(sourceAudio: false, outputAudio: false))
        XCTAssertFalse(keep(size: 0))
    }

    func testHDRTransferFunctionsAreDetected() throws {
        func format(_ transfer: CFString?) throws -> CMFormatDescription {
            var extensions: [CFString: Any] = [:]
            if let transfer { extensions[kCMFormatDescriptionExtension_TransferFunction] = transfer }
            var description: CMFormatDescription?
            let status = CMVideoFormatDescriptionCreate(allocator: kCFAllocatorDefault, codecType: kCMVideoCodecType_HEVC, width: 1920, height: 1080,
                                                        extensions: extensions as CFDictionary, formatDescriptionOut: &description)
            XCTAssertEqual(status, 0)
            return try XCTUnwrap(description)
        }
        XCTAssertTrue(AttachmentPreparer.isHDRFormat(try format(kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG)))
        XCTAssertTrue(AttachmentPreparer.isHDRFormat(try format(kCMFormatDescriptionTransferFunction_SMPTE_ST_2084_PQ)))
        XCTAssertFalse(AttachmentPreparer.isHDRFormat(try format(kCMFormatDescriptionTransferFunction_ITU_R_709_2)))
        XCTAssertFalse(AttachmentPreparer.isHDRFormat(try format(nil)))
    }

    // MARK: Image pipeline

    private func rgbaImage(width: Int, height: Int, space: CGColorSpace = CGColorSpace(name: CGColorSpace.sRGB)!,
                           pixel: (Int, Int) -> [UInt8]) -> CGImage {
        var bytes: [UInt8] = []
        for y in 0..<height { for x in 0..<width { bytes += pixel(x, y) } }
        // Premultiply for CoreGraphics.
        for offset in stride(from: 0, to: bytes.count, by: 4) {
            let alpha = Int(bytes[offset + 3])
            for channel in 0..<3 { bytes[offset + channel] = UInt8((Int(bytes[offset + channel]) * alpha + 127) / 255) }
        }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4, space: space,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }

    /// Thousands of colours, like a real screenshot with anti-aliased text.
    private func manyColours(width: Int, height: Int, space: CGColorSpace = CGColorSpace(name: CGColorSpace.sRGB)!) -> CGImage {
        rgbaImage(width: width, height: height, space: space) { x, y in [UInt8(x % 256), UInt8(y % 256), UInt8((x * 7 + y * 13) % 256), 255] }
    }

    /// Deterministic noise, like a photo.
    private func noise(width: Int, height: Int) -> CGImage {
        var state: UInt32 = 12_345
        func next() -> UInt8 {
            state = state &* 1_103_515_245 &+ 12_345
            return UInt8(truncatingIfNeeded: state >> 16)
        }
        return rgbaImage(width: width, height: height) { _, _ in [next(), next(), next(), 255] }
    }

    private func encoded(_ image: CGImage, type: UTType, quality: Double = 1, properties: [CFString: Any] = [:]) throws -> Data {
        let output = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output as CFMutableData, type.identifier as CFString, 1, nil))
        var options = properties
        options[kCGImageDestinationLossyCompressionQuality] = quality
        CGImageDestinationAddImage(destination, image, options as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return output as Data
    }

    private func properties(_ data: Data) throws -> [CFString: Any] {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        return try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    }

    private func decodedPixels(_ data: Data) throws -> (width: Int, height: Int, rgba: [UInt8]) {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        return (image.width, image.height, pixels.straight)
    }

    /// Opaque pixels exactly equal; alpha equal everywhere.
    private func assertSamePixels(_ lhs: [UInt8], _ rhs: [UInt8], file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(lhs.count, rhs.count, file: file, line: line)
        guard lhs.count == rhs.count else { return }
        var mismatches = 0
        for offset in stride(from: 0, to: lhs.count, by: 4) {
            if lhs[offset + 3] != rhs[offset + 3] { mismatches += 1; continue }
            if lhs[offset + 3] == 255, lhs[offset..<offset + 3] != rhs[offset..<offset + 3] { mismatches += 1 }
        }
        XCTAssertEqual(mismatches, 0, "pixels differ", file: file, line: line)
    }

    func testIndexedPNGRoundTripsExactPixelsThroughImageIO() throws {
        let colors: [[UInt8]] = [[255, 0, 0, 255], [0, 128, 255, 255], [0, 0, 0, 0], [20, 200, 40, 255]]
        let width = 7, height = 5
        var rgba: [UInt8] = []
        for y in 0..<height { for x in 0..<width { rgba += colors[(x * 3 + y) % colors.count] } }
        let palette = try XCTUnwrap(IndexedPNG.palette(width: width, height: height, rgba: rgba))
        XCTAssertEqual(palette.colors.count, 4)
        XCTAssertEqual(palette.colors.first, 0x0000_0000, "translucent colours come first for a short tRNS chunk")
        let png = try XCTUnwrap(IndexedPNG.encode(palette))
        XCTAssertEqual(Array(png.prefix(8)), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        XCTAssertEqual(png[24], 2, "four colours pack at two bits per pixel")
        XCTAssertEqual(png[25], 3, "colour type 3 (indexed)")
        XCTAssertNotNil(png.range(of: Data("tRNS".utf8)))

        let decoded = try decodedPixels(png)
        XCTAssertEqual(decoded.width, width)
        XCTAssertEqual(decoded.height, height)
        assertSamePixels(decoded.rgba, rgba)
    }

    func testIndexedPNGEightBitAndTranslucentPalette() throws {
        let width = 30, height = 8 // 240 distinct colours
        var rgba: [UInt8] = []
        for y in 0..<height { for x in 0..<width { rgba += [UInt8(x * 6), UInt8(y * 20), 77, x == 0 ? 128 : 255] } }
        let palette = try XCTUnwrap(IndexedPNG.palette(width: width, height: height, rgba: rgba))
        XCTAssertEqual(IndexedPNG.bitDepth(colorCount: palette.colors.count), 8)
        let decoded = try decodedPixels(try XCTUnwrap(IndexedPNG.encode(palette)))
        for pixel in 0..<(width * height) {
            for channel in 0..<4 {
                let expected = Int(rgba[pixel * 4 + channel]), actual = Int(decoded.rgba[pixel * 4 + channel])
                // Half-transparent colours pass through premultiplied drawing; allow rounding.
                XCTAssertLessThanOrEqual(abs(expected - actual), rgba[pixel * 4 + 3] == 255 ? 0 : 2, "pixel \(pixel) channel \(channel)")
            }
        }
        var many: [UInt8] = []
        for value in 0..<257 { many += [UInt8(value % 256), UInt8(value / 256), 0, 255] }
        XCTAssertNil(IndexedPNG.palette(width: 257, height: 1, rgba: many), "more than 256 colours")
        XCTAssertNil(IndexedPNG.palette(width: 2, height: 1, rgba: [0, 0, 0, 255, 1, 1, 1, 255], maxColors: 1))
    }

    func testIndexedPNGEmbedsAWideGamutProfile() throws {
        let profile = try XCTUnwrap(CGColorSpace(name: CGColorSpace.displayP3)?.copyICCData().map { $0 as Data })
        let palette = try XCTUnwrap(IndexedPNG.palette(width: 2, height: 1, rgba: [255, 0, 0, 255, 0, 255, 0, 255]))
        let png = try XCTUnwrap(IndexedPNG.encode(palette, iccProfile: profile))
        XCTAssertEqual(Array(png[37..<41]), Array("iCCP".utf8), "the colour chunk follows IHDR")
        let source = try XCTUnwrap(CGImageSourceCreateWithData(png as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        XCTAssertNotNil(image.colorSpace?.copyICCData(), "ImageIO reads the embedded profile")
    }

    func testChecksumsMatchKnownVectors() {
        XCTAssertEqual(IndexedPNG.adler32(Array("Wikipedia".utf8)), 0x11E6_0398)
        XCTAssertEqual(IndexedPNG.crc32(Array("IEND".utf8)), 0xAE42_6082)
        let large = [UInt8](repeating: 0xFF, count: 100_000)
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in large { a = (a + UInt32(byte)) % 65521; b = (b + a) % 65521 }
        XCTAssertEqual(IndexedPNG.adler32(large), b << 16 | a)
    }

    func testLosslessWebPRoundTripsExactPixelsThroughImageIO() throws {
        let image = rgbaImage(width: 300, height: 200) { x, y in
            [UInt8(x % 256), UInt8(y % 256), UInt8((x * 7 + y * 13) % 256), x % 5 == 0 ? 0 : (x % 3 == 0 ? 128 : 255)]
        }
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        XCTAssertNil(pixels.iccProfile, "sRGB needs no profile")
        let webP = try XCTUnwrap(AttachmentPreparer.encodeLosslessWebP(pixels))
        XCTAssertEqual(AttachmentPolicy.webPFormat(webP), .lossless)
        XCTAssertTrue(AttachmentPreparer.reproduces(webP, pixels))
        let decoded = try decodedPixels(webP)
        XCTAssertEqual(decoded.width, 300)
        XCTAssertEqual(decoded.height, 200)
        assertSamePixels(decoded.rgba, pixels.straight)
    }

    func testLosslessWebPKeepsAWideGamutProfile() throws {
        let p3 = try XCTUnwrap(CGColorSpace(name: CGColorSpace.displayP3))
        let image = manyColours(width: 120, height: 80, space: p3)
        let pixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(image))
        XCTAssertNotNil(pixels.iccProfile, "drawn in its own Display P3 space, not converted to sRGB")
        let webP = try XCTUnwrap(AttachmentPreparer.encodeLosslessWebP(pixels))
        XCTAssertEqual(Array([UInt8](webP)[12..<16]), Array("VP8X".utf8))
        XCTAssertEqual(AttachmentPolicy.webPFormat(webP), .lossless)
        XCTAssertTrue(AttachmentPreparer.reproduces(webP, pixels), "decodes to the same Display P3 values")
    }

    func testScreenshotWithManyColoursBecomesLosslessWebPNeverLossy() async throws {
        let png = try encoded(manyColours(width: 800, height: 300), type: .png)
        let original = try AttachmentStaging.stage(data: png, filename: "shot.png")
        defer { AttachmentStaging.remove(original.url) }
        for quality in [92, 100] {
            var settings = AttachmentCompression()
            settings.imageQuality = quality
            let prepared = await AttachmentPreparer.prepare(original, settings: settings)
            defer { AttachmentStaging.remove(prepared.fileURL) }
            XCTAssertEqual(prepared.contentType, "image/webp", "quality \(quality)")
            XCTAssertEqual(prepared.name, "shot.webp")
            XCTAssertLessThan(prepared.byteSize, png.count)
            let data = try Data(contentsOf: prepared.fileURL)
            XCTAssertEqual(data.count, prepared.byteSize)
            XCTAssertEqual(AttachmentPolicy.webPFormat(data), .lossless)
            assertSamePixels(try decodedPixels(data).rgba, try decodedPixels(png).rgba)
            XCTAssertEqual(prepared.width, 800)
            XCTAssertEqual(prepared.height, 300)
            XCTAssertNotNil(prepared.preview, "800 px exceeds the 640 px preview edge")
        }
    }

    func testFlatPNGBecomesAnExactSmallerLosslessFile() async throws {
        let png = try encoded(rgbaImage(width: 900, height: 500) { x, _ in x < 450 ? [12, 13, 15, 255] : [182, 77, 50, 255] }, type: .png)
        let original = try AttachmentStaging.stage(data: png, filename: "ui.png")
        defer { AttachmentStaging.remove(original.url) }
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertTrue(["image/png", "image/webp"].contains(prepared.contentType), prepared.contentType)
        XCTAssertLessThan(prepared.byteSize, png.count)
        assertSamePixels(try decodedPixels(try Data(contentsOf: prepared.fileURL)).rgba, try decodedPixels(png).rgba)
    }

    func testPrepareImageConvertsFlatJPEGToIndexedPNGWithPreview() async throws {
        let jpeg = try encoded(rgbaImage(width: 900, height: 500) { _, _ in [99, 122, 67, 255] }, type: .jpeg)
        let original = try AttachmentStaging.stage(data: jpeg, filename: "screen.jpg")
        defer { AttachmentStaging.remove(original.url) }
        XCTAssertEqual(original.contentType, "image/jpeg")
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertEqual(prepared.contentType, "image/png")
        XCTAssertEqual(prepared.name, "screen.png")
        XCTAssertEqual(prepared.kind, .image)
        XCTAssertEqual(prepared.width, 900)
        XCTAssertEqual(prepared.height, 500)
        XCTAssertEqual(prepared.sourceSize, jpeg.count)
        XCTAssertLessThan(prepared.byteSize, jpeg.count)
        XCTAssertEqual((try prepared.fileURL.resourceValues(forKeys: [.fileSizeKey])).fileSize, prepared.byteSize)
        let preview = try XCTUnwrap(prepared.preview, "900 px exceeds the 640 px preview edge")
        XCTAssertTrue(["image/webp", "image/jpeg"].contains(preview.contentType))
        XCTAssertLessThanOrEqual(preview.data.count, 512 * 1024)
        let previewPixels = try decodedPixels(preview.data)
        XCTAssertEqual(max(previewPixels.width, previewPixels.height), 640)

        var capped = AttachmentCompression()
        capped.imageMaxEdge = 300
        let scaled = await AttachmentPreparer.prepare(original, settings: capped)
        defer { AttachmentStaging.remove(scaled.fileURL) }
        XCTAssertEqual(scaled.width, 300)
        XCTAssertTrue((166...167).contains(scaled.height ?? 0), "\(String(describing: scaled.height))")
    }

    func testPhotoIsReencodedScaledRotatedAndLosesGPS() async throws {
        let gps: [CFString: Any] = [kCGImagePropertyGPSLatitude: 37.3349, kCGImagePropertyGPSLatitudeRef: "N",
                                    kCGImagePropertyGPSLongitude: 122.009, kCGImagePropertyGPSLongitudeRef: "W"]
        let jpeg = try encoded(noise(width: 600, height: 400), type: .jpeg, quality: 1,
                               properties: [kCGImagePropertyOrientation: 6, kCGImagePropertyGPSDictionary: gps])
        XCTAssertNotNil(try properties(jpeg)[kCGImagePropertyGPSDictionary], "fixture carries a location")
        let original = try AttachmentStaging.stage(data: jpeg, filename: "IMG_0001.JPG")
        defer { AttachmentStaging.remove(original.url) }
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertTrue(["image/webp", "image/jpeg"].contains(prepared.contentType), prepared.contentType)
        XCTAssertNotEqual(prepared.fileURL, original.url)
        XCTAssertLessThanOrEqual(Double(prepared.byteSize), Double(jpeg.count) * 0.9)
        XCTAssertEqual(prepared.width, 400, "orientation 6 is applied")
        XCTAssertEqual(prepared.height, 600)
        let output = try properties(try Data(contentsOf: prepared.fileURL))
        XCTAssertNil(output[kCGImagePropertyGPSDictionary])
        XCTAssertEqual((output[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1, 1)

        var capped = AttachmentCompression()
        capped.imageMaxEdge = 300
        let scaled = await AttachmentPreparer.prepare(original, settings: capped)
        defer { AttachmentStaging.remove(scaled.fileURL) }
        XCTAssertEqual(scaled.height, 300, "longest edge scaled to imageMaxEdge")
        XCTAssertEqual(scaled.width, 200)
    }

    func testNonMediaFilesUploadUnchanged() async throws {
        let original = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        defer { AttachmentStaging.remove(original.url) }
        let prepared = await AttachmentPreparer.prepare(original, settings: AttachmentCompression())
        XCTAssertEqual(prepared.fileURL, original.url)
        XCTAssertEqual(prepared.contentType, "text/plain")
        XCTAssertEqual(prepared.kind, .file)
        XCTAssertEqual(prepared.byteSize, 5)
        XCTAssertNil(prepared.preview)
    }

    // MARK: Metadata on unchanged originals

    func testKeptJPEGLosesLocationButKeepsOrientationAndScanData() throws {
        let gps: [CFString: Any] = [kCGImagePropertyGPSLatitude: 51.5, kCGImagePropertyGPSLatitudeRef: "N",
                                    kCGImagePropertyGPSLongitude: 0.12, kCGImagePropertyGPSLongitudeRef: "W"]
        let jpeg = try encoded(noise(width: 64, height: 48), type: .jpeg, quality: 0.5,
                               properties: [kCGImagePropertyOrientation: 6, kCGImagePropertyGPSDictionary: gps])
        XCTAssertNotNil(try properties(jpeg)[kCGImagePropertyGPSDictionary])
        let stripped = try XCTUnwrap(AttachmentMetadata.strippedJPEG(jpeg, orientation: 6))
        let output = try properties(stripped)
        XCTAssertNil(output[kCGImagePropertyGPSDictionary], "location removed")
        XCTAssertEqual((output[kCGImagePropertyOrientation] as? NSNumber)?.intValue, 6, "orientation kept in a minimal Exif segment")
        // Baseline JPEG has one SOS; entropy-coded data never contains FF DA.
        let scan = Data([0xFF, 0xDA])
        let originalScan = try XCTUnwrap(jpeg.range(of: scan, options: .backwards)).lowerBound
        let strippedScan = try XCTUnwrap(stripped.range(of: scan, options: .backwards)).lowerBound
        XCTAssertLessThan(stripped.count, jpeg.count)
        XCTAssertEqual(jpeg[originalScan...], stripped[strippedScan...], "entropy-coded data is byte-identical")
        XCTAssertEqual(try decodedPixels(stripped).rgba, try decodedPixels(jpeg).rgba)

        let upright = try XCTUnwrap(AttachmentMetadata.strippedJPEG(jpeg, orientation: 1))
        XCTAssertNil(try properties(upright)[kCGImagePropertyGPSDictionary])
        XCTAssertEqual((try properties(upright)[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1, 1)
        XCTAssertLessThan(upright.count, stripped.count)
        XCTAssertNil(AttachmentMetadata.strippedJPEG(Data("not a jpeg".utf8), orientation: 1))

        let segment = AttachmentMetadata.orientationExifSegment(3)
        XCTAssertEqual(segment.count, 36)
        XCTAssertEqual(Array(segment[0..<4]), [0xFF, 0xE1, 0x00, 0x22])
    }

    func testJPEGKeptAtFullQualityUploadsAStrippedCopy() async throws {
        let gps: [CFString: Any] = [kCGImagePropertyGPSLatitude: 1.0, kCGImagePropertyGPSLatitudeRef: "N"]
        let jpeg = try encoded(noise(width: 64, height: 48), type: .jpeg, quality: 0.5, properties: [kCGImagePropertyGPSDictionary: gps])
        let original = try AttachmentStaging.stage(data: jpeg, filename: "photo.jpg")
        defer { AttachmentStaging.remove(original.url) }
        var settings = AttachmentCompression()
        settings.imageQuality = 100
        let prepared = await AttachmentPreparer.prepare(original, settings: settings)
        defer { AttachmentStaging.remove(prepared.fileURL) }
        XCTAssertNotEqual(prepared.fileURL, original.url, "a stripped copy replaces the original")
        XCTAssertEqual(prepared.contentType, "image/jpeg")
        XCTAssertEqual(prepared.name, "photo.jpg")
        let data = try Data(contentsOf: prepared.fileURL)
        XCTAssertEqual(data.count, prepared.byteSize)
        XCTAssertNil(try properties(data)[kCGImagePropertyGPSDictionary])
    }

    private func pngChunk(_ type: String, _ payload: [UInt8]) -> [UInt8] {
        let length = UInt32(payload.count)
        var chunk: [UInt8] = [UInt8(length >> 24 & 0xFF), UInt8(length >> 16 & 0xFF), UInt8(length >> 8 & 0xFF), UInt8(length & 0xFF)]
        var body = Array(type.utf8)
        body += payload
        chunk += body
        let crc = IndexedPNG.crc32(body)
        chunk += [UInt8(crc >> 24 & 0xFF), UInt8(crc >> 16 & 0xFF), UInt8(crc >> 8 & 0xFF), UInt8(crc & 0xFF)]
        return chunk
    }

    func testKeptPNGLosesTextAndExifChunks() throws {
        let palette = try XCTUnwrap(IndexedPNG.palette(width: 2, height: 1, rgba: [255, 0, 0, 255, 0, 0, 255, 255]))
        let png = [UInt8](try XCTUnwrap(IndexedPNG.encode(palette)))
        let idat = try XCTUnwrap(Data(png).range(of: Data("IDAT".utf8))).lowerBound - 4
        var tagged = Array(png[..<idat])
        tagged += pngChunk("tEXt", Array("Location\u{0}Home".utf8))
        tagged += pngChunk("eXIf", [0x4D, 0x4D, 0x00, 0x2A])
        tagged += pngChunk("iTXt", Array("Comment\u{0}\u{0}\u{0}\u{0}\u{0}hi".utf8))
        tagged += pngChunk("zTXt", [0x41, 0x00, 0x00, 0x78, 0x9C])
        tagged += png[idat...]
        let stripped = try XCTUnwrap(AttachmentMetadata.strippedPNG(Data(tagged)))
        XCTAssertEqual(stripped, Data(png), "every other chunk is copied byte for byte")
        XCTAssertNil(AttachmentMetadata.strippedPNG(Data(png)), "nothing to remove")
        XCTAssertNil(AttachmentMetadata.strippedPNG(Data("nope".utf8)))
    }

    private func concat(_ parts: [UInt8]...) -> [UInt8] { parts.flatMap { $0 } }

    private func mp4RawBox(_ type: [UInt8], _ payload: [UInt8], sizeField: Int? = nil) -> [UInt8] {
        let size = sizeField ?? (8 + payload.count)
        var box: [UInt8] = [UInt8(size >> 24 & 0xFF), UInt8(size >> 16 & 0xFF), UInt8(size >> 8 & 0xFF), UInt8(size & 0xFF)]
        box += type
        box += payload
        return box
    }

    private func mp4Box(_ type: String, _ payload: [UInt8], sizeField: Int? = nil) -> [UInt8] {
        mp4RawBox(Array(type.utf8), payload, sizeField: sizeField)
    }

    /// `size == 1` with a 64-bit length.
    private func mp4LargeBox(_ type: String, _ payload: [UInt8]) -> [UInt8] {
        let size = UInt64(16 + payload.count)
        var box: [UInt8] = [0, 0, 0, 1]
        box += Array(type.utf8)
        for shift in stride(from: 56, through: 0, by: -8) { box.append(UInt8(size >> UInt64(shift) & 0xFF)) }
        box += payload
        return box
    }

    private func childTypes(_ bytes: [UInt8], in box: AttachmentMetadata.Box) throws -> [String] {
        try XCTUnwrap(AttachmentMetadata.boxes(from: box.contentStart, to: box.end) { offset, count in
            offset + count <= bytes.count ? Array(bytes[offset..<offset + count]) : nil
        }).map(\.type)
    }

    func testQuickTimeLocationBoxesAreRenamedWithoutMovingSamples() throws {
        let location = mp4RawBox([0xA9, 0x78, 0x79, 0x7A], Array("+37.3349-122.0090/".utf8)) // ©xyz
        let keys = mp4Box("meta", Array("com.apple.quicktime.location.ISO6709 +51.5-000.1/".utf8))
        let trackHeader = mp4Box("tkhd", [UInt8](repeating: 0, count: 12))
        let trackUserData = mp4Box("udta", mp4Box("name", Array("cam".utf8)))
        let media = mp4Box("mdia", [UInt8](repeating: 1, count: 8))
        let track = mp4Box("trak", concat(trackHeader, trackUserData, media))
        let movieHeader = mp4Box("mvhd", [UInt8](repeating: 0, count: 20))
        let moov = mp4Box("moov", concat(movieHeader, mp4Box("udta", location), keys, track))
        let samples: [UInt8] = (0..<200).map { UInt8(truncatingIfNeeded: $0 &* 37) }
        // moov at the end, after a 64-bit mdat.
        let file = concat(mp4Box("ftyp", Array("qt  0000".utf8)), mp4LargeBox("mdat", samples), moov)
        let original = try AttachmentStaging.stage(data: Data(file), filename: "IMG_0002.MOV")
        defer { AttachmentStaging.remove(original.url) }

        let strippedURL = try XCTUnwrap(AttachmentMetadata.strippedQuickTimeCopy(of: original.url, name: original.name))
        defer { AttachmentStaging.remove(strippedURL) }
        let stripped = [UInt8](try Data(contentsOf: strippedURL))
        XCTAssertEqual(stripped.count, file.count, "no box changes size")
        XCTAssertEqual([UInt8](try Data(contentsOf: original.url)), file, "the staged original is untouched")
        let text = String(decoding: stripped, as: UTF8.self)
        XCTAssertFalse(text.contains("+37.3349-122.0090"), "location bytes are zeroed, not just hidden")
        XCTAssertFalse(text.contains("ISO6709"), "QuickTime location keys are zeroed")
        let mdatStart = 16
        XCTAssertEqual(Array(stripped[mdatStart..<mdatStart + 16 + samples.count]), Array(file[mdatStart..<mdatStart + 16 + samples.count]),
                       "sample data is byte-identical")

        let read: (Int, Int) -> [UInt8]? = { offset, count in offset + count <= stripped.count ? Array(stripped[offset..<offset + count]) : nil }
        let top = try XCTUnwrap(AttachmentMetadata.boxes(from: 0, to: stripped.count, read: read))
        XCTAssertEqual(top.map(\.type), ["ftyp", "mdat", "moov"])
        let moovBox = try XCTUnwrap(top.last)
        XCTAssertEqual(try childTypes(stripped, in: moovBox), ["mvhd", "free", "free", "trak"])
        let trakBox = try XCTUnwrap(AttachmentMetadata.boxes(from: moovBox.contentStart, to: moovBox.end, read: read)?.last)
        XCTAssertEqual(try childTypes(stripped, in: trakBox), ["tkhd", "free", "mdia"])

        XCTAssertNil(AttachmentMetadata.strippedQuickTimeCopy(of: strippedURL, name: "again.mov"), "nothing left to remove")
    }

    func testQuickTimeStripperHandlesSizeZeroAndRejectsMalformedFiles() throws {
        let moov = mp4Box("moov", mp4Box("udta", mp4RawBox([0xA9, 0x78, 0x79, 0x7A], [1, 2, 3])))
        let tail = mp4Box("mdat", [9, 9, 9, 9], sizeField: 0) // Runs to the end of the file.
        let file = concat(mp4Box("ftyp", Array("isom".utf8)), moov, tail)
        let offsets = AttachmentMetadata.quickTimeMetadataTypeOffsets(fileSize: file.count) { offset, count in
            offset + count <= file.count ? Array(file[offset..<offset + count]) : nil
        }
        XCTAssertEqual(offsets, [12 + 8 + 4])

        var broken = file
        broken[15] = 0xFF // moov now claims more bytes than the file has.
        XCTAssertNil(AttachmentMetadata.quickTimeMetadataTypeOffsets(fileSize: broken.count) { offset, count in
            offset + count <= broken.count ? Array(broken[offset..<offset + count]) : nil
        })
        let text = try AttachmentStaging.stage(data: Data("plain text, not a movie".utf8), filename: "a.mov")
        defer { AttachmentStaging.remove(text.url) }
        XCTAssertNil(AttachmentMetadata.strippedQuickTimeCopy(of: text.url, name: text.name))
    }

    // MARK: Upload sequence

    private func header(_ request: AttachmentURLProtocol.Recorded, _ name: String) -> String? {
        request.headers.first { $0.key.lowercased() == name }?.value
    }

    func testUploadReservesPutsPreviewThenFileWithExactHeadersWithoutCredentialsThenCompletes() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 7, count: 2048), filename: "shot.webp")
        defer { AttachmentStaging.remove(original.url) }
        var prepared = PreparedAttachment(original: original)
        prepared.sourceSize = 9000
        prepared.width = 800
        prepared.height = 600
        prepared.preview = AttachmentPreviewImage(data: Data(repeating: 1, count: 100), contentType: "image/jpeg")
        let completes = FractionLog()
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.absoluteString ?? "") {
            case ("POST", "https://caper.invalid/api/assets"):
                return (201, Data(#"{"id":"Asset0000001","kind":"image","upload":{"method":"PUT","url":"https://r2.invalid/original/Asset0000001?X-Amz-Signature=o","headers":{"content-type":"image/webp","content-disposition":"attachment; filename=\"shot.webp\""}},"previewUpload":{"method":"PUT","url":"https://r2.invalid/preview/Asset0000001?X-Amz-Signature=p","headers":{"content-type":"image/jpeg"}},"storage":{"used":2148,"limit":10000}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "https://caper.invalid/api/assets/Asset0000001/complete"):
                completes.append(1)
                // Storage has not reported the object yet on the first attempt.
                if completes.values.count < 2 { return (409, Data(#"{"error":"upload not finished"}"#.utf8)) }
                return (200, Data(#"{"id":"Asset0000001","kind":"image","contentType":"image/webp","name":"shot.webp","size":2048,"width":800,"height":600,"preview":{}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let fractions = FractionLog()
        let attachment = try await AttachmentUploader.upload(prepared, channelID: channelID, api: client(),
                                                             completeRetryDelays: [.zero]) { fractions.append($0) }
        XCTAssertEqual(attachment.id, "Asset0000001")
        XCTAssertTrue(attachment.hasPreview)
        XCTAssertEqual(attachment.status, .ready, "today's API omits status")

        let requests = AttachmentURLProtocol.requests
        XCTAssertEqual(requests.map { "\($0.method) \($0.url.host ?? "")\($0.url.path)" }, [
            "POST caper.invalid/api/assets",
            "PUT r2.invalid/preview/Asset0000001",
            "PUT r2.invalid/original/Asset0000001",
            "POST caper.invalid/api/assets/Asset0000001/complete",
            "POST caper.invalid/api/assets/Asset0000001/complete",
        ])
        let body = try XCTUnwrap(json(requests[0].body))
        XCTAssertEqual(Set(body.keys), ["channelId", "filename", "contentType", "byteSize", "sourceByteSize", "width", "height", "preview"],
                       "absent values are omitted, not null")
        XCTAssertEqual(body["channelId"] as? String, channelID)
        XCTAssertEqual(body["filename"] as? String, "shot.webp")
        XCTAssertEqual(body["contentType"] as? String, "image/webp")
        XCTAssertEqual(body["byteSize"] as? Int, 2048)
        XCTAssertEqual(body["sourceByteSize"] as? Int, 9000)
        XCTAssertEqual(body["width"] as? Int, 800)
        XCTAssertEqual((body["preview"] as? [String: Any])?["byteSize"] as? Int, 100)
        XCTAssertEqual((body["preview"] as? [String: Any])?["contentType"] as? String, "image/jpeg")
        XCTAssertEqual(header(requests[0], "authorization"), "Bearer account-secret")
        XCTAssertEqual(header(requests[1], "content-type"), "image/jpeg")
        XCTAssertEqual(header(requests[2], "content-type"), "image/webp")
        XCTAssertEqual(header(requests[2], "content-disposition"), "attachment; filename=\"shot.webp\"")
        for storage in requests[1...2] {
            XCTAssertNil(header(storage, "authorization"), "storage PUTs never carry the account credential")
            XCTAssertNil(header(storage, "cookie"))
        }
        XCTAssertEqual(header(requests[3], "authorization"), "Bearer account-secret")
        XCTAssertEqual(fractions.values.last, 1)
    }

    func testDeclaredPreviewWithoutAPreviewUploadFailsBeforeStorage() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 7, count: 10), filename: "a.png")
        defer { AttachmentStaging.remove(original.url) }
        var prepared = PreparedAttachment(original: original)
        prepared.preview = AttachmentPreviewImage(data: Data(repeating: 1, count: 4), contentType: "image/jpeg")
        AttachmentURLProtocol.handler = { _, _ in
            (201, Data(#"{"id":"Asset0000004","upload":{"method":"PUT","url":"https://r2.invalid/o","headers":{"content-type":"image/png"}}}"#.utf8))
        }
        do {
            _ = try await AttachmentUploader.upload(prepared, channelID: channelID, api: client()) { _ in }
            XCTFail("expected an invalid reservation")
        } catch {
            XCTAssertEqual((error as? APIError)?.status, 502)
        }
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.method == "PUT" })
    }

    func testCompleteGivesUpAfterRepeated409() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 1, count: 10), filename: "a.bin")
        defer { AttachmentStaging.remove(original.url) }
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"):
                return (201, Data(#"{"id":"Asset0000003","upload":{"method":"PUT","url":"https://r2.invalid/o","headers":{"content-type":"application/octet-stream"}}}"#.utf8))
            case ("PUT", _): return (200, Data())
            default: return (409, Data(#"{"error":"not uploaded"}"#.utf8))
            }
        }
        do {
            _ = try await AttachmentUploader.upload(PreparedAttachment(original: original), channelID: channelID, api: client(),
                                                    completeRetryDelays: [.zero, .zero]) { _ in }
            XCTFail("expected 409")
        } catch {
            XCTAssertEqual((error as? APIError)?.status, 409)
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "The upload didn’t finish. Remove it and try again.")
        }
        XCTAssertEqual(AttachmentURLProtocol.requests.filter { $0.url.path.hasSuffix("/complete") }.count, 3, "one attempt plus two retries")
    }

    func testStorageFullAndStorageRefusalErrorsAreExplained() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 1, count: 10), filename: "a.bin")
        defer { AttachmentStaging.remove(original.url) }
        AttachmentURLProtocol.handler = { _, _ in (413, Data(#"{"error":"storage limit reached","code":"storage_full"}"#.utf8)) }
        do {
            _ = try await AttachmentUploader.upload(PreparedAttachment(original: original), channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage_full")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "You’ve used all of your file storage.")
        }
        XCTAssertEqual(AttachmentURLProtocol.requests.count, 1, "nothing is uploaded after a refused reservation")

        AttachmentURLProtocol.reset()
        AttachmentURLProtocol.handler = { request, _ in
            if request.httpMethod == "PUT" { return (403, Data()) }
            return (201, Data(#"{"id":"Asset0000002","upload":{"method":"PUT","url":"https://r2.invalid/o","headers":{"content-type":"application/octet-stream"}}}"#.utf8))
        }
        do {
            _ = try await AttachmentUploader.upload(PreparedAttachment(original: original), channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage refusal")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "Storage refused the upload.")
        }
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.url.path.hasSuffix("/complete") })
        XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(APIError(status: 429, message: "x")), "Uploading too quickly. Try again shortly.")
    }

    private func composerHandler() -> (URLRequest, Data?) throws -> (Int, Data) {
        { request, body in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"):
                return (200, Data(#"{"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\#(channelID)","name":"general"},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case ("GET", "/api/assets/usage"):
                return (200, Data(#"{"used":0,"limit":1000000,"compression":{"imageQuality":90,"imageMaxEdge":4096,"paletteColors":256,"previewEdge":640,"videoMaxHeight":1080,"videoBitrateKbps":6000,"audioBitrateKbps":128}}"#.utf8))
            case ("POST", "/api/assets"):
                return (201, Data(#"{"id":"Asset0000009","kind":"file","upload":{"method":"PUT","url":"https://r2.invalid/original/Asset0000009","headers":{"content-type":"text/plain","content-disposition":"attachment; filename=\"notes.txt\""}}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/Asset0000009/complete"):
                return (200, Data(#"{"id":"Asset0000009","kind":"file","contentType":"text/plain","name":"notes.txt","size":5}"#.utf8))
            case ("POST", "/api/chat/channels/\(channelID)/messages"):
                let input = json(body) ?? [:]
                let response: [String: Any] = [
                    "id": "Message0000001", "channelId": channelID, "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
                    "clientMessageId": input["clientMessageId"] as? String ?? "", "author": ["id": "self", "name": "Me", "isGuest": false],
                    "content": ["version": 1, "type": "text", "text": input["text"] as? String ?? "",
                                "attachments": [["id": "Asset0000009", "kind": "file", "contentType": "text/plain", "name": "notes.txt", "size": 5]]],
                ]
                return (200, try JSONSerialization.data(withJSONObject: response))
            default: throw URLError(.badURL)
            }
        }
    }

    @MainActor
    func testComposerCompressesUploadsThenSendsFileOnlyMessageWithAttachmentIDs() async throws {
        AttachmentURLProtocol.handler = composerHandler()
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        XCTAssertFalse(chat.canAttach, "hidden until usage succeeds")
        await chat.checkUploadAvailability()
        XCTAssertTrue(chat.canAttach)
        XCTAssertEqual(chat.attachmentCompression.imageQuality, 90, "the server's settings are applied")
        XCTAssertFalse(chat.canSubmit, "nothing to send yet")

        let file = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        chat.addAttachments([file])
        XCTAssertEqual(chat.attachmentDrafts.count, 1)
        XCTAssertEqual(chat.attachmentDrafts.first?.statusLabel, "Compressing…")
        await waitUntil("upload") { chat.attachmentDrafts.first?.attachment != nil }
        XCTAssertNil(chat.attachmentDrafts.first?.error)
        XCTAssertEqual(chat.attachmentDrafts.first?.storedSize, 5)
        XCTAssertEqual(chat.attachmentDrafts.first?.attachment?.status, .ready)
        XCTAssertTrue(chat.canSubmit)
        let reserve = try XCTUnwrap(AttachmentURLProtocol.requests.first { $0.method == "POST" && $0.url.path == "/api/assets" })
        XCTAssertEqual(json(reserve.body)?["sourceByteSize"] as? Int, 5)
        XCTAssertNil(json(reserve.body)?["preview"], "files have no preview")

        await chat.send()
        XCTAssertNil(chat.error)
        XCTAssertTrue(chat.attachmentDrafts.isEmpty)
        let send = try XCTUnwrap(AttachmentURLProtocol.requests.last { $0.method == "POST" && $0.url.path == "/api/chat/channels/\(channelID)/messages" })
        let body = try XCTUnwrap(json(send.body))
        XCTAssertEqual(body["attachmentIds"] as? [String], ["Asset0000009"])
        XCTAssertEqual(body["text"] as? String, "")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.id, "Asset0000009")
        await chat.stop()
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.url.path), "staged files are removed when the conversation closes")
    }

    @MainActor
    func testRemovingADraftWhileCompressingNeverUploads() async throws {
        AttachmentURLProtocol.handler = composerHandler()
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        await chat.checkUploadAvailability()
        let file = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        chat.addAttachments([file])
        chat.removeAttachmentDraft(id: try XCTUnwrap(chat.attachmentDrafts.first?.id))
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertTrue(chat.attachmentDrafts.isEmpty)
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.url.path == "/api/assets" }, "nothing is reserved")
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.url.path))
        await chat.stop()
    }

    func testTextOnlySendOmitsAttachmentIDs() async throws {
        AttachmentURLProtocol.handler = { _, body in
            let input = json(body) ?? [:]
            let response: [String: Any] = [
                "id": "Message0000001", "channelId": channelID, "seq": "1", "createdAt": "now",
                "clientMessageId": input["clientMessageId"] as? String ?? "", "author": ["id": "self", "name": "Me", "isGuest": false],
                "content": ["version": 1, "type": "text", "text": "hi"],
            ]
            return (200, try JSONSerialization.data(withJSONObject: response))
        }
        _ = try await client().send(channelID: channelID, sessionToken: "chat", clientMessageID: "c", text: "hi")
        let body = try XCTUnwrap(json(AttachmentURLProtocol.requests.first?.body))
        XCTAssertNil(body["attachmentIds"], "the key is included only when non-empty")
    }

    @MainActor
    func testProcessingFilesRefreshTheirPreviewURLWithoutAFileURL() async throws {
        let history = Data("""
        {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"1","hasMore":false,"messages":[
          {"id":"m1","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
           "content":{"version":1,"type":"text","text":"","attachments":[
             {"id":"Asset0000001","kind":"video","contentType":"video/mp4","name":"v.mp4","size":2048,"preview":{},"status":"processing",
              "previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=1000&sig=old"}
           ]}}
        ]}
        """.utf8)
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history)
            case ("POST", "/api/assets/urls"):
                return (200, Data(#"{"urls":{"Asset0000001":{"previewUrl":"https://cdn.caper.chat/preview/Asset0000001?exp=\#(futureExp)&sig=new"}}}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        chat.requestFreshAttachmentURLs(ids: ["Asset0000001"])
        await waitUntil("fresh preview") { chat.messages.first?.content.attachments?.first?.previewUrl?.contains("sig=new") == true }
        XCTAssertNil(chat.messages.first?.content.attachments?.first?.url, "only ready files have url")
        await chat.stop()
    }
}

private final class FractionLog: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: [Double] = []
    func append(_ value: Double) { lock.withLock { stored.append(value) } }
    var values: [Double] { lock.withLock { stored } }
}
