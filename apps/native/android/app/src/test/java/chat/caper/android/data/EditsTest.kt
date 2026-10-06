package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class EditsTest {
    private val author = ChatAuthor("author", "Author", false)
    private val original = ChatMessage(
        "message00000001", "channel00001", "3", author, ChatContent(1, "text", "original"),
        "2026-10-01T00:00:00Z", "00000000-0000-4000-8000-000000000001",
        reactions = listOf(MessageReaction("👍", listOf("other"))), reactionSeq = "12",
        pin = MessagePin(author, "2026-10-02T00:00:00Z"), pinSeq = "13",
        threadRootId = "root00000001", broadcast = true, thread = ThreadSummary(4, listOf(author), "11"),
    )
    private val edited = original.copy(
        content = ChatContent(1, "text", "corrected"), revision = 2,
        editedAt = "2026-10-06T00:00:00Z", editSeq = "14", reactions = emptyList(),
        reactionSeq = null, pin = null, pinSeq = null, thread = ThreadSummary(1, listOf(author), "4"),
    )

    @Test fun `content edits never replace identity or independently revised metadata`() {
        val merged = mergeEdit(original, edited)
        assertEquals("corrected", merged.content.text)
        assertEquals("3", merged.seq)
        assertEquals(original.createdAt, merged.createdAt)
        assertEquals("12", merged.reactionSeq)
        assertEquals(original.reactions, merged.reactions)
        assertEquals(original.pin, merged.pin)
        assertEquals("13", merged.pinSeq)
        assertEquals("11", merged.thread?.seq)
        assertTrue(merged.broadcast)
        assertEquals("corrected", mergeEdit(merged, original).content.text)
        assertEquals(original, mergeEdit(original, edited.copy(id = "another")))
        assertEquals("corrected", mergeMessages(listOf(merged), listOf(original.copy(reactionSeq = "15")), mutableMapOf()).single().content.text)
    }

    @Test fun `loaded edits survive the unseen bound and overflow does not evict older corrections`() {
        val snapshots = mutableMapOf<String, ChatMessage>()
        val loaded = (0..256).map { edited.copy(id = "loaded-$it") }
        val loadedIds = loaded.map { it.id }.toSet()
        loaded.forEach { assertTrue(cacheEditSnapshot(snapshots, it, loadedIds)) }
        repeat(256) { assertTrue(cacheEditSnapshot(snapshots, edited.copy(id = "unseen-$it"), loadedIds)) }
        assertFalse(cacheEditSnapshot(snapshots, edited.copy(id = "overflow"), loadedIds))
        assertEquals("corrected", snapshots["unseen-0"]?.content?.text)
        assertTrue(cacheEditSnapshot(snapshots, loaded.first().copy(revision = 3), loadedIds))
        assertTrue(cacheEditSnapshot(snapshots, loaded.first(), loadedIds))
        assertEquals(3, snapshots[loaded.first().id]?.revision)
        snapshots.clear() // Channel resync clears the cache before applying fresh history.
        assertTrue(cacheEditSnapshot(snapshots, edited.copy(id = "overflow"), emptySet()))
    }

    @Test fun `edit events use edit sequence not creation sequence`() {
        assertEquals("14", EditUpdate("message.edited", 1, "channel00001", "14", edited).validated("channel00001").seq)
        for (event in listOf(
            EditUpdate("message.edited", 1, "channel00001", "3", edited),
            EditUpdate("message.edited", 1, "other", "14", edited),
            EditUpdate("message.edited", 1, "channel00001", "14", edited.copy(editedAt = null)),
        )) {
            assertTrue(runCatching { event.validated("channel00001") }.isFailure)
        }
    }
}
