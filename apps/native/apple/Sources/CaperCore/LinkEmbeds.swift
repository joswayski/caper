import Foundation

// MARK: Link preview cards (docs/media.md "Link embeds", version 1)

/// One link's preview card, stored as `content.embeds` and sent in
/// `message.embeds`. Mirrors apps/web/src/chat/embeds.ts; LinkEmbedsTests runs
/// every case in shared/messages/embed-cases.json. Decoding is lenient like
/// attachments: unknown or malformed blocks are skipped, and a card left
/// without blocks (or with a bad version, link or site) fails to decode, so
/// `decodeLossyEmbeds` drops it without failing the message. Every string is
/// plain text; every link is `http(s)` only.
public struct ChatEmbed: Codable, Equatable, Sendable {
    public var version: Int { 1 }
    /// The link as posted; the card opens it.
    public let url: String
    public let site: EmbedSite
    /// Drawn in order. Never empty.
    public let blocks: [EmbedBlock]

    public init(url: String, site: EmbedSite, blocks: [EmbedBlock]) {
        self.url = url; self.site = site; self.blocks = blocks
    }

    public init(from decoder: Decoder) throws {
        guard let card = Self.card(from: decoder) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unusable link preview."))
        }
        self = card
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: EmbedKey.self)
        try values.encode(1, forKey: EmbedKey("version"))
        try values.encode(url, forKey: EmbedKey("url"))
        try values.encode(site, forKey: EmbedKey("site"))
        try values.encode(blocks, forKey: EmbedKey("blocks"))
    }

    /// The card's name for accessibility and media: its title, author or site.
    public var heading: String {
        for block in blocks { if case .title(let title) = block { return title.text } }
        for block in blocks { if case .author(let author) = block { return author.name } }
        return site.name
    }

    /// An `images` block marked `thumbnail`: one small image beside the text.
    public var thumbnail: EmbedImage? {
        for block in blocks { if case .images(let images) = block, images.thumbnail { return images.items.first } }
        return nil
    }

    static func card(from decoder: Decoder) -> ChatEmbed? {
        guard let container = try? decoder.container(keyedBy: EmbedKey.self) else { return nil }
        let fields = EmbedFields(container)
        guard (try? container.decode(Int.self, forKey: EmbedKey("version"))) == 1, let url = fields.link("url"),
              let siteContainer = try? container.nestedContainer(keyedBy: EmbedKey.self, forKey: EmbedKey("site")),
              let name = EmbedFields(siteContainer).text("name"), let domain = EmbedFields(siteContainer).text("domain"),
              var list = try? container.nestedUnkeyedContainer(forKey: EmbedKey("blocks")) else { return nil }
        var blocks: [EmbedBlock] = []
        while !list.isAtEnd {
            let index = list.currentIndex
            if (try? list.decodeNil()) == true { continue }
            if let entry = try? list.decode(LenientEmbedBlock.self), let block = entry.value { blocks.append(block) }
            if list.currentIndex == index { break }
        }
        guard !blocks.isEmpty else { return nil }
        return ChatEmbed(url: url, site: EmbedSite(name: name, domain: domain), blocks: blocks)
    }
}

/// `site.domain` comes from the posted URL, never from the page; cards always show it.
public struct EmbedSite: Codable, Equatable, Sendable {
    public let name: String
    public let domain: String
    public init(name: String, domain: String) { self.name = name; self.domain = domain }
}

public struct EmbedAuthor: Equatable, Sendable {
    public let name: String
    public let handle: String?
    public let url: String?
    public let icon: String?
}

public struct EmbedTitle: Equatable, Sendable {
    public let text: String
    public let url: String?
}

public struct EmbedImage: Equatable, Sendable {
    public let url: String
    public let width: Double?
    public let height: Double?
    public let alt: String?
}

public struct EmbedImages: Equatable, Sendable {
    /// One to four.
    public let items: [EmbedImage]
    public let thumbnail: Bool
}

