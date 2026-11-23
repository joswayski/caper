package chat.caper.android.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ActivityGroupingTest {
    private fun item(id: String, kind: String = "message", actor: String = "Maya", channel: String? = "channel00001",
        conversation: String? = null, thread: String? = null, message: String? = "msg-$id", emoji: String? = null, unread: Boolean = true) =
        ActivityItem(id, kind, "actor-$actor", actor, body = "body $id", createdAt = "2026-11-23T12:00:00Z", unread = unread,
            spaceId = channel?.let { "space0000001" }, channelId = channel, conversationId = conversation,
            messageId = message, threadRootId = thread, emoji = emoji)

    private fun ids(items: List<ActivityItem>) = groupActivity(items).map { it.ids }

    @Test fun `neighbours in one conversation merge and the order never changes`() {
        val groups = groupActivity(listOf(item("a"), item("b", actor = "Alex"), item("c", conversation = "direct000001", channel = null),
            item("d", conversation = "direct000001", channel = null)))
        assertEquals(listOf(listOf("a", "b"), listOf("c", "d")), groups.map { it.ids })
        val group = groups.first()
        assertEquals("a", group.id)
        assertEquals("a", group.newest.id)
        assertEquals("b", group.oldest.id)
        assertEquals("Maya and Alex", group.nameLine)
        assertEquals("2 messages", group.summary)
    }

    @Test fun `same conversation items that are not neighbours stay apart`() {
        assertEquals(listOf(listOf("a"), listOf("b"), listOf("c")),
            ids(listOf(item("a"), item("b", channel = "channel00002"), item("c"))))
    }

    @Test fun `read and unread neighbours do not merge`() {
        assertEquals(listOf(listOf("a"), listOf("b")), ids(listOf(item("a"), item("b", unread = false))))
        assertEquals(listOf(listOf("a", "b")), ids(listOf(item("a", unread = false), item("b", unread = false))))
        assertEquals(listOf(listOf("a"), listOf("b")), ids(listOf(item("a", "reaction", message = "m1", emoji = "👍"),
            item("b", "reaction", message = "m1", emoji = "👍", unread = false))))
    }

    @Test fun `different threads and kinds do not merge`() {
        assertEquals(listOf(listOf("a"), listOf("b"), listOf("c")),
            ids(listOf(item("a", "reply", thread = "root1"), item("b", "reply", thread = "root2"), item("c", "reply"))))
        assertEquals(listOf(listOf("a"), listOf("b")), ids(listOf(item("a", "message"), item("b", "mention"))))
        val replies = groupActivity(listOf(item("a", "reply", thread = "root1"), item("b", "reply", thread = "root1")))
        assertEquals("2 replies", replies.single().summary)
        assertEquals("Mentioned you 3 times", groupActivity(List(3) { item("m$it", "mention") }).single().summary)
    }

    @Test fun `reactions group by message with distinct emojis newest first`() {
        val groups = groupActivity(listOf(
            item("a", "reaction", actor = "Maya", message = "m1", emoji = "🎉"),
            item("b", "reaction", actor = "Alex", message = "m1", emoji = "👍"),
            item("c", "reaction", actor = "Sam", message = "m1", emoji = "🎉"),
            item("d", "reaction", actor = "Maya", message = "m1", emoji = "👍"),
            item("e", "reaction", actor = "Maya", message = "m2", emoji = "👍"),
        ))
        assertEquals(listOf(listOf("a", "b", "c", "d"), listOf("e")), groups.map { it.ids })
        assertEquals("Maya and 2 others", groups[0].nameLine)
        assertEquals("Reacted 🎉 👍 to your message", groups[0].summary)
        assertEquals("Maya", groups[1].nameLine)
        assertEquals("Reacted 👍 to your message", groups[1].summary)
    }

    @Test fun `invitations and requests are always their own row`() {
        for (kind in listOf("space.invitation", "channel.invitation", "direct.request", "something.new")) {
            assertNull(activityGroupKey(item("x", kind)))
            assertEquals(listOf(listOf("a"), listOf("b")), ids(listOf(item("a", kind), item("b", kind))))
        }
        assertEquals("Invited you to a space", groupActivity(listOf(item("a", "space.invitation"))).single().summary)
        assertNull("a single message keeps today's row (no description)", groupActivity(listOf(item("a"))).single().summary)
    }

    @Test fun `a group stops at the read request limit`() {
        val groups = groupActivity(List(ACTIVITY_GROUP_MAX + 1) { item("i$it") })
        assertEquals(listOf(ACTIVITY_GROUP_MAX, 1), groups.map { it.items.size })
    }
}
