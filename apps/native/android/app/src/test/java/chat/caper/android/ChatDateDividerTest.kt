package chat.caper.android

import chat.caper.android.data.groupBlocked
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatDateDividerTest {
    private val losAngeles = ZoneId.of("America/Los_Angeles")

    @Test fun `local day boundaries use the device zone`() {
        assertTrue(sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T06:45:00Z", losAngeles))
        assertFalse(sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T07:30:00Z", losAngeles))
    }

    private val maya = ChatAuthor("maya0000000a", "Maya", false)
    private val blocked = ChatAuthor("block000000a", "Blocked", false)
    private fun message(id: String, createdAt: String, author: ChatAuthor = maya, root: String? = null) =
        ChatMessage(id, "channel00001", id.takeLast(1), author, ChatContent(1, "text", id), createdAt, "client-$id", threadRootId = root)

    // Los Angeles days: the root on Sep 28, two replies on Sep 29 (07:30Z is past its midnight), one on Sep 30.
    private val thread = listOf(
        message("m1", "2026-09-29T06:30:00Z"),
        message("m2", "2026-09-29T07:30:00Z", root = "m1"),
        message("m3", "2026-09-29T20:00:00Z", root = "m1"),
        message("m4", "2026-09-30T20:00:00Z", root = "m1"),
    )
    private fun dividers(messages: List<ChatMessage>, blockedIds: Set<String> = emptySet(), revealed: Set<String> = emptySet()) =
        timelineRows(groupBlocked(messages, blockedIds, maya.id, revealed)).filter { startsDay(it, losAngeles) }.map { it.key }

    @Test fun `a thread has a divider above its root and above each reply on another day than the row above`() {
        assertEquals(listOf("m1", "m2", "m4"), dividers(thread))
    }

    @Test fun `replies on the root's day have only the root's divider`() {
        val root = message("m5", "2026-09-29T07:30:00Z")
        assertEquals(listOf("m5"), dividers(listOf(root, message("m6", "2026-09-29T20:00:00Z", root = "m5"), message("m7", "2026-09-30T06:59:00Z", root = "m5"))))
    }

    @Test fun `without its root, a thread's first shown reply has the divider`() {
        assertEquals(listOf("m2", "m4"), dividers(thread.drop(1)))
    }

    @Test fun `a blocked run takes its day's divider, and revealing it does not repeat it`() {
        val messages = listOf(thread[0], thread[1].copy(author = blocked), thread[2].copy(author = blocked), thread[3])
        assertEquals(listOf("m1", "blocked:m2", "m4"), dividers(messages, setOf(blocked.id)))
        assertEquals(listOf("m1", "blocked:m2", "m4"), dividers(messages, setOf(blocked.id), revealed = setOf("m2")))
    }

    @Test fun `full date is localized`() {
        assertEquals("Monday, September 28, 2026", fullDateLabel("2026-09-29T06:30:00Z", losAngeles, Locale.US))
    }
}
