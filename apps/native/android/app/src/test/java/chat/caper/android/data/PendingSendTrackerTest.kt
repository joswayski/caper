package chat.caper.android.data

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import org.junit.Assert.*
import org.junit.Test

class PendingSendTrackerTest {
    @Test fun `unknown outcome retains id and original text despite edits`() {
        val tracker = PendingSendTracker()
        val first = tracker.begin("channel", author(), "original") {}
        assertEquals(first.id, tracker.begin("channel", author(), "original") {}.id)
        val retry = tracker.begin("channel", author(), "edited") {}
        assertEquals(first.id, retry.id)
        assertEquals("original", retry.text)
    }

    @Test fun `definitive rejection unlocks a new id`() {
        val tracker = PendingSendTracker()
        val first = tracker.begin("channel", author(), "text") {}
        tracker.definitiveFailure(first.id)
        assertNotEquals(first.id, tracker.begin("channel", author(), "text") {}.id)
    }

    @Test fun `only matching own gateway event confirms pending send exactly once`() {
        val tracker = PendingSendTracker()
        var confirmations = 0
        val pending = tracker.begin("channel", author(), "text") { confirmations++ }
        assertNull(tracker.confirm(message(pending.id.toString(), author = "other")))
        assertNull(tracker.confirm(message(pending.id.toString(), channel = "other")))
        tracker.confirm(message(pending.id.toString()))?.confirmed?.invoke()
        tracker.confirm(message(pending.id.toString()))?.confirmed?.invoke()
        assertEquals(1, confirmations)
    }

    @Test fun `channel switch clears unknown outcome`() {
        val tracker = PendingSendTracker()
        val first = tracker.begin("old-channel", author(), "text") {}
        tracker.clear()
        assertNotEquals(first.id, tracker.begin("old-channel", author(), "text") {}.id)
    }

    @Test fun `matching guest chat identity confirms its pending send`() {
        val tracker = PendingSendTracker()
        val guest = ChatAuthor("guest-account", "Guest", true)
        val pending = tracker.begin("channel", guest, "text") {}
        assertNotNull(tracker.confirm(message(pending.id.toString(), author = guest.id, guest = true)))
    }

    private fun author() = ChatAuthor("account", "Jose", false)

    private fun message(id: String, channel: String = "channel", author: String = "account", guest: Boolean = false) = ChatMessage(
        id = "message00001", channelId = channel, seq = "1",
        author = ChatAuthor(author, "Jose", guest), content = ChatContent(1, "text", "text"),
        createdAt = "2026-09-23T00:00:00Z", clientMessageId = id,
    )
}
