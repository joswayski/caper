import Foundation
@_implementationOnly import CaperMobile

/// The API's name-character rule (apps/api/src/names.rs), run by the shared
/// Rust copy (apps/native/client/src/names.rs): names (display and space
/// names) keep visible text but not control characters, Unicode
/// Default_Ignorable_Code_Point characters (bidi controls, zero-width
/// characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji
/// keep their parts: a lone ZWJ/ZWNJ between visible characters, one variation
/// selector after a visible character, and tags inside a subdivision flag.
/// Message text is not checked by this rule.
public enum NameCharacters {
    /// Whether `value` holds a character a name may not (shared/names/cases.json).
    public static func hasHidden(_ value: String) -> Bool {
        var value = value
        return value.withUTF8 { caper_mobile_name_has_hidden($0.baseAddress, $0.count) }
    }
}
