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

    /** `[start, end]` per `@mention` token (shared/messages/mentions.rs), covering the `@` and its name. */
    @JvmStatic external fun mentionSpans(text: String): IntArray

    /**
     * What an edit-history comparison highlights: the number of `before` spans, then `[start, end]`
     * per highlighted span, `before` spans first.
     */
    @JvmStatic external fun historyDiffSpans(before: String, after: String): IntArray

    /**
     * One composer edit (shared/messages/composer.rs): `[start, end, pending]` and then the edited
     * text's UTF-16 units. [start] and [end] are offsets into [next].
     */
    @JvmStatic external fun composerEdit(previous: String, next: String, start: Int, end: Int, continuing: Int): IntArray

    /** Loads the catalog.json asset's bytes for [emojiSuggestions]; false when they are not a catalog. */
    @JvmStatic external fun emojiCatalog(json: ByteArray): Boolean

    /**
     * The `:` token with the caret at [caret] (shared/messages/emoji.rs): `[start, end]` in [text],
     * then the query's characters, or empty when none is open.
     */
    @JvmStatic external fun emojiToken(text: String, caret: Int): IntArray

    /** Positions in the loaded catalog web offers for [query], best first. */
    @JvmStatic external fun emojiSuggestions(query: String): IntArray

    /** Whether a display or space name holds a character names may not (shared/names/cases.json). */
    @JvmStatic external fun nameHasHidden(text: String): Boolean

    /** Web's normalizeChannelName, one ASCII character per value. */
    @JvmStatic external fun channelName(text: String): IntArray

    /** Web's channelNameError ([channel]) or spaceNameError: the message's ASCII characters, or empty when valid. */
    @JvmStatic external fun nameError(text: String, channel: Boolean): IntArray

    /** `[start, end]` of the space name web sends (`name.trim()`). */
    @JvmStatic external fun spaceName(text: String): IntArray

    /** Web's inviteCode: the 10-character code, one ASCII character per value, or empty. */
    @JvmStatic external fun inviteCode(text: String): IntArray
}
