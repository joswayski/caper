package chat.caper.android

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.ui.EmojiEntry
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class EmojiAutocompleteTest {
    private fun atEnd(text: String) = TextFieldValue(text, TextRange(text.length))

    @Test fun `token requires a boundary and rejects completed or split shortcodes`() {
        listOf("word:sm", "https://x:sm", "12:30").forEach { assertNull(emojiToken(atEnd(it))) }
        assertNotNull(emojiToken(atEnd(":sm")))
        assertNotNull(emojiToken(atEnd("hello (:sm")))
        assertEquals("", emojiToken(atEnd("hello :"))?.query)
        listOf(":D", "ok :P", "hi (:3").forEach { assertNull(it, emojiToken(atEnd(it))) }
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

    /** Every client replays these, generated from web; see shared/messages/emoji-cases.json. */
    @Test fun `autocomplete follows the shared cases`() {
        fun shared(path: String) = generateSequence(java.io.File("").absoluteFile) { it.parentFile }
            .map { java.io.File(it, path) }.first { it.isFile }
        val bytes = shared("shared/emoji/catalog.json").readBytes()
        assertTrue(chat.caper.android.data.CaperMobile.emojiCatalog(bytes))
        val catalog = Json { ignoreUnknownKeys = true }.decodeFromString<List<EmojiEntry>>(bytes.decodeToString())
        val cases = Json.parseToJsonElement(shared("shared/messages/emoji-cases.json").readText()).jsonObject
        val tokens = cases.getValue("tokens").jsonArray.map { it.jsonObject }
        assertTrue(tokens.size > 400)
        for (case in tokens) {
            val text = case.getValue("text").jsonPrimitive.content
            val caret = case.getValue("caret").jsonPrimitive.int
            val expected = (case.getValue("token") as? JsonObject)?.let {
                EmojiToken(it.getValue("start").jsonPrimitive.int, it.getValue("end").jsonPrimitive.int, it.getValue("query").jsonPrimitive.content)
            }
            assertEquals("$text at $caret", expected, emojiToken(TextFieldValue(text, TextRange(caret))))
        }
        for (case in cases.getValue("suggestions").jsonArray.map { it.jsonObject }) {
            val query = case.getValue("query").jsonPrimitive.content
            assertEquals(query, case.getValue("ids").jsonArray.map { it.jsonPrimitive.content }, emojiSuggestions(catalog, query).map { it.id })
        }
    }

    @Test fun `insertion enforces scalar limit`() {
        val value = atEnd("😀".repeat(3998) + " :xy")
        assertNotNull(insertEmoji(value, emojiToken(value)!!, "🚀"))
        assertNull(insertEmoji(value, emojiToken(value)!!, "👩‍💻"))
    }
}
