package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.KlipyGif
import chat.caper.android.model.MessageReply
import java.util.UUID

internal data class PendingSend(
    val channel: String,
    val author: ChatAuthor,
    val text: String,
    val id: UUID,
    val confirmed: () -> Unit,
    /** Uploaded file IDs; they join the idempotency hash, so retries resend the same list. */
    val attachmentIds: List<String> = emptyList(),
    val threadRootId: String? = null,
    val broadcast: Boolean = false,
    val replyTo: MessageReply? = null,
    val spoilerAttachmentIds: List<String> = emptyList(),
    /** A KLIPY GIF, part of the idempotency hash like the text. */
    val gif: KlipyGif? = null,
)

/** Retains an idempotency key only while the server outcome is unknown. */
internal class PendingSendTracker {
    private var pending: PendingSend? = null

    fun begin(
        channel: String, author: ChatAuthor, text: String, attachmentIds: List<String> = emptyList(),
        threadRootId: String? = null, broadcast: Boolean = false, replyTo: MessageReply? = null,
        spoilerAttachmentIds: List<String> = emptyList(), gif: KlipyGif? = null, confirmed: () -> Unit,
    ): PendingSend =
        retrying(channel, author)
            ?: PendingSend(channel, author, text, UUID.randomUUID(), confirmed, attachmentIds, threadRootId, broadcast, replyTo, spoilerAttachmentIds, gif).also { pending = it }

    /** The unknown-outcome send a new attempt must repeat, if any. */
    fun retrying(channel: String, author: ChatAuthor): PendingSend? =
        pending?.takeIf { it.channel == channel && it.author.id == author.id && it.author.isGuest == author.isGuest }

    fun confirm(message: ChatMessage): PendingSend? = pending?.takeIf {
        it.id.toString() == message.clientMessageId && it.channel == message.channelId &&
            it.threadRootId == message.threadRootId && it.broadcast == message.broadcast &&
            it.replyTo?.id == message.replyTo?.id &&
            it.author.id == message.author.id && it.author.isGuest == message.author.isGuest
    }?.also { pending = null }

    fun definitiveFailure(id: UUID) {
        if (pending?.id == id) pending = null
    }

    /** Only before HTTP, or after a definitive attachment refusal, never after a timeout. */
    fun replaceAttachments(id: UUID, attachmentIds: List<String>): PendingSend? =
        pending?.takeIf { it.id == id }?.let { old -> old.copy(id = UUID.randomUUID(), attachmentIds = attachmentIds,
            spoilerAttachmentIds = attachmentIds.filterIndexed { index, _ -> old.attachmentIds.getOrNull(index) in old.spoilerAttachmentIds }) }
            ?.also { pending = it }

    fun clear() { pending = null }
}
