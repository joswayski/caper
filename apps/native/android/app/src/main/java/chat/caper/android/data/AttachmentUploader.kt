package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.MultipartPlan
import chat.caper.android.model.PresignedUpload
import java.io.File
import java.io.FilterInputStream
import java.io.InputStream
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLongArray
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

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
    /** A recorded voice message, uploaded as recorded (never compressed) with its [waveform]. */
    val voice: Boolean = false,
    val waveform: String? = null,
)

/**
 * Reserve, upload the preview then the file straight to storage (in parts when the server asks),
 * confirm (web `uploadPrepared`).
 * Every body is metadata-cleaned before reservation; storage receives only presigned headers. Returns
 * the attachment description whose ID is sent with a message.
 */
class AttachmentUploader(
    private val api: CaperApi,
    /** Allow ten queued files to finish metadata processing (409 while queued). */
    private val completeRetryDelaysMs: List<Long> = List(240) { 5_000L },
    /** A failed part is sent again with a fresh URL after each of these pauses. */
    private val partRetryDelaysMs: List<Long> = listOf(500L, 2_000L, 5_000L, 15_000L),
) {
    suspend fun upload(token: String, channelId: String, file: PreparedAttachment, progress: (Float) -> Unit = {}): ChatAttachment {
        suspend fun withPreview(prepared: PreparedAttachment): ChatAttachment {
            val preview = prepared.preview
            val type = prepared.previewContentType
            return if (preview != null && type != null) MetadataStrip.withCleanFile(preview, type) {
                uploadCleaned(token, channelId, prepared.copy(preview = it), progress)
            } else uploadCleaned(token, channelId, prepared, progress)
        }
        // Only composer recordings retain server-side re-encoding. Imported audio is cleaned.
        return if (file.voice) withPreview(file) else MetadataStrip.withCleanFile(file.file, file.contentType) {
            withPreview(file.copy(file = it))
        }
    }

    private suspend fun uploadCleaned(token: String, channelId: String, file: PreparedAttachment, progress: (Float) -> Unit): ChatAttachment {
        val size = file.file.length()
        if (size < 1) throw UploadException("This file is empty.")
        val preview = file.preview?.takeIf { file.previewContentType != null && it.length() in 1..AttachmentPolicy.PREVIEW_MAX_BYTES }
        val reservation = api.createAsset(
            token, channelId, file.name, AttachmentPolicy.normalizedType(file.contentType), size,
            sourceByteSize = file.sourceSize.takeIf { it > 0 }, width = file.width, height = file.height, durationMs = file.durationMs,
            previewContentType = preview?.let { file.previewContentType }, previewByteSize = preview?.length(),
            voice = file.voice, waveform = file.waveform.takeIf { file.voice },
        )
        try {
            currentCoroutineContext().ensureActive()
            val previewUpload = reservation.previewUpload?.takeIf { preview != null }
            val previewSize = if (preview != null && previewUpload != null) preview.length() else 0L
            val total = (size + previewSize).toFloat()
            if (preview != null && previewUpload != null) api.putUpload(previewUpload, previewSize, preview::inputStream)
            val report = { sent: Long -> progress((previewSize + sent) / total) }
            val plan = reservation.multipart
            if (plan != null) uploadParts(token, reservation.id, file.file, plan, report)
            else api.putUpload(checkNotNull(reservation.upload), size, file.file::inputStream, report)
            val attachment = complete(token, reservation.id)
            currentCoroutineContext().ensureActive()
            if (attachment.id != reservation.id) throw UploadException("The upload service returned an invalid response.")
            progress(1f)
            return attachment
        } catch (error: Throwable) {
            api.cancelAsset(token, reservation.id)
            throw error
        }
    }

    /**
     * Sends a large file in parts, each straight to storage (web `uploadParts`): [PART_CONCURRENCY]
     * at a time, URLs asked for as they are needed, and a failed part sent again with a fresh URL
     * after each of [partRetryDelaysMs]. The first part that can't be sent stops the rest.
     */
    private suspend fun uploadParts(token: String, id: String, file: File, plan: MultipartPlan, progress: (Long) -> Unit) {
        val size = file.length()
        val sent = AtomicLongArray(plan.partCount)
        val report = { progress((0 until plan.partCount).sumOf(sent::get)) }
        val urls = HashMap<Int, PresignedUpload>()
        val asking = Mutex()
        val handedOut = HashSet<Int>()
        // Ask ahead only for parts whose URL no worker has taken, so a retry doesn't re-sign parts in flight.
        suspend fun urlFor(number: Int): PresignedUpload = asking.withLock {
            val url = urls.remove(number) ?: run {
                val ahead = (number + 1..plan.partCount).asSequence().filter { it !in urls && it !in handedOut }
                urls += api.assetParts(token, id, (sequenceOf(number) + ahead).take(PART_BATCH).toList())
                urls.remove(number) ?: throw UploadException("The upload service returned an invalid response.")
            }
            handedOut += number
            url
        }
        val next = AtomicInteger(1)
        coroutineScope {
            repeat(minOf(PART_CONCURRENCY, plan.partCount)) {
                launch {
                    while (true) {
                        val number = next.getAndIncrement()
                        if (number > plan.partCount) break
                        val offset = (number - 1) * plan.partBytes
                        val length = minOf(plan.partBytes, size - offset)
                        for (attempt in 0..partRetryDelaysMs.size) {
                            val url = urlFor(number)
                            try {
                                api.putUpload(url, length, { file.part(offset, length) }) { bytes -> sent[number - 1] = bytes; report() }
                                break
                            } catch (error: UploadException) {
                                if (attempt == partRetryDelaysMs.size) throw error
                                sent[number - 1] = 0
                                report()
                                delay(partRetryDelaysMs[attempt])
                            }
                        }
                    }
                }
            }
        }
    }

    private suspend fun complete(token: String, id: String): ChatAttachment {
        for (wait in completeRetryDelaysMs) {
            try {
                return api.completeAsset(token, id)
            } catch (error: ApiException) {
                // 409: storage is not visible yet, or metadata processing is queued. Missing parts won't arrive by waiting.
                if (error.status != 409 || error.code == "upload_incomplete") throw error
            }
            delay(wait)
        }
        return api.completeAsset(token, id)
    }

    private companion object {
        /** Parts in flight at once, and part URLs asked for at a time. */
        const val PART_CONCURRENCY = 4
        const val PART_BATCH = 4
    }
}

/** Exactly [length] bytes of this file from [offset], so a part's upload checks its own size. */
private fun File.part(offset: Long, length: Long): InputStream {
    val input = inputStream().apply { channel.position(offset) }
    return object : FilterInputStream(input) {
        private var left = length
        override fun read(): Int = if (left <= 0) -1 else super.read().also { if (it >= 0) left-- }
        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (left <= 0) return -1
            return super.read(b, off, minOf(len.toLong(), left).toInt()).also { if (it > 0) left -= it }
        }
    }
}
