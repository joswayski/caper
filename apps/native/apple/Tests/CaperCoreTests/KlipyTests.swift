import XCTest
@testable import CaperCore

private final class KlipyMockURLProtocol: URLProtocol, @unchecked Sendable {
    static var handler: ((URLRequest) throws -> (Int, Data))!
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.handler(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                                                                  headerFields: ["content-type": "application/json"])!,
                                cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

private final class KlipyTokenStore: TokenStore, @unchecked Sendable {
    var token: String? = "account-secret"
    func load() throws -> String? { token }
    func save(_ token: String) throws { self.token = token }
    func clear() throws { token = nil }
}

private func bodyData(_ request: URLRequest) -> Data? {
    if let body = request.httpBody { return body }
    guard let stream = request.httpBodyStream else { return nil }
    stream.open()
    defer { stream.close() }
    var body = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    while true {
        let count = stream.read(&buffer, maxLength: buffer.count)
        if count <= 0 { break }
        body.append(buffer, count: count)
    }
    return body
}

/// Runs shared/klipy/cases.json, as the API (apps/api/src/klipy.rs) and the
/// other clients do, plus the request shapes in docs/media.md.
final class KlipyTests: XCTestCase {
    override func tearDown() {
        KlipyMockURLProtocol.handler = nil
        super.tearDown()
    }

