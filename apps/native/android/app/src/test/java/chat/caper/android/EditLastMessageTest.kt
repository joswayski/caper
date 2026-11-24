package chat.caper.android

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.MessageForward
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class EditLastMessageTest {
    private val me = ChatAuthor("self0000000a", "Me", false)
    private val maya = ChatAuthor("maya0000000a", "Maya", false)

    private fun message(id: String, author: ChatAuthor = me, root: String? = null, broadcast: Boolean = false, deleted: Boolean = false) =
        ChatMessage(id, "channel00001", id.takeLast(1), author, ChatContent(1, "text", id), "2026-10-06T19:00:0${id.last()}Z", "client-$id",
            threadRootId = root, broadcast = broadcast, deletedAt = if (deleted) "2026-10-06T19:01:00Z" else null)

    @Test fun `channel skips hidden replies, others, deleted and forwarded messages`() {
        val forwarded = message("m6").copy(forward = MessageForward(message("m0", maya), "6"))
        val messages = listOf(message("m1"), message("m2"), message("m3", root = "m1"), message("m4", maya), message("m5", deleted = true), forwarded)
        assertEquals("m2", lastEditableMessage(messages, me.id)?.id)
        // A reply also sent to the channel shows in its timeline, so it counts there.
        assertEquals("m7", lastEditableMessage(messages + message("m7", root = "m1", broadcast = true), me.id)?.id)
        assertNull(lastEditableMessage(messages, null))
        assertNull(lastEditableMessage(messages, "nobody000000"))
    }

    @Test fun `thread counts its root and replies only`() {
        val messages = listOf(message("m1"), message("m2", maya), message("m3", root = "m2"), message("m4", root = "m9"), message("m5", maya, root = "m2"), message("m6"))
        assertEquals("m3", lastEditableMessage(messages, me.id, "m2")?.id)
        assertEquals("m1", lastEditableMessage(messages, me.id, "m1")?.id)
        assertNull(lastEditableMessage(listOf(message("m2", maya), message("m3", maya, root = "m2")), me.id, "m2"))
    }
}
