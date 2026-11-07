import Foundation

/// A provider-only fixture for the labelled local parity app. Installation is
/// session-local: never register a global protocol or replace Caper's HTTP/WS.
enum KlipyFixtureTransport {
    static func install(into configuration: URLSessionConfiguration,
                        environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard environment["CAPER_TEST_MODE"] == "parity", environment["CAPER_UI_FIXTURE"] == "klipy",
              let rawURL = environment["CAPER_API_BASE_URL"], let origin = URLComponents(string: rawURL),
              origin.url != nil, ["http", "https"].contains(origin.scheme?.lowercased() ?? ""),
              ["localhost", "127.0.0.1", "::1", "[::1]"].contains(origin.host?.lowercased() ?? ""),
              origin.user == nil, origin.password == nil,
              origin.path.isEmpty || origin.path == "/", origin.query == nil, origin.fragment == nil,
              origin.port.map({ (1...65535).contains($0) }) ?? true else { return }
        var protocols = configuration.protocolClasses ?? []
        guard !protocols.contains(where: { $0 == KlipyFixtureURLProtocol.self }) else { return }
        protocols.insert(KlipyFixtureURLProtocol.self, at: 0)
        configuration.protocolClasses = protocols
    }
}

private final class KlipyFixtureURLProtocol: URLProtocol, @unchecked Sendable {
    private static let mediaRoot = "/caper-test-fixtures/klipy/v1/"

    private enum Route {
        case list(ChatGif.Kind, query: String?, page: Int)
        case share
        case media(Data, contentType: String)
    }

    /// Match exact encoded paths, not suffixes, decoded lookalikes or real keys.
    /// This transport never makes another request, forwards headers or redirects.
    private static func route(_ request: URLRequest) -> Route? {
        guard let url = request.url, let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              components.scheme?.lowercased() == "https", components.user == nil, components.password == nil,
              components.port == nil, components.fragment == nil else { return nil }
        let path = components.percentEncodedPath
        if components.host?.lowercased() == "static.klipy.com", request.httpMethod == "GET", components.query == nil {
            for kind in [ChatGif.Kind.gif, .sticker] {
                let item = FixtureItem(kind: kind)
                if path == mediaRoot + item.slug + ".gif" { return .media(item.animation, contentType: "image/gif") }
                if path == mediaRoot + item.slug + ".png" { return .media(item.poster, contentType: "image/png") }
            }
            return nil
        }
        guard components.host?.lowercased() == "api.klipy.com" else { return nil }
        for kind in [ChatGif.Kind.gif, .sticker] {
            let base = "/api/v1/fixtureKlipyKey/\(kind.path)/"
            if request.httpMethod == "GET", path == base + "trending" || path == base + "search" {
                let items = components.queryItems ?? []
                let query = path == base + "search" ? items.first(where: { $0.name == "q" })?.value : nil
                let page = max(1, Int(items.first(where: { $0.name == "page" })?.value ?? "1") ?? 1)
                return .list(kind, query: query, page: page)
            }
            let share = base + "share/"
            if request.httpMethod == "POST", path.hasPrefix(share), KlipyRules.slug(String(path.dropFirst(share.count))) {
                return .share
            }
        }
        return nil
    }

