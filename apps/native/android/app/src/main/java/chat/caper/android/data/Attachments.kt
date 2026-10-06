package chat.caper.android.data

import chat.caper.android.model.AttachmentState
import chat.caper.android.model.AttachmentUrls
import chat.caper.android.model.ChatAttachment
import java.net.URI
import java.util.Locale
import kotlin.math.roundToInt

/**
 * Pure attachment rules shared by the composer, uploader and timeline (mirrors web `uploads.ts`).
 * Clients upload originals unchanged; the server's media worker does all compression.
 */
object AttachmentPolicy {
    const val MAX_ATTACHMENTS = 10

    private val inlineImages = setOf("image/png", "image/jpeg", "image/gif", "image/webp", "image/avif")
    private val inlineVideos = setOf("video/mp4", "video/webm", "video/quicktime")
    private val inlineAudio = setOf(
        "audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac",
    )

    /** Mirrors `assets::kind` on the API; used for the composer chip before the server answers. */
    fun kind(contentType: String): String = when (normalizedType(contentType)) {
        in inlineImages -> "image"
        in inlineVideos -> "video"
        in inlineAudio -> "audio"
        else -> "file"
    }

    /** The declared MIME type: lower-case without parameters, `application/octet-stream` when unknown. */
    fun normalizedType(contentType: String?): String =
        contentType?.substringBefore(';')?.trim()?.lowercase(Locale.ROOT)?.takeIf { it.contains('/') } ?: "application/octet-stream"

    fun formatBytes(bytes: Long): String {
        if (bytes < 1024) return "$bytes B"
        val units = listOf("KB", "MB", "GB")
        var value = bytes / 1024.0
        var unit = 0
        while (value >= 1024 && unit < units.lastIndex) { value /= 1024; unit++ }
        val number = if (value >= 10) value.roundToInt().toString() else String.format(Locale.US, "%.1f", value)
        return "$number ${units[unit]}"
    }

    fun uploadErrorMessage(error: Throwable): String = when {
        error is ApiException && error.code == "storage_full" -> "You’ve used all of your file storage."
        error is ApiException && error.status == 413 -> "This file is too large to upload."
        error is ApiException && error.status == 429 -> "Uploading too quickly. Try again shortly."
        error is ApiException && error.status == 404 -> "You can no longer upload to this conversation."
        error is ApiException && error.status == 422 -> "The file changed while uploading. Try again."
        error is UploadException -> error.message
        error is ApiException -> error.message
        else -> "This file could not be uploaded."
    }
}

class UploadException(override val message: String) : java.io.IOException(message)

/** Seconds since epoch at which a signed delivery URL stops working, if it carries `exp`. */
fun attachmentUrlExpiry(url: String?): Long? {
    if (url == null) return null
    val query = runCatching { URI(url).rawQuery }.getOrNull() ?: return null
    return query.split('&').firstNotNullOfOrNull { part ->
        part.takeIf { it.startsWith("exp=") }?.substring(4)?.toLongOrNull()?.takeIf { it > 0 }
    }
}

/** The signed URL whose expiry matters: the file once ready, otherwise its preview. */
private val ChatAttachment.signedUrl: String? get() = url ?: previewUrl

/**
 * The freshest URLs for [attachment]: a re-signed pair wins only when it expires later. Only a
 * ready file takes a `url`, so a stale entry cannot make a processing or failed file look ready.
 */
fun ChatAttachment.withFreshUrls(fresh: AttachmentUrls?): ChatAttachment {
    if (fresh == null || unavailable) return this
    val current = attachmentUrlExpiry(signedUrl)
    val candidate = attachmentUrlExpiry(fresh.url ?: fresh.previewUrl)
    if (signedUrl != null && current != null && candidate != null && candidate <= current) return this
    val nextUrl = if (state == AttachmentState.READY) fresh.url ?: url else url
    val nextPreview = fresh.previewUrl ?: previewUrl
    return if (nextUrl == url && nextPreview == previewUrl) this else copy(url = nextUrl, previewUrl = nextPreview)
}

/**
 * When to ask `POST /api/assets/urls` for new signatures: shortly before `exp`, and once
 * after a 403/404 load failure. Each expiry is requested once; each attachment retries a
 * failed load once, so a deleted file cannot cause a refresh loop.
 */
class AttachmentUrlRefresh(
    private val nowSeconds: () -> Long = { System.currentTimeMillis() / 1000 },
    private val marginSeconds: Long = 60 * 60,
) {
    private val requestedExpiry = mutableMapOf<String, Long>()
    private val failureRetried = mutableSetOf<String>()

    fun expiring(attachments: Iterable<ChatAttachment>): List<String> {
        val now = nowSeconds()
        return attachments.mapNotNull { attachment ->
            val expires = attachmentUrlExpiry(attachment.signedUrl) ?: return@mapNotNull null
            if (attachment.unavailable || expires - now > marginSeconds || requestedExpiry[attachment.id] == expires) return@mapNotNull null
            requestedExpiry[attachment.id] = expires
            attachment.id
        }.distinct()
    }

    fun afterLoadFailure(attachment: ChatAttachment, status: Int?): Boolean {
        if (attachment.unavailable || attachment.signedUrl == null) return false
        val expired = attachmentUrlExpiry(attachment.signedUrl)?.let { it <= nowSeconds() } == true
        if (status !in setOf(403, 404) && !expired) return false
        return failureRetried.add(attachment.id)
    }

    fun reset() { requestedExpiry.clear(); failureRetried.clear() }
}
