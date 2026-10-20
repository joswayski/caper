import XCTest
@testable import CaperCore

private final class EmbedTokenStore: TokenStore, @unchecked Sendable {
    var token: String?
    init(_ token: String?) { self.token = token }
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

/// Records every request (method, path, JSON body) and answers from `handler`.
private final class EmbedURLProtocol: URLProtocol, @unchecked Sendable {
    struct Recorded { let method: String; let path: String; let body: Data? }
    private static let lock = NSLock()
    nonisolated(unsafe) static var handler: ((URLRequest) throws -> (Int, Data))?
    nonisolated(unsafe) private static var recorded: [Recorded] = []

    static var requests: [Recorded] { lock.withLock { recorded } }
    static func reset() { lock.withLock { recorded = []; handler = nil } }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let body = Self.body(of: request)
        Self.lock.withLock {
            Self.recorded.append(Recorded(method: request.httpMethod ?? "GET", path: request.url?.path ?? "", body: body))
        }
        do {
            guard let handler = Self.handler else { throw URLError(.badURL) }
            let (status, data) = try handler(request)
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

private let channelID = "Chan12345678"
private let card: [String: Any] = [
    "version": 1, "url": "https://news.example/story",
    "site": ["name": "Example News", "domain": "news.example"],
    "blocks": [["type": "title", "text": "A headline"], ["type": "text", "text": "What happened."]],
]

private func history() -> Data {
    Data("""
    {"space":{"id":"Space1234567","name":"Space"},"channel":{"id":"\(channelID)","name":"general"},"cursor":"1","hasMore":false,"messages":[
      {"id":"Message00000001","channelId":"\(channelID)","seq":"1","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c1",
       "content":{"version":1,"type":"text","text":"see https://news.example/story"}}
    ]}
    """.utf8)
}

/// The API a signed-in account sees; sending fails, so the request is only recorded.
private func answer(_ request: URLRequest) throws -> (Int, Data) {
    switch (request.httpMethod ?? "GET", request.url?.path ?? "") {
    case ("POST", "/api/chat/session"): return (200, Data(#"{"token":"chat","author":{"id":"self","name":"Me","isGuest":false}}"#.utf8))
    case ("GET", "/api/chat/channels/\(channelID)/messages"): return (200, history())
    case ("POST", "/api/embeds/preview"): return (200, try JSONSerialization.data(withJSONObject: ["embed": card]))
    case ("POST", "/api/chat/channels/\(channelID)/messages"): return (503, Data(#"{"error":"Try again."}"#.utf8))
    default: throw URLError(.badURL)
    }
}

private func json(_ data: Data?) -> [String: Any]? {
    data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
}

/// Runs every case in shared/messages/embed-cases.json, as web and the other
/// native clients do, plus the revision rule, buffering and composer preview.
final class LinkEmbedsTests: XCTestCase {
    override func setUp() {
        super.setUp()
        EmbedURLProtocol.reset()
    }

    /// The repository's shared cases, found from this file: Tests/CaperCoreTests
    /// sits five directories below the repository root.
    private func sharedCases() throws -> [[String: Any]] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let url = root.appendingPathComponent("shared/messages/embed-cases.json")
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
        return try XCTUnwrap(object?["cases"] as? [[String: Any]])
    }

    private func decode(_ value: Any) throws -> ChatEmbed? {
        try? JSONDecoder().decode(ChatEmbed.self, from: JSONSerialization.data(withJSONObject: value))
    }

    private func message(_ id: String = "Message00000001", seq: String = "1", revision: Int? = nil, deleted: Bool = false,
                         embeds: [Any]? = nil) throws -> ChatMessage {
        var content: [String: Any] = ["version": 1, "type": "text", "text": deleted ? "" : "see https://news.example/story"]
        if let embeds { content["embeds"] = embeds }
        var value: [String: Any] = ["id": id, "channelId": channelID, "seq": seq, "createdAt": "2026-10-20T10:00:00Z",
                                    "clientMessageId": "c-\(id)", "author": ["id": "u", "name": "U", "isGuest": false] as [String: Any],
                                    "content": content]
        if let revision { value["revision"] = revision }
        if deleted { value["deletedAt"] = "2026-10-20T10:01:00Z" }
        return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: value))
    }

    private func event(_ messageID: String = "Message00000001", revision: Any = 1, embeds: Any = [card]) -> [String: Any] {
        ["type": "message.embeds", "schemaVersion": 1, "channelId": channelID, "messageId": messageID, "revision": revision, "embeds": embeds]
    }

    func testSharedEmbedCases() throws {
        let cases = try sharedCases()
        XCTAssertGreaterThanOrEqual(cases.count, 8, "embed-cases.json lost its cases")
        for item in cases {
            let note = item["note"] as? String ?? ""
            let decoded = try decode(try XCTUnwrap(item["embed"], note))
            guard let expected = item["blocks"] as? [String] else {
                XCTAssertNil(decoded, note)
                continue
            }
            let embed = try XCTUnwrap(decoded, note)
            XCTAssertEqual(embed.blocks.map(\.type), expected, note)
            if let counts = item["imageCounts"] as? [Int] {
                let actual = embed.blocks.compactMap { block -> Int? in
                    guard case .images(let images) = block else { return nil }
                    return images.items.count
                }
                XCTAssertEqual(actual, counts, note)
            }
        }
    }

    func testBlocksKeepOnlySafeFields() throws {
        let cases = try sharedCases()
        let x = try XCTUnwrap(decode(try XCTUnwrap(cases.first?["embed"])))
        guard case .author(let author) = x.blocks[0], case .video(let video) = x.blocks[2], case .footer(let footer) = x.blocks[3] else {
            return XCTFail("unexpected blocks \(x.blocks.map(\.type))")
        }
        XCTAssertEqual(author.handle, "@jack")
        XCTAssertEqual(author.icon, "https://embed.caper.chat/m/sig/i?u=a")
        XCTAssertEqual(video.width, 1280)
        XCTAssertFalse(video.gif)
        XCTAssertEqual(footer.text, "309K likes")
        XCTAssertEqual(x.heading, "jack")

        let youtube = try XCTUnwrap(decode(try XCTUnwrap(cases[1]["embed"])))
        guard case .player(let player) = youtube.blocks[2] else { return XCTFail("no player") }
        XCTAssertEqual(player.provider, .youtube)
        XCTAssertEqual(LinkEmbeds.playURL(player.play)?.absoluteString, "https://embed.caper.chat/play/youtube/dQw4w9WgXcQ")
        XCTAssertEqual(youtube.heading, "Never Gonna Give You Up")

        let generic = try XCTUnwrap(decode(try XCTUnwrap(cases[2]["embed"])))
        XCTAssertEqual(generic.thumbnail?.alt, "lead")
        guard case .video(let gif) = generic.blocks[3] else { return XCTFail("no video") }
        XCTAssertTrue(gif.gif)
    }

    func testMalformedCardsNeverDropTheMessage() throws {
        let badLink: [String: Any] = ["version": 1, "url": "ftp://x.example", "site": ["name": "X", "domain": "x"], "blocks": [["type": "text", "text": "a"]]]
        let badSite: [String: Any] = ["version": 1, "url": "https://x.example", "site": "x", "blocks": [Any]()]
        let broken: [Any] = [card, badLink, "not a card", NSNull(), badSite]
        let decoded = try message(embeds: broken)
        XCTAssertEqual(decoded.content.embeds?.map(\.url), ["https://news.example/story"])
        XCTAssertEqual(decoded.content.text, "see https://news.example/story")

        let raw = #"""
        {"id":"m2","channelId":"Chan12345678","seq":"2","author":{"id":"u","name":"U","isGuest":false},"createdAt":"now","clientMessageId":"c2",
         "content":{"version":1,"type":"text","text":"plain","embeds":"oops","suppressEmbeds":true}}
        """#
        let odd = try JSONDecoder().decode(ChatMessage.self, from: Data(raw.utf8))
        XCTAssertNil(odd.content.embeds, "not an array: ignored")
        XCTAssertEqual(odd.content.suppressEmbeds, true)

        // Stored copies (snapshots) round-trip.
        let again = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(decoded))
        XCTAssertEqual(again.content, decoded.content)
    }

    func testEventParsing() throws {
        let parsed = try XCTUnwrap(MessageEmbedsEvent(event()))
        XCTAssertEqual(parsed.messageId, "Message00000001")
        XCTAssertEqual(parsed.revision, 1)
        XCTAssertEqual(parsed.embeds.count, 1)
        XCTAssertEqual(MessageEmbedsEvent(event(embeds: [card, "bad", ["version": 2]] as [Any]))?.embeds.count, 1, "bad cards are skipped")
        XCTAssertEqual(MessageEmbedsEvent(event(embeds: [Any]()))?.embeds, [], "an empty list removes cards")

        var missing = event()
        missing["revision"] = nil
        XCTAssertNil(MessageEmbedsEvent(missing))
        XCTAssertNil(MessageEmbedsEvent(event(revision: 0)))
        XCTAssertNil(MessageEmbedsEvent(event(revision: "1")))
        XCTAssertNil(MessageEmbedsEvent(event(embeds: "oops")))
        XCTAssertNil(MessageEmbedsEvent(event("")))
        var version = event()
        version["schemaVersion"] = 2
        XCTAssertNil(MessageEmbedsEvent(version))
        var type = event()
        type["type"] = "message.attachments"
        XCTAssertNil(MessageEmbedsEvent(type))
    }

    func testRevisionRule() throws {
        var snapshots = EmbedSnapshots()
        let update = try XCTUnwrap(MessageEmbedsEvent(event()))
        XCTAssertTrue(snapshots.apply(update))
        XCTAssertEqual(snapshots.overlay(try message()).content.embeds?.count, 1, "no revision means revision 1")
        XCTAssertEqual(snapshots.overlay(try message(revision: 1)).content.embeds?.count, 1)
        XCTAssertNil(snapshots.overlay(try message(revision: 2)).content.embeds, "another revision is untouched")
        XCTAssertNil(snapshots.overlay(try message(revision: 1, deleted: true)).content.embeds, "deleted messages get none")

        // A newer revision's cards win; a late older event is ignored.
        XCTAssertTrue(snapshots.apply(try XCTUnwrap(MessageEmbedsEvent(event(revision: 2, embeds: [Any]())))))
        XCTAssertFalse(snapshots.apply(update))
        XCTAssertNil(snapshots.overlay(try message(revision: 2, embeds: [card])).content.embeds, "an empty list removes them")
        XCTAssertNil(snapshots.overlay(try message(revision: 1)).content.embeds)
    }

    func testUnseenEventsAreBufferedAndBounded() throws {
        var snapshots = EmbedSnapshots()
        XCTAssertTrue(snapshots.apply(try XCTUnwrap(MessageEmbedsEvent(event("Later000000001")))))
        for index in 0..<(EmbedSnapshots.maximumEntries - 1) {
            XCTAssertTrue(snapshots.apply(MessageEmbedsEvent(channelId: channelID, messageId: "unseen-\(index)", revision: 1, embeds: [])))
        }
        XCTAssertEqual(snapshots.overlay(try message("Later000000001", seq: "9")).content.embeds?.count, 1, "applied when the message arrives")
        XCTAssertTrue(snapshots.apply(MessageEmbedsEvent(channelId: channelID, messageId: "one-more", revision: 1, embeds: [])))
        XCTAssertNil(snapshots.overlay(try message("Later000000001", seq: "9")).content.embeds, "the oldest entry is dropped first")
    }

    func testPreviewLink() {
        XCTAssertEqual(LinkEmbeds.previewLink(in: "see https://a.example/x and https://b.example"), "https://a.example/x")
        XCTAssertEqual(LinkEmbeds.previewLink(in: "<https://a.example/x> then https://b.example"), "https://b.example")
        XCTAssertNil(LinkEmbeds.previewLink(in: "only <https://a.example/x>"))
        XCTAssertEqual(LinkEmbeds.previewLink(in: "half <https://a.example/x"), "https://a.example/x")
        XCTAssertEqual(LinkEmbeds.previewLink(in: "www.example.com/a."), "https://www.example.com/a")
        XCTAssertNil(LinkEmbeds.previewLink(in: "no links, javascript:alert(1)"))
        XCTAssertNil(LinkEmbeds.previewLink(in: ""))
    }

    func testPlayerPagesLoadOnlyFromCaper() {
        XCTAssertNotNil(LinkEmbeds.playURL("https://embed.caper.chat/play/youtube/abc"))
        XCTAssertNotNil(LinkEmbeds.playURL("https://embed.staging.caper.chat/play/twitch/clip:x"))
        XCTAssertNotNil(LinkEmbeds.playURL("HTTPS://EMBED.CAPER.CHAT/play/vimeo/1"))
        XCTAssertNil(LinkEmbeds.playURL("http://embed.caper.chat/play/youtube/abc"))
        XCTAssertNil(LinkEmbeds.playURL("https://www.youtube.com/embed/abc"))
        XCTAssertNil(LinkEmbeds.playURL("https://embed.caper.chat.evil.example/play/youtube/abc"))
        XCTAssertNil(LinkEmbeds.playURL("https://user:pw@embed.caper.chat/play/youtube/abc"))
        XCTAssertNil(LinkEmbeds.playURL("https://embed.caper.chat:8443/play/youtube/abc"))
        XCTAssertNil(LinkEmbeds.playURL("javascript:alert(1)"))
        XCTAssertNil(LinkEmbeds.playURL(nil))
        XCTAssertNil(LinkEmbeds.openableURL("javascript:alert(1)"))
        XCTAssertNotNil(LinkEmbeds.openableURL("https://x.com/jack"))
    }

    func testFitAndFooter() {
        XCTAssertEqual(LinkEmbeds.fit(width: nil, height: nil).width, 400)
        XCTAssertEqual(LinkEmbeds.fit(width: nil, height: nil).aspectRatio, 16.0 / 9.0, accuracy: 0.0001)
        XCTAssertEqual(LinkEmbeds.fit(width: 1280, height: 720).width, 400)
        XCTAssertEqual(LinkEmbeds.fit(width: 200, height: 100).width, 200, "never enlarged")
        XCTAssertEqual(LinkEmbeds.fit(width: 300, height: 600).width, 150, "tall media keeps to 300 high")

        XCTAssertEqual(LinkEmbeds.footerLine(EmbedFooter(text: "309K likes", timestamp: nil)), "309K likes")
        let both = LinkEmbeds.footerLine(EmbedFooter(text: "309K likes", timestamp: "2006-03-21T20:50:14.000Z"))
        XCTAssertTrue(both?.hasPrefix("309K likes · ") == true)
        XCTAssertTrue(both?.contains("2006") == true)
        XCTAssertEqual(LinkEmbeds.footerLine(EmbedFooter(text: "a", timestamp: "not a date")), "a")
        XCTAssertNil(LinkEmbeds.footerLine(EmbedFooter(text: nil, timestamp: "not a date")))
    }

    // MARK: Chat model

    private func client() -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [EmbedURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration),
                         tokenStore: EmbedTokenStore("account-secret"))
    }

    @MainActor
    func testEmbedsEventsUpdateTheTimeline() async throws {
        EmbedURLProtocol.handler = { try answer($0) }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")
        let generation = chat.eventGeneration
        XCTAssertNil(chat.messages.first?.content.embeds)

        chat.receive(event(), generation: generation, channelID: channelID)
        XCTAssertEqual(chat.messages.first?.content.embeds?.first?.site.name, "Example News")

        // Another revision's cards and other channels' events are ignored.
        chat.receive(event(revision: 2, embeds: [Any]()), generation: generation, channelID: channelID)
        var elsewhere = event(embeds: [Any]())
        elsewhere["channelId"] = "Other1234567"
        chat.receive(elsewhere, generation: generation, channelID: channelID)
        XCTAssertEqual(chat.messages.first?.content.embeds?.count, 1)

        // Malformed frames are dropped, never a reload.
        chat.receive(event(revision: "1"), generation: generation, channelID: channelID)
        chat.receive(["type": "message.embeds", "schemaVersion": 1, "channelId": channelID], generation: generation, channelID: channelID)
        XCTAssertNil(chat.error, "previews never resync")
        XCTAssertEqual(chat.messages.first?.content.embeds?.count, 1)

        // Cards for a message not loaded yet apply when it arrives.
        chat.receive(event("Message00000002"), generation: generation, channelID: channelID)
        let created: [String: Any] = [
            "id": "Message00000002", "channelId": channelID, "seq": "2", "createdAt": "now", "clientMessageId": "c2",
            "author": ["id": "self", "name": "Me", "isGuest": false] as [String: Any],
            "content": ["version": 1, "type": "text", "text": "and https://news.example/story"] as [String: Any],
        ]
        chat.receive(["type": "message.created", "seq": "2", "message": created], generation: generation, channelID: channelID)
        XCTAssertEqual(chat.messages.map(\.id), ["Message00000001", "Message00000002"])
        XCTAssertEqual(chat.messages.last?.content.embeds?.count, 1)
        XCTAssertNil(chat.error)
        await chat.stop()
    }

    @MainActor
    func testComposerPreviewAndRemoval() async throws {
        EmbedURLProtocol.handler = { try answer($0) }
        let chat = ChatModel(api: client())
        await chat.open(channelID: channelID, displayName: "Me")

        chat.draft = "read <https://skip.example> and https://news.example/story"
        XCTAssertEqual(chat.composerPreviewLink, "https://news.example/story")
        await chat.loadComposerLinkPreview()
        XCTAssertEqual(chat.composerLinkPreview?.site.domain, "news.example")
        let preview = try XCTUnwrap(EmbedURLProtocol.requests.last { $0.path == "/api/embeds/preview" })
        XCTAssertEqual(json(preview.body)?["url"] as? String, "https://news.example/story")

        // Removing the card sends without previews.
        chat.removeComposerLinkPreview()
        XCTAssertNil(chat.composerLinkPreview)
        await chat.send()
        let sent = try XCTUnwrap(EmbedURLProtocol.requests.last { $0.method == "POST" && $0.path == "/api/chat/channels/\(channelID)/messages" })
        XCTAssertEqual(json(sent.body)?["suppressEmbeds"] as? Bool, true)

        // A cleared draft starts over: the same link gets its card again.
        chat.draft = ""
        chat.draft = "read https://news.example/story"
        XCTAssertEqual(chat.composerLinkPreview?.site.domain, "news.example")
        await chat.stop()
    }

    func testSendOmitsSuppressEmbedsUnlessSet() async throws {
        EmbedURLProtocol.handler = { _ in (503, Data(#"{"error":"Try again."}"#.utf8)) }
        let api = client()
        _ = try? await api.send(channelID: channelID, sessionToken: "chat", clientMessageID: "a", text: "hi")
        _ = try? await api.send(channelID: channelID, sessionToken: "chat", clientMessageID: "b", text: "hi", suppressEmbeds: true)
        let bodies = EmbedURLProtocol.requests.map { json($0.body) }
        XCTAssertEqual(bodies.count, 2)
        XCTAssertNil(bodies.first??["suppressEmbeds"])
        XCTAssertEqual(bodies.last??["suppressEmbeds"] as? Bool, true)
    }

    func testPreviewOfAnUnusableCardIsNone() async throws {
        EmbedURLProtocol.handler = { _ in (200, Data(#"{"embed":{"version":2,"url":"https://x.example"}}"#.utf8)) }
        let embed = try await client().embedPreview(url: "https://x.example")
        XCTAssertNil(embed)
    }
}
