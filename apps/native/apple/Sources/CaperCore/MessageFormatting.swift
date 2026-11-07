import Foundation
@_implementationOnly import CaperMobile

struct FormattedMessageRun: Equatable, Decodable {
    var text: String
    let bold: Bool
    let italic: Bool
}

/// The style web carries from one typed key to the next one typed at the caret
/// it left (shared/messages/composer.rs). Each composer keeps one.
public struct ComposerTyping {
    private var pending: Int32 = -1
    private var left: (text: String, caret: Int)?
    public init() {}

    /// `MessageFormatting.repairEdit` for an edit that replaced `range` in
    /// `previous`. The last key's style continues only when this edit types at
    /// the caret it left, in the text it left: any edit made elsewhere (a draft
    /// swap, undo, an IME) ends it.
    public mutating func repairEdit(_ previous: String, text: String, selection: NSRange, replacing range: NSRange) -> (text: String, selection: NSRange) {
        let continuing = range.length == 0 && left?.caret == range.location && left?.text == previous ? pending : -1
        let result = MessageFormatting.edit(previous, text: text, selection: selection, continuing: continuing)
        // The caret the composer shows: every composer normalizes it past a
        // closed style's delimiters (and lets the system place it there when
        // the text is unchanged), so the next key arrives at this one.
        let shown = MessageFormatting.normalizedSelection(result.text, selection: result.selection)
        pending = result.pending
        left = shown.length == 0 ? (result.text, shown.location) : nil
        return (result.text, shown)
    }

    /// Call on every selection change: the style ends unless the composer still
    /// shows the text and caret the last edit left (typing's own notifications
    /// do), so a moved caret, undo, redo or an input method ends it.
    /// The system reports its own caret before the composer moves it off
    /// hidden delimiters, so this compares the caret the composer will show.
    public mutating func observe(_ text: String, selection: NSRange) {
        guard let left, left.text == text, selection.length == 0,
              MessageFormatting.normalizedSelection(text, selection: selection).location == left.caret else { return end() }
    }

    /// Ends the style: when the composer loses focus or shows another draft or
    /// conversation, even an identical one.
    public mutating func end() {
        pending = -1
        left = nil
    }
}

/// The deliberately small Markdown subset shared with the web composer.
public enum MessageFormatting {
    static func runs(_ text: String) -> [FormattedMessageRun] {
        projection(text).runs
    }

    struct Projection {
        let runs: [FormattedMessageRun]
        /// Visible UTF-16 code units mapped back to the unmodified wire draft.
        let sourceIndices: [Int]
        /// The wire draft's UTF-16 length.
        let sourceLength: Int

        var text: String { runs.map(\.text).joined() }
        func displayed(_ offset: Int) -> Int { sourceIndices.filter { $0 < offset }.count }
        /// A caret before a visible unit sits just before its source unit; the
        /// end sits after any closing delimiters, so typing on leaves the style.
        func original(_ offset: Int) -> Int {
            offset < sourceIndices.count ? sourceIndices[max(0, offset)] : sourceLength
        }
        func sourceRange(_ range: NSRange) -> NSRange {
            guard range.length > 0 else { return NSRange(location: original(range.location), length: 0) }
            let start = sourceIndices[range.location], end = sourceIndices[NSMaxRange(range) - 1] + 1
            return NSRange(location: start, length: end - start)
        }
        func visibleRange(_ range: NSRange) -> NSRange {
            let start = displayed(range.location), end = displayed(NSMaxRange(range))
            return NSRange(location: start, length: end - start)
        }
    }

    public static func normalizedSelection(_ text: String, selection: NSRange) -> NSRange {
        let parsed = projection(text)
        return parsed.sourceRange(parsed.visibleRange(selection))
    }

    static func copiedText(_ text: String, selection: NSRange) -> String {
        let parsed = projection(text)
        return (parsed.text as NSString).substring(with: parsed.visibleRange(selection))
    }

