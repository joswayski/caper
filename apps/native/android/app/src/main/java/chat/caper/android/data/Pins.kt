package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.PinUpdate
import java.math.BigInteger

internal fun PinUpdate.validated(expectedChannel: String, expectedMessage: String? = null): PinUpdate {
    require(type == "message.pin" && schemaVersion == 1) { "Invalid pin update type or schema." }
    require(channelId == expectedChannel && message.channelId == channelId) { "Pin channel mismatch." }
    require(expectedMessage == null || message.id == expectedMessage) { "Pin message mismatch." }
    val revision = seq.toBigIntegerOrNull()
    require(revision != null && revision.signum() >= 0 && revision.toString() == seq && message.pinSeq == seq) { "Invalid pin sequence." }
    message.validated(channelId)
    return this
}

/** Pin and reaction revisions are intentionally merged independently. */
internal fun mergePin(current: ChatMessage, incoming: ChatMessage): ChatMessage {
    if (current.id != incoming.id || current.channelId != incoming.channelId) return current
    val old = current.pinSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
    val next = incoming.pinSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
    return if (next > old) current.copy(pin = incoming.pin, pinSeq = incoming.pinSeq) else current
}

internal fun mergePinned(current: List<ChatMessage>, snapshots: List<ChatMessage>): List<ChatMessage> {
    val byId = (current + snapshots).groupBy { it.id }.mapValues { (_, rows) -> rows.reduce(::mergePin) }.toMutableMap()
    // A newer unpin snapshot must remove a pin even when a stale history list still contains it.
    snapshots.filter { it.pin == null }.forEach { byId[it.id] = mergePin(byId.getValue(it.id), it) }
    return byId.values.filter { it.pin != null }.sortedByDescending { it.pinSeq?.toBigIntegerOrNull() ?: BigInteger.ZERO }
}

internal fun replayCursorAfterPin(current: String?, update: PinUpdate, sequenced: Boolean) = if (sequenced) update.seq else current