/// A direct MP4/WebM; `gif` loops muted without controls.
public struct EmbedVideo: Equatable, Sendable {
    public let url: String
    public let poster: String?
    public let width: Double?
    public let height: Double?
    public let durationMs: Double?
    public let gif: Bool
}

public enum EmbedPlayerProvider: String, Equatable, Sendable { case youtube, vimeo, twitch }

/// A provider's player. Native apps load `play` (Caper's player page) only
/// after Play, and only from `LinkEmbeds.playURL`'s hosts.
public struct EmbedPlayer: Equatable, Sendable {
    public let provider: EmbedPlayerProvider
    public let id: String
    public let play: String?
    public let poster: String?
    public let width: Double?
    public let height: Double?
    public let start: Double?
}

public struct EmbedFooter: Equatable, Sendable {
    public let text: String?
    public let timestamp: String?
}

public enum EmbedBlock: Codable, Equatable, Sendable {
    case author(EmbedAuthor)
    case title(EmbedTitle)
    case text(String)
    case images(EmbedImages)
    case video(EmbedVideo)
    case player(EmbedPlayer)
    case footer(EmbedFooter)

    public var type: String {
        switch self {
        case .author: return "author"
        case .title: return "title"
        case .text: return "text"
        case .images: return "images"
        case .video: return "video"
        case .player: return "player"
        case .footer: return "footer"
        }
    }

    public init(from decoder: Decoder) throws {
        guard let block = Self.block(from: decoder) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unusable link preview block."))
        }
        self = block
    }

    /// A block safe to draw, or nil for unknown or malformed blocks (skipped).
    static func block(from decoder: Decoder) -> EmbedBlock? {
        guard let container = try? decoder.container(keyedBy: EmbedKey.self) else { return nil }
        let fields = EmbedFields(container)
        switch fields.string("type") ?? "" {
        case "author":
            guard let name = fields.text("name"), fields.validText("handle"), fields.validLink("url"), fields.validLink("icon") else { return nil }
            return .author(EmbedAuthor(name: name, handle: fields.string("handle"), url: fields.link("url"), icon: fields.link("icon")))
        case "title":
            guard let text = fields.text("text"), fields.validLink("url") else { return nil }
            return .title(EmbedTitle(text: text, url: fields.link("url")))
        case "text":
            guard let text = fields.text("text") else { return nil }
            return .text(text)
        case "images":
            guard var list = try? container.nestedUnkeyedContainer(forKey: EmbedKey("items")) else { return nil }
            var items: [EmbedImage] = []
            while !list.isAtEnd {
                let index = list.currentIndex
                if (try? list.decodeNil()) == true { continue }
                if let entry = try? list.decode(LenientEmbedImage.self), let image = entry.value { items.append(image) }
                if list.currentIndex == index { break }
            }
            guard !items.isEmpty else { return nil }
            return .images(EmbedImages(items: Array(items.prefix(4)), thumbnail: fields.flag("thumbnail")))
        case "video":
            guard let url = fields.link("url"), fields.validLink("poster"), fields.validSize("width"), fields.validSize("height"),
                  fields.validSize("durationMs") else { return nil }
            return .video(EmbedVideo(url: url, poster: fields.link("poster"), width: fields.size("width"), height: fields.size("height"),
                                     durationMs: fields.size("durationMs"), gif: fields.flag("gif")))
        case "player":
            guard let provider = fields.string("provider").flatMap(EmbedPlayerProvider.init(rawValue:)), let id = fields.text("id"),
                  fields.validLink("poster"), fields.validSize("width"), fields.validSize("height"), fields.validSize("start") else { return nil }
            // Web never reads `play`; a missing or unusable one only means Play opens the card's link instead.
            return .player(EmbedPlayer(provider: provider, id: id, play: fields.link("play"), poster: fields.link("poster"),
                                       width: fields.size("width"), height: fields.size("height"), start: fields.size("start")))
        case "footer":
            guard fields.text("text") != nil || fields.text("timestamp") != nil, fields.validText("text"), fields.validText("timestamp") else { return nil }
            return .footer(EmbedFooter(text: fields.text("text"), timestamp: fields.text("timestamp")))
        default:
            return nil
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: EmbedKey.self)
        try values.encode(type, forKey: EmbedKey("type"))
        switch self {
        case .author(let author):
            try values.encode(author.name, forKey: EmbedKey("name"))
            try values.encodeIfPresent(author.handle, forKey: EmbedKey("handle"))
            try values.encodeIfPresent(author.url, forKey: EmbedKey("url"))
            try values.encodeIfPresent(author.icon, forKey: EmbedKey("icon"))
        case .title(let title):
            try values.encode(title.text, forKey: EmbedKey("text"))
            try values.encodeIfPresent(title.url, forKey: EmbedKey("url"))
        case .text(let text):
            try values.encode(text, forKey: EmbedKey("text"))
        case .images(let images):
            var list = values.nestedUnkeyedContainer(forKey: EmbedKey("items"))
            for image in images.items {
                var item = list.nestedContainer(keyedBy: EmbedKey.self)
                try item.encode(image.url, forKey: EmbedKey("url"))
                try item.encodeIfPresent(image.width, forKey: EmbedKey("width"))
                try item.encodeIfPresent(image.height, forKey: EmbedKey("height"))
                try item.encodeIfPresent(image.alt, forKey: EmbedKey("alt"))
            }
            if images.thumbnail { try values.encode(true, forKey: EmbedKey("thumbnail")) }
        case .video(let video):
            try values.encode(video.url, forKey: EmbedKey("url"))
            try values.encodeIfPresent(video.poster, forKey: EmbedKey("poster"))
            try values.encodeIfPresent(video.width, forKey: EmbedKey("width"))
            try values.encodeIfPresent(video.height, forKey: EmbedKey("height"))
            try values.encodeIfPresent(video.durationMs, forKey: EmbedKey("durationMs"))
            if video.gif { try values.encode(true, forKey: EmbedKey("gif")) }
        case .player(let player):
            try values.encode(player.provider.rawValue, forKey: EmbedKey("provider"))
            try values.encode(player.id, forKey: EmbedKey("id"))
            try values.encodeIfPresent(player.play, forKey: EmbedKey("play"))
            try values.encodeIfPresent(player.poster, forKey: EmbedKey("poster"))
            try values.encodeIfPresent(player.width, forKey: EmbedKey("width"))
            try values.encodeIfPresent(player.height, forKey: EmbedKey("height"))
            try values.encodeIfPresent(player.start, forKey: EmbedKey("start"))
        case .footer(let footer):
            try values.encodeIfPresent(footer.text, forKey: EmbedKey("text"))
            try values.encodeIfPresent(footer.timestamp, forKey: EmbedKey("timestamp"))
        }
    }
}

