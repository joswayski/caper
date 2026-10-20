import Foundation

struct FormattedMessageRun: Equatable, Decodable {
    var text: String
    let bold: Bool
    let italic: Bool
}

/// The deliberately small Markdown subset shared with the web composer.
enum MessageFormatting {
    static func runs(_ text: String) -> [FormattedMessageRun] {
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
        func append(_ start: Int, _ end: Int, _ bold: Bool, _ italic: Bool) {
            guard end > start else { return }
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
        return result
    }

    static func toggle(_ text: String, selection: NSRange, marker: String) -> (text: String, selection: NSRange)? {
        let source = text as NSString, width = (marker as NSString).length
        guard ["**", "_"].contains(marker), selection.location >= 0, selection.location <= source.length,
              selection.length >= 0, selection.length <= source.length - selection.location else { return nil }
        var start = selection.location, end = NSMaxRange(selection)
        if start != end {
            let whitespace = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{feff}")).subtracting(CharacterSet(charactersIn: "\u{85}"))
            while start < end, let scalar = UnicodeScalar(source.character(at: start)), whitespace.contains(scalar) { start += 1 }
            while end > start, let scalar = UnicodeScalar(source.character(at: end - 1)), whitespace.contains(scalar) { end -= 1 }
            if start == end { return (text, NSRange(location: start, length: 0)) }
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
        return (source.replacingCharacters(in: selection, with: marker + selected + marker), NSRange(location: start + width, length: selected.utf16.count))
    }
}
