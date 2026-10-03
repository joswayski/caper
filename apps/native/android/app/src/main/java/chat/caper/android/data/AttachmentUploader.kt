package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import java.io.File

/** A file ready to upload: compressed (or original) bytes, measured metadata and an optional preview. */
data class PreparedAttachment(
    val file: File,
    val name: String,
    val contentType: String,
    val kind: String,
    val sourceSize: Long,
    val width: Int? = null,
    val height: Int? = null,
    val durationMs: Long? = null,
    val preview: File? = null,
    val previewContentType: String? = null,
)

/**
 * Reserve, upload straight to storage, then confirm (web `uploadPrepared`). Bytes never pass
 * through the API. Returns the attachment description to send with a message.
 */
class AttachmentUploader(private val api: CaperApi) {
    suspend fun upload(token: String, channelId: String, file: PreparedAttachment, progress: (Float) -> Unit = {}): ChatAttachment {
        val size = file.file.length()
        if (size < 1) throw UploadException("This file is empty.")
        val preview = file.preview?.takeIf { file.previewContentType != null && it.length() in 1..AttachmentPolicy.PREVIEW_MAX_BYTES }
        val reservation = api.createAsset(
            token, channelId, file.name, file.contentType, size,
            sourceByteSize = file.sourceSize.takeIf { it > 0 }, width = file.width, height = file.height, durationMs = file.durationMs,
            previewContentType = preview?.let { file.previewContentType }, previewByteSize = preview?.length(),
        )
        val previewSize = if (preview != null && reservation.previewUpload != null) preview.length() else 0L
        val total = (size + previewSize).toFloat()
        if (preview != null && reservation.previewUpload != null) api.putUpload(reservation.previewUpload, preview)
        api.putUpload(reservation.upload, file.file) { sent -> progress((previewSize + sent) / total) }
        val attachment = api.completeAsset(token, reservation.id)
        if (attachment.id != reservation.id) throw UploadException("The upload service returned an invalid response.")
        progress(1f)
        return attachment
    }
}
