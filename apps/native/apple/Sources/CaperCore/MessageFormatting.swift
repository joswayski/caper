import Foundation

struct FormattedMessageRun: Equatable, Decodable {
    var text: String
    let bold: Bool
    let italic: Bool
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

    static func projection(_ text: String) -> Projection {
        let units = Array(text.utf16)
        var links: [Range<Int>] = []
        var offset = 0
        for segment in MessageLinks.segments(in: text) {
            let end = offset + segment.text.utf16.count
            if segment.href != nil { links.append(offset..<end) }
            offset = end
        }
        func at(_ index: Int) -> UInt16? { units.indices.contains(index) ? units[index] : nil }
        func word(_ value: UInt16?) -> Bool {
            guard let value else { return false }
            return (65...90).contains(value) || (97...122).contains(value) || (48...57).contains(value) || value == 95 || value == 64
        }
        func space(_ value: UInt16?) -> Bool {
            guard let value else { return false }
            // ECMAScript \s (unlike Unicode White_Space, includes BOM, excludes NEL).
            return (9...13).contains(value) || [32, 160, 5760, 8232, 8233, 8239, 8287, 12288, 65279].contains(value) || (8192...8202).contains(value)
        }
        func markerAt(_ index: Int, _ marker: [UInt16]) -> Bool {
            guard index + marker.count <= units.count, Array(units[index..<(index + marker.count)]) == marker,
                  !links.contains(where: { $0.contains(index) }) else { return false }
            var slash = index - 1
            while at(slash) == 92 { slash -= 1 }
            guard (index - 1 - slash) % 2 == 0 else { return false }
            return marker.count != 2 || (at(index - 1) != 42 && at(index + 2) != 42)
        }
        var result: [FormattedMessageRun] = []
        var sourceIndices: [Int] = []
        func append(_ start: Int, _ end: Int, _ bold: Bool, _ italic: Bool) {
            guard end > start else { return }
            sourceIndices.append(contentsOf: start..<end)
            let value = String(decoding: units[start..<end], as: UTF16.self)
            if let last = result.last, last.bold == bold, last.italic == italic { result[result.count - 1].text += value }
            else { result.append(.init(text: value, bold: bold, italic: italic)) }
        }
        func parse(_ start: Int, _ end: Int, _ bold: Bool, _ italic: Bool) {
            var plain = start, index = start
            while index < end {
                let marker: [UInt16] = !bold && markerAt(index, [42, 42]) ? [42, 42] : !italic && markerAt(index, [95]) ? [95] : []
                if !marker.isEmpty, !(marker.count == 1 && word(at(index - 1))) {
                    let inner = index + marker.count
                    if inner < end, !space(at(inner)) {
                        var close = inner
                        while close + marker.count <= end {
                            if close != inner, markerAt(close, marker), !space(at(close - 1)), !(marker.count == 1 && word(at(close + 1))) {
                                append(plain, index, bold, italic)
                                parse(inner, close, bold || marker.count == 2, italic || marker.count == 1)
                                index = close + marker.count - 1; plain = index + 1
                                break
                            }
                            close += 1
                        }
                    }
                }
                index += 1
            }
            append(plain, end, bold, italic)
        }
        parse(0, units.count, false, false)
        return Projection(runs: result, sourceIndices: sourceIndices, sourceLength: units.count)
    }

    /// Edits may erase one side or all contents of a style. Drop only punctuation
    /// that was hidden before the edit and would otherwise become an orphan marker.
    public static func repairEdit(_ previous: String, text: String, selection: NSRange) -> (text: String, selection: NSRange) {
        guard previous != text else { return (text, selection) }
        let before = Array(previous.utf16), after = Array(text.utf16)
        let oldVisible = Set(projection(previous).sourceIndices)
        guard oldVisible.count != before.count else { return (text, selection) }
        var prefix = 0, suffix = 0
        while prefix < min(before.count, after.count), before[prefix] == after[prefix] { prefix += 1 }
        while suffix < min(before.count, after.count) - prefix,
              before[before.count - suffix - 1] == after[after.count - suffix - 1] { suffix += 1 }
        let newVisible = Set(projection(text).sourceIndices)
        let removed = Set(before.indices.filter { !oldVisible.contains($0) }.compactMap { index -> Int? in
            let mapped: Int
            if index < prefix { mapped = index }
            else if index >= before.count - suffix { mapped = index + after.count - before.count }
            else { return nil }
            return newVisible.contains(mapped) ? mapped : nil
        })
        guard !removed.isEmpty else { return (text, selection) }
        func offset(_ value: Int) -> Int { value - removed.filter { $0 < value }.count }
        let start = offset(selection.location), end = offset(NSMaxRange(selection))
        return repairEdit(previous, text: String(decoding: after.enumerated().filter { !removed.contains($0.offset) }.map(\.element), as: UTF16.self),
                          selection: NSRange(location: start, length: end - start))
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
