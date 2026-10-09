package chat.caper.android.ui

import java.io.File
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class EmojiNamesTest {
    private val json = Json { ignoreUnknownKeys = true }

    private fun entry(id: String, emoji: String, name: String, selectable: Boolean = true) =
        EmojiEntry(id, emoji, name, name, "symbols", selectable, 0, 0, 0)

    @Test fun `fully qualified reactions find catalog names keyed without U+FE0F`() {
        val index = emojiNameIndex(listOf(entry("2764", "\u2764", "red-heart"), entry("1f44d", "👍", "thumbs-up")))
        assertEquals("red-heart", emojiName("\u2764\uFE0F", index))
        assertEquals("red-heart", emojiName("\u2764", index))
        assertEquals("thumbs-up", emojiName("👍", index))
        assertNull(emojiName("🫠", index))
    }

    @Test fun `picker names win over unnamed unqualified duplicates`() {
        val index = emojiNameIndex(listOf(
            entry("1f441-200d-1f5e8", "👁\u200D🗨", "1f441-200d-1f5e8", selectable = false),
            entry("1f441-fe0f-200d-1f5e8-fe0f", "👁\uFE0F\u200D🗨\uFE0F", "eye-in-speech-bubble"),
            entry("1f44d-1f3fd", "👍🏽", "1f44d-1f3fd", selectable = false),
        ))
        assertEquals("eye-in-speech-bubble", emojiName("👁\u200D🗨", index))
        assertEquals("eye-in-speech-bubble", emojiName("👁\uFE0F\u200D🗨\uFE0F", index))
        assertNull("code-point IDs are not names", emojiName("👍🏽", index))
    }

    @Test fun `shared catalog gives every client the same quick reaction names`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/emoji/catalog.json") }.first { it.isFile }
        val index = emojiNameIndex(json.decodeFromString<List<EmojiEntry>>(file.readText()))
        assertEquals("thumbs-up", emojiName("👍", index))
        assertEquals("face-with-tears-of-joy", emojiName("😂", index))
        assertEquals("party-popper", emojiName("🎉", index))
        assertEquals("eyes", emojiName("👀", index))
        assertEquals("fire", emojiName("🔥", index))
        assertEquals("red-heart", emojiName("\u2764\uFE0F", index))
    }
}
