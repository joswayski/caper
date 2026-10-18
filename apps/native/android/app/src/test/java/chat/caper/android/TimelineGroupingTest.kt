package chat.caper.android

import chat.caper.android.data.groupBlocked
import chat.caper.android.data.channelNewMessagesId
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.UnreadChannel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TimelineGroupingTest {
    private val maya = ChatAuthor("maya0000000a", "Maya", false)
    private val blocked = ChatAuthor("block000000a", "Blocked", false)

    private fun message(id: String, author: ChatAuthor) =
        ChatMessage(id, "channel00001", id.takeLast(1), author, ChatContent(1, "text", id), "2026-10-06T19:00:0${id.last()}Z", "client-$id")

    private val messages = listOf(message("m1", maya), message("m2", blocked), message("m3", blocked), message("m4", maya), message("m5", maya))
    private fun entry(read: String = "1", last: String = "5") = UnreadChannel("channel00001", last, read)

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

    @Test fun `new messages skip self and blocked authors even when revealed`() {
        assertEquals("m4", channelNewMessagesId(messages, entry(), false, setOf(blocked.id), "self"))
        assertNull(channelNewMessagesId(messages, entry(), false, setOf(blocked.id), maya.id))
    }

    @Test fun `read or old-server visits never gain a divider from live arrivals`() {
        assertNull(channelNewMessagesId(messages, null, false, emptySet(), "self"))
        assertNull(channelNewMessagesId(messages.takeLast(1), entry(last = "4"), false, emptySet(), "self"))
    }

    @Test fun `older paging discovers the boundary rather than marking the latest page start`() {
        assertNull(channelNewMessagesId(messages.drop(3), entry(), true, emptySet(), "self"))
        assertEquals("m2", channelNewMessagesId(messages, entry(), true, emptySet(), "self"))
        assertEquals("m2", channelNewMessagesId(messages.drop(1), entry(), true, emptySet(), "self"))
        assertEquals("m4", channelNewMessagesId(messages.drop(3), entry(), false, emptySet(), "self"))
    }

    @Test fun `sequence comparison supports values larger than signed long`() {
        val large = messages.take(1).map { it.copy(seq = "18446744073709551615") }
        assertEquals("m1", channelNewMessagesId(large, entry("18446744073709551614", "18446744073709551615"), true, emptySet(), "self"))
    }
}
