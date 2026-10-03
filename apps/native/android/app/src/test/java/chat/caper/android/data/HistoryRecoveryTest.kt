package chat.caper.android.data

import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatHistory
import chat.caper.android.model.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Test

class HistoryRecoveryTest {
    @Test fun `overlap keeps retained pages and fresh messages win by id`() {
        val stale = message("3", "same", "stale")
        val result = recoverHistory(
            retained = listOf(message("1"), message("2"), stale), retainedHasMore = true,
            appliedCursor = "4", refreshed = history(message("3", "same", "fresh"), message("4")),
        )

        assertEquals(listOf("1", "2", "3", "4"), result.messages.map { it.seq })
        assertEquals("fresh", result.messages.single { it.id == "same" }.content.text)
        assertEquals(true, result.hasMore)
    }

    @Test fun `adjacent refreshed page retains older prefix`() {
        val result = recoverHistory(
            retained = listOf(message("2"), message("3")), retainedHasMore = true,
            appliedCursor = "3", refreshed = history(message("4"), hasMore = false),
        )

        assertEquals(listOf("2", "3", "4"), result.messages.map { it.seq })
        assertEquals(true, result.hasMore)
    }

    @Test fun `gap drops retained range`() {
        val result = recoverHistory(
            retained = listOf(message("1"), message("5")), retainedHasMore = true,
            appliedCursor = "5", refreshed = history(message("7"), hasMore = false),
        )

        assertEquals(listOf("7"), result.messages.map { it.seq })
        assertEquals(false, result.hasMore)
    }

    @Test fun `empty refreshed history does not retain stale range`() {
        val result = recoverHistory(
            retained = listOf(message("1")), retainedHasMore = true,
            appliedCursor = "1", refreshed = history(hasMore = false),
        )

        assertEquals(emptyList<ChatMessage>(), result.messages)
        assertEquals(false, result.hasMore)
    }

    @Test fun `sequence comparison preserves precision above javascript safe integers`() {
        val cursor = "9007199254740992"
        val next = "9007199254740993"
        val result = recoverHistory(
            retained = listOf(message(cursor)), retainedHasMore = false,
            appliedCursor = cursor, refreshed = history(message(next), hasMore = true),
        )

        assertEquals(listOf(cursor, next), result.messages.map { it.seq })
        assertEquals(false, result.hasMore)
    }

    @Test fun `fresh pagination flag wins when no older prefix is retained`() {
        val result = recoverHistory(
            retained = listOf(message("3")), retainedHasMore = true,
            appliedCursor = "3", refreshed = history(message("2"), message("3"), hasMore = false),
        )

        assertEquals(listOf("2", "3"), result.messages.map { it.seq })
        assertEquals(false, result.hasMore)
    }

    @Test fun `highest http confirmed send cannot bridge an event gap`() {
        val result = recoverHistory(
            retained = listOf(message("1"), message("9", "http-send")), retainedHasMore = true,
            appliedCursor = "5", refreshed = history(message("7"), hasMore = false),
        )

        assertEquals(listOf("7"), result.messages.map { it.seq })
        assertEquals(false, result.hasMore)
    }

    private fun history(vararg messages: ChatMessage, hasMore: Boolean = false) =
        ChatHistory(messages.toList(), messages.lastOrNull()?.seq ?: "0", hasMore)

    private fun message(seq: String, id: String = "message-$seq", text: String = seq) = ChatMessage(
        id = id, channelId = "channel", seq = seq,
        author = ChatAuthor("account", "Jose", false), content = ChatContent(1, "text", text),
        createdAt = "2026-10-03T00:00:00Z", clientMessageId = "client-$seq",
    )
}
