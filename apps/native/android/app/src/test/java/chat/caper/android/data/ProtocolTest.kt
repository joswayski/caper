package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class ProtocolTest {
    @Test fun `chat sequence remains a decimal string without precision loss`() {
        val message = Json.decodeFromString<ChatMessage>(
            """{"id":"message00001","channelId":"channel00001","seq":"9007199254740993","author":{"id":"account00001","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-09-23T00:00:00Z","clientMessageId":"id"}""",
        )
        assertEquals("9007199254740993", message.seq)
    }

    @Test fun `path ids reject traversal and wrong lengths`() {
        assertEquals("AbCd1234EfGh", "AbCd1234EfGh".pathId())
        assertThrows(IllegalArgumentException::class.java) { "../messages".pathId() }
        assertThrows(IllegalArgumentException::class.java) { "short".pathId() }
    }

    @Test fun `send failures distinguish definitive rejection from uncertain server outage`() {
        assertEquals(SendFailure.DEFINITIVE, classifySendFailure(422))
        assertEquals(SendFailure.REVOKED, classifySendFailure(403))
        assertEquals(SendFailure.UNKNOWN, classifySendFailure(503))
    }

    @Test fun `message limit counts Unicode code points not UTF16 units`() {
        val message = Json.decodeFromString<ChatMessage>(
            """{"id":"message00001","channelId":"channel00001","seq":"1","author":{"id":"account00001","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-09-23T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}""",
        )
        val limit = message.copy(content = message.content.copy(text = "🪐".repeat(4000)))
        assertEquals(limit, limit.validated("channel00001"))
        val tooLong = message.copy(content = message.content.copy(text = "🪐".repeat(4001)))
        assertThrows(IllegalArgumentException::class.java) { tooLong.validated("channel00001") }
    }
}
