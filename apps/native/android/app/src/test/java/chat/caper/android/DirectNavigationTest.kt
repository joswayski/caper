package chat.caper.android

import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectPeer
import org.junit.Assert.assertEquals
import org.junit.Test

class DirectNavigationTest {
    @Test fun `opened conversations precede existing server order and preserve sequence strings`() {
        val peer = DirectPeer("account00002", "mira", "Mira")
        val result = mergeDirects(
            listOf(DirectConversation("direct000001", peer, "9007199254740993", "7"), DirectConversation("direct000003", peer, "14", "0")),
            listOf(DirectConversation("direct000002", peer, "2", "0"), DirectConversation("direct000001", peer, "9007199254740993", "8")),
        )
        // Sequences from different conversations do not describe recency.
        assertEquals(listOf("direct000002", "direct000001", "direct000003"), result.map { it.id })
        assertEquals("8", result[1].readSeq)
        assertEquals("9007199254740993", result[1].lastSeq)
    }
}
