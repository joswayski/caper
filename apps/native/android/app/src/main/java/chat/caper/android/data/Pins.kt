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

/** Complete pin history covers unloaded messages as well as known snapshots. */
internal fun overlayPin(message: ChatMessage, snapshot: ChatMessage?, historyCursor: String?): ChatMessage {
    val cursor = historyCursor?.toBigIntegerOrNull()
    val revision = message.pinSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
    val authoritative = if (cursor != null && revision <= cursor) {
        message.copy(pin = snapshot?.pin, pinSeq = snapshot?.pinSeq ?: historyCursor)
    } else message
    return snapshot?.let { mergePin(authoritative, it) } ?: authoritative
}

internal fun replayCursorAfterPin(current: String?, update: PinUpdate, sequenced: Boolean) = if (sequenced) update.seq else current
