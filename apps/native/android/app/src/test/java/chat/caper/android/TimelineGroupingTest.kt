package chat.caper.android

import chat.caper.android.data.groupBlocked
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TimelineGroupingTest {
    private val maya = ChatAuthor("maya0000000a", "Maya", false)
    private val blocked = ChatAuthor("block000000a", "Blocked", false)

    private fun message(id: String, author: ChatAuthor) =
        ChatMessage(id, "channel00001", id.takeLast(1), author, ChatContent(1, "text", id), "2026-10-06T19:00:0${id.last()}Z", "client-$id")

    private val messages = listOf(message("m1", maya), message("m2", blocked), message("m3", blocked), message("m4", maya), message("m5", maya))

    private fun above(revealed: Set<String>) = timelineRows(groupBlocked(messages, setOf(blocked.id), maya.id, revealed))
        .filterIsInstance<TimelineRow.Message>().associate { it.message.id to it.above?.id }

    @Test fun `a blocked-messages row above a message starts a new group`() {
        val rows = above(emptySet())
        assertEquals(setOf("m1", "m4", "m5"), rows.keys)
        assertNull(rows["m1"])
        assertNull("the collapsed run's row sits between m1 and m4", rows["m4"])
        assertEquals("m4", rows["m5"])
    }

    @Test fun `a revealed run lists its messages under its Hide row`() {
        val rows = above(setOf("m2"))
        assertNull("the Hide row sits above the run's first message", rows["m2"])
        assertEquals("m2", rows["m3"])
        assertEquals("m3", rows["m4"])
    }
}
