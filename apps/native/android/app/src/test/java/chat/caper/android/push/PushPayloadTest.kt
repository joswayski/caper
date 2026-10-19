package chat.caper.android.push

import org.junit.Assert.*
import org.junit.Test

class PushPayloadTest {
    private val direct = mapOf(
        "kind" to "direct.message",
        "messageId" to "message00000001",
        "conversationId" to "direct000001",
        "title" to "Maya",
        "body" to "Are you around?",
        "sender" to "Maya",
        "senderId" to "account00002",
        "senderAvatarId" to "317",
    )
    private val channel = mapOf(
        "kind" to "channel.message",
        "messageId" to "message00000002",
        "spaceId" to "space0000001",
        "channelId" to "channel00001",
        "title" to "Maya · #general (Caper)",
        "body" to "Standup in five",
        "sender" to "Maya",
        "senderId" to "account00002",
        "conversationTitle" to "#general (Caper)",
    )

    @Test fun `a DM is one person's conversation in the direct messages channel`() {
        val payload = requireNotNull(PushPayload.parse(direct))
        assertTrue(payload.direct)
        assertEquals("direct000001", payload.tag)
        assertEquals(DIRECT_MESSAGES_CHANNEL, payload.channel)
        assertFalse(payload.groupConversation)
        assertNull(payload.conversationTitle)
        assertEquals("Maya", payload.title)
        assertEquals("Maya", payload.sender)
        assertEquals("account00002", payload.senderId)
        assertEquals(317, payload.senderAvatarId)
        assertEquals("Are you around?", payload.body)
        assertNull(payload.spaceId)
        assertNull(payload.channelId)
    }

    @Test fun `a channel message is a group conversation titled with the channel`() {
        val payload = requireNotNull(PushPayload.parse(channel))
        assertFalse(payload.direct)
        assertEquals("channel00001", payload.tag)
        assertEquals("space0000001", payload.spaceId)
        assertEquals(CHANNEL_MESSAGES_CHANNEL, payload.channel)
        assertTrue(payload.groupConversation)
        assertEquals("#general (Caper)", payload.conversationTitle)
        assertEquals("Maya · #general (Caper)", payload.title)
    }

    @Test fun `voice start needs notification ID not message and has no thread target`() {
        val data = (channel - "messageId") + mapOf("kind" to "voice.started", "notificationId" to "94", "threadRootId" to "message00000001")
        val payload = requireNotNull(PushPayload.parse(data))
        assertEquals("94", payload.notificationId)
        assertEquals("voice:94", payload.messageId)
        assertNull(payload.threadRootId)
        assertFalse(payload.consent)
        assertNull(PushPayload.parse(data - "notificationId"))
        assertNull(PushPayload.parse(data + ("conversationId" to "direct000001")))
    }

    @Test fun `mentions use the mentions channel and unknown kinds are plain messages`() {
        assertEquals(MENTIONS_CHANNEL, PushPayload.parse(channel + ("kind" to "mention.user"))?.channel)
        assertEquals(MENTIONS_CHANNEL, PushPayload.parse(channel + ("kind" to "mention.everyone"))?.channel)
        assertEquals(CHANNEL_MESSAGES_CHANNEL, PushPayload.parse(channel + ("kind" to "reaction.added"))?.channel)
        assertEquals(CHANNEL_MESSAGES_CHANNEL, PushPayload.parse(channel - "kind")?.channel)
        assertEquals(DIRECT_MESSAGES_CHANNEL, PushPayload.parse(direct + ("kind" to "something.new"))?.channel)
        assertEquals(listOf(DIRECT_MESSAGES_CHANNEL, CHANNEL_MESSAGES_CHANNEL, MENTIONS_CHANNEL, INVITATIONS_CHANNEL, MESSAGE_REQUESTS_CHANNEL), pushChannels.map { it.first })
        assertEquals(listOf("Direct messages", "Channel messages", "Mentions", "Invitations", "Message requests"), pushChannels.map { it.second })
    }

    @Test fun `invitations have independent tags and do not require a message or grant`() {
        val invitation = mapOf("kind" to "space.invitation", "notificationId" to "91",
            "spaceId" to "space0000001", "title" to "You’re invited!", "body" to "Maya invited you to Studio.")
        val space = requireNotNull(PushPayload.parse(invitation))
        assertTrue(space.invitation)
        assertEquals(INVITATIONS_CHANNEL, space.channel)
        assertEquals("invitation:91", space.tag)
        assertNull(space.channelId)
        val privateInvite = invitation + mapOf("kind" to "channel.invitation", "notificationId" to "92", "channelId" to "channel00001")
        assertEquals("invitation:92", PushPayload.parse(privateInvite)?.tag)
        assertNull(PushPayload.parse(invitation - "notificationId"))
        assertNull(PushPayload.parse(invitation + ("notificationId" to "0")))
        assertNull(PushPayload.parse(invitation + ("channelId" to "channel00001")))
        assertNull(PushPayload.parse(privateInvite - "channelId"))
        assertNull(PushPayload.parse(privateInvite + ("conversationId" to "direct000001")))
    }

