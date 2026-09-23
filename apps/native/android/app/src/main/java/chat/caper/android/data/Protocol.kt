package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import java.time.Instant
import java.util.UUID

private val sequencePattern = Regex("^(0|[1-9][0-9]*)$")

internal fun ChatMessage.validated(
    channelId: String,
    expectedAuthorId: String? = null,
    expectedClientMessageId: UUID? = null,
    expectedText: String? = null,
): ChatMessage = apply {
    require(this.channelId == channelId) { "Message channel mismatch." }
    require(sequencePattern.matches(seq) && seq.toLongOrNull() != null) { "Invalid message sequence." }
    require(content.version == 1 && content.type == "text") { "Unsupported message content." }
    require(content.text.isNotEmpty() && content.text.codePointCount(0, content.text.length) <= 4000) { "Invalid message text." }
    require(content.text.none { it.isISOControl() && it != '\n' && it != '\t' }) { "Invalid message text." }
    require(runCatching { Instant.parse(createdAt) }.isSuccess) { "Invalid message timestamp." }
    require(runCatching { UUID.fromString(clientMessageId) }.isSuccess) { "Invalid client message ID." }
    if (expectedAuthorId != null) {
        require(author.id == expectedAuthorId && !author.isGuest) { "Message author mismatch." }
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