struct EmbedKey: CodingKey {
    let stringValue: String
    var intValue: Int? { nil }
    init(_ stringValue: String) { self.stringValue = stringValue }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { return nil }
}

/// One object's fields as web's validators read them: a key that is present
/// must hold the expected JSON type (`null` included as a mismatch).
private struct EmbedFields {
    let container: KeyedDecodingContainer<EmbedKey>
    init(_ container: KeyedDecodingContainer<EmbedKey>) { self.container = container }

    private func has(_ name: String) -> Bool { container.contains(EmbedKey(name)) }
    func string(_ name: String) -> String? { try? container.decode(String.self, forKey: EmbedKey(name)) }
    /// A non-empty string.
    func text(_ name: String) -> String? { string(name).flatMap { $0.isEmpty ? nil : $0 } }
    /// Absent, or any string.
    func validText(_ name: String) -> Bool { !has(name) || string(name) != nil }
    /// An `http(s)://` string.
    func link(_ name: String) -> String? { string(name).flatMap { LinkEmbeds.isWebLink($0) ? $0 : nil } }
    func validLink(_ name: String) -> Bool { !has(name) || link(name) != nil }
    /// A finite number above zero.
    func size(_ name: String) -> Double? {
        (try? container.decode(Double.self, forKey: EmbedKey(name))).flatMap { $0.isFinite && $0 > 0 ? $0 : nil }
    }
    func validSize(_ name: String) -> Bool { !has(name) || size(name) != nil }
    /// Exactly `true`; anything else is false.
    func flag(_ name: String) -> Bool { (try? container.decode(Bool.self, forKey: EmbedKey(name))) == true }
}

