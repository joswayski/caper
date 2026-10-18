package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class EditsTest {
    @Test fun `word diffs preserve separate changes Unicode and whitespace`() {
        val before = "Meet Friday 🙂\nKeep this unchanged\nAt 9"
        val after = "Meet Saturday 🚀\nKeep this unchanged\nAt 11"
        val (old, next) = messageDiff(before, after)
        assertEquals(before, old.joinToString("") { it.text })
        assertEquals(after, next.joinToString("") { it.text })
        assertEquals("Friday🙂9", old.filter { it.changed }.joinToString("") { it.text })
        assertEquals("Saturday🚀11", next.filter { it.changed }.joinToString("") { it.text })
        assertTrue(old.filter { it.text in listOf("Keep", "this", "unchanged") }.all { !it.changed })
        assertTrue(validEditText("🙂".repeat(4000)))
        assertFalse(validEditText("🙂".repeat(4001)))
        assertFalse(validEditText(" \n\t"))
        assertFalse(validEditText("Text\u0000"))
        assertTrue(validEditText("Text\n\t🙂"))
    }

    @Test fun `history pages are descending bounded and identity checked`() {
        val first = MessageVersion(1, ChatContent(1, "text", "original"), "2026-10-01T00:00:00Z")
        val second = first.copy(revision = 2)
        val page = MessageVersions("message", listOf(second, first), false)
        assertEquals(page, page.validated("message", null))
        for (invalid in listOf(
            page.copy(messageId = "another"), page.copy(versions = listOf(first, second)),
            page.copy(versions = listOf(second, second)), page.copy(versions = listOf(second.copy(createdAt = "invalid"))),
        )) assertTrue(runCatching { invalid.validated("message", null) }.isFailure)
        assertTrue(runCatching { page.validated("message", 2) }.isFailure)
    }

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

    private val tombstone = original.copy(
        content = ChatContent(1, "text", ""), revision = 3, editedAt = "2026-10-07T00:00:00Z", editSeq = "16",
        attachmentsSeq = "16", deletedAt = "2026-10-07T00:00:00Z", reactions = emptyList(), reactionSeq = null,
        pin = null, pinSeq = null,
    )

    @Test fun `a deletion merges as an edit that drops files and blocks older content`() {
        val withFile = mergeEdit(original, edited).copy(
            content = edited.content.copy(attachments = listOf(ChatAttachment("AbCdEfGh12345678", "image", "image/avif", "a.avif", 10))), attachmentsSeq = "15",
        )
        val deleted = mergeEdit(withFile, tombstone)
        assertEquals(tombstone.deletedAt, deleted.deletedAt)
        assertEquals("", deleted.content.text)
        assertTrue(deleted.content.attachments.isEmpty())
        assertEquals("16", deleted.attachmentsSeq)
        // Pins and reactions follow their own events; the thread summary stays reachable.
        assertEquals(original.pin, deleted.pin)
        assertEquals(original.thread, deleted.thread)
        assertEquals(deleted, mergeEdit(deleted, edited))
        assertEquals(tombstone.deletedAt, mergeEdit(deleted, deleted.copy(deletedAt = null)).deletedAt)
        assertEquals("16", EditUpdate("message.edited", 1, "channel00001", "16", tombstone).validated("channel00001").seq)
        assertTrue(runCatching { tombstone.copy(content = ChatContent(1, "text", "kept")).validated("channel00001") }.isFailure)
        assertTrue(runCatching { tombstone.copy(deletedAt = "yesterday").validated("channel00001") }.isFailure)
    }

    @Test fun `loaded replies stop quoting a deleted message`() {
        val reference = MessageReply(original.id, author, "original", true, fullText = "original")
        val reply = edited.copy(id = "message00000002", replyTo = reference)
        val quoted = quoteDeleted(reply, original.id).replyTo
        assertEquals(MessageReply(original.id, author, "", false, deleted = true), quoted)
        assertSame(reply, quoteDeleted(reply, "message00000009"))
        assertSame(original, quoteDeleted(original, original.id))
    }
}
