import Foundation
@_implementationOnly import CaperMobile

/// Style bits of formatted message text (shared/messages/formatting.rs).
public struct MessageStyle: OptionSet, Sendable {
    public let rawValue: Int32
    public init(rawValue: Int32) { self.rawValue = rawValue }
    public static let bold = MessageStyle(rawValue: 1)
    public static let italic = MessageStyle(rawValue: 2)
    public static let underline = MessageStyle(rawValue: 4)
    public static let strikethrough = MessageStyle(rawValue: 8)
    public static let spoiler = MessageStyle(rawValue: 16)
    public static let quote = MessageStyle(rawValue: 32)
}

/// One button of the formatting toolbar (and the composer menus), in web's order.
public struct MessageFormattingAction: Sendable {
    public let style: MessageStyle
    /// The accessible name.
    public let title: String
    /// SF Symbol, matching web's Lucide icon.
    public let symbol: String
    /// The lowercase Command shortcut, if any.
    public let key: String?
    /// Quote and Spoiler follow the other styles after a divider.
    public var block: Bool { style == .quote || style == .spoiler }

    public static let all: [MessageFormattingAction] = [
        .init(style: .bold, title: "Bold", symbol: "bold", key: "b"),
        .init(style: .italic, title: "Italic", symbol: "italic", key: "i"),
        .init(style: .underline, title: "Underline", symbol: "underline", key: "u"),
        .init(style: .strikethrough, title: "Strikethrough", symbol: "strikethrough", key: nil),
        .init(style: .quote, title: "Quote", symbol: "text.quote", key: nil),
        .init(style: .spoiler, title: "Spoiler", symbol: "eye.slash", key: nil),
    ]

    /// The action for a Command shortcut (B, I or U).
    public static func shortcut(_ key: String) -> MessageFormattingAction? {
        all.first { $0.key == key.lowercased() }
    }
}

struct FormattedMessageRun: Equatable, Decodable {
    var text: String
    let style: MessageStyle
    var bold: Bool { style.contains(.bold) }
    var italic: Bool { style.contains(.italic) }
    var underline: Bool { style.contains(.underline) }
    var strikethrough: Bool { style.contains(.strikethrough) }
    var spoiler: Bool { style.contains(.spoiler) }
    var quote: Bool { style.contains(.quote) }

    init(text: String, style: MessageStyle) {
        self.text = text
        self.style = style
    }

    private enum CodingKeys: String, CodingKey { case text, bold, italic, underline, strikethrough, spoiler, quote }

    /// The shared fixtures list `bold` and `italic`, and the other styles only when true.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        text = try container.decode(String.self, forKey: .text)
        var style: MessageStyle = []
        let keys: [(CodingKeys, MessageStyle)] = [(.bold, .bold), (.italic, .italic), (.underline, .underline),
                                                  (.strikethrough, .strikethrough), (.spoiler, .spoiler), (.quote, .quote)]
        for (key, bit) in keys {
            if try container.decodeIfPresent(Bool.self, forKey: key) == true { style.insert(bit) }
        }
        self.style = style
    }
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
                let style = MessageStyle(rawValue: values[4])
                sourceIndices.append(contentsOf: Int(values[0])..<Int(values[1]))
                if let last = result.last, last.style == style { result[result.count - 1].text += value }
                else { result.append(.init(text: value, style: style)) }
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

    /// Displayed-text UTF-16 ranges of quoted and unquoted runs, in order (web's quoteBlocks).
    static func quoteBlocks(_ runs: [FormattedMessageRun]) -> [(range: Range<Int>, quote: Bool)] {
        var blocks: [(range: Range<Int>, quote: Bool)] = []
        var offset = 0
        for run in runs {
            let end = offset + run.text.utf16.count
            if let last = blocks.last, last.quote == run.quote { blocks[blocks.count - 1].range = last.range.lowerBound..<end }
            else { blocks.append((offset..<end, run.quote)) }
            offset = end
        }
        return blocks
    }

    /// Whether a source selection is a valid UTF-16 range of `text`.
    private static func valid(_ selection: NSRange, in text: String) -> Bool {
        selection.location != NSNotFound && selection.location >= 0 && selection.length >= 0
            && selection.length <= text.utf16.count - selection.location
    }

    /// Turns `style` on or off for the selection as web's toolbar does, through
    /// the shared composer (shared/messages/composer.rs `toggle`): the new
    /// draft and its selection, or nil when the selection cannot take that
    /// style (an empty selection, part of a link, part of a word for italic or
    /// underline, or a change that would not survive its own Markdown).
    public static func toggle(_ text: String, selection: NSRange, style: MessageStyle) -> (text: String, selection: NSRange)? {
        guard valid(selection, in: text) else { return nil }
        var source = text
        return source.withUTF8 { bytes -> (text: String, selection: NSRange)? in
            let result = caper_mobile_composer_toggle(bytes.baseAddress, bytes.count, Int32(selection.location),
                                                      Int32(NSMaxRange(selection)), style.rawValue)
            defer { caper_mobile_ints_free(result) }
            guard result.len >= 2 else { return nil }
            let units = UnsafeBufferPointer(start: result.data.advanced(by: 2), count: result.len - 2).map { UInt16(truncatingIfNeeded: $0) }
            let start = Int(result.data[0]), end = Int(result.data[1])
            return (String(decoding: units, as: UTF16.self), NSRange(location: start, length: end - start))
        }
    }

    /// The styles any visible character of the source selection has: the
    /// pressed toolbar buttons (shared/messages/composer.rs `selection_styles`).
    public static func selectionStyles(_ text: String, selection: NSRange) -> MessageStyle {
        guard selection.length > 0, valid(selection, in: text) else { return [] }
        var source = text
        let bits = source.withUTF8 { bytes in
            caper_mobile_selection_styles(bytes.baseAddress, bytes.count, Int32(selection.location), Int32(NSMaxRange(selection)))
        }
        return MessageStyle(rawValue: bits)
    }
}
