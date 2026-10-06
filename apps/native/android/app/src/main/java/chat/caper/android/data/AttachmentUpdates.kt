package chat.caper.android.data

import chat.caper.android.model.AttachmentsUpdate
import chat.caper.android.model.ChatMessage
import java.math.BigInteger
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

/** Validates a `message.attachments` event exactly like a `message.reactions` one. */
internal fun AttachmentsUpdate.validated(expectedChannel: String): AttachmentsUpdate {
    require(type == "message.attachments" && schemaVersion == 1) { "Invalid attachments update type or schema." }
    require(channelId == expectedChannel && channelId.isNotEmpty()) { "Attachments channel mismatch." }
    require(messageId.isNotEmpty()) { "Attachments message mismatch." }
    val number = seq.toBigIntegerOrNull()
    require(number != null && number.signum() >= 0 && number.toString() == seq) { "Invalid attachments sequence." }
    return this
}

private val updateJson = Json { ignoreUnknownKeys = true }

internal fun attachmentsSequence(event: JsonObject, channelId: String): String =
    updateJson.decodeFromJsonElement(AttachmentsUpdate.serializer(), event).validated(channelId).seq

private fun String?.sequenceOrZero(): BigInteger = this?.toBigIntegerOrNull() ?: BigInteger.ZERO

/**
 * Applies only a strictly newer attachments snapshot (`seq > attachmentsSeq ?? 0`), replacing the
 * message's files, so a replayed "processing" event cannot overwrite a newer "ready" one.
 */
internal fun mergeAttachments(message: ChatMessage, update: AttachmentsUpdate): ChatMessage {
    if (message.id != update.messageId || message.channelId != update.channelId) return message
    val next = update.seq.toBigIntegerOrNull() ?: return message
    if (next <= message.attachmentsSeq.sequenceOrZero()) return message
    return message.copy(content = message.content.copy(attachments = update.attachments), attachmentsSeq = update.seq)
}

/** Takes [candidate]'s files into [current] only when its `attachmentsSeq` is newer. */
internal fun newerAttachments(current: ChatMessage, candidate: ChatMessage): ChatMessage =
    if (candidate.attachmentsSeq.sequenceOrZero() > current.attachmentsSeq.sequenceOrZero()) {
        current.copy(content = current.content.copy(attachments = candidate.content.attachments), attachmentsSeq = candidate.attachmentsSeq)
    } else current

/** Returns false when a new unseen message would exceed the bound and needs a resync. */
internal fun cacheUnseenAttachments(
    unseen: MutableMap<String, AttachmentsUpdate>, update: AttachmentsUpdate, limit: Int = 256,
): Boolean {
    val old = unseen[update.messageId]
    if (old == null && unseen.size >= limit) return false
    val next = update.seq.toBigIntegerOrNull() ?: return true
    if (old == null || next > old.seq.sequenceOrZero()) unseen[update.messageId] = update
    return true
}

/** An ephemeral `attachment.progress` event; malformed ones are dropped, never fatal. */
data class AttachmentProgress(val messageId: String, val attachmentId: String, val percent: Int)

internal fun attachmentProgress(event: JsonObject, channelId: String): AttachmentProgress? = runCatching {
    if (event["channelId"]?.jsonPrimitive?.content != channelId) return null
    val messageId = event["messageId"]?.jsonPrimitive?.content?.takeIf { it.isNotEmpty() } ?: return null
    val attachmentId = event["attachmentId"]?.jsonPrimitive?.content?.takeIf { it.isNotEmpty() } ?: return null
    val percent = event["percent"]?.jsonPrimitive?.let { it.intOrNull ?: it.content.toDoubleOrNull()?.toInt() } ?: return null
    AttachmentProgress(messageId, attachmentId, percent.coerceIn(0, 100))
}.getOrNull()
