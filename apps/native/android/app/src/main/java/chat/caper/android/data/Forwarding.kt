package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ForwardUpdate
import java.math.BigInteger

internal fun ForwardUpdate.validated(channel: String): ForwardUpdate = apply {
    require(type == "message.forward" && schemaVersion == 1 && channelId == channel) { "Invalid forward event." }
    require(Regex("^(0|[1-9][0-9]*)$").matches(seq) && message.forwardSeq == seq && message.forward != null) { "Invalid forward revision." }
    message.validated(channel)
}

/** A source snapshot and the destination event revision advance independently. */
internal fun mergeForward(message: ChatMessage, incoming: ChatMessage): ChatMessage {
    if (message.id != incoming.id || message.channelId != incoming.channelId || incoming.forward == null) return message
    val current = message.forward
    val next = incoming.forward
    val shared = if (current != null && next.message != null &&
        (current.message == null || BigInteger(current.seq) > BigInteger(next.seq))) current else next
    val revision = if (BigInteger(message.forwardSeq ?: "0") > BigInteger(incoming.forwardSeq ?: "0")) message.forwardSeq else incoming.forwardSeq
    return message.copy(forward = shared, forwardSeq = revision)
}