    override class func canInit(with request: URLRequest) -> Bool { route(request) != nil }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url, let route = Self.route(request) else {
            client?.urlProtocol(self, didFailWithError: URLError(.unsupportedURL))
            return
        }
        do {
            let status: Int
            let contentType: String
            let data: Data
            switch route {
            case let .media(bytes, type):
                status = 200; contentType = type; data = bytes
            case .share:
                status = 200; contentType = "application/json"
                data = Data(#"{"result":true}"#.utf8)
            case let .list(kind, query, page):
                contentType = "application/json"
                if query == "fail" {
                    status = 503
                    data = Data(#"{"result":false,"error":"TEST FIXTURE — KLIPY unavailable"}"#.utf8)
                } else {
                    status = 200
                    let entries: [[String: Any]] = query == "empty" || page > 1 ? [] : [FixtureItem(kind: kind).entry]
                    data = try JSONSerialization.data(withJSONObject: [
                        "result": true,
                        "data": ["data": entries, "current_page": page, "per_page": 24, "has_next": false],
                    ], options: [.sortedKeys])
                }
            }
            guard let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: [
                "Content-Type": contentType, "Content-Length": String(data.count), "Cache-Control": "no-store",
            ]) else { throw URLError(.badServerResponse) }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }

    override func stopLoading() {} // All bytes are delivered synchronously; no backing task to cancel.

    private struct FixtureItem {
        let kind: ChatGif.Kind
        var slug: String { "caper-fixture-\(kind.rawValue)" }
        var width: Int { kind == .gif ? 96 : 64 }
        var height: Int { kind == .gif ? 64 : 96 }
        var animation: Data { kind == .gif ? KlipyFixtureURLProtocol.gifAnimation : KlipyFixtureURLProtocol.stickerAnimation }
        var poster: Data { kind == .gif ? KlipyFixtureURLProtocol.gifPoster : KlipyFixtureURLProtocol.stickerPoster }

        var entry: [String: Any] {
            let base = "https://static.klipy.com" + KlipyFixtureURLProtocol.mediaRoot + slug
            let size: [String: Any] = [
                "gif": ["url": base + ".gif", "width": width, "height": height, "size": animation.count],
                "png": ["url": base + ".png", "width": width, "height": height, "size": poster.count],
            ]
            return [
                "id": kind == .gif ? 1 : 2, "slug": slug, "title": "TEST FIXTURE — \(kind.label)",
                "type": kind.rawValue, "file": ["sm": size, "md": size],
            ]
        }
    }

    // Original, dependency-free geometric art, not downloaded provider media.
    // Landscape GIF: eggshell ground, terracotta left bar, moving green right bar.
    // Portrait sticker: transparent ground, green body, eggshell eye, moving
    // terracotta arm. Both have two full-size frames (350/650 ms), loop forever,
    // and use a matching first-frame PNG. Asymmetry exposes swapped layout axes;
    // real encoded bytes exercise ImageIO in tiles, the timeline and the viewer.
    private static let gifAnimation = Data(base64Encoded: "R0lGODlhYABAAPEAAGN6Q7ZNMvHt5AAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAIwAAACwAAAAAYABAAAAC0pSPqcvtD6OctNqLs968+w+G4kiW5omm6hm07gvH7lrK9k2T9w7nIw8M+ERB3jBU3B1BSdzS07Q9odHYtFO1XjfZ3lbTfX3B4dY4UzafL2nh2tJ+w9PySrw+ueMj+v2j79cAGLgwSJhgeHjQBtDo+AgZKTlJ6cjBWJmpuQlwmcYJGmrJ9SlqqulZdrpKmRrGCgvp2hVb20mqags7m6W7i/vqu8pbJTwMTGtsShylvIzc6xzK3CQ9DV1szanI3e39DR4uPk5ebn6Onq6+zt7u/n5RAAAh+QQAQQAAACwAAAAAYABAAAAC35SPqcvtD6OctNqLs968+w+G4kiW5omm6hm07gvH7lrK9k2T9w7nIw8M+ERB3jBU3B1BSdzS07Q9odHYtFO1XjfZ3lbTfX3B4dY4UzafL2nh2tJ+w9PySrw+ueMj+v2j79cAGLgwSJhgeHiQyADg+AgZKTlJCUnBuFCpual5STfBGSoK4FlGMYpaWRp2mupqmfcp8UpLGmsKWuu62tWqi8qb5fsrGlw1TMxpHIWc3HnLmuusDN0rPf0sgamATa0tG9G9udzULA6rmK6+zt7u/g4fLz9PX29/j5+vv8/fUAAAOw==")!
    private static let gifPoster = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAGAAAABAAgMAAACYWpqdAAAACVBMVEXx7eS2TTJjekOrkiQ4AAAAIElEQVQ4y2NgGDYgFAxGJUYlaCGxCgmMSgykxCgYRAAAUczPMUfGwKsAAAAASUVORK5CYII=")!
    private static let stickerAnimation = Data(base64Encoded: "R0lGODlhQABgAPEAAAAAAGN6Q7ZNMvHt5CH/C05FVFNDQVBFMi4wAwEAAAAh+QQJIwAAACwAAAAAQABgAAAC+YSPqcvtD6OctNqLs968+w+G4kiW5omm6sq27gvHQEDX9o3nNqn3/s37CXXBobFWPBqTSiGz6XtCiSPd4Iq9KqWBbHZbzXmxYJF1PCiHzmM1iO11f+DfIxedtoenPy5/t/dHZSbY41d4KJj4t8jXOPUIFdk0KedRaBiICUi4CaTpGVB5JFBqelqKEXqD2iqgulrjigobGzB7WhuLa6q7ypt6YUsD/Co8XOwbmnxsy2wxfAus7PlcEW1NgT3dvMsNjfx9PTy6BOpZPpTudL653teO+R4Vj1iveM+Y77gP2S/5j5KMgQQLGjyIMKHChQwbOnwIMaLEiRILAAAh+QQJQQAAACwAAAAAQABgAAAC+YSPqcvtD6OctNqLs968+w+G4kiW5omm6sq27gvHQEDX9o3nNqn3/s37CXXBobFWPBqTSiGz6XtCiSPd4Iq9KqWBbHZbzXmxYJF1PCiHzmM1iO11f+DfIxedtoenP4H/D+iHwScUaCgwSOhzGJioqMMI6PiIE/k3SWljKXiRmbOJ2Ol5A4o5Wio6WoNqoaq5aerJWuG6CpvqOktRS6M7wRvgKwHMRVn8eKyYTLjM1zz1DBXdNC3nQbznWq1nxru9lK36PTTuFD5a/pMede653vNO1V0bn1OPcw/Unpm/s2/8D5mMgQQLGjyIMKHChQwbOnwIMaLEiRILAAA7")!
    private static let stickerPoster = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAEAAAABgAgMAAADQq5x7AAAADFBMVEUAAABjekPx7eS2TTIDMJWcAAAAAXRSTlMAQObYZgAAACtJREFUOMtjYBgF6CAUCgZeYNWqwSEwWMJjcAn8//9/VGDgo2HQCYwCBAAA6iTfIQrLsOcAAAAASUVORK5CYII=")!
}
