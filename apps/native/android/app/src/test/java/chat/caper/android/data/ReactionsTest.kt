package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class ReactionsTest {
    private val message = ChatMessage("message00000001", "channel00001", "3", ChatAuthor("author", "A", false), ChatContent(1, "text", "hi"), "now", "client")
    private fun update(seq: String, authors: List<String>, messageId: String = message.id) = ReactionUpdate(type = "message.reactions", schemaVersion = 1, channelId = "channel00001", seq = seq, messageId = messageId, reactions = listOf(MessageReaction("👍", authors)))

    @Test fun `new snapshots merge while stale and duplicate snapshots do not`() {
        val newest = mergeReaction(message, update("8", listOf("a")))
        assertEquals("8", newest.reactionSeq)
        assertSame(newest, mergeReaction(newest, update("7", listOf("b"))))
        assertSame(newest, mergeReaction(newest, update("8", listOf("b"))))
    }

    @Test fun `strict validator rejects repeated actors`() {
        assertThrows(IllegalArgumentException::class.java) { update("4", listOf("a", "a", "b")).validated("channel00001") }
    }

    @Test fun `unloaded updates apply when history page arrives`() {
        val unseen = mutableMapOf(message.id to update("9", listOf("a")))
        assertEquals("9", mergeMessages(emptyList(), listOf(message), unseen).single().reactionSeq)
        assertTrue(unseen.isEmpty())
    }

    @Test fun `stale older overlap cannot replace loaded reactions`() {
        val newer = mergeReaction(message, update("999999999999999999999", listOf("new")))
        val stale = mergeReaction(message, update("8", listOf("old")))
        assertEquals(listOf("new"), mergeMessages(listOf(newer), listOf(stale), mutableMapOf()).single().reactions.single().authorIds)
    }

    @Test fun `unseen cache requests resync beyond boundary without dropping cached updates`() {
        val unseen = mutableMapOf<String, ReactionUpdate>()
        repeat(256) { assertTrue(cacheUnseenReaction(unseen, update((it + 1).toString(), listOf("a"), "message$it"))) }
        assertFalse(cacheUnseenReaction(unseen, update("257", listOf("a"), "overflow")))
        assertEquals(256, unseen.size)
    }
}