    /// The repository's shared cases, found from this file: Tests/CaperCoreTests
    /// sits five directories below the repository root.
    private func cases() throws -> [String: Any] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("shared/klipy/cases.json"))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func json(_ value: Any) throws -> Data {
        try JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed])
    }

    private func assertParse(_ name: String, kind: ChatGif.Kind) throws {
        let parse = try XCTUnwrap(try cases()[name] as? [String: Any])
        XCTAssertEqual(parse["kind"] as? String, kind.rawValue)
        let page = try XCTUnwrap(KlipyParser.parse(response: try XCTUnwrap(parse["response"]), kind: kind))
        // The public entry point reads the same bytes the same way.
        XCTAssertEqual(try KlipyParser.parse(try json(try XCTUnwrap(parse["response"])), kind: kind), page)
        XCTAssertEqual(page.hasNext, parse["hasNext"] as? Bool)
        let expected = try XCTUnwrap(parse["items"] as? [[String: Any]])
        XCTAssertEqual(page.items.count, expected.count, name)
        for (index, (item, want)) in zip(page.items, expected).enumerated() {
            switch item {
            case let .ad(ad):
                XCTAssertEqual(want["type"] as? String, "ad", "\(name) #\(index)")
                XCTAssertEqual(ad.html, want["html"] as? String)
                XCTAssertEqual(ad.width, want["width"] as? Double)
                XCTAssertEqual(ad.height, want["height"] as? Double)
            case let .item(result):
                XCTAssertEqual(want["type"] as? String, "item", "\(name) #\(index)")
                XCTAssertEqual(result.slug, want["slug"] as? String)
                XCTAssertEqual(result.title, want["title"] as? String)
                XCTAssertEqual(result.tile, try JSONDecoder().decode(KlipyTile.self, from: json(try XCTUnwrap(want["tile"]))), "\(name) #\(index) tile")
                XCTAssertEqual(result.gif, try JSONDecoder().decode(ChatGif.self, from: json(try XCTUnwrap(want["gif"]))), "\(name) #\(index) gif")
                XCTAssertTrue(result.gif.isValid)
            }
        }
    }

    func testSharedParseCase() throws { try assertParse("parse", kind: .gif) }

    func testSharedStickerParseCase() throws { try assertParse("parseSticker", kind: .sticker) }

    func testSharedMessageRules() throws {
        let messages = try XCTUnwrap(try cases()["messages"] as? [String: Any])
        let valid = try XCTUnwrap(messages["valid"] as? [Any]), invalid = try XCTUnwrap(messages["invalid"] as? [Any])
        XCTAssertGreaterThan(valid.count, 2); XCTAssertGreaterThan(invalid.count, 10)
        for (index, value) in valid.enumerated() {
            let gif = try JSONDecoder().decode(ChatGif.self, from: json(value))
            XCTAssertTrue(gif.isValid, "valid #\(index)")
            // Encoding keeps KLIPY's URLs exactly and omits absent ones.
            let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(gif)) as? NSDictionary)
            XCTAssertEqual(encoded, value as? NSDictionary, "valid #\(index)")
        }
        for (index, value) in invalid.enumerated() {
            XCTAssertThrowsError(try JSONDecoder().decode(ChatGif.self, from: json(value)), "invalid #\(index)")
        }
    }

    func testSharedURLRules() throws {
        let urls = try XCTUnwrap(try cases()["urls"] as? [[String: Any]])
        XCTAssertGreaterThan(urls.count, 8)
        for item in urls {
            let url = try XCTUnwrap(item["url"] as? String)
            XCTAssertEqual(KlipyRules.mediaURL(url), item["valid"] as? Bool, url)
        }
    }

    /// An unusable GIF never fails its message or history page: the text shows alone.
    func testInvalidGifLeavesTheMessage() throws {
        let messages = try XCTUnwrap(try cases()["messages"] as? [String: Any])
        let valid = try XCTUnwrap((messages["valid"] as? [Any])?.first)
        let samples = (try XCTUnwrap(messages["invalid"] as? [Any])).map { ($0, false) } + [(valid, true)]
        for (index, (gif, usable)) in samples.enumerated() {
            let content: [String: Any] = ["version": 1, "type": "text", "text": "caption", "gif": gif]
            let decoded = try JSONDecoder().decode(ChatContent.self, from: json(content))
            XCTAssertEqual(decoded.text, "caption")
            XCTAssertEqual(decoded.gif != nil, usable, "sample #\(index)")
        }
        let plain = try JSONDecoder().decode(ChatContent.self, from: Data(#"{"version":1,"type":"text","text":"hi"}"#.utf8))
        XCTAssertNil(plain.gif)
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(plain), as: UTF8.self).contains("gif"))
    }

    func testCaptionlessGifIsAValidMessage() throws {
        XCTAssertEqual(MessageValidation.error(for: ""), "Write a message first.")
        XCTAssertNil(MessageValidation.error(for: "", hasGif: true))
        let gif = ChatGif(kind: .sticker, slug: "wave", title: "", width: 10, height: 10, webp: "https://static.klipy.com/a.webp")
        let message = ChatMessage(id: "m1", channelId: "Aaaaaaaaaaaa", seq: "1", author: ChatAuthor(id: "u", name: "U", isGuest: false),
                                  content: ChatContent(version: 1, type: "text", text: "", gif: gif), createdAt: "now", clientMessageId: "c1")
        XCTAssertEqual(MessageReply(message: message).text, "Sticker")
        XCTAssertEqual(gif.accessibilityLabel, "Sticker")
        XCTAssertEqual(ChatGif(kind: .gif, slug: "a", title: "Hello", width: 1, height: 1, gif: "https://klipy.com/a.gif").accessibilityLabel, "Hello")
    }

    func testRetriesKeepTheGif() {
        let gif = ChatGif(kind: .gif, slug: "hi", title: "Hi", width: 2, height: 1, gif: "https://static.klipy.com/a.gif")
        var delivery = ChatDeliveryState()
        let command = delivery.begin(text: "", gif: gif)
        XCTAssertEqual(command.gif, gif)
        XCTAssertEqual(delivery.begin(text: "other").id, command.id)
        XCTAssertEqual(delivery.replaceAttachments(id: command.id, attachments: [])?.gif, gif)
    }

    func testAdsTakeFullRowsInKlipyOrder() {
        let gif = ChatGif(kind: .gif, slug: "a", title: "", width: 1, height: 1, gif: "https://klipy.com/a.gif")
        let tile = KlipyTile(url: "https://klipy.com/a.webp", still: nil, width: 1, height: 1)
        func item(_ slug: String) -> KlipyItem { .item(KlipyResult(slug: slug, title: "", tile: tile, gif: gif)) }
        let ad = KlipyAd(html: "<p>ad</p>", width: nil, height: nil)
        let rows = KlipyGridRow.rows([item("1"), item("2"), item("3"), .ad(ad), item("4"), item("5")], columns: 2)
        XCTAssertEqual(rows.map(\.id), [0, 2, 3, 4])
        guard case let .tiles(_, first) = rows[0], case let .tiles(_, partial) = rows[1], case .ad = rows[2], case let .tiles(_, last) = rows[3] else {
            return XCTFail("unexpected rows \(rows)")
        }
        XCTAssertEqual(first.map(\.slug), ["1", "2"])
        XCTAssertEqual(partial.map(\.slug), ["3"])
        XCTAssertEqual(last.map(\.slug), ["4", "5"])
    }

    func testListRequests() throws {
        let client = KlipyClient(configuration: KlipyConfiguration(appKey: "fixtureKlipyKey", customerId: "0123456789abcdef0123456789abcdef"))
        let trending = try XCTUnwrap(client.listURL(kind: .gif, query: "", page: 1, locale: "us"))
        XCTAssertEqual(trending.absoluteString,
                       "https://api.klipy.com/api/v1/fixtureKlipyKey/gifs/trending?page=1&per_page=24&customer_id=0123456789abcdef0123456789abcdef&content_filter=off&locale=us")
        let search = try XCTUnwrap(client.listURL(kind: .sticker, query: "cats & dogs+", page: 3, locale: nil))
        XCTAssertEqual(search.absoluteString,
                       "https://api.klipy.com/api/v1/fixtureKlipyKey/stickers/search?q=cats%20%26%20dogs%2B&page=3&per_page=24&customer_id=0123456789abcdef0123456789abcdef&content_filter=off")
        let ads = KlipyAdRequest(maxWidth: 351, osVersion: "18.1", model: "iPhone16,2", deviceWidth: 393, deviceHeight: 852, pixelRatio: 3,
                                 language: "en", userAgent: "Mozilla/5.0 (iPhone)")
        let withAds = try XCTUnwrap(client.listURL(kind: .gif, query: "", page: 1, locale: "gb", ads: ads))
        XCTAssertTrue(withAds.absoluteString.hasSuffix(
            "&locale=gb&ad-min-width=50&ad-max-width=351&ad-min-height=50&ad-max-height=250&ad-os=ios&ad-osv=18.1&ad-make=apple"
                + "&ad-model=iPhone16%2C2&ad-device-w=393&ad-device-h=852&ad-pxratio=3&ad-language=en"), withAds.absoluteString)
    }

    func testShareRequest() throws {
        let client = KlipyClient(configuration: KlipyConfiguration(appKey: "fixtureKlipyKey", customerId: "0123456789abcdef0123456789abcdef"))
        XCTAssertNil(client.shareRequest(kind: .gif, slug: "../etc", query: ""))
        let request = try XCTUnwrap(client.shareRequest(kind: .sticker, slug: "grrr-rawr-sticker", query: "cat", userAgent: "UA"))
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.url?.absoluteString, "https://api.klipy.com/api/v1/fixtureKlipyKey/stickers/share/grrr-rawr-sticker")
        XCTAssertEqual(request.value(forHTTPHeaderField: "user-agent"), "UA")
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: try XCTUnwrap(request.httpBody)) as? [String: String])
        XCTAssertEqual(body, ["customer_id": "0123456789abcdef0123456789abcdef", "q": "cat"])
        // An unusable key from the API never reaches KLIPY.
        XCTAssertNil(KlipyClient(configuration: KlipyConfiguration(appKey: "bad/key", customerId: "x")).shareRequest(kind: .gif, slug: "a", query: ""))
    }

    private func api() -> APIClient {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [KlipyMockURLProtocol.self]
        return APIClient(baseURL: URL(string: "https://caper.invalid")!, session: URLSession(configuration: configuration), tokenStore: KlipyTokenStore())
    }

    func testConfigurationAndGifSend() async throws {
        let gif = ChatGif(kind: .gif, slug: "hello-hi-662", title: "Hello", width: 498, height: 498,
                          gif: "https://static.klipy.com/a.gif", still: "https://static.klipy.com/a.jpg")
        KlipyMockURLProtocol.handler = { request in
            switch (request.httpMethod, request.url?.path) {
            case ("GET", "/api/klipy"):
                XCTAssertEqual(request.value(forHTTPHeaderField: "authorization"), "Bearer account-secret")
                return (200, Data(#"{"appKey":"fixtureKlipyKey","customerId":"0123456789abcdef0123456789abcdef"}"#.utf8))
            case ("POST", "/api/chat/channels/Aaaaaaaaaaaa/messages"):
                let body = try XCTUnwrap(JSONSerialization.jsonObject(with: try XCTUnwrap(bodyData(request))) as? [String: Any])
                XCTAssertEqual(body["text"] as? String, "")
                XCTAssertNil(body["attachmentIds"])
                let sent = try XCTUnwrap(body["gif"] as? [String: Any])
                XCTAssertEqual(sent["provider"] as? String, "klipy")
                XCTAssertEqual(sent["slug"] as? String, "hello-hi-662")
                XCTAssertNil(sent["webp"])
                let content = try JSONSerialization.data(withJSONObject: ["version": 1, "type": "text", "text": "", "gif": sent])
                return (200, Data("""
                {"id":"m1","channelId":"Aaaaaaaaaaaa","seq":"1","author":{"id":"u","name":"U","isGuest":false},"content":\(String(decoding: content, as: UTF8.self)),"createdAt":"now","clientMessageId":"c1"}
                """.utf8))
            default: throw URLError(.badURL)
            }
        }
        let api = api()
        let configuration = try await api.klipyConfiguration()
        XCTAssertEqual(configuration, KlipyConfiguration(appKey: "fixtureKlipyKey", customerId: "0123456789abcdef0123456789abcdef"))
        XCTAssertTrue(configuration.isValid)
        let message = try await api.send(channelID: "Aaaaaaaaaaaa", sessionToken: "chat", clientMessageID: "c1", text: "", gif: gif)
        XCTAssertEqual(message.content.gif, gif)
        // Never alongside files.
        do {
            _ = try await api.send(channelID: "Aaaaaaaaaaaa", sessionToken: "chat", clientMessageID: "c2", text: "", attachmentIDs: ["a"], gif: gif)
            XCTFail("sent a GIF with files")
        } catch let error as APIError { XCTAssertEqual(error.status, 400) }
    }
}
