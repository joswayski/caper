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

    @Test fun `a new message leaves loaded messages as the same instances`() {
        val next = message.copy(id = "message00000002", seq = "4", clientMessageId = "client2")
        val merged = mergeMessages(listOf(message), listOf(next), mutableMapOf())
        assertEquals(listOf(message, next), merged)
        assertSame(message, merged.first())
    }

    @Test fun `deleting replies lowers the parent's summary and the last one removes it`() {
        fun summary(count: Int, seq: String) = ThreadSummary(count, listOf(message.author), seq)
        val parent = message.copy(thread = summary(2, "5"))
        val reply = message.copy(id = "message00000002", seq = "5", clientMessageId = "client2", threadRootId = message.id, thread = summary(2, "5"))
        fun deleted(seq: String, thread: ThreadSummary?) = reply.copy(revision = 2, editedAt = "now", editSeq = seq, deletedAt = "now", thread = thread)
        val emptied = mutableMapOf<String, java.math.BigInteger>()
        // Another reply remains: the tombstone carries the lowered summary to the parent.
        val lowered = mergeMessages(listOf(parent, reply), listOf(deleted("6", summary(1, "6"))), mutableMapOf(), emptiedThreads = emptied)
        assertEquals(listOf(1, 1), lowered.map { it.thread?.replyCount })
        assertTrue(emptied.isEmpty())
        // The last reply's tombstone carries none, which clears the parent.
        val cleared = mergeMessages(lowered, listOf(deleted("7", null)), mutableMapOf(), emptiedThreads = emptied)
        assertEquals(listOf(null, null), cleared.map { it.thread })
        assertEquals(null, threadSummaries(lowered + deleted("7", null), emptied)[message.id])
        // An older copy of the parent cannot bring it back, and a later reply starts the count again.
        assertNull(mergeMessages(cleared, listOf(parent), mutableMapOf(), emptiedThreads = emptied).first().thread)
        val next = reply.copy(id = "message00000003", seq = "8", clientMessageId = "client3", thread = summary(1, "8"))
        assertEquals("8", mergeMessages(cleared, listOf(next), mutableMapOf(), emptiedThreads = emptied).first().thread?.seq)
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

    @Test fun `pin snapshots preserve newer reactions without replacing independent content`() {
        val pinned = message.copy(pin = MessagePin(message.author, "later"), pinSeq = "12",
            revision = 2, content = ChatContent(1, "text", "edited"))
        val reacted = mergeReaction(message, update("13", listOf("peer")))
        val merged = mergeReaction(pinned, reacted)
        assertEquals(listOf("peer"), merged.reactions.single().authorIds)
        assertEquals("13", merged.reactionSeq)
        assertEquals(pinned.pin, merged.pin)
        assertEquals("12", merged.pinSeq)
        assertEquals("edited", merged.content.text)
        assertEquals(2, merged.revision)
        val removed = reacted.copy(reactionSeq = "14", reactions = emptyList())
        val latest = mergeReaction(merged, removed)
        assertTrue(latest.reactions.isEmpty())
        assertSame(latest, mergeReaction(latest, reacted))
        assertSame(latest, mergeReaction(latest, removed.copy(id = "another-message", reactionSeq = "15")))
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
