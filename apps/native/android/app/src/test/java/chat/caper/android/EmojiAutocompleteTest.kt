package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.ui.EmojiEntry
import org.junit.Assert.*
import org.junit.Test

class EmojiAutocompleteTest {
    private fun entry(id: String, name: String, keywords: String = "", selectable: Boolean = true) =
        EmojiEntry(id, "😀", name, keywords, "faces", selectable, 0, 0, 0)
    private fun atEnd(text: String) = TextFieldValue(text, TextRange(text.length))

    @Test fun `token requires a boundary and rejects completed or split shortcodes`() {
        listOf("word:sm", "https://x:sm", "12:30").forEach { assertNull(emojiToken(atEnd(it))) }
        assertNotNull(emojiToken(atEnd(":sm")))
        assertNotNull(emojiToken(atEnd("hello (:sm")))
        assertEquals("", emojiToken(atEnd("hello :"))?.query)
        assertNull(emojiToken(TextFieldValue(":sm:", TextRange(3))))
        assertNull(emojiToken(TextFieldValue(":smile", TextRange(3))))
        assertNull(emojiToken(TextFieldValue(":sm", TextRange(1, 2))))
        assertNull(emojiToken(TextFieldValue(":sm", TextRange(3), TextRange(1, 3))))
    }

    @Test fun `replacement preserves astral and ZWJ prefix suffix and caret`() {
        val original = TextFieldValue("👩‍💻 hi :roc suffix", TextRange("👩‍💻 hi :roc".length))
        val result = insertEmoji(original, emojiToken(original)!!, "🚀")!!
        assertEquals("👩‍💻 hi 🚀 suffix", result.text)
        assertEquals("👩‍💻 hi 🚀".length, result.selection.start)
        assertTrue(result.selection.collapsed)
    }

    @Test fun `ranking aliases selectability catalog order and limit`() {
        val entries = listOf(
            entry("0", "other", "rocketship"), entry("1", "rock et"), entry("2", "rock et fuel"),
            entry("3", "stone", "space rock et"), entry("4", "pocket rock et", ""), entry("x", "rock et", selectable = false),
            entry("5", "rock one"), entry("6", "rock two"), entry("7", "rock three"),
        )
        assertEquals(listOf("1", "2", "3", "4"), emojiSuggestions(entries, "rock_et").map { it.id })
        assertEquals(6, emojiSuggestions(entries, "rock").size)
    }

    @Test fun `insertion enforces scalar limit`() {
        val value = atEnd("😀".repeat(3998) + " :x")
        assertNotNull(insertEmoji(value, emojiToken(value)!!, "🚀"))
        assertNull(insertEmoji(value, emojiToken(value)!!, "👩‍💻"))
    }
}
