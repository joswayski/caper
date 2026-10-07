package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import java.io.File
import kotlinx.coroutines.delay

/**
 * A file ready to upload: the compressed (or unchanged original) bytes in a local working copy,
 * measured metadata and an optional preview (docs/media.md "Client compression and previews").
 */
data class PreparedAttachment(
    val file: File,
    val name: String,
    val contentType: String,
    val kind: String,
    /** Size of the picked file before compression (`sourceByteSize`). */
    val sourceSize: Long,
    val width: Int? = null,
    val height: Int? = null,
    val durationMs: Long? = null,
    val preview: File? = null,
    val previewContentType: String? = null,
)

/**
 * Reserve, upload the preview then the file straight to storage, confirm (web `uploadPrepared`).
 * Bytes never pass through the API and storage receives only the presigned headers. Returns the
 * attachment description whose ID is sent with a message.
 */
class AttachmentUploader(
    private val api: CaperApi,
    /** Waits between `/complete` attempts while storage reports the upload has not arrived (409). */
    private val completeRetryDelaysMs: List<Long> = listOf(500, 1_000, 2_000, 4_000),
) {
    suspend fun upload(token: String, channelId: String, file: PreparedAttachment, progress: (Float) -> Unit = {}): ChatAttachment {
        val size = file.file.length()
        if (size < 1) throw UploadException("This file is empty.")
        val preview = file.preview?.takeIf { file.previewContentType != null && it.length() in 1..AttachmentPolicy.PREVIEW_MAX_BYTES }
        val reservation = api.createAsset(
            token, channelId, file.name, AttachmentPolicy.normalizedType(file.contentType), size,
            sourceByteSize = file.sourceSize.takeIf { it > 0 }, width = file.width, height = file.height, durationMs = file.durationMs,
            previewContentType = preview?.let { file.previewContentType }, previewByteSize = preview?.length(),
        )
        val previewUpload = reservation.previewUpload?.takeIf { preview != null }
        val previewSize = if (preview != null && previewUpload != null) preview.length() else 0L
        val total = (size + previewSize).toFloat()
        if (preview != null && previewUpload != null) api.putUpload(previewUpload, previewSize, preview::inputStream)
        api.putUpload(reservation.upload, size, file.file::inputStream) { sent -> progress((previewSize + sent) / total) }
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
