package chat.caper.android.data

/**
 * Caper's shared Rust logic (apps/native/mobile), the same sources the desktop
 * clients compile. Results are flat arrays of UTF-16 offsets into the string
 * passed in; apps/native/mobile/src/lib.rs documents each layout.
 */
internal object CaperMobile {
    init {
        System.loadLibrary("caper_mobile")
    }

    /** `[start, end, style]` per visible span of formatted text; style bit 1 is bold, 2 italic. */
    @JvmStatic external fun formatSpans(text: String): IntArray

    /** `[start, end, scheme]` per link: scheme 8 is `https://`, 7 `http://`, 0 `www.`. */
    @JvmStatic external fun linkSpans(text: String): IntArray

    /** Whether a display or space name holds a character names may not (shared/names/cases.json). */
    @JvmStatic external fun nameHasHidden(text: String): Boolean
}
