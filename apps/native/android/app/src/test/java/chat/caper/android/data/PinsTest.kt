package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class PinsTest {
    private val author = ChatAuthor("author", "Author", false)
    private fun message(pinSeq: String, pinned: Boolean) = ChatMessage(
        "message00000001", "channel00001", "3", author, ChatContent(1, "text", "old message"),
        "2026-10-01T00:00:00Z", "00000000-0000-4000-8000-000000000001",
        pin = if (pinned) MessagePin(author, "2026-10-02T00:00:00Z") else null, pinSeq = pinSeq,
    )

    @Test fun `stale pinned snapshot cannot resurrect newer unpin`() {
        val unpinned = message("9", false)
        val stale = message("7", true)
        assertNull(mergePin(unpinned, stale).pin)
        assertEquals("9", mergePin(unpinned, stale).pinSeq)
    }

    @Test fun `complete history removes stale pins from unloaded pages but preserves newer snapshots`() {
        val stale = message("4", true)
        assertNull(overlayPin(stale, null, "60").pin)
        assertEquals("60", overlayPin(stale, null, "60").pinSeq)
        assertNull(overlayPin(message("60", true), null, "60").pin)
        val newer = message("61", true)
        assertEquals(newer.pin, overlayPin(stale, newer, "60").pin)
        assertEquals("61", overlayPin(stale, newer, "60").pinSeq)
        assertEquals(newer.pin, overlayPin(newer, null, "60").pin)
        assertEquals("61", overlayPin(newer, null, "60").pinSeq)
    }

    @Test fun `pin acknowledgement does not advance replay cursor`() {
        val update = PinUpdate("message.pin", 1, "channel00001", "9", message("9", true))
        assertEquals("5", replayCursorAfterPin("5", update, false))
        assertEquals("9", replayCursorAfterPin("5", update, true))
    }

    @Test fun `local pin and edit projections never replace authoritative revisions`() {
        val original = message("4", false).let { it.copy(content = it.content.copy(text = "@peer original",
            mentions = listOf(MessageMention("user", "peer", "peer")))) }
        val pin = MessagePin(author, "2026-10-08T00:00:00Z")
        val pending = AppUiState(messages = listOf(original),
            pinIntents = mapOf(original.id to PinIntentUi(original, pin)),
            editIntents = mapOf(original.id to EditIntentUi("local draft", 1)))
        assertEquals(pin, pending.displayedMessages.single().pin)
        assertEquals("local draft", pending.displayedPins.single().content.text)
        assertEquals(original, pending.messages.single())
        assertEquals(1, pending.displayedPins.single().revision)
        assertEquals("4", pending.displayedPins.single().pinSeq)
        assertNull(pending.displayedPins.single().editSeq)
        assertTrue(pending.displayedMessages.single().content.mentions.isEmpty())
        assertTrue(pending.pinnedMessages.isEmpty())
        val remote = original.copy(content = original.content.copy(text = "other tab"), revision = 2,
            editSeq = "9", editedAt = "2026-10-08T01:00:00Z",
            reactions = listOf(MessageReaction("🚀", listOf("peer"))), pin = pin, pinSeq = "10")
        val updated = pending.copy(messages = listOf(remote), pinnedMessages = listOf(remote))
        assertEquals("other tab", updated.displayedMessages.single().content.text)
        val unpin = updated.copy(pinIntents = mapOf(original.id to PinIntentUi(original, null)))
        assertTrue(unpin.displayedPins.isEmpty())
        assertNull(unpin.displayedMessages.single().pin)
        val rolledBack = unpin.copy(pinIntents = emptyMap(), editIntents = emptyMap())
        assertEquals(remote, rolledBack.displayedMessages.single())
        assertEquals(remote, rolledBack.displayedPins.single())
        val unloaded = unpin.copy(messages = emptyList())
        assertTrue(unloaded.displayedMessages.isEmpty())
        assertEquals(remote, unloaded.copy(pinIntents = emptyMap()).displayedPins.single())
    }

    @Test fun `displayed channel context is bounded and includes local mutations`() {
        val original = message("4", false)
        val before = original.copy(id = "before", seq = "2")
        val after = original.copy(id = "after", seq = "4")
        val pin = MessagePin(author, "2026-10-08T00:00:00Z")
        val pending = AppUiState(messages = listOf(before, original, after),
            contextStart = "3", contextEnd = "3",
            pinIntents = mapOf(original.id to PinIntentUi(original, pin)),
            editIntents = mapOf(original.id to EditIntentUi("local draft", 1)))
        assertEquals(listOf(original.id), pending.displayedChannelMessages.map { it.id })
        assertEquals(pin, pending.displayedChannelMessages.single().pin)
        assertEquals("local draft", pending.displayedChannelMessages.single().content.text)
        assertEquals(listOf(before, original, after), pending.messages)
    }
}
