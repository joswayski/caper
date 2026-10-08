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

    @Test fun `pin payloads preserve independent newer reaction revisions`() {
        val original = message.copy(pin = MessagePin(message.author, "now"), pinSeq = "4")
        val reacted = mergeReaction(original, update("12", listOf("alice", "bob")))
        val laterPin = original.copy(pinSeq = "13")
        val merged = mergeReaction(mergePin(reacted, laterPin), laterPin)
        assertEquals("13", merged.pinSeq)
        assertEquals("12", merged.reactionSeq)
        assertEquals(listOf("alice", "bob"), merged.reactions.single().authorIds)
        assertEquals("12", mergeReaction(message, merged).reactionSeq)
        val removed = reacted.copy(reactionSeq = "14", reactions = emptyList())
        assertTrue(mergeReaction(laterPin, removed).reactions.isEmpty())
        assertEquals("14", mergeReaction(laterPin, removed).reactionSeq)
        assertSame(laterPin, mergeReaction(laterPin, removed.copy(id = "another")))
        assertSame(laterPin, mergeReaction(laterPin, removed.copy(channelId = "another")))
    }

    @Test fun `only sequenced reactions advance durable recovery cursor`() {
        assertEquals("7", replayCursorAfterReaction("5", update("7", listOf("a")), sequenced = true))
        assertEquals("5", replayCursorAfterReaction("5", update("9", listOf("a")), sequenced = false))
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

    @Test fun `pending add is immediate and preserves other authors`() {
        val base = mergeReaction(message, update("8", listOf("other")))
        val shown = projectReactionIntents(base, "me", listOf(ReactionSaveUi("👍", true)))
        assertEquals(listOf("other", "me"), shown.reactions.single().authorIds)
        assertEquals("8", shown.reactionSeq)
    }

    @Test fun `pending remove omits zero count chips`() {
        val base = mergeReaction(message, update("8", listOf("me")))
        assertTrue(projectReactionIntents(base, "me", listOf(ReactionSaveUi("👍", false))).reactions.isEmpty())
    }

    @Test fun `latest intents overlay authoritative updates and rollback reveals authority`() {
        val pending = listOf(ReactionSaveUi("👍", false), ReactionSaveUi("❤️", true))
        val gateway = mergeReaction(message, update("9", listOf("other", "me", "new-person")))
        val shown = projectReactionIntents(gateway, "me", pending)
        assertEquals(listOf("other", "new-person"), shown.reactions.first { it.emoji == "👍" }.authorIds)
        assertEquals(listOf("me"), shown.reactions.first { it.emoji == "❤️" }.authorIds)
        assertEquals(gateway, projectReactionIntents(gateway, "me", emptyList()))
    }

    @Test fun `repeated additions cannot double count and final removal preserves others`() {
        val base = mergeReaction(message, update("8", listOf("other", "me")))
        val added = projectReactionIntents(base, "me", listOf(ReactionSaveUi("👍", true)))
        assertEquals(listOf("other", "me"), added.reactions.single().authorIds)
        val shown = projectReactionIntents(base, "me", listOf(ReactionSaveUi("👍", true), ReactionSaveUi("👍", false)))
        assertFalse("me" in shown.reactions.single().authorIds)
        assertEquals(listOf("other"), shown.reactions.single().authorIds)
        assertEquals("8", shown.reactionSeq)
    }

    @Test fun `unseen cache requests resync beyond boundary without dropping cached updates`() {
        val unseen = mutableMapOf<String, ReactionUpdate>()
        repeat(256) { assertTrue(cacheUnseenReaction(unseen, update((it + 1).toString(), listOf("a"), "message$it"))) }
        assertFalse(cacheUnseenReaction(unseen, update("257", listOf("a"), "overflow")))
        assertEquals(256, unseen.size)
    }
}