    /// Native keyboard movement is in visible graphemes, not Markdown code units.
    static func movedSelection(_ text: String, selection: NSRange, backward: Bool, extending: Bool, activeAtStart: Bool = false) -> NSRange {
        let parsed = projection(text), visible = parsed.visibleRange(selection)
        let string = parsed.text as NSString
        let active = activeAtStart ? visible.location : NSMaxRange(visible)
        let anchor = activeAtStart ? NSMaxRange(visible) : visible.location
        let next: Int
        if visible.length > 0 && !extending { next = backward ? visible.location : NSMaxRange(visible) }
        else if backward { next = active > 0 ? string.rangeOfComposedCharacterSequence(at: active - 1).location : 0 }
        else { next = active < string.length ? NSMaxRange(string.rangeOfComposedCharacterSequence(at: active)) : string.length }
        return parsed.sourceRange(extending ? NSRange(location: min(anchor, next), length: abs(anchor - next)) : NSRange(location: next, length: 0))
    }

    static func deletionRange(_ text: String, selection: NSRange, backward: Bool) -> NSRange? {
        let parsed = projection(text), visible = parsed.visibleRange(selection)
        if visible.length > 0 { return parsed.sourceRange(visible) }
        let string = parsed.text as NSString
        if backward {
            guard visible.location > 0 else { return nil }
            return parsed.sourceRange(string.rangeOfComposedCharacterSequence(at: visible.location - 1))
        }
        guard visible.location < string.length else { return nil }
        return parsed.sourceRange(string.rangeOfComposedCharacterSequence(at: visible.location))
    }

    /// Parsed by the shared Rust grammar (shared/messages/formatting.rs), which
    /// returns each visible span's UTF-16 and UTF-8 offsets.
    static func projection(_ text: String) -> Projection {
        var source = text
        return source.withUTF8 { bytes in
            let spans = caper_mobile_format_spans(bytes.baseAddress, bytes.count)
            defer { caper_mobile_ints_free(spans) }
            var result: [FormattedMessageRun] = []
            var sourceIndices: [Int] = []
            for span in stride(from: 0, to: spans.len, by: 5) {
                let values = spans.data.advanced(by: span)
                let value = String(decoding: UnsafeBufferPointer(rebasing: bytes[Int(values[2])..<Int(values[3])]), as: UTF8.self)
                let bold = values[4] & 1 != 0, italic = values[4] & 2 != 0
                sourceIndices.append(contentsOf: Int(values[0])..<Int(values[1]))
                if let last = result.last, last.bold == bold, last.italic == italic { result[result.count - 1].text += value }
                else { result.append(.init(text: value, bold: bold, italic: italic)) }
            }
            return Projection(runs: result, sourceIndices: sourceIndices, sourceLength: text.utf16.count)
        }
    }

    /// One composer edit without a style carried from the previous key:
    /// autocomplete and other replacements. Composers use `ComposerTyping`.
    public static func repairEdit(_ previous: String, text: String, selection: NSRange) -> (text: String, selection: NSRange) {
        let result = edit(previous, text: text, selection: selection, continuing: -1)
        return (result.text, result.selection)
    }

    /// One composer edit as web makes it (shared/messages/composer.rs, through
    /// Rust). `continuing` is the previous edit's `pending` when this one types
    /// at the caret it left, otherwise -1.
    static func edit(_ previous: String, text: String, selection: NSRange, continuing: Int32) -> (text: String, selection: NSRange, pending: Int32) {
        guard previous != text else { return (text, selection, -1) }
        var old = previous, new = text
        return old.withUTF8 { oldBytes in
            new.withUTF8 { newBytes in
                let result = caper_mobile_composer_edit(oldBytes.baseAddress, oldBytes.count, newBytes.baseAddress, newBytes.count,
                                                        Int32(selection.location), Int32(NSMaxRange(selection)), continuing)
                defer { caper_mobile_ints_free(result) }
                let units = UnsafeBufferPointer(start: result.data.advanced(by: 3), count: result.len - 3).map { UInt16(truncatingIfNeeded: $0) }
                let start = Int(result.data[0]), end = Int(result.data[1])
                return (String(decoding: units, as: UTF16.self), NSRange(location: start, length: end - start), result.data[2])
            }
        }
    }

