/// The API's name-character rule (apps/api/src/names.rs): names (display and
/// space names) keep visible text but not control characters, Unicode
/// Default_Ignorable_Code_Point characters (bidi controls, zero-width
/// characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji
/// keep their parts: a lone ZWJ/ZWNJ between visible characters, one variation
/// selector after a visible character, and tags inside a subdivision flag.
/// Message text is not checked by this rule.
public enum NameCharacters {
    private static let flag: UInt32 = 0x1F3F4
    private static let cancelTag: UInt32 = 0xE007F
    private static let hiddenRanges: [ClosedRange<UInt32>] = [
        0x0000...0x001F, 0x007F...0x009F, 0x00AD...0x00AD, 0x034F...0x034F, 0x061C...0x061C, 0x115F...0x1160,
        0x17B4...0x17B5, 0x180B...0x180F, 0x200B...0x200F, 0x202A...0x202E, 0x2060...0x206F, 0x2800...0x2800,
        0x3164...0x3164, 0xFE00...0xFE0F, 0xFEFF...0xFEFF, 0xFFA0...0xFFA0, 0xFFF0...0xFFF8, 0x1BCA0...0x1BCA3,
        0x1D173...0x1D17A, 0xE0000...0xE0FFF,
    ]

    private static func joiner(_ c: UInt32) -> Bool { c == 0x200C || c == 0x200D }
    private static func selector(_ c: UInt32) -> Bool {
        (0x180B...0x180D).contains(c) || c == 0x180F || (0xFE00...0xFE0F).contains(c) || (0xE0100...0xE01EF).contains(c)
    }
    private static func tag(_ c: UInt32) -> Bool { (0xE0020...0xE007E).contains(c) }
    private static func hidden(_ c: UInt32) -> Bool { hiddenRanges.contains { $0.contains(c) } }
    /// Unicode White_Space (Rust's `char::is_whitespace`) outside the controls above.
    private static func space(_ c: UInt32) -> Bool {
        c == 0x20 || c == 0xA0 || c == 0x1680 || (0x2000...0x200A).contains(c) || c == 0x2028 || c == 0x2029
            || c == 0x202F || c == 0x205F || c == 0x3000
    }

    private enum Last { case start, space, visible, flag, selector }

    /// Whether `value` holds a character a name may not (see above).
    public static func hasHidden(_ value: String) -> Bool {
        var last = Last.start
        var joined = false
        var tags = 0
        for scalar in value.unicodeScalars {
            let c = scalar.value
            if tags > 0 && !tag(c) && c != cancelTag { return true }
            if joined && (joiner(c) || selector(c) || tag(c) || c == cancelTag || hidden(c) || space(c)) { return true }
            joined = false
            if tag(c) {
                if last != .flag && tags == 0 { return true }
                tags += 1
            } else if c == cancelTag {
                if tags == 0 { return true }
                tags = 0
                last = .visible
            } else if joiner(c) {
                if last != .visible && last != .flag && last != .selector { return true }
                joined = true
            } else if selector(c) {
                if last != .visible && last != .flag { return true }
                last = .selector
            } else if hidden(c) {
                return true
            } else if space(c) {
                last = .space
            } else {
                last = c == flag ? .flag : .visible
            }
        }
        return joined || tags > 0
    }
}
