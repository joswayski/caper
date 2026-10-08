package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.ui.EmojiEntry
import java.util.Locale

internal data class EmojiToken(val start: Int, val end: Int, val query: String)

internal fun emojiToken(value: TextFieldValue): EmojiToken? {
    if (!value.selection.collapsed || value.composition != null) return null
    val text = value.text
    val caret = value.selection.start
    if (caret < text.length && (text[caret] == ':' || text[caret].isEmojiQueryChar())) return null
    var colon = caret - 1
    while (colon >= 0 && text[colon].isEmojiQueryChar()) colon--
    if (colon < 0 || text[colon] != ':') return null
    if (colon > 0 && !text[colon - 1].isWhitespace() && text[colon - 1] !in "([{") return null
    return EmojiToken(colon, caret, text.substring(colon + 1, caret))
}

private fun Char.isEmojiQueryChar(): Boolean =
    this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9' || this == '_' || this == '+' || this == '-'

private fun normalized(value: String) = value.lowercase(Locale.ROOT).replace('_', ' ').replace('-', ' ')

internal fun emojiSuggestions(entries: List<EmojiEntry>, query: String, limit: Int = 6): List<EmojiEntry> {
    if (query.isEmpty()) {
        val defaults = listOf("1f44d", "1f600", "2764", "1f389", "1f680", "1f440")
        return defaults.mapNotNull { id -> entries.firstOrNull { it.selectable && it.id == id } }.take(limit)
    }
    val needle = normalized(query)
    return entries.asSequence().filter { it.selectable }.mapNotNull { entry ->
        val name = normalized(entry.name)
        val keywords = normalized(entry.keywords)
        val rank = when {
            name == needle -> 0
            name.startsWith(needle) -> 1
            keywords.startsWith(needle) || " $needle" in keywords -> 2
            needle in name || needle in keywords -> 3
            else -> return@mapNotNull null
        }
        rank to entry
    // Within a rank the shorter name is the closer match: ":fi" offers fire before film-frames.
    }.sortedWith(compareBy({ it.first }, { it.second.name.length })).map { it.second }.take(limit).toList()
}

internal fun insertEmoji(value: TextFieldValue, token: EmojiToken, emoji: String, limit: Int = 4000): TextFieldValue? {
    val text = value.text.substring(0, token.start) + emoji + value.text.substring(token.end)
    if (text.codePointCount(0, text.length) > limit) return null
    val caret = token.start + emoji.length
    return TextFieldValue(text, TextRange(caret))
}

internal fun emojiShortcodeLabel(name: String) = ":${name.replace(' ', '_')}:"