private struct LenientEmbedImage: Decodable {
    let value: EmbedImage?
    init(from decoder: Decoder) {
        guard let container = try? decoder.container(keyedBy: EmbedKey.self) else { value = nil; return }
        let fields = EmbedFields(container)
        guard let url = fields.link("url"), fields.validSize("width"), fields.validSize("height"), fields.validText("alt") else { value = nil; return }
        value = EmbedImage(url: url, width: fields.size("width"), height: fields.size("height"), alt: fields.string("alt"))
    }
}

private struct LenientEmbedBlock: Decodable {
    let value: EmbedBlock?
    init(from decoder: Decoder) { value = EmbedBlock.block(from: decoder) }
}

private struct LenientEmbed: Decodable {
    let value: ChatEmbed?
    init(from decoder: Decoder) { value = ChatEmbed.card(from: decoder) }
}

/// A JSON array of cards, keeping the usable ones.
private struct LenientEmbedList: Decodable {
    let embeds: [ChatEmbed]
    init(from decoder: Decoder) throws {
        var list = try decoder.unkeyedContainer()
        embeds = LinkEmbeds.cards(in: &list)
    }
}

extension KeyedDecodingContainer {
    /// `content.embeds`: nil when absent or not an array, otherwise its usable
    /// cards (malformed ones are skipped, never failing the message).
    func decodeLossyEmbeds(forKey key: Key) -> [ChatEmbed]? {
        guard var list = try? nestedUnkeyedContainer(forKey: key) else { return nil }
        return LinkEmbeds.cards(in: &list)
    }
}

// MARK: message.embeds

/// `message.embeds`: unsequenced, sent once a revision's cards are ready (only
/// to subscriptions that asked with `embeds: true`). Applies only to that
/// revision of the message.
public struct MessageEmbedsEvent: Equatable, Sendable {
    public let channelId: String
    public let messageId: String
    public let revision: Int
    /// The usable cards; empty removes them.
    public let embeds: [ChatEmbed]

    public init(channelId: String, messageId: String, revision: Int, embeds: [ChatEmbed]) {
        self.channelId = channelId; self.messageId = messageId; self.revision = revision; self.embeds = embeds
    }

    /// Nil for anything but a well-formed version 1 event; callers drop it
    /// rather than resyncing, like `attachment.progress`.
    public init?(_ event: [String: Any]) {
        guard event["type"] as? String == "message.embeds", event["schemaVersion"] as? Int == 1,
              let channelId = event["channelId"] as? String, !channelId.isEmpty,
              let messageId = event["messageId"] as? String, !messageId.isEmpty,
              let revision = event["revision"] as? Int, revision >= 1,
              let raw = event["embeds"] as? [Any], JSONSerialization.isValidJSONObject(raw),
              let data = try? JSONSerialization.data(withJSONObject: raw),
              let list = try? JSONDecoder().decode(LenientEmbedList.self, from: data) else { return nil }
        self.init(channelId: channelId, messageId: messageId, revision: revision, embeds: list.embeds)
    }
}

/// The latest `message.embeds` per message, so cards survive a later merge of
/// the same revision that lacks them (a late send response) and reach a
/// message that was not loaded yet when its event arrived. Bounded: the
/// oldest entries go first, and a message keeps any cards already applied.
struct EmbedSnapshots: Sendable {
    static let maximumEntries = 256
    private var values: [String: (revision: Int, embeds: [ChatEmbed])] = [:]
    private var order: [String] = []

    /// False when an event for a newer revision is already recorded.
    mutating func apply(_ event: MessageEmbedsEvent) -> Bool {
        if let current = values[event.messageId] {
            guard event.revision >= current.revision else { return false }
        } else {
            order.append(event.messageId)
            if order.count > Self.maximumEntries { values[order.removeFirst()] = nil }
        }
        values[event.messageId] = (event.revision, event.embeds)
        return true
    }

