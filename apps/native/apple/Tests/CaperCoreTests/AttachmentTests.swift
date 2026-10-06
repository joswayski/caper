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

        let usage = try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":5,"limit":10,"maxUploadBytes":2147483648}"#.utf8))
        XCTAssertEqual(usage.maxUploadBytes, 2_147_483_648)
        XCTAssertNil(try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":0,"limit":10}"#.utf8)).maxUploadBytes)
        XCTAssertNil(try JSONDecoder().decode(AssetUsage.self, from: Data(#"{"used":0,"limit":10,"compression":{"imageQuality":80}}"#.utf8)).maxUploadBytes,
                     "retired compression settings are ignored")
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
        XCTAssertEqual(AttachmentStaging.contentType(forFilename: "IMG_0001.HEIC"), "image/heic", "HEIC uploads as-is; the server converts it")
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

    func testDisplaySizeAndUploadLimit() {
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 1200, height: 600)! == (360, 180))
        XCTAssertTrue(AttachmentPolicy.displaySize(width: 600, height: 1200)! == (150, 300))
        XCTAssertNil(AttachmentPolicy.displaySize(width: nil, height: 10))
        XCTAssertNil(AttachmentPolicy.tooLargeMessage(size: 10, maxUploadBytes: nil))
        XCTAssertNil(AttachmentPolicy.tooLargeMessage(size: 10, maxUploadBytes: 10))
        XCTAssertEqual(AttachmentPolicy.tooLargeMessage(size: 11, maxUploadBytes: 2_147_483_648),
                       nil)
        XCTAssertEqual(AttachmentPolicy.tooLargeMessage(size: 2_147_483_649, maxUploadBytes: 2_147_483_648),
                       "This file is too large to upload (max 2.0 GB).")
    }

    func testFormatBytesAndDraftLabels() {
        XCTAssertEqual(AttachmentPolicy.formatBytes(512), "512 B")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1536), "1.5 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(146_432), "143 KB")
        XCTAssertEqual(AttachmentPolicy.formatBytes(1_677_722), "1.6 MB")
        var draft = AttachmentDraft(id: "d", name: "IMG.heic", kind: .image, localURL: URL(fileURLWithPath: "/tmp/x"), size: 1_677_722)
        draft.progress = 0.42
        XCTAssertEqual(draft.statusLabel, "Uploading… 42%")
        draft.attachment = ChatAttachment(id: "A", kind: .image, contentType: "image/heic", name: "IMG.heic", size: 1_677_722, status: .processing)
        XCTAssertEqual(draft.statusLabel, "1.6 MB", "the original's size; the server compresses after sending")
        draft.error = "You’ve used all of your file storage."
        XCTAssertEqual(draft.statusLabel, "You’ve used all of your file storage.")
    }

    func testFileOnlyMessagesValidate() {
        XCTAssertEqual(MessageValidation.error(for: "  "), "Write a message first.")
        XCTAssertNil(MessageValidation.error(for: "", attachmentCount: 1))
        XCTAssertNotNil(MessageValidation.error(for: "", attachmentCount: 11))
    }

    // MARK: Upload sequence

    private func header(_ request: AttachmentURLProtocol.Recorded, _ name: String) -> String? {
        request.headers.first { $0.key.lowercased() == name }?.value
    }

    func testUploadReservesPutsOriginalWithExactHeadersThenCompletesAfter409Retries() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 7, count: 2048), filename: "IMG_0001.HEIC")
        defer { AttachmentStaging.remove(original.url) }
        let completes = FractionLog()
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.absoluteString ?? "") {
            case ("POST", "https://caper.invalid/api/assets"):
                return (201, Data(#"{"id":"Asset0000001","kind":"file","upload":{"method":"PUT","url":"https://incoming.invalid/incoming/Asset0000001?X-Amz-Signature=o","headers":{"content-type":"image/heic"}},"storage":{"used":2048,"limit":10000}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "https://caper.invalid/api/assets/Asset0000001/complete"):
                completes.append(1)
                // The storage notification has not arrived yet for the first two attempts.
                if completes.values.count < 3 { return (409, Data(#"{"error":"upload not found yet"}"#.utf8)) }
                return (200, Data(#"{"id":"Asset0000001","kind":"file","contentType":"image/heic","name":"IMG_0001.HEIC","size":2048,"status":"processing"}"#.utf8))
            default: throw URLError(.badURL)
            }
        }
        let fractions = FractionLog()
        let attachment = try await AttachmentUploader.upload(original, channelID: channelID, api: client(),
                                                             completeRetryDelays: [.zero, .zero, .zero]) { fractions.append($0) }
        XCTAssertEqual(attachment.id, "Asset0000001")
        XCTAssertEqual(attachment.status, .processing)
        XCTAssertNil(attachment.url)

        let requests = AttachmentURLProtocol.requests
        XCTAssertEqual(requests.map { "\($0.method) \($0.url.host ?? "")\($0.url.path)" }, [
            "POST caper.invalid/api/assets",
            "PUT incoming.invalid/incoming/Asset0000001",
            "POST caper.invalid/api/assets/Asset0000001/complete",
            "POST caper.invalid/api/assets/Asset0000001/complete",
            "POST caper.invalid/api/assets/Asset0000001/complete",
        ])
        let body = try XCTUnwrap(json(requests[0].body))
        XCTAssertEqual(Set(body.keys), ["channelId", "filename", "contentType", "byteSize"], "only the original is described")
        XCTAssertEqual(body["channelId"] as? String, channelID)
        XCTAssertEqual(body["filename"] as? String, "IMG_0001.HEIC")
        XCTAssertEqual(body["contentType"] as? String, "image/heic")
        XCTAssertEqual(body["byteSize"] as? Int, 2048)
        XCTAssertEqual(header(requests[0], "authorization"), "Bearer account-secret")
        XCTAssertEqual(header(requests[1], "content-type"), "image/heic")
        XCTAssertNil(header(requests[1], "authorization"), "storage PUTs never carry the account credential")
        XCTAssertNil(header(requests[1], "cookie"))
        XCTAssertEqual(header(requests[2], "authorization"), "Bearer account-secret")
        XCTAssertEqual(fractions.values.last, 1)
    }

    func testCompleteGivesUpAfterRepeated409() async throws {
        let original = try AttachmentStaging.stage(data: Data(repeating: 1, count: 10), filename: "a.bin")
        defer { AttachmentStaging.remove(original.url) }
        AttachmentURLProtocol.handler = { request, _ in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/assets"):
                return (201, Data(#"{"id":"Asset0000003","upload":{"method":"PUT","url":"https://incoming.invalid/o","headers":{"content-type":"application/octet-stream"}}}"#.utf8))
            case ("PUT", _): return (200, Data())
            default: return (409, Data(#"{"error":"not uploaded"}"#.utf8))
            }
        }
        do {
            _ = try await AttachmentUploader.upload(original, channelID: channelID, api: client(), completeRetryDelays: [.zero, .zero]) { _ in }
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
            _ = try await AttachmentUploader.upload(original, channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage_full")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "You’ve used all of your file storage.")
        }
        XCTAssertEqual(AttachmentURLProtocol.requests.count, 1, "nothing is uploaded after a refused reservation")

        AttachmentURLProtocol.reset()
        AttachmentURLProtocol.handler = { request, _ in
            if request.httpMethod == "PUT" { return (403, Data()) }
            return (201, Data(#"{"id":"Asset0000002","upload":{"method":"PUT","url":"https://incoming.invalid/o","headers":{"content-type":"application/octet-stream"}}}"#.utf8))
        }
        do {
            _ = try await AttachmentUploader.upload(original, channelID: channelID, api: client()) { _ in }
            XCTFail("expected storage refusal")
        } catch {
            XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(error), "Storage refused the upload.")
        }
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.url.path.hasSuffix("/complete") })
        XCTAssertEqual(AttachmentPolicy.uploadErrorMessage(APIError(status: 429, message: "x")), "Uploading too quickly. Try again shortly.")
    }

    private func composerHandler(maxUploadBytes: Int = 1_000_000) -> (URLRequest, Data?) throws -> (Int, Data) {
        { request, body in
            switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
            case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
            case ("GET", "/api/chat/channels/\(channelID)/messages"):
                return (200, Data(#"{"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\#(channelID)","name":"general"},"messages":[],"cursor":"0","hasMore":false}"#.utf8))
            case ("GET", "/api/assets/usage"):
                return (200, Data(#"{"used":0,"limit":1000000,"maxUploadBytes":\#(maxUploadBytes)}"#.utf8))
            case ("POST", "/api/assets"):
                return (201, Data(#"{"id":"Asset0000009","kind":"file","upload":{"method":"PUT","url":"https://incoming.invalid/incoming/Asset0000009","headers":{"content-type":"text/plain"}}}"#.utf8))
            case ("PUT", _): return (200, Data())
            case ("POST", "/api/assets/Asset0000009/complete"):
                return (200, Data(#"{"id":"Asset0000009","kind":"file","contentType":"text/plain","name":"notes.txt","size":5,"status":"processing"}"#.utf8))
            case ("POST", "/api/chat/channels/\(channelID)/messages"):
                let input = json(body) ?? [:]
                let response: [String: Any] = [
                    "id": "Message0000001", "channelId": channelID, "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
                    "clientMessageId": input["clientMessageId"] as? String ?? "", "author": ["id": "self", "name": "Me", "isGuest": false],
                    "content": ["version": 1, "type": "text", "text": input["text"] as? String ?? "",
                                "attachments": [["id": "Asset0000009", "kind": "file", "contentType": "text/plain", "name": "notes.txt", "size": 5,
                                                 "status": "processing"]]],
                ]
                return (200, try JSONSerialization.data(withJSONObject: response))
            default: throw URLError(.badURL)
            }
        }
    }

    @MainActor
    func testComposerUploadsThenSendsFileOnlyMessageWithAttachmentIDs() async throws {
        AttachmentURLProtocol.handler = composerHandler()
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        XCTAssertFalse(chat.canAttach, "hidden until usage succeeds")
        await chat.checkUploadAvailability()
        XCTAssertTrue(chat.canAttach)
        XCTAssertEqual(chat.maxUploadBytes, 1_000_000)
        XCTAssertFalse(chat.canSubmit, "nothing to send yet")

        let file = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        chat.addAttachments([file])
        XCTAssertEqual(chat.attachmentDrafts.count, 1)
        await waitUntil("upload") { chat.attachmentDrafts.first?.attachment != nil }
        XCTAssertNil(chat.attachmentDrafts.first?.error)
        XCTAssertEqual(chat.attachmentDrafts.first?.attachment?.status, .processing)
        XCTAssertTrue(chat.canSubmit, "sending does not wait for server processing")

        await chat.send()
        XCTAssertNil(chat.error)
        XCTAssertTrue(chat.attachmentDrafts.isEmpty)
        let send = try XCTUnwrap(AttachmentURLProtocol.requests.last { $0.method == "POST" && $0.url.path == "/api/chat/channels/\(channelID)/messages" })
        let body = try XCTUnwrap(json(send.body))
        XCTAssertEqual(body["attachmentIds"] as? [String], ["Asset0000009"])
        XCTAssertEqual(body["text"] as? String, "")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.id, "Asset0000009")
        XCTAssertEqual(chat.messages.first?.content.attachments?.first?.status, .processing)
        XCTAssertEqual(chat.localCopy(for: "Asset0000009"), file.url, "the sender keeps its original while the server processes")
        await chat.stop()
        XCTAssertNil(chat.localCopy(for: "Asset0000009"))
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.url.path), "staged originals are removed when the conversation closes")
    }

    @MainActor
    func testOversizedOriginalIsRefusedBeforeReserving() async throws {
        AttachmentURLProtocol.handler = composerHandler(maxUploadBytes: 4)
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        await chat.checkUploadAvailability()
        let file = try AttachmentStaging.stage(data: Data("hello".utf8), filename: "notes.txt")
        chat.addAttachments([file])
        XCTAssertEqual(chat.attachmentDrafts.first?.error, "This file is too large to upload (max 4 B).")
        XCTAssertFalse(chat.canSubmit)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(AttachmentURLProtocol.requests.contains { $0.url.path == "/api/assets" }, "nothing is reserved")
        chat.removeAttachmentDraft(id: try XCTUnwrap(chat.attachmentDrafts.first?.id))
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
