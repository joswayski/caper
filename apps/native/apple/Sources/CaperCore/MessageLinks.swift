import Foundation
import Observation
@_implementationOnly import CaperMobile

/// One run of message text: plain text, or a link with the address it opens.
struct LinkSegment: Equatable {
    let text: String
    /// Always `http://` or `https://`; nil for plain text.
    var href: String? = nil
}

/// Plain-text link detection for message text, shared by every client.
///
/// Messages stay plain text on the wire; links are found at render time, on
/// the text between mention pills. The rules follow GitHub Flavored
/// Markdown's autolink literals, so a later Markdown renderer can call this
/// for bare URLs without changing how old messages look. They run in Rust
/// (shared/messages/links.rs, as apps/web/src/chat/links.ts);
/// MessageLinksTests runs every case in shared/messages/link-cases.json.
enum MessageLinks {
    /// Splits plain text into text runs and `http(s)://` / `www.` links.
    static func segments(in text: String) -> [LinkSegment] {
        var source = text
        return source.withUTF8 { bytes in
            let spans = caper_mobile_link_spans(bytes.baseAddress, bytes.count)
            defer { caper_mobile_ints_free(spans) }
            func string(_ start: Int, _ end: Int) -> String {
                String(decoding: UnsafeBufferPointer(rebasing: bytes[start..<end]), as: UTF8.self)
            }
            var segments: [LinkSegment] = []
            var plainStart = 0
            for span in stride(from: 0, to: spans.len, by: 5) {
                let values = spans.data.advanced(by: span)
                let start = Int(values[2]), end = Int(values[3]), scheme = Int(values[4])
                if start > plainStart { segments.append(LinkSegment(text: string(plainStart, start))) }
                let visible = string(start, end)
                // `www.` (scheme 0) opens as https; otherwise only the scheme is lowercased.
                let href = scheme == 0 ? "https://" + visible
                    : (scheme == 8 ? "https://" : "http://") + string(start + scheme, end)
                segments.append(LinkSegment(text: visible, href: href))
                plainStart = end
            }
            if plainStart < bytes.count { segments.append(LinkSegment(text: string(plainStart, bytes.count))) }
            return segments
        }
    }
}

/// Links to one message: `https://caper.chat/m/<channel>/<message>` (a
/// message's Copy link) and `caper://m/<channel>/<message>`, through shared
/// Rust (shared/messages/message_links.rs). Only caper.chat hosts count, so
/// another site's `/m/...` stays an ordinary link. Every client runs
/// shared/messages/message-link-cases.json.
public struct MessageLinkTarget: Equatable, Sendable {
    public let channelID: String
    public let messageID: String

    public init?(_ input: String) {
        var source = input
        let value: String = source.withUTF8 { bytes in
            let link = caper_mobile_message_link(bytes.baseAddress, bytes.count)
            defer { caper_mobile_ints_free(link) }
            return String(decoding: UnsafeBufferPointer(start: link.data, count: link.len).map { UInt8(truncatingIfNeeded: $0) }, as: UTF8.self)
        }
        let parts = value.split(separator: "/")
        guard parts.count == 2 else { return nil }
        channelID = String(parts[0]); messageID = String(parts[1])
    }

    public init?(_ url: URL) { self.init(url.absoluteString) }

    /// The link Copy link puts on the clipboard.
    public static func url(channelID: String, messageID: String) -> String {
        "https://caper.chat/m/\(channelID)/\(messageID)"
    }
}

/// Live cards for the message links in message text, as web's
/// `MessageLinkCards`: up to five distinct links per message, each resolved
/// through `GET /api/chat/message-links/{channel}/{message}`, which answers
/// only for readers who can see the original. Until the first answer, or
/// when it fails, no card shows and the link stays plain text. A link still
/// being shown asks again every `refresh`, so edits, reactions and deletions
/// follow the original. `AppModel` supplies `resolve` and `open`.
@MainActor @Observable public final class MessageLinkCards {
    public static let shared = MessageLinkCards()
    nonisolated public static let maximum = 5
    static let refresh: TimeInterval = 30

    public enum Card: Equatable, Sendable {
        /// The API answered that this account can't see it.
        case unavailable
        case original(ChatMessage)
    }

    /// One link and its answer.
    public struct Linked: Identifiable, Sendable {
        public let target: MessageLinkTarget
        public let card: Card
        public var id: String { target.messageID }
    }

    /// Advanced whenever an answer changes a card.
    public private(set) var revision = 0
    private var cards: [String: Card] = [:]
    @ObservationIgnored private var asked: [String: Date] = [:]
    @ObservationIgnored private var pending: Set<String> = []
    @ObservationIgnored private var shown: [String: Date] = [:]
    @ObservationIgnored private var generation = 0
    @ObservationIgnored var resolve: ((MessageLinkTarget) async throws -> ForwardSource?)?
    @ObservationIgnored var open: ((URL) -> Bool)?

    /// The distinct message links in `text`, at most `maximum`.
    nonisolated public static func targets(in text: String) -> [MessageLinkTarget] {
        guard text.contains("/m/") else { return [] }
        var found: [MessageLinkTarget] = []
        for href in MessageLinks.segments(in: text).compactMap(\.href) {
            guard let target = MessageLinkTarget(href), !found.contains(where: { $0.messageID == target.messageID }) else { continue }
            found.append(target)
            if found.count == maximum { break }
        }
        return found
    }

    nonisolated public static func hasLinks(_ message: ChatMessage) -> Bool {
        !message.isDeleted && !targets(in: message.content.text).isEmpty
    }

    /// The answered cards for `message`'s links, in order; asks about any
    /// that are due. Reading it observes the answers.
    public func cards(for message: ChatMessage) -> [Linked] {
        guard !message.isDeleted else { return [] }
        let targets = Self.targets(in: message.content.text)
        let now = Date()
        for target in targets { shown[target.messageID] = now; ask(target, now: now) }
        return targets.compactMap { target in cards[target.messageID].map { Linked(target: target, card: $0) } }
    }

    /// Jump to message: opens it where it lives, as an opened link does.
    public func jump(_ target: MessageLinkTarget) {
        if let url = URL(string: MessageLinkTarget.url(channelID: target.channelID, messageID: target.messageID)) { _ = open?(url) }
    }

    /// Signed out or another account: nothing carries over.
    public func clear() {
        generation += 1
        cards = [:]; asked = [:]; pending = []; shown = [:]; revision += 1
    }

    private func ask(_ target: MessageLinkTarget, now: Date) {
        let id = target.messageID
        guard let resolve, !pending.contains(id), asked[id].map({ now.timeIntervalSince($0) >= Self.refresh }) ?? true else { return }
        pending.insert(id); asked[id] = now
        let attempt = generation
        Task { [weak self] in
            var answer: Card?
            do {
                if let source = try await resolve(target) { answer = .original(source.message) } else { answer = .unavailable }
            } catch { answer = nil } // Asked again at the next refresh; the last answer stays.
            guard let self, self.generation == attempt else { return }
            self.pending.remove(id)
            if let answer, self.cards[id] != answer { self.cards[id] = answer; self.revision += 1 }
            // Shown cards keep following the original while they are on screen.
            try? await Task.sleep(for: .seconds(Self.refresh))
            guard self.generation == attempt, let seen = self.shown[id], Date().timeIntervalSince(seen) < 10 * 60 else { return }
            self.ask(target, now: Date())
        }
    }
}
