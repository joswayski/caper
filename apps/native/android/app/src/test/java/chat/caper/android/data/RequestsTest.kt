package chat.caper.android.data

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectConversationList
import chat.caper.android.model.DirectPeer
import chat.caper.android.model.BlockList
import chat.caper.android.model.DirectPrivacy
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class RequestsTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val peer = DirectPeer("jordan000001", "jordan", "Jordan")
    private fun direct(id: String, status: String?, lastSeq: String = "3", readSeq: String = "0") = DirectConversation(id, peer, lastSeq, readSeq, status)
    private fun message(id: String, author: String, guest: Boolean = false) =
        ChatMessage(id, "channel00001", id.takeLast(1), ChatAuthor(author, author, guest), ChatContent(1, "text", "hi"), "2026-10-07T12:00:00Z", "00000000-0000-4000-8000-000000000001")

    @Test fun `old servers omit status and blocked`() {
        val old = json.decodeFromString<DirectConversationList>("""{"conversations":[{"id":"direct000001","peer":{"id":"alex00000001","username":"alex","displayName":"Alex"},"lastSeq":"4","readSeq":"2"}]}""")
            .conversations.single()
        assertNull(old.status)
        assertFalse(old.incoming || old.outgoing || old.blocked)
        assertNull(old.peer.avatarId)
        assertEquals(listOf(old), mainDirects(listOf(old)))
        assertTrue(directUnread(old))
    }

    @Test fun `new servers send status, blocked and peer avatars`() {
        val list = json.decodeFromString<DirectConversationList>("""{"conversations":[
            {"id":"direct000001","peer":{"id":"alex00000001","username":"alex","displayName":"Alex","avatarId":7},"lastSeq":"4","readSeq":"4","status":"accepted","blocked":true},
            {"id":"direct000002","peer":{"id":"sam000000001","username":"sam","displayName":"Sam","avatarId":null},"lastSeq":"1","readSeq":"0","status":"outgoing","blocked":false},
            {"id":"direct000003","peer":{"id":"jordan000001","username":"jordan","displayName":"Jordan","avatarId":412},"lastSeq":"2","readSeq":"0","status":"incoming","blocked":false}]}""").conversations
        assertEquals(listOf("accepted", "outgoing", "incoming"), list.map { it.status })
        assertEquals(listOf(true, false, false), list.map { it.blocked })
        assertEquals(listOf(7, null, 412), list.map { it.peer.avatarId })
        assertTrue(list[1].outgoing)
        assertTrue(list[2].incoming)
        assertEquals("tom000000001", json.decodeFromString<BlockList>("""{"blocks":[{"id":"tom000000001","username":"tom","displayName":"Tom","avatarId":null}]}""").blocks.single().id)
        assertEquals("spaces", json.decodeFromString<DirectPrivacy>("""{"directMessages":"spaces"}""").directMessages)
    }

    @Test fun `requests stay out of the main list and unread dots`() {
        val accepted = direct("direct000001", "accepted")
        val outgoing = direct("direct000002", "outgoing")
        val incoming = direct("direct000003", "incoming")
        val legacy = direct("direct000004", null)
        val all = listOf(incoming, accepted, outgoing, legacy)
        assertEquals(listOf(accepted, outgoing, legacy), mainDirects(all))
        assertEquals(listOf(incoming), messageRequests(all))
        assertFalse(directUnread(incoming))
        assertTrue(directUnread(accepted))
        assertTrue(directUnread(outgoing))
        assertFalse(directUnread(direct("direct000005", "accepted", lastSeq = "3", readSeq = "3")))
        assertFalse(directUnread(direct("direct000006", "accepted", lastSeq = "bad", readSeq = "0")))
    }

    @Test fun `consecutive blocked messages collapse into runs`() {
        val messages = listOf(
            message("m1", "maya"), message("m2", "maya"), message("m3", "alex"),
            message("m4", "maya"), message("m5", "me"), message("m6", "maya"), message("m7", "tom"),
        )
        val entries = groupBlocked(messages, setOf("maya", "tom"), "me", emptySet())
        assertEquals(5, entries.size)
        val first = entries[0] as TimelineEntry.BlockedRun
        assertEquals("m1", first.key)
        assertEquals(listOf("m1", "m2"), first.messages.map { it.id })
        assertFalse(first.revealed)
        assertEquals(TimelineEntry.Shown(messages[2]), entries[1])
        assertEquals(listOf("m4"), (entries[2] as TimelineEntry.BlockedRun).messages.map { it.id })
        assertEquals(TimelineEntry.Shown(messages[4]), entries[3])
        // Different blocked authors in a row still form one run.
        assertEquals(listOf("m6", "m7"), (entries[4] as TimelineEntry.BlockedRun).messages.map { it.id })
        assertEquals("m1", entries[0].first.id)
        assertEquals("m7", entries[4].last.id)
    }

    @Test fun `revealed runs, guests, yourself and no blocks`() {
        val messages = listOf(message("m1", "maya"), message("m2", "guest1", guest = true), message("m3", "me"))
        val revealed = groupBlocked(messages, setOf("maya", "guest1", "me"), "me", setOf("m1"))
        assertTrue((revealed[0] as TimelineEntry.BlockedRun).revealed)
        // A guest author or your own messages never collapse.
        assertEquals(listOf(TimelineEntry.Shown(messages[1]), TimelineEntry.Shown(messages[2])), revealed.drop(1))
        assertEquals(messages.map { TimelineEntry.Shown(it) }, groupBlocked(messages, emptySet(), "me", emptySet()))
        assertTrue(groupBlocked(emptyList(), setOf("maya"), "me", emptySet()).isEmpty())
        assertEquals("1 blocked message", blockedRunLabel(1))
        assertEquals("2 blocked messages", blockedRunLabel(2))
    }

    @Test fun `privacy and block refusals have their own copy`() {
        assertEquals("This person isn't accepting direct messages.", directMessageError("dm_not_accepted"))
        assertEquals("You blocked this person. Unblock them to message them.", directMessageError("dm_blocked"))
        assertNull(directMessageError(null))
        assertNull(directMessageError("rate_limited"))
        assertEquals(listOf("anyone", "spaces", "nobody"), directPrivacyOptions.map { it.first })
    }
}
