package chat.caper.android.data

import chat.caper.android.model.*
import org.junit.Assert.*
import org.junit.Test

class ForwardingTest {
    private fun wrapper(sourceSeq: String, destinationSeq: String): ChatMessage {
        val source = ChatMessage("source000000001", "private00001", "89", ChatAuthor("alice", "Alice", false),
            ChatContent(1, "text", "source $sourceSeq"), "2026-10-06T12:00:00Z", "00000000-0000-4000-8000-000000000001")
        return ChatMessage("wrapper00000001", "channel00001", "1", ChatAuthor("bob", "Bob", false),
            ChatContent(1, "text", "note"), "2026-10-06T12:01:00Z", "00000000-0000-4000-8000-000000000002",
            forward = MessageForward(source, sourceSeq), forwardSeq = destinationSeq)
    }

    @Test fun `source cursor does not regress when destination delivery advances`() {
        val merged = mergeForward(wrapper("9007199254740995", "1"), wrapper("9007199254740993", "2"))
        assertEquals("9007199254740995", merged.forward?.seq)
        assertEquals("source 9007199254740995", merged.forward?.message?.content?.text)
        assertEquals("2", merged.forwardSeq)
        assertEquals("1", merged.seq)
        assertEquals("note", merged.content.text)
    }

    @Test fun `stale confirmation cannot restore an unavailable original or cross channels`() {
        val removed = wrapper("0", "12").copy(forward = MessageForward(null, "0"))
        assertNull(mergeForward(removed, wrapper("9007199254740993", "1")).forward?.message)
        assertEquals(removed, mergeForward(removed, wrapper("20", "13").copy(channelId = "other0000001")))
        val valid = ForwardUpdate("message.forward", 1, "channel00001", "2", wrapper("20", "2"))
        assertEquals(valid, valid.validated("channel00001"))
        assertThrows(IllegalArgumentException::class.java) { valid.copy(seq = "3").validated("channel00001") }
    }
}