    @Test fun `message requests have independent tags and open the DM review without a message`() {
        val request = mapOf("kind" to "direct.request", "notificationId" to "93",
            "conversationId" to "direct000001", "title" to "Message request", "body" to "Maya wants to message you.")
        val payload = requireNotNull(PushPayload.parse(request))
        assertTrue(payload.consent)
        assertFalse(payload.invitation)
        assertEquals("direct000001", payload.conversationId)
        assertEquals("request:93", payload.tag)
        assertEquals(MESSAGE_REQUESTS_CHANNEL, payload.channel)
        assertEquals("Maya wants to message you.", payload.body)
        assertNull(payload.attachment)
        assertNull(PushPayload.parse(request - "notificationId"))
        assertNull(PushPayload.parse(request - "conversationId"))
        assertNull(PushPayload.parse(request + ("spaceId" to "space0000001")))
    }

    @Test fun `feedback reports use the mentions channel and carry their thread`() {
        val report = requireNotNull(PushPayload.parse(channel + mapOf("kind" to "feedback", "title" to "Maya · Feedback", "threadRootId" to "message00000001")))
        assertEquals(MENTIONS_CHANNEL, report.channel)
        assertEquals("feedback", report.kind)
        assertEquals("Maya · Feedback", report.title)
        assertEquals("message00000001", report.threadRootId)
        assertEquals("channel00001", report.tag)
        val inDirect = requireNotNull(PushPayload.parse(direct + ("kind" to "feedback")))
        assertEquals(MENTIONS_CHANNEL, inDirect.channel)
        assertNull(inDirect.threadRootId)
        // Any kind may carry a thread; a malformed one is dropped, never the notification.
        assertEquals("message00000009", PushPayload.parse(direct + ("threadRootId" to "message00000009"))?.threadRootId)
        assertNull(requireNotNull(PushPayload.parse(direct + ("threadRootId" to "../thread"))).threadRootId)
    }

    @Test fun `empty text reads as sent a message and missing names fall back`() {
        assertEquals("Sent a message", PushPayload.parse(direct + ("body" to "  "))?.body)
        assertEquals("Sent a message", PushPayload.parse(direct - "body")?.body)
        val untitled = requireNotNull(PushPayload.parse(channel - "title"))
        assertEquals("Maya · #general (Caper)", untitled.title)
        assertEquals("Maya", requireNotNull(PushPayload.parse(channel - "sender")).sender)
        assertEquals("Maya", requireNotNull(PushPayload.parse(direct - "title")).title)
        val bare = requireNotNull(PushPayload.parse(mapOf("messageId" to "message00000003", "conversationId" to "direct000001")))
        assertEquals("Caper", bare.title)
        assertEquals("Caper", bare.sender)
        assertNull(PushPayload.parse(direct + ("conversationTitle" to "#general (Caper)"))?.conversationTitle)
    }

    @Test fun `long text is bounded on a code point`() {
        val body = requireNotNull(PushPayload.parse(direct + ("body" to "🙂".repeat(1_200)))).body
        assertEquals(1_001, body.codePointCount(0, body.length))
        assertTrue(body.endsWith("🙂…"))
    }

    @Test fun `malformed payloads show nothing`() {
        assertNull(PushPayload.parse(direct - "messageId"))
        assertNull(PushPayload.parse(direct + ("messageId" to "short")))
        assertNull(PushPayload.parse(direct + ("conversationId" to "../../etc000")))
        assertNull(PushPayload.parse(direct + ("channelId" to "channel00001")))
        assertNull(PushPayload.parse(channel - "spaceId"))
        assertNull(PushPayload.parse(channel + ("channelId" to "bad")))
        assertNull(PushPayload.parse(mapOf("messageId" to "message00000001")))
    }

    @Test fun `missing or invalid avatars preserve text notifications`() {
        for (value in listOf("-1", "800", "1.5", "avatar", "999999999999999")) {
            val payload = requireNotNull(PushPayload.parse(direct + ("senderAvatarId" to value)))
            assertNull(payload.senderAvatarId)
            assertEquals("Are you around?", payload.body)
        }
        assertNull(PushPayload.parse(direct - "senderAvatarId")?.senderAvatarId)
        for (index in listOf(0, 799)) assertEquals(index, PushPayload.parse(channel + ("senderAvatarId" to "$index"))?.senderAvatarId)
    }

