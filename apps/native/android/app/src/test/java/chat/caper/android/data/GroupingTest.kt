package chat.caper.android.data

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.MessageForward
import chat.caper.android.model.MessagePin
import java.time.ZoneId
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GroupingTest {
    private val zone = ZoneId.of("America/Los_Angeles")
    private val maya = ChatAuthor("maya0000000a", "Maya", false)
    private val alex = ChatAuthor("alex0000000a", "Alex", false)

    private fun message(id: String, createdAt: String, author: ChatAuthor = maya, threadRootId: String? = null, broadcast: Boolean = false) =
        ChatMessage(id, "channel00001", id.takeLast(1), author, ChatContent(1, "text", "hi $id"), createdAt, "client-$id",
            threadRootId = threadRootId, broadcast = broadcast)

    private fun grouped(previous: ChatMessage?, current: ChatMessage, inThread: Boolean = false, jumpTarget: String? = null) =
        groupsWithPrevious(previous, current, inThread, jumpTarget, zone)

    @Test fun `same author within five minutes groups`() {
        val first = message("m1", "2026-10-06T19:00:00Z")
        assertTrue(grouped(first, message("m2", "2026-10-06T19:00:30Z")))
        assertTrue("exactly five minutes is inside the window", grouped(first, message("m2", "2026-10-06T19:05:00Z")))
        assertFalse(grouped(first, message("m2", "2026-10-06T19:05:00.001Z")))
        assertTrue("the gap is absolute", grouped(message("m2", "2026-10-06T19:04:00Z"), message("m3", "2026-10-06T19:00:00Z")))
    }

    @Test fun `the first row, another author or an unreadable time starts a new group`() {
        val first = message("m1", "2026-10-06T19:00:00Z")
        assertFalse(grouped(null, first))
        assertFalse(grouped(first, message("m2", "2026-10-06T19:00:30Z", alex)))
        assertFalse(grouped(first, message("m2", "not a time")))
        assertFalse(groupsWithPrevious(first, null, "2026-10-06T19:00:30Z", zoneId = zone))
    }

    @Test fun `a date divider between rows breaks the group`() {
        // 23:58 and 00:01 Los Angeles time: three minutes apart, on two local days.
        assertFalse(grouped(message("m1", "2026-10-07T06:58:00Z"), message("m2", "2026-10-07T07:01:00Z")))
        assertTrue(grouped(message("m1", "2026-10-07T06:55:00Z"), message("m2", "2026-10-07T06:59:00Z")))
    }

    @Test fun `a Replied to a thread broadcast keeps its header in the timeline only`() {
        val first = message("m1", "2026-10-06T19:00:00Z")
        val broadcast = message("m2", "2026-10-06T19:01:00Z", threadRootId = "root00000001", broadcast = true)
        assertFalse(grouped(first, broadcast))
        assertTrue("a thread panel shows no context line", grouped(first, broadcast, inThread = true))
    }

    @Test fun `a thread root is never grouped and its first reply keeps a header`() {
        val root = message("root00000001", "2026-10-06T19:00:00Z")
        val reply = message("m2", "2026-10-06T19:01:00Z", threadRootId = root.id)
        val second = message("m3", "2026-10-06T19:02:00Z", threadRootId = root.id)
        assertFalse(grouped(null, root, inThread = true))
        assertFalse(grouped(root, reply, inThread = true))
        assertTrue(grouped(reply, second, inThread = true))
    }

    @Test fun `the jump target keeps the row below it separate`() {
        val first = message("m1", "2026-10-06T19:00:00Z")
        val second = message("m2", "2026-10-06T19:01:00Z")
        assertFalse(grouped(first, second, jumpTarget = first.id))
        assertTrue(grouped(first, second, jumpTarget = "other0000001"))
    }

    @Test fun `pins and forwards do not break a group`() {
        val pinned = message("m1", "2026-10-06T19:00:00Z").copy(pin = MessagePin(alex, "2026-10-06T19:03:00Z"))
        val forwarded = message("m2", "2026-10-06T19:01:00Z").copy(forward = MessageForward(null, "1"))
        assertTrue(grouped(pinned, forwarded))
        assertTrue(grouped(forwarded, message("m3", "2026-10-06T19:02:00Z")))
    }

    @Test fun `a pending send groups by its author id`() {
        val sent = message("m1", "2026-10-06T19:00:00Z")
        assertTrue(groupsWithPrevious(sent, maya.id, "2026-10-06T19:00:10.250Z", zoneId = zone))
        assertFalse(groupsWithPrevious(sent, alex.id, "2026-10-06T19:00:10.250Z", zoneId = zone))
        // A first reply still pending in a thread keeps its header under the root.
        assertFalse(groupsWithPrevious(sent, maya.id, "2026-10-06T19:00:10Z", threadRootId = sent.id, zoneId = zone))
    }
}