    public static func toggle(_ text: String, selection: NSRange, marker: String) -> (text: String, selection: NSRange)? {
        let source = text as NSString, width = (marker as NSString).length
        guard ["**", "_"].contains(marker), selection.location >= 0, selection.location <= source.length,
              selection.length >= 0, selection.length <= source.length - selection.location else { return nil }
        guard selection.length > 0 else { return (text, selection) }
        var start = selection.location, end = NSMaxRange(selection)
        var offset = 0
        for segment in MessageLinks.segments(in: text) {
            let limit = offset + segment.text.utf16.count
            if segment.href != nil, (start > offset && start < limit) || (end > offset && end < limit) {
                return (text, selection)
            }
            offset = limit
        }
        if start != end {
            let whitespace = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{feff}")).subtracting(CharacterSet(charactersIn: "\u{85}"))
            while start < end, let scalar = UnicodeScalar(source.character(at: start)), whitespace.contains(scalar) { start += 1 }
            while end > start, let scalar = UnicodeScalar(source.character(at: end - 1)), whitespace.contains(scalar) { end -= 1 }
            if start == end { return (text, NSRange(location: start, length: 0)) }
        }
        if start != end {
            let visible = projection(text).sourceIndices.filter { $0 >= start && $0 < end }
            if let first = visible.first, let last = visible.last, first >= width, last + 1 + width <= source.length,
               source.substring(with: NSRange(location: first - width, length: width)) == marker,
               source.substring(with: NSRange(location: last + 1, length: width)) == marker,
               runs(source.substring(with: NSRange(location: first - width, length: last + 1 - first + 2 * width))).allSatisfy({ marker == "**" ? $0.bold : $0.italic }) {
                start = first; end = last + 1
            }
        }
        let selection = NSRange(location: start, length: end - start)
        var selected = source.substring(with: selection)
        if start >= width, end + width <= source.length,
           source.substring(with: NSRange(location: start - width, length: width)) == marker,
           source.substring(with: NSRange(location: end, length: width)) == marker {
            return (source.replacingCharacters(in: NSRange(location: start - width, length: selection.length + 2 * width), with: selected), NSRange(location: start - width, length: selection.length))
        }
        let parsed = runs(selected)
        func hasStyle(_ run: FormattedMessageRun) -> Bool { marker == "**" ? run.bold : run.italic }
        if selection.length >= 2 * width, selected.hasPrefix(marker), selected.hasSuffix(marker), parsed.allSatisfy(hasStyle) {
            return (source.replacingCharacters(in: selection, with: (selected as NSString).substring(with: NSRange(location: width, length: selection.length - 2 * width))), NSRange(location: start, length: selection.length - 2 * width))
        }
        if parsed.contains(where: hasStyle) {
            var remaining: [(text: String, styled: Bool)] = []
            for run in parsed {
                let styled = marker == "**" ? run.italic : run.bold
                if remaining.last?.styled == styled { remaining[remaining.count - 1].text += run.text }
                else { remaining.append((run.text, styled)) }
            }
            let other = marker == "**" ? "_" : "**"
            selected = remaining.map { $0.styled ? other + $0.text + other : $0.text }.joined()
        }
        let result = source.replacingCharacters(in: selection, with: marker + selected + marker)
        guard start == end || runs(result).map(\.text).joined() == runs(text).map(\.text).joined() else { return nil }
        return (result, NSRange(location: start + width, length: selected.utf16.count))
    }
}