    @Test fun `one bounded image or poster is optional and invalid media preserves the message`() {
        val media = mapOf("attachmentUrl" to "https://cdn.caper.chat/assets/photo1?exp=123&sig=test",
            "attachmentContentType" to "image/webp", "attachmentSize" to "50000000")
        assertEquals(50_000_000L, PushPayload.parse(direct + media)?.attachment?.size)
        assertEquals(50_000_000L, PushPayload.parse(direct + media +
            ("attachmentUrl" to "https://cdn.caper.chat/original/photo1?exp=123&sig=test"))?.attachment?.size)
        val poster = requireNotNull(PushPayload.parse(direct + media + mapOf(
            "attachmentUrl" to "https://cdn.caper.chat/preview/video1?exp=123&sig=test",
            "attachmentContentType" to "image/jpeg", "attachmentSize" to "321"))?.attachment)
        assertEquals("image/jpeg", poster.contentType)
        assertEquals(321L, poster.size)
        assertNull(PushPayload.parse(direct)?.attachment)
        for (bad in listOf(
            media + ("attachmentSize" to "50000001"), media + ("attachmentSize" to "0"),
            media + ("attachmentUrl" to "http://cdn.caper.chat/assets/photo1"),
            media + ("attachmentUrl" to "https://cdn.caper.chat/other/photo1"),
            media + ("attachmentUrl" to "https://user:password@cdn.caper.chat/assets/photo1"),
            media + ("attachmentContentType" to "video/mp4"),
            media + ("attachmentContentType" to "application/pdf"), media - "attachmentSize",
        )) {
            val payload = requireNotNull(PushPayload.parse(direct + bad))
            assertNull(payload.attachment)
            assertEquals("Are you around?", payload.body)
        }
    }

    @Test fun `a retraction names the deleted message and its conversation, never a message to show`() {
        val dm = mapOf("kind" to "message.deleted", "deletedMessageId" to "message00000001", "conversationId" to "direct000001")
        assertEquals(PushRetraction("message00000001", "direct000001", null, null), PushRetraction.parse(dm))
        assertEquals("direct000001", PushRetraction.parse(dm)?.tag)
        val inChannel = mapOf("kind" to "message.deleted", "deletedMessageId" to "message00000002", "spaceId" to "space0000001", "channelId" to "channel00001")
        assertEquals("channel00001", PushRetraction.parse(inChannel)?.tag)
        // Even with a stray messageId, a retraction never posts a notification of its own.
        assertNull(PushPayload.parse(dm + ("messageId" to "message00000001")))
        assertNull(PushPayload.parse(inChannel + direct.filterKeys { it != "kind" && it != "conversationId" } + ("messageId" to "message00000002")))
        assertNull(PushRetraction.parse(direct))
        for (bad in listOf(
            dm - "deletedMessageId", dm + ("deletedMessageId" to "short"), dm + ("kind" to "direct.message"),
            dm + ("conversationId" to "bad"), dm + ("spaceId" to "space0000001"),
            inChannel - "channelId", inChannel - "spaceId", inChannel + ("channelId" to "../channel01"),
        )) assertNull(bad.toString(), PushRetraction.parse(bad))
    }

    @Test fun `a retraction removes exactly the line that shows its message`() {
        // Lines posted by this version carry their message ID.
        assertEquals(1, retractedLine(listOf("message00000001", "message00000002"), listOf("message00000001", "message00000002"), "message00000002"))
        assertEquals(0, retractedLine(listOf("message00000001"), emptyList(), "message00000001"))
        assertNull(retractedLine(listOf("message00000001"), listOf("message00000001"), "message00000009"))
        // Older lines fall back to the notification's ID list when it lines up one to one.
        assertEquals(0, retractedLine(listOf(null, null), listOf("message00000001", "message00000002"), "message00000001"))
        assertEquals(0, retractedLine(listOf(null, "message00000002"), listOf("message00000001", "message00000002"), "message00000001"))
        assertNull("counts differ", retractedLine(listOf(null), listOf("message00000001", "message00000002"), "message00000001"))
        assertNull("order differs", retractedLine(listOf(null, "message00000001"), listOf("message00000001", "message00000002"), "message00000002"))
        assertNull(retractedLine(emptyList(), emptyList(), "message00000001"))
    }

    @Test fun `retracted ids are remembered briefly so a late original is never shown`() {
        var now = 0L
        val recent = RecentRetractions(nowMs = { now }, ttlMs = 1_000, max = 2)
        recent.add("message00000001")
        assertTrue("message00000001" in recent)
        assertFalse("message00000002" in recent)
        now = 1_001
        assertFalse("forgotten after a while", "message00000001" in recent)
        recent.add("message00000001"); recent.add("message00000002"); recent.add("message00000003")
        assertFalse("bounded", "message00000001" in recent)
        assertTrue("message00000002" in recent && "message00000003" in recent)
    }
}
