package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import java.util.UUID

internal data class PendingSend(
    val channel: String,
    val authorId: String,
    val text: String,
    val id: UUID,
    val confirmed: () -> Unit,
)

/** Retains an idempotency key only while the server outcome is unknown. */
internal class PendingSendTracker {
    private var pending: PendingSend? = null

    fun begin(channel: String, authorId: String, text: String, confirmed: () -> Unit): PendingSend =
        pending?.takeIf { it.channel == channel && it.authorId == authorId }
            ?: PendingSend(channel, authorId, text, UUID.randomUUID(), confirmed).also { pending = it }

    fun confirm(message: ChatMessage): PendingSend? = pending?.takeIf {
        it.id.toString() == message.clientMessageId && it.channel == message.channelId &&
            it.authorId == message.author.id && !message.author.isGuest
    }?.also { pending = null }

    fun definitiveFailure(id: UUID) {
        if (pending?.id == id) pending = null
    }

    fun clear() { pending = null }
}
