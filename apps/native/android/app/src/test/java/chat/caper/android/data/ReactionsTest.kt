package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class ReactionsTest {
    private val message = ChatMessage("message00000001", "channel00001", "3", ChatAuthor("author", "A", false), ChatContent(1, "text", "hi"), "now", "client")
    private fun update(seq: String, authors: List<String>) = ReactionUpdate(schemaVersion = 1, channelId = "channel00001", seq = seq, messageId = message.id, reactions = listOf(MessageReaction("👍", authors)))

    @Test fun `new snapshots merge while stale and duplicate snapshots do not`() {
        val newest = mergeReaction(message, update("8", listOf("a")))
        assertEquals("8", newest.reactionSeq)
        assertSame(newest, mergeReaction(newest, update("7", listOf("b"))))
        assertSame(newest, mergeReaction(newest, update("8", listOf("b"))))
    }

    @Test fun `authors are unique when server snapshot repeats an actor`() {
        assertEquals(listOf("a", "b"), mergeReaction(message, update("4", listOf("a", "a", "b"))).reactions.single().authorIds)
    }

    @Test fun `unloaded updates apply when history page arrives`() {
        assertEquals("9", mergeReactions(listOf(message), mapOf(message.id to update("9", listOf("a")))).single().reactionSeq)
    }
}
