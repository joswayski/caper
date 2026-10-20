import Foundation

// KLIPY GIFs and stickers (docs/media.md#gifs-and-stickers-klipy). Search,
// trending, share and media requests go straight from this device to KLIPY;
// Caper's API only hands out the app key and a customer id, and stores the
// chosen GIF's KLIPY URLs unchanged as `content.gif`.

/// A message's `content.gif`. Decoding throws unless every shared rule
/// holds (`shared/klipy/cases.json` `messages`), so `ChatContent` can drop an
/// unusable value with `try?` and show the text alone.
public struct ChatGif: Codable, Equatable, Hashable, Sendable {
    public enum Kind: String, Codable, Hashable, Sendable {
        case gif, sticker
        /// The KLIPY API path segment.
        var path: String { self == .gif ? "gifs" : "stickers" }
        /// What a captionless message or an untitled GIF reads as.
        public var label: String { self == .gif ? "GIF" : "Sticker" }
    }

    public let provider: String
    public let kind: Kind
    public let slug: String
    public let title: String
    public let width: Int
    public let height: Int
    public let gif: String?
    public let webp: String?
    public let mp4: String?
    public let still: String?

    public init(kind: Kind, slug: String, title: String, width: Int, height: Int,
                gif: String? = nil, webp: String? = nil, mp4: String? = nil, still: String? = nil) {
        provider = "klipy"
        self.kind = kind; self.slug = slug; self.title = title; self.width = width; self.height = height
        self.gif = gif; self.webp = webp; self.mp4 = mp4; self.still = still
    }

    private enum CodingKeys: String, CodingKey { case provider, kind, slug, title, width, height, gif, webp, mp4, still }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        provider = try values.decode(String.self, forKey: .provider)
        kind = try values.decode(Kind.self, forKey: .kind)
        slug = try values.decode(String.self, forKey: .slug)
        // The API stores a title on every GIF; treat a missing one as untitled, as it does.
        title = try values.decodeIfPresent(String.self, forKey: .title) ?? ""
        width = try values.decode(Int.self, forKey: .width)
        height = try values.decode(Int.self, forKey: .height)
        gif = try values.decodeIfPresent(String.self, forKey: .gif)
        webp = try values.decodeIfPresent(String.self, forKey: .webp)
        mp4 = try values.decodeIfPresent(String.self, forKey: .mp4)
        still = try values.decodeIfPresent(String.self, forKey: .still)
        guard isValid else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Invalid KLIPY GIF"))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(provider, forKey: .provider)
        try values.encode(kind, forKey: .kind)
        try values.encode(slug, forKey: .slug)
        try values.encode(title, forKey: .title)
        try values.encode(width, forKey: .width)
        try values.encode(height, forKey: .height)
        try values.encodeIfPresent(gif, forKey: .gif)
        try values.encodeIfPresent(webp, forKey: .webp)
        try values.encodeIfPresent(mp4, forKey: .mp4)
        try values.encodeIfPresent(still, forKey: .still)
    }

    /// The shared `content.gif` rules, as `apps/api/src/klipy.rs` applies them.
    public var isValid: Bool {
        provider == "klipy" && KlipyRules.slug(slug) && KlipyRules.title(title)
            && KlipyRules.edge(width) && KlipyRules.edge(height)
            && (gif != nil || webp != nil)
            && [gif, webp, mp4, still].allSatisfy { $0.map(KlipyRules.mediaURL) ?? true }
    }

    /// The accessible label: the title, or `GIF` / `Sticker`.
    public var accessibilityLabel: String { title.isEmpty ? kind.label : title }

    /// Animated sources in the order this device's decoder (ImageIO, which
    /// reads animated WebP and GIF) should try them. WebP keeps stickers'
    /// transparency and is smaller; GIF is the fallback.
    var animatedSources: [URL] { [webp, gif].compactMap { $0.flatMap(URL.init(string:)) } }
}

public enum KlipyRules {
    static let maxURL = 2048
    static let maxText = 200
    static let maxEdge = 4096

