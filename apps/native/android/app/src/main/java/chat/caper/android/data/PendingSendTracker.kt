package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ChatAuthor
import java.util.UUID

internal data class PendingSend(
    val channel: String,
    val author: ChatAuthor,
    val text: String,
    val id: UUID,
    val confirmed: () -> Unit,
    val threadRootId: String? = null,
    val broadcast: Boolean = false,
)

/** Retains an idempotency key only while the server outcome is unknown. */
internal class PendingSendTracker {
    private var pending: PendingSend? = null

    fun begin(channel: String, author: ChatAuthor, text: String, threadRootId: String? = null, broadcast: Boolean = false, confirmed: () -> Unit): PendingSend =
        pending?.takeIf { it.channel == channel && it.author.id == author.id && it.author.isGuest == author.isGuest }
            ?: PendingSend(channel, author, text, UUID.randomUUID(), confirmed, threadRootId, broadcast).also { pending = it }

    fun confirm(message: ChatMessage): PendingSend? = pending?.takeIf {
        it.id.toString() == message.clientMessageId && it.channel == message.channelId &&
            it.threadRootId == message.threadRootId && it.broadcast == message.broadcast &&
            it.author.id == message.author.id && it.author.isGuest == message.author.isGuest
    }?.also { pending = null }

    fun definitiveFailure(id: UUID) {
        if (pending?.id == id) pending = null
    }

    fun clear() { pending = null }
}
