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

    @Test fun `mentions use the mentions channel and unknown kinds are plain messages`() {
        assertEquals(MENTIONS_CHANNEL, PushPayload.parse(channel + ("kind" to "mention.user"))?.channel)
        assertEquals(MENTIONS_CHANNEL, PushPayload.parse(channel + ("kind" to "mention.everyone"))?.channel)
        assertEquals(CHANNEL_MESSAGES_CHANNEL, PushPayload.parse(channel + ("kind" to "reaction.added"))?.channel)
        assertEquals(CHANNEL_MESSAGES_CHANNEL, PushPayload.parse(channel - "kind")?.channel)
        assertEquals(DIRECT_MESSAGES_CHANNEL, PushPayload.parse(direct + ("kind" to "something.new"))?.channel)
        assertEquals(listOf(DIRECT_MESSAGES_CHANNEL, CHANNEL_MESSAGES_CHANNEL, MENTIONS_CHANNEL, INVITATIONS_CHANNEL), pushChannels.map { it.first })
        assertEquals(listOf("Direct messages", "Channel messages", "Mentions", "Invitations"), pushChannels.map { it.second })
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
}
