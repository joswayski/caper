import XCTest
@testable import CaperCore

/// A terminal transport, not a provider mock: anything the fixture declines
/// reaches this unchanged. No test can accidentally contact KLIPY or an API.
private final class KlipyFixturePassthrough: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let response = HTTPURLResponse(url: request.url!, statusCode: 299, httpVersion: nil, headerFields: [
            "X-Test-Authorization": request.value(forHTTPHeaderField: "Authorization") ?? "",
            "X-Test-Chat-Token": request.value(forHTTPHeaderField: "X-Caper-Chat-Token") ?? "",
        ])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("PASSTHROUGH".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class KlipyFixtureTransportTests: XCTestCase {
    private let environment = [
        "CAPER_TEST_MODE": "parity", "CAPER_UI_FIXTURE": "klipy", "CAPER_API_BASE_URL": "http://127.0.0.1:3001",
    ]
    private let provider = KlipyClient(configuration: KlipyConfiguration(
        appKey: "fixtureKlipyKey", customerId: "0123456789abcdef0123456789abcdef"
    ))

    private func configuration(_ environment: [String: String]) -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [KlipyFixturePassthrough.self]
        configuration.urlCache = nil
        KlipyFixtureTransport.install(into: configuration, environment: environment)
        return configuration
    }

    func testProviderResultsDecodeThroughTheRealTileAndViewerLoader() async throws {
        let configuration = configuration(environment)
        // Installing twice is harmless (the image loader installs it too).
        KlipyFixtureTransport.install(into: configuration, environment: environment)
        let session = URLSession(configuration: configuration, delegate: SecureSessionDelegate(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let loader = AttachmentImageLoader(configuration: configuration)
        for kind in [ChatGif.Kind.gif, .sticker] {
            let url = try XCTUnwrap(provider.listURL(kind: kind, query: "", page: 1))
            let (data, response) = try await session.data(from: url)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            XCTAssertEqual(response.mimeType, "application/json")
            let page = try KlipyParser.parse(data, kind: kind)
            XCTAssertFalse(page.hasNext)
            XCTAssertEqual(page.items.count, 1)
            guard case let .item(result) = try XCTUnwrap(page.items.first) else { return XCTFail("Expected a fixture tile") }
            XCTAssertEqual(result.slug, "caper-fixture-\(kind.rawValue)")
            XCTAssertEqual(result.title, "TEST FIXTURE — \(kind.label)")
            XCTAssertEqual(result.gif.kind, kind)
            XCTAssertTrue(result.gif.isValid, "No exception to the shared ChatGif rules")
            XCTAssertEqual(result.gif.width, kind == .gif ? 96 : 64)
            XCTAssertEqual(result.gif.height, kind == .gif ? 64 : 96)
            XCTAssertEqual(result.tile.width, result.gif.width)
            XCTAssertEqual(result.tile.height, result.gif.height)
            let base = "https://static.klipy.com/caper-test-fixtures/klipy/v1/\(result.slug)"
            XCTAssertEqual(result.gif.gif, base + ".gif")
            XCTAssertEqual(result.gif.still, base + ".png")

            // Persisted content.gif uses the ordinary decoder, and the viewer
            // keeps the same provider URL instead of refreshing Caper signatures.
            let decoded = try JSONDecoder().decode(ChatGif.self, from: JSONEncoder().encode(result.gif))
            XCTAssertEqual(decoded, result.gif)
            let viewer = try XCTUnwrap(decoded.mediaViewerAttachment)
            XCTAssertEqual(viewer.url, decoded.gif)
            XCTAssertEqual(viewer.contentType, "image/gif")
            let tile = try await loader.load(key: "tile:\(result.slug)", url: XCTUnwrap(URL(string: result.tile.url)), maxPixelSize: 96)
            let poster = try await loader.load(key: "poster:\(result.slug)", url: XCTUnwrap(URL(string: XCTUnwrap(decoded.still))), maxPixelSize: 96)
            let frames = try await loader.loadFrames(key: "viewer:\(result.slug)", url: XCTUnwrap(URL(string: XCTUnwrap(viewer.url))), maxPixelSize: 96)
            XCTAssertEqual(tile.width, decoded.width)
            XCTAssertEqual(tile.height, decoded.height)
            XCTAssertEqual(poster.width, decoded.width)
            XCTAssertEqual(poster.height, decoded.height)
            XCTAssertEqual(frames.images.count, 2)
            XCTAssertEqual(frames.durations, [0.35, 0.65])
            let first = try XCTUnwrap(frames.images.first)
            let last = try XCTUnwrap(frames.images.last)
            XCTAssertEqual(first.width, decoded.width)
            XCTAssertEqual(first.height, decoded.height)
            let firstPixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(first)).straight
            let lastPixels = try XCTUnwrap(AttachmentPreparer.rgbaPixels(last)).straight
            XCTAssertNotEqual(firstPixels, lastPixels, "The animation must actually change pixels")
            XCTAssertEqual(firstPixels[3], kind == .sticker ? 0 : 255, "Sticker corners remain transparent")
        }
    }

    func testSearchEmptyFailureAndShareAreHTTPResponses() async throws {
        let session = URLSession(configuration: configuration(environment))
        defer { session.invalidateAndCancel() }
        for kind in [ChatGif.Kind.gif, .sticker] {
            for (query, pageNumber, status, count) in [("cat & dog+wave", 1, 200, 1), ("empty", 1, 200, 0), ("fail", 1, 503, 0), ("", 2, 200, 0)] {
                let url = try XCTUnwrap(provider.listURL(kind: kind, query: query, page: pageNumber))
                let (data, response) = try await session.data(from: url)
                XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, status)
                XCTAssertEqual((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Cache-Control"), "no-store")
                if status == 200 {
                    let page = try KlipyParser.parse(data, kind: kind)
                    XCTAssertEqual(page.items.count, count)
                    XCTAssertFalse(page.hasNext)
                } else {
                    XCTAssertTrue(String(decoding: data, as: UTF8.self).contains("TEST FIXTURE"))
                }
            }
            let share = try XCTUnwrap(provider.shareRequest(kind: kind, slug: "caper-fixture-\(kind.rawValue)", query: "cat"))
            XCTAssertNil(share.value(forHTTPHeaderField: "Authorization"))
            let (data, response) = try await session.data(for: share)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            XCTAssertEqual((try JSONSerialization.jsonObject(with: data) as? [String: Bool])?["result"], true)
        }
    }

    func testInstallationRequiresAllThreeFlagsAndACredentialFreeLoopbackOrigin() async throws {
        let mutations: [(String, String?)] = [
            ("CAPER_TEST_MODE", nil), ("CAPER_TEST_MODE", "production"),
            ("CAPER_UI_FIXTURE", nil), ("CAPER_UI_FIXTURE", "whats-new"),
            ("CAPER_API_BASE_URL", nil), ("CAPER_API_BASE_URL", "not a URL"),
        ] + [
            "https://caper.chat", "http://127.0.0.2:3001", "http://localhost.evil.invalid", "//localhost:3001",
            "file://localhost/", "ws://127.0.0.1:3001", "http://user@localhost:3001",
            "http://user:password@127.0.0.1:3001", "http://@localhost", "http://localhost:0", "http://localhost:65536",
            "http://localhost:3001/api", "http://localhost:3001?fixture=klipy", "http://localhost:3001#fixture",
        ].map { ("CAPER_API_BASE_URL", Optional($0)) }
        let url = try XCTUnwrap(provider.listURL(kind: .gif, query: "", page: 1))
        for (key, value) in mutations {
            var disabled = environment
            disabled[key] = value
            let session = URLSession(configuration: configuration(disabled))
            defer { session.invalidateAndCancel() }
            let (data, response) = try await session.data(from: url)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 299, "\(key)=\(value ?? "missing")")
            XCTAssertEqual(String(decoding: data, as: UTF8.self), "PASSTHROUGH")
        }
        for origin in ["http://127.0.0.1:3001", "https://localhost/", "http://[::1]:3001", "https://[::1]/"] {
            var enabled = environment
            enabled["CAPER_API_BASE_URL"] = origin
            let session = URLSession(configuration: configuration(enabled))
            defer { session.invalidateAndCancel() }
            let (_, response) = try await session.data(from: url)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200, origin)
        }
    }

    func testOtherKeysHostsPathsAndAccountTrafficPassThroughUnchanged() async throws {
        let session = URLSession(configuration: configuration(environment))
        defer { session.invalidateAndCancel() }
        let api = "https://api.klipy.com/api/v1/"
        let fixture = api + "fixtureKlipyKey/gifs/"
        let media = "https://static.klipy.com/caper-test-fixtures/klipy/v1/caper-fixture-gif.gif"
        let excluded = [
            api + "realProviderKey/gifs/trending", api + "fixtureKlipyKeyExtra/gifs/trending",
            api + "fixtureKlipyKey/clips/trending", fixture + "reports", fixture + "trending/",
            fixture + "%74rending", fixture + "trending#fragment",
            fixture.replacingOccurrences(of: "https:", with: "http:") + "trending",
            fixture.replacingOccurrences(of: "api.klipy.com", with: "api.klipy.com.evil.invalid") + "trending",
            fixture.replacingOccurrences(of: "api.klipy.com", with: "other.klipy.com") + "trending",
            fixture.replacingOccurrences(of: "api.klipy.com", with: "user@api.klipy.com") + "trending",
            fixture.replacingOccurrences(of: "api.klipy.com", with: "api.klipy.com:443") + "trending",
            "https://static.klipy.com/ii/real.gif", media + "?other=1", media + "#fragment", media + ".extra",
            media.replacingOccurrences(of: "v1", with: "v2"),
            media.replacingOccurrences(of: "https:", with: "http:"),
            media.replacingOccurrences(of: "static.klipy.com", with: "static2.klipy.com"),
            media.replacingOccurrences(of: "static.klipy.com", with: "user@static.klipy.com"),
            media.replacingOccurrences(of: "static.klipy.com", with: "static.klipy.com:443"),
            "http://127.0.0.1:3001/api/klipy", "http://127.0.0.1:3001/api/chat/channels/test/messages",
        ]
        for raw in excluded {
            let (_, response) = try await session.data(from: XCTUnwrap(URL(string: raw)))
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 299, raw)
        }
        for (method, raw) in [("POST", fixture + "trending"), ("DELETE", fixture + "search?q=cat"),
                              ("GET", fixture + "share/caper-fixture-gif"), ("POST", fixture + "share/bad/slug"), ("POST", media)] {
            var request = URLRequest(url: try XCTUnwrap(URL(string: raw)))
            request.httpMethod = method
            let (_, response) = try await session.data(for: request)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 299, "\(method) \(raw)")
        }
        var account = URLRequest(url: URL(string: "http://127.0.0.1:3001/api/chat/channels/test/messages")!)
        account.httpMethod = "POST"
        account.setValue("Bearer fixture-account", forHTTPHeaderField: "Authorization")
        account.setValue("fixture-chat", forHTTPHeaderField: "X-Caper-Chat-Token")
        let (_, response) = try await session.data(for: account)
        let http = try XCTUnwrap(response as? HTTPURLResponse)
        XCTAssertEqual(http.statusCode, 299)
        XCTAssertEqual(http.value(forHTTPHeaderField: "X-Test-Authorization"), "Bearer fixture-account")
        XCTAssertEqual(http.value(forHTTPHeaderField: "X-Test-Chat-Token"), "fixture-chat")
    }
}
