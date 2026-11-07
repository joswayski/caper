import Foundation
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