    /// The message with its revision's cards; any other revision, or a
    /// deleted message, is left as it is.
    func overlay(_ message: ChatMessage) -> ChatMessage {
        guard let snapshot = values[message.id], !message.isDeleted, snapshot.revision == (message.revision ?? 1) else { return message }
        let embeds: [ChatEmbed]? = snapshot.embeds.isEmpty ? nil : snapshot.embeds
        guard message.content.embeds != embeds else { return message }
        var result = message
        result.content.embeds = embeds
        return result
    }

    mutating func reset() {
        values.removeAll(keepingCapacity: false)
        order.removeAll(keepingCapacity: false)
    }
}

// MARK: Shared rules

enum LinkEmbeds {
    /// Media of unknown size gets the full width at 16:9 (web `fit`).
    static let maxMediaWidth: Double = 400
    static let maxMediaHeight: Double = 300
    /// The only hosts whose player page an app loads.
    static let playHosts: Set<String> = ["embed.caper.chat", "embed.staging.caper.chat"]

    static func isWebLink(_ value: String) -> Bool {
        let scheme = value.prefix(8).lowercased()
        return scheme.hasPrefix("https://") || scheme.hasPrefix("http://")
    }

    /// The usable cards in a JSON array.
    static func cards(in list: inout UnkeyedDecodingContainer) -> [ChatEmbed] {
        var cards: [ChatEmbed] = []
        while !list.isAtEnd {
            let index = list.currentIndex
            if (try? list.decodeNil()) == true { continue }
            if let entry = try? list.decode(LenientEmbed.self), let card = entry.value { cards.append(card) }
            if list.currentIndex == index { break }
        }
        return cards
    }

    /// The first link in a draft that will get a card, for the composer
    /// preview: the shared link rules, skipping a link written `<like this>`
    /// (web `previewLink`, the API's rule).
    static func previewLink(in draft: String) -> String? {
        let segments = MessageLinks.segments(in: draft)
        for (index, segment) in segments.enumerated() {
            guard let href = segment.href else { continue }
            let opened = index > 0 && segments[index - 1].text.hasSuffix("<")
            let closed = index + 1 < segments.count && segments[index + 1].text.hasPrefix(">")
            if opened && closed { continue }
            return href
        }
        return nil
    }

    /// Display width and aspect ratio that reserve space before media loads:
    /// at most 400 by 300, never enlarged; unknown sizes get 400 at 16:9.
    static func fit(width: Double?, height: Double?) -> (width: Double, aspectRatio: Double) {
        guard let width, let height, width > 0, height > 0 else { return (maxMediaWidth, 16.0 / 9.0) }
        let scale = min(1, maxMediaWidth / width, maxMediaHeight / height)
        return ((width * scale).rounded(), width / height)
    }

    /// A player block's page, only when it is `https` on Caper's embed host
    /// (no credentials or other port). Anything else is never loaded.
    static func playURL(_ value: String?) -> URL? {
        guard let value, let url = URL(string: value), isPlayPage(url) else { return nil }
        return url
    }

    static func isPlayPage(_ url: URL) -> Bool {
        guard url.scheme?.lowercased() == "https", let host = url.host?.lowercased(), playHosts.contains(host),
              url.user == nil, url.password == nil else { return false }
        return url.port == nil || url.port == 443
    }

    /// A link a card may open: http(s) only, like message links.
    static func openableURL(_ value: String?) -> URL? {
        guard let value, isWebLink(value) else { return nil }
        return URL(string: value)
    }

    /// The footer's small line: "309K likes · Mar 21, 2006".
    static func footerLine(_ footer: EmbedFooter) -> String? {
        let date = footer.timestamp.flatMap(timestamp).map { $0.formatted(date: .abbreviated, time: .omitted) }
        let parts = [footer.text, date].compactMap { $0 }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    static func timestamp(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }
}
