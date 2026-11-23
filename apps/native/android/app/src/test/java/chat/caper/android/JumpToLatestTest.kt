package chat.caper.android

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class JumpToLatestTest {
    private val me = ChatAuthor("me0000000000", "Me", false, 1)
    private val maya = ChatAuthor("maya00000000", "Maya", false, 2)
    private val blockedPerson = ChatAuthor("blocked00000", "Blocked", false, 3)
    private fun message(id: String, author: ChatAuthor = maya) = ChatMessage(id, "channel00001", id.filter(Char::isDigit), author, ChatContent(1, "text", "hi"), "now", "client-$id")

    @Test fun `only other people's messages after the newest known one count`() {
        val known = listOf(message("m1"), message("m2"))
        val ids = known.map { it.id }.toSet()
        assertEquals(2, liveArrivals(ids, known + message("m3") + message("m4"), me.id, emptySet()))
        // Your own sends and collapsed blocked messages are not news.
        assertEquals(1, liveArrivals(ids, known + message("m3", me) + message("m4", blockedPerson) + message("m5"), me.id, setOf(blockedPerson.id)))
        // An older page lands before the known rows; an edit or reaction changes no IDs.
        assertEquals(0, liveArrivals(ids, listOf(message("m0")) + known, me.id, emptySet()))
        assertEquals(0, liveArrivals(ids, known, me.id, emptySet()))
        // The newest known message deleted: what follows the remaining known one was known too, except the arrival.
        assertEquals(1, liveArrivals(ids, listOf(known[0], message("m3")), me.id, emptySet()))
    }

    @Test fun `shows past half a viewport at the live end and hides back at the bottom`() {
        assertFalse(jumpToLatestShown(shown = false, live = true, distance = 400, viewport = 800, threshold = 80))
        assertTrue(jumpToLatestShown(shown = false, live = true, distance = 401, viewport = 800, threshold = 80))
        // Once shown it stays until the reader is near the bottom again.
        assertTrue(jumpToLatestShown(shown = true, live = true, distance = 200, viewport = 800, threshold = 80))
        assertFalse(jumpToLatestShown(shown = true, live = true, distance = 80, viewport = 800, threshold = 80))
        // A detached history window doesn't count; it shows its own toast (below).
        assertFalse(jumpToLatestShown(shown = true, live = false, distance = 5_000, viewport = 800, threshold = 80))
    }

    @Test fun `a detached window always shows the toast, uncounted`() {
        assertEquals(0, jumpToastCount(detached = true, shown = false, unseen = 0))
        assertEquals(0, jumpToastCount(detached = true, shown = true, unseen = 3))
        assertEquals(3, jumpToastCount(detached = false, shown = true, unseen = 3))
        assertNull(jumpToastCount(detached = false, shown = false, unseen = 0))
    }

    @Test fun `the label says older messages until a count, capped at 99+`() {
        assertEquals("You’re viewing older messages", jumpToLatestLabel(0))
        assertEquals("1 new message", jumpToLatestLabel(1))
        assertEquals("3 new messages", jumpToLatestLabel(3))
        assertEquals("99 new messages", jumpToLatestLabel(99))
        assertEquals("99+ new messages", jumpToLatestLabel(100))
        assertEquals("Jump to latest", jumpToLatestName(0))
        assertEquals("Jump to latest, 3 new messages", jumpToLatestName(3))
    }
}