    /// KLIPY media only: HTTPS on klipy.com or a subdomain, at most 2048
    /// characters, without credentials, port or fragment. A query is fine.
    public static func mediaURL(_ value: String) -> Bool {
        guard value.utf8.count <= maxURL, URL(string: value) != nil,
              let components = URLComponents(string: value),
              components.scheme?.lowercased() == "https",
              (components.user ?? "").isEmpty, components.password == nil,
              components.port == nil, components.fragment == nil,
              let host = components.host?.lowercased() else { return false }
        return host == "klipy.com" || host.hasSuffix(".klipy.com")
    }

    /// 1–200 letters, digits, `-` or `_`.
    static func slug(_ value: String) -> Bool {
        (1...maxText).contains(value.unicodeScalars.count)
            && value.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "-" || $0 == "_") }
    }

    /// At most 200 characters, none of them control characters.
    static func title(_ value: String) -> Bool {
        value.unicodeScalars.count <= maxText && !value.unicodeScalars.contains(where: isControl)
    }

    static func edge(_ value: Int) -> Bool { (1...maxEdge).contains(value) }

    static func isControl(_ scalar: Unicode.Scalar) -> Bool { scalar.properties.generalCategory == .control }

    /// Control characters stripped, then cut to 200 characters.
    static func cleanTitle(_ value: String) -> String {
        String(String.UnicodeScalarView(value.unicodeScalars.filter { !isControl($0) }.prefix(maxText)))
    }
}

// MARK: Picker results

/// A grid tile: KLIPY's small animated preview and an optional still.
public struct KlipyTile: Codable, Equatable, Sendable {
    public let url: String
    public let still: String?
    public let width: Int
    public let height: Int
}

public struct KlipyResult: Equatable, Sendable {
    public let slug: String
    public let title: String
    public let tile: KlipyTile
    /// What choosing it sends.
    public let gif: ChatGif
}

/// A KLIPY ad. Its HTML only ever renders in an isolated, non-persistent
/// web view (iPhone); sizes are nil when KLIPY sent none.
public struct KlipyAd: Equatable, Sendable {
    public let html: String
    public let width: Double?
    public let height: Double?
}

public enum KlipyItem: Equatable, Sendable {
    case item(KlipyResult)
    case ad(KlipyAd)
}

public struct KlipyPage: Equatable, Sendable {
    public let items: [KlipyItem]
    public let hasNext: Bool
}

/// Turns a KLIPY list response into picker items in KLIPY's order
/// (`shared/klipy/cases.json` `parse`). Anything unusable is skipped.
public enum KlipyParser {
    public static func parse(_ data: Data, kind: ChatGif.Kind) throws -> KlipyPage {
        guard let page = parse(response: try JSONSerialization.jsonObject(with: data), kind: kind) else {
            throw URLError(.cannotParseResponse)
        }
        return page
    }

    static func parse(response: Any, kind: ChatGif.Kind) -> KlipyPage? {
        guard let root = response as? [String: Any], let body = root["data"] as? [String: Any],
              let entries = body["data"] as? [Any] else { return nil }
        let items = entries.compactMap { entry -> KlipyItem? in
            guard let entry = entry as? [String: Any] else { return nil }
            if entry["type"] as? String == "ad" { return ad(entry).map(KlipyItem.ad) }
            return result(entry, kind: kind).map(KlipyItem.item)
        }
        return KlipyPage(items: items, hasNext: body["has_next"] as? Bool ?? false)
    }

    private static func ad(_ entry: [String: Any]) -> KlipyAd? {
        guard let html = entry["content"] as? String, !html.isEmpty else { return nil }
        func size(_ value: Any?) -> Double? {
            guard let number = value as? NSNumber, number.doubleValue.isFinite, number.doubleValue > 0 else { return nil }
            return number.doubleValue
        }
        return KlipyAd(html: html, width: size(entry["width"]), height: size(entry["height"]))
    }

    /// One format of one size (`file.md.gif`), when its URL passes the media rule.
    private struct Media {
        let url: String
        let width: Int?
        let height: Int?
        init?(_ value: Any?) {
            guard let value = value as? [String: Any], let url = value["url"] as? String, KlipyRules.mediaURL(url) else { return nil }
            self.url = url
            width = Self.edge(value["width"]); height = Self.edge(value["height"])
        }
        static func edge(_ value: Any?) -> Int? {
            guard let number = value as? NSNumber else { return nil }
            let double = number.doubleValue
            guard double.rounded() == double, double >= 1, double <= Double(KlipyRules.maxEdge) else { return nil }
            return Int(double)
        }
        var size: (Int, Int)? { width.flatMap { width in height.map { (width, $0) } } }
    }

