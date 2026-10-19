package chat.caper.android.model

/*
 * The API's name-character rule (apps/api/src/names.rs): names (display and space names) keep
 * visible text but not control characters, Unicode Default_Ignorable_Code_Point characters (bidi
 * controls, zero-width characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji
 * keep their parts: a lone ZWJ/ZWNJ between visible characters, one variation selector after a
 * visible character, and tags inside a subdivision flag. Message text is not checked by this rule.
 */

private const val FLAG = 0x1F3F4
private const val CANCEL_TAG = 0xE007F

private fun joiner(c: Int) = c == 0x200C || c == 0x200D
private fun selector(c: Int) = c in 0x180B..0x180D || c == 0x180F || c in 0xFE00..0xFE0F || c in 0xE0100..0xE01EF
private fun tag(c: Int) = c in 0xE0020..0xE007E
private val HIDDEN = listOf(
    0x0000..0x001F, 0x007F..0x009F, 0x00AD..0x00AD, 0x034F..0x034F, 0x061C..0x061C, 0x115F..0x1160,
    0x17B4..0x17B5, 0x180B..0x180F, 0x200B..0x200F, 0x202A..0x202E, 0x2060..0x206F, 0x2800..0x2800,
    0x3164..0x3164, 0xFE00..0xFE0F, 0xFEFF..0xFEFF, 0xFFA0..0xFFA0, 0xFFF0..0xFFF8, 0x1BCA0..0x1BCA3,
    0x1D173..0x1D17A, 0xE0000..0xE0FFF,
)
private fun hidden(c: Int) = HIDDEN.any { c in it }
/** Unicode White_Space (Rust's `char::is_whitespace`) outside the controls above; Java's own test differs. */
private fun space(c: Int) = c == 0x20 || c == 0xA0 || c == 0x1680 || c in 0x2000..0x200A || c == 0x2028 || c == 0x2029 ||
    c == 0x202F || c == 0x205F || c == 0x3000

private enum class Last { Start, Space, Visible, Flag, Selector }

/** Whether [value] holds a character a name may not (see above). */
fun hasHiddenNameCharacters(value: String): Boolean {
    var last = Last.Start
    var joined = false
    var tags = 0
    var index = 0
    while (index < value.length) {
        val c = value.codePointAt(index)
        index += Character.charCount(c)
        if (tags > 0 && !tag(c) && c != CANCEL_TAG) return true
        if (joined && (joiner(c) || selector(c) || tag(c) || c == CANCEL_TAG || hidden(c) || space(c))) return true
        joined = false
        when {
            tag(c) -> { if (last != Last.Flag && tags == 0) return true; tags += 1 }
            c == CANCEL_TAG -> { if (tags == 0) return true; tags = 0; last = Last.Visible }
            joiner(c) -> { if (last != Last.Visible && last != Last.Flag && last != Last.Selector) return true; joined = true }
            selector(c) -> { if (last != Last.Visible && last != Last.Flag) return true; last = Last.Selector }
            hidden(c) -> return true
            space(c) -> last = Last.Space
            else -> last = if (c == FLAG) Last.Flag else Last.Visible
        }
    }
    return joined || tags > 0
}
