package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.ui.EmojiEntry

internal data class EmojiToken(val start: Int, val end: Int, val query: String)

/** Web's `:` token at the caret (shared/messages/emoji.rs, read in the visible text). */
internal fun emojiToken(value: TextFieldValue): EmojiToken? {
    if (!value.selection.collapsed || value.composition != null) return null
    val token = chat.caper.android.data.CaperMobile.emojiToken(value.text, value.selection.start)
    if (token.size < 2) return null
    return EmojiToken(token[0], token[1], String(CharArray(token.size - 2) { token[it + 2].toChar() }))
}

/** Up to six entries of [catalog], the one [EmojiArtwork] loaded, that web offers for [query]. */
internal fun emojiSuggestions(catalog: List<EmojiEntry>, query: String): List<EmojiEntry> =
    chat.caper.android.data.CaperMobile.emojiSuggestions(query).asList().mapNotNull { catalog.getOrNull(it) }

internal fun insertEmoji(value: TextFieldValue, token: EmojiToken, emoji: String, limit: Int = 4000): TextFieldValue? {
    val text = value.text.substring(0, token.start) + emoji + value.text.substring(token.end)
    if (text.codePointCount(0, text.length) > limit) return null
    val caret = token.start + emoji.length
    val repaired = chat.caper.android.data.repairFormatEdit(value.text, chat.caper.android.data.FormatSelection(text, caret, caret))
    return TextFieldValue(repaired.text, TextRange(repaired.start))
}

internal fun emojiShortcodeLabel(name: String) = ":${name.replace(' ', '_')}:"