    private static func result(_ entry: [String: Any], kind: ChatGif.Kind) -> KlipyResult? {
        guard let slug = entry["slug"] as? String, KlipyRules.slug(slug),
              let file = entry["file"] as? [String: Any] else { return nil }
        let title = KlipyRules.cleanTitle(entry["title"] as? String ?? "")
        // The sent GIF: the first of md, hd, sm that yields one.
        var sent: ChatGif?
        for name in ["md", "hd", "sm"] {
            guard let size = file[name] as? [String: Any] else { continue }
            let gif = Media(size["gif"]), webp = Media(size["webp"])
            guard let dimensions = gif?.size ?? webp?.size else { continue }
            let jpg = Media(size["jpg"])?.url, png = Media(size["png"])?.url
            let candidate = ChatGif(kind: kind, slug: slug, title: title, width: dimensions.0, height: dimensions.1,
                                    gif: gif?.url, webp: webp?.url, mp4: Media(size["mp4"])?.url,
                                    still: kind == .sticker ? png ?? jpg : jpg ?? png)
            if candidate.isValid { sent = candidate; break }
        }
        guard let sent else { return nil }
        // The tile: the first of sm, xs, md, hd with an animated preview.
        for name in ["sm", "xs", "md", "hd"] {
            guard let size = file[name] as? [String: Any], let preview = Media(size["webp"]) ?? Media(size["gif"]) else { continue }
            let still = Media(size["jpg"])?.url ?? Media(size["png"])?.url
            let tile = KlipyTile(url: preview.url, still: still, width: preview.width ?? sent.width, height: preview.height ?? sent.height)
            return KlipyResult(slug: slug, title: title, tile: tile, gif: sent)
        }
        return nil
    }
}

/// Picker rows: tiles fill `columns` per row in KLIPY's order, and each ad
/// takes a full row of its own where it appears.
public enum KlipyGridRow: Equatable, Identifiable, Sendable {
    case tiles(first: Int, [KlipyResult])
    case ad(index: Int, KlipyAd)

    public var id: Int {
        switch self {
        case let .tiles(first, _): return first
        case let .ad(index, _): return index
        }
    }

    public static func rows(_ items: [KlipyItem], columns: Int) -> [KlipyGridRow] {
        let columns = max(1, columns)
        var rows: [KlipyGridRow] = []
        var current: [KlipyResult] = []
        var first = 0
        for (index, item) in items.enumerated() {
            switch item {
            case let .item(result):
                if current.isEmpty { first = index }
                current.append(result)
                if current.count == columns { rows.append(.tiles(first: first, current)); current = [] }
            case let .ad(ad):
                if !current.isEmpty { rows.append(.tiles(first: first, current)); current = [] }
                rows.append(.ad(index: index, ad))
            }
        }
        if !current.isEmpty { rows.append(.tiles(first: first, current)) }
        return rows
    }
}

// MARK: Requests

/// `GET /api/klipy`: KLIPY's client key (it ships to devices by design) and
/// this account's stable, unlinkable customer id.
public struct KlipyConfiguration: Codable, Equatable, Sendable {
    public let appKey: String
    public let customerId: String

    public init(appKey: String, customerId: String) { self.appKey = appKey; self.customerId = customerId }

    var isValid: Bool {
        (1...200).contains(appKey.unicodeScalars.count)
            && appKey.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "-" || $0 == "_") }
            && customerId.unicodeScalars.count == 32
            && customerId.unicodeScalars.allSatisfy { $0.isASCII && CharacterSet(charactersIn: "0123456789abcdefABCDEF").contains($0) }
    }
}

/// The ad fields an iPhone sends (macOS sends none, so it never gets ads).
/// Only values readable without a permission prompt: no carrier,
/// advertising id or location.
public struct KlipyAdRequest: Equatable, Sendable {
    public var maxWidth: Int
    public var osVersion: String
    public var model: String
    public var deviceWidth: Int
    public var deviceHeight: Int
    public var pixelRatio: Double
    public var language: String?
    /// The system WebView's `navigator.userAgent`; KLIPY serves ads only to mobile agents.
    public var userAgent: String?

