import Foundation

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
/// for bare URLs without changing how old messages look. Mirrors
/// apps/web/src/chat/links.ts; MessageLinksTests runs every case in
/// shared/messages/link-cases.json. The text is scanned by Unicode scalar:
/// the rules only name characters that are one scalar (and one UTF-16 unit).
enum MessageLinks {
    /// Unicode White_Space; links end at the first of these or `<`.
    static func isWhitespace(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x09...0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000: return true
        default: return false
        }
    }

    /// A link may start at the beginning, after whitespace, or after one of these.
    private static let openers: Set<Unicode.Scalar> = ["(", "[", "{", "<", "\"", "'", "*", "_", "~"]
    private static let trailing: Set<Unicode.Scalar> = ["?", "!", ".", ",", ":", ";", "*", "_", "~", "\"", "'", ">"]

    /// Splits plain text into text runs and `http(s)://` / `www.` links.
    static func segments(in text: String) -> [LinkSegment] {
        let scalars = Array(text.unicodeScalars)
        var segments: [LinkSegment] = []
        var plainStart = 0
        var index = 0
        while index < scalars.count {
            let canStart = index == 0 || isWhitespace(scalars[index - 1]) || openers.contains(scalars[index - 1])
            guard canStart, let scheme = schemeLength(scalars, at: index) else {
                index += 1
                continue
            }
            var end = index
            while end < scalars.count, !isWhitespace(scalars[end]), scalars[end] != "<" { end += 1 }
            let candidate = trim(Array(scalars[index..<end]))
            var hostEnd = min(scheme, candidate.count)
            while hostEnd < candidate.count, isHostScalar(candidate[hostEnd]) { hostEnd += 1 }
            guard scheme <= candidate.count, validHost(candidate[scheme..<hostEnd], www: scheme == 0) else {
                index += 1
                continue
            }
            if index > plainStart { segments.append(LinkSegment(text: string(scalars[plainStart..<index]))) }
            let visible = string(candidate[...])
            // `www.` opens as https; otherwise only the scheme is lowercased.
            let href = scheme == 0 ? "https://" + visible
                : (scheme == 8 ? "https://" : "http://") + string(candidate[scheme...])
            segments.append(LinkSegment(text: visible, href: href))
            index += candidate.count
            plainStart = index
        }
        if plainStart < scalars.count { segments.append(LinkSegment(text: string(scalars[plainStart...]))) }
        return segments
    }

    /// The scheme's length when a link candidate starts here (`www.` counts
    /// as 0, its host starting at the `w`), compared ASCII case-insensitively.
    private static func schemeLength(_ scalars: [Unicode.Scalar], at index: Int) -> Int? {
        if hasPrefix(scalars, at: index, "https://") { return 8 }
        if hasPrefix(scalars, at: index, "http://") { return 7 }
        if hasPrefix(scalars, at: index, "www.") { return 0 }
        return nil
    }

    private static func hasPrefix(_ scalars: [Unicode.Scalar], at index: Int, _ prefix: String) -> Bool {
        var position = index
        for expected in prefix.unicodeScalars {
            guard position < scalars.count, asciiLowercased(scalars[position]) == expected else { return false }
            position += 1
        }
        return true
    }

    private static func asciiLowercased(_ scalar: Unicode.Scalar) -> Unicode.Scalar {
        guard (65...90).contains(scalar.value) else { return scalar }
        return Unicode.Scalar(UInt8(scalar.value + 32))
    }

    private static func isASCIIAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 48...57, 65...90, 97...122: return true
        default: return false
        }
    }

    /// Hosts are ASCII letters, digits, `_`, `.` and `-`.
    private static func isHostScalar(_ scalar: Unicode.Scalar) -> Bool {
        isASCIIAlphanumeric(scalar) || scalar == "_" || scalar == "." || scalar == "-"
    }

    /// Drops trailing punctuation, unbalanced closers and a trailing `&entity;`.
    private static func trim(_ candidate: [Unicode.Scalar]) -> [Unicode.Scalar] {
        var value = candidate
        while let last = value.last {
            if trailing.contains(last) {
                if last == ";", let entity = entityStart(value) { value.removeSubrange(entity...) }
                else { value.removeLast() }
                continue
            }
            if last == ")" && count(")", in: value) > count("(", in: value) {
                value.removeLast()
                continue
            }
            if last == "]" && count("]", in: value) > count("[", in: value) {
                value.removeLast()
                continue
            }
            break
        }
        return value
    }

    /// Where a final `&` + ASCII letters or digits + `;` begins, if the value ends with one.
    private static func entityStart(_ value: [Unicode.Scalar]) -> Int? {
        let semicolon = value.count - 1
        var index = semicolon - 1
        while index >= 0, isASCIIAlphanumeric(value[index]) { index -= 1 }
        guard index >= 0, index < semicolon - 1, value[index] == "&" else { return nil }
        return index
    }

    private static func count(_ scalar: Unicode.Scalar, in value: [Unicode.Scalar]) -> Int {
        value.reduce(0) { $1 == scalar ? $0 + 1 : $0 }
    }

    /// At least two non-empty labels (three for `www.`), no `_` in the last two.
    private static func validHost(_ host: ArraySlice<Unicode.Scalar>, www: Bool) -> Bool {
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count >= (www ? 3 : 2), !labels.contains(where: { $0.isEmpty }) else { return false }
        return !labels.suffix(2).contains { $0.contains("_") }
    }

    private static func string(_ scalars: ArraySlice<Unicode.Scalar>) -> String {
        var result = ""
        result.unicodeScalars.append(contentsOf: scalars)
        return result
    }
}
