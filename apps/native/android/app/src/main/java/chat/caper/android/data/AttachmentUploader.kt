package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import java.io.InputStream
import kotlinx.coroutines.delay

/**
 * A picked original, uploaded byte for byte. [size] is exact (the presigned PUT signs it) and
 * [open] returns a fresh stream each time, so a retried request can resend from the start.
 */
interface UploadSource {
    val name: String
    /** Declared MIME type; `application/octet-stream` when unknown. */
    val contentType: String
    val size: Long
    fun open(): InputStream
}

/**
 * Reserve, upload the original straight to storage, confirm, then send its ID with a message
 * (docs/media.md "Client upload flow"). Bytes never pass through the API and are never
 * re-encoded here: the server's media worker compresses every file.
 */
class AttachmentUploader(
    private val api: CaperApi,
    /** Waits between `/complete` attempts while storage reports the upload has not arrived (409). */
    private val completeRetryDelaysMs: List<Long> = listOf(500, 1_000, 2_000, 4_000),
) {
    suspend fun upload(
        token: String, channelId: String, source: UploadSource, maxUploadBytes: Long? = null, progress: (Float) -> Unit = {},
    ): ChatAttachment {
        val size = source.size
        if (size < 1) throw UploadException("This file is empty.")
        if (maxUploadBytes != null && maxUploadBytes > 0 && size > maxUploadBytes) {
            throw UploadException("Files can be up to ${AttachmentPolicy.formatBytes(maxUploadBytes)}.")
        }
        val reservation = api.createAsset(token, channelId, source.name, AttachmentPolicy.normalizedType(source.contentType), size)
        api.putUpload(reservation.upload, size, source::open) { sent -> progress(sent.toFloat() / size) }
        val attachment = complete(token, reservation.id)
        if (attachment.id != reservation.id) throw UploadException("The upload service returned an invalid response.")
        progress(1f)
        return attachment
    }

    private suspend fun complete(token: String, id: String): ChatAttachment {
        for (wait in completeRetryDelaysMs) {
            try {
                return api.completeAsset(token, id)
            } catch (error: ApiException) {
                if (error.status != 409) throw error
            }
            delay(wait)
        }
        return api.completeAsset(token, id)
    }
}
