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
        assertEquals(listOf(DIRECT_MESSAGES_CHANNEL, CHANNEL_MESSAGES_CHANNEL, MENTIONS_CHANNEL), pushChannels.map { it.first })
        assertEquals(listOf("Direct messages", "Channel messages", "Mentions"), pushChannels.map { it.second })
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
}