    var queryItems: [(String, String)] {
        let ratio = pixelRatio.rounded() == pixelRatio ? String(Int(pixelRatio)) : String(pixelRatio)
        var items: [(String, String)] = [
            ("ad-min-width", "50"), ("ad-max-width", String(max(50, maxWidth))),
            ("ad-min-height", "50"), ("ad-max-height", "250"),
            ("ad-os", "ios"), ("ad-osv", osVersion), ("ad-make", "apple"), ("ad-model", model),
            ("ad-device-w", String(deviceWidth)), ("ad-device-h", String(deviceHeight)), ("ad-pxratio", ratio),
        ]
        if let language, !language.isEmpty { items.append(("ad-language", language)) }
        return items
    }
}

public struct KlipyClient: Sendable {
    public static let perPage = 24
    private static let base = "https://api.klipy.com/api/v1/"
    /// Cookie-less and refusing cross-origin redirects, like every Caper request.
    private static let session = SecureSession.make()

    public let configuration: KlipyConfiguration

    public init(configuration: KlipyConfiguration) { self.configuration = configuration }

    /// The device region as ISO 3166 alpha-2, lowercase; nil when the region
    /// is not a country (such as a UN M49 area code).
    public static var locale: String? {
        guard let region = Locale.current.region?.identifier, region.unicodeScalars.count == 2,
              region.unicodeScalars.allSatisfy({ $0.isASCII && CharacterSet.letters.contains($0) }) else { return nil }
        return region.lowercased()
    }

    /// Trending while `query` is empty, otherwise search.
    public func listURL(kind: ChatGif.Kind, query: String, page: Int, locale: String? = KlipyClient.locale, ads: KlipyAdRequest? = nil) -> URL? {
        var items: [(String, String)] = []
        if !query.isEmpty { items.append(("q", query)) }
        items += [("page", String(max(1, page))), ("per_page", String(Self.perPage)),
                  ("customer_id", configuration.customerId), ("content_filter", "off")]
        if let locale { items.append(("locale", locale)) }
        if let ads { items += ads.queryItems }
        return Self.url("\(kind.path)/\(query.isEmpty ? "trending" : "search")", query: items, appKey: configuration.appKey)
    }

    public func page(kind: ChatGif.Kind, query: String, page: Int, ads: KlipyAdRequest? = nil) async throws -> KlipyPage {
        guard configuration.isValid, let url = listURL(kind: kind, query: query, page: page, ads: ads) else { throw URLError(.badURL) }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        request.setValue("application/json", forHTTPHeaderField: "accept")
        if let agent = ads?.userAgent { request.setValue(agent, forHTTPHeaderField: "user-agent") }
        let (data, response) = try await Self.session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw URLError(.badServerResponse)
        }
        return try KlipyParser.parse(data, kind: kind)
    }

    /// `POST {kind}/share/{slug}` with `{"customer_id","q"}`.
    public func shareRequest(kind: ChatGif.Kind, slug: String, query: String, userAgent: String? = nil) -> URLRequest? {
        guard configuration.isValid, KlipyRules.slug(slug), let url = Self.url("\(kind.path)/share/\(slug)", query: [], appKey: configuration.appKey) else { return nil }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 20)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        if let userAgent { request.setValue(userAgent, forHTTPHeaderField: "user-agent") }
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["customer_id": configuration.customerId, "q": query], options: [.sortedKeys])
        return request
    }

    /// Tells KLIPY a GIF was sent, after the send succeeded. Best effort: failures are ignored.
    public func share(kind: ChatGif.Kind, slug: String, query: String, userAgent: String? = nil) async {
        guard let request = shareRequest(kind: kind, slug: slug, query: query, userAgent: userAgent) else { return }
        _ = try? await Self.session.data(for: request)
    }

    private static let unreserved = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

    /// Percent-encodes everything but unreserved characters, so `&`, `+` and `=` in a search stay literal.
    private static func url(_ path: String, query: [(String, String)], appKey: String) -> URL? {
        func encode(_ value: String) -> String { value.addingPercentEncoding(withAllowedCharacters: unreserved) ?? "" }
        let suffix = query.isEmpty ? "" : "?" + query.map { "\(encode($0.0))=\(encode($0.1))" }.joined(separator: "&")
        return URL(string: base + encode(appKey) + "/" + path + suffix)
    }
}
