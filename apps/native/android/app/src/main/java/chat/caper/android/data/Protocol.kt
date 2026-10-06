package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ChatAuthor
import java.math.BigInteger
import java.time.Instant
import java.util.UUID

private val sequencePattern = Regex("^(0|[1-9][0-9]*)$")

internal fun ChatMessage.validated(
    channelId: String,
    expectedAuthor: ChatAuthor? = null,
    expectedClientMessageId: UUID? = null,
    expectedText: String? = null,
): ChatMessage = apply {
    require(this.channelId == channelId) { "Message channel mismatch." }
    require(sequencePattern.matches(seq) && runCatching { BigInteger(seq) }.isSuccess) { "Invalid message sequence." }
    require(content.version == 1 && content.type == "text") { "Unsupported message content." }
    require(content.text.isNotEmpty() && content.text.codePointCount(0, content.text.length) <= 4000) { "Invalid message text." }
    require(content.text.none { it.isISOControl() && it != '\n' && it != '\t' }) { "Invalid message text." }
    require(runCatching { Instant.parse(createdAt) }.isSuccess) { "Invalid message timestamp." }
    require(revision >= 1) { "Invalid content revision." }
    if (revision == 1) require(editedAt == null && editSeq == null) { "Unexpected edit metadata." }
    else {
        require(editedAt != null && runCatching { Instant.parse(editedAt) }.isSuccess) { "Invalid edit timestamp." }
        require(editSeq != null && sequencePattern.matches(editSeq) && BigInteger(editSeq) > BigInteger(seq)) { "Invalid edit sequence." }
    }
    pinSeq?.let { require(sequencePattern.matches(it) && runCatching { BigInteger(it) }.isSuccess) { "Invalid pin sequence." } }
    pin?.let {
        require(it.author.id.isNotEmpty() && it.author.name.isNotEmpty()) { "Invalid pin author." }
        require(runCatching { Instant.parse(it.createdAt) }.isSuccess) { "Invalid pin timestamp." }
        require(pinSeq != null) { "Pinned message is missing its revision." }
    }
    require(runCatching { UUID.fromString(clientMessageId) }.isSuccess) { "Invalid client message ID." }
    require(!broadcast || threadRootId != null) { "Invalid broadcast reply." }
    thread?.let {
        require(it.replyCount > 0 && it.participants.size <= 5 && it.participants.map { person -> person.id }.distinct().size == it.participants.size && sequencePattern.matches(it.seq)) { "Invalid thread summary." }
    }
    if (expectedAuthor != null) {
        require(author.id == expectedAuthor.id && author.isGuest == expectedAuthor.isGuest) { "Message author mismatch." }
    }
    if (expectedClientMessageId != null) require(clientMessageId == expectedClientMessageId.toString()) { "Message ID mismatch." }
    if (expectedText != null) require(content.text == expectedText) { "Message text mismatch." }
}

internal enum class SendFailure { REVOKED, DEFINITIVE, UNKNOWN }

internal fun classifySendFailure(status: Int): SendFailure = when (status) {
    401, 403 -> SendFailure.REVOKED
    400, 404, 409, 413, 422 -> SendFailure.DEFINITIVE
    else -> SendFailure.UNKNOWN
}
