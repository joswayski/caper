package chat.caper.android.data

import chat.caper.android.model.AttachmentUrls
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.CompressionSettings
import java.io.ByteArrayOutputStream
import java.io.OutputStream
import java.net.URI
import java.util.Locale
import java.util.zip.CRC32
import java.util.zip.Deflater
import java.util.zip.DeflaterOutputStream
import kotlin.math.max
import kotlin.math.roundToInt

/** Pure attachment rules shared by the composer, uploader and timeline (mirrors web `uploads.ts`). */
object AttachmentPolicy {
    const val MAX_ATTACHMENTS = 10
    const val PREVIEW_MAX_BYTES = 512 * 1024L
    const val PREVIEW_QUALITY = 80
    /** Decoding enormous images can exhaust memory on phones; upload those as-is. */
    const val MAX_COMPRESS_PIXELS = 50_000_000L

    private val inlineImages = setOf("image/png", "image/jpeg", "image/gif", "image/webp", "image/avif")
    private val inlineVideos = setOf("video/mp4", "video/webm", "video/quicktime")
    private val inlineAudio = setOf(
        "audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac",
    )
    private val stills = setOf("image/png", "image/jpeg", "image/webp", "image/heic", "image/heif")

    /** Mirrors `assets::kind` on the API: only these render inline. */
    fun kind(contentType: String): String = when (normalizedType(contentType)) {
        in inlineImages -> "image"
        in inlineVideos -> "video"
        in inlineAudio -> "audio"
        else -> "file"
    }

    fun normalizedType(contentType: String?): String =
        contentType?.substringBefore(';')?.trim()?.lowercase(Locale.ROOT)?.takeIf { it.contains('/') } ?: "application/octet-stream"

    /** Stills worth re-encoding. GIF (animation), SVG (vector) and AVIF stay as they are. */
    fun compressibleStill(contentType: String) = normalizedType(contentType) in stills

    /** Longest edge scaled down to [edge] (never enlarged); a non-positive edge keeps the size. */
    fun fitWithin(width: Int, height: Int, edge: Int): Pair<Int, Int> {
        if (edge <= 0 || width <= 0 || height <= 0) return width to height
        val scale = minOf(1.0, edge.toDouble() / max(width, height))
        return max(1, (width * scale).roundToInt()) to max(1, (height * scale).roundToInt())
    }

    enum class StillEncoding { PALETTE_PNG, LOSSY, KEEP }

    /**
     * Screenshots and UI captures (at most `paletteColors` distinct colours) become exact
     * indexed PNGs; photos become lossy WebP at `imageQuality`. Quality 100 disables lossy
     * re-encoding unless the image had to shrink or cannot be shown inline as it is.
     */
    fun stillEncoding(distinctColors: Int?, settings: CompressionSettings, resized: Boolean, originalInline: Boolean): StillEncoding = when {
        settings.paletteColors > 0 && distinctColors != null && distinctColors <= settings.paletteColors -> StillEncoding.PALETTE_PNG
        settings.imageQuality in 1..99 || resized || !originalInline -> StillEncoding.LOSSY
        else -> StillEncoding.KEEP
    }

    /** Keep a re-encoded file only when it is meaningfully smaller, or the original cannot render inline. */
    fun keepReencoded(originalType: String, originalSize: Long, encodedSize: Long): Boolean =
        kind(originalType) != "image" || encodedSize <= originalSize * 0.9

    /** Transcoded video: keep when smaller, or when the original would only be a download. */
    fun keepTranscoded(originalType: String, originalSize: Long, transcodedSize: Long): Boolean =
        transcodedSize in 1 until originalSize || (kind(originalType) != "video" && transcodedSize > 0)

    fun renamed(name: String, contentType: String): String {
        val extension = when (contentType) {
            "image/webp" -> "webp"; "image/jpeg" -> "jpg"; "image/png" -> "png"; "video/mp4" -> "mp4"
            else -> return name
        }
        val dot = name.lastIndexOf('.')
        return "${if (dot > 0) name.substring(0, dot) else name}.$extension"
    }

    /** Images get a preview when large in pixels or bytes; videos always get a poster frame. */
    fun needsPreview(kind: String, width: Int?, height: Int?, bytes: Long, previewEdge: Int): Boolean = when (kind) {
        "video" -> true
        "image" -> max(width ?: 0, height ?: 0) > previewEdge || bytes > PREVIEW_MAX_BYTES
        else -> false
    }

    /**
     * Output (display) height for a video transcode, or null when none should run (disabled or
     * unknown size). `videoMaxHeight` bounds the SHORT edge ("1080p"): 3840x2160 -> 1920x1080,
     * 2160x3840 -> 1080x1920, and 1080x1920 keeps its size. Never upscales; kept even for H.264.
     */
    fun videoTargetHeight(width: Int?, height: Int?, settings: CompressionSettings): Int? {
        if (settings.videoMaxHeight <= 0 || width == null || height == null || width <= 0 || height <= 0) return null
        val short = minOf(width, height)
        if (short <= settings.videoMaxHeight) return height
        val scaled = (height.toLong() * settings.videoMaxHeight / short).toInt()
        return maxOf(2, scaled - scaled % 2)
    }

    fun formatBytes(bytes: Long): String {
        if (bytes < 1024) return "$bytes B"
        val units = listOf("KB", "MB", "GB")
        var value = bytes / 1024.0
        var unit = 0
        while (value >= 1024 && unit < units.lastIndex) { value /= 1024; unit++ }
        val number = if (value >= 10) value.roundToInt().toString() else String.format(Locale.US, "%.1f", value)
        return "$number ${units[unit]}"
    }

    /** "1.6 MB → 143 KB" once compression saved space, otherwise the stored size. */
    fun sizeLabel(sourceSize: Long, storedSize: Long?): String =
        if (storedSize != null && storedSize < sourceSize) "${formatBytes(sourceSize)} → ${formatBytes(storedSize)}"
        else formatBytes(storedSize ?: sourceSize)

    fun uploadErrorMessage(error: Throwable): String = when {
        error is ApiException && error.code == "storage_full" -> "You’ve used all of your file storage."
        error is ApiException && error.status == 413 -> "This file is too large to upload."
        error is ApiException && error.status == 429 -> "Uploading too quickly. Try again shortly."
        error is ApiException && error.status == 404 -> "You can no longer upload to this conversation."
        error is UploadException -> error.message
        error is ApiException -> error.message
        else -> "This file could not be uploaded."
    }
}

class UploadException(override val message: String) : java.io.IOException(message)

/** Bounded open-addressing ARGB → index table; avoids boxing millions of pixels. */
internal class ColorTable(private val limit: Int) {
    private val capacity = Integer.highestOneBit(max(limit, 1) * 4 - 1) shl 1
    private val keys = IntArray(capacity)
    private val values = IntArray(capacity) { -1 }
    private val order = IntArray(limit + 1)
    var size = 0
        private set

    /** Adds [color]; returns false once more than [limit] distinct colours were seen. */
    fun add(color: Int): Boolean {
        var slot = mix(color) and (capacity - 1)
        while (values[slot] >= 0) {
            if (keys[slot] == color) return true
            slot = (slot + 1) and (capacity - 1)
        }
        if (size == limit) { size = limit + 1; return false }
        keys[slot] = color; values[slot] = size; order[size] = color; size++
        return true
    }

    fun indexOf(color: Int): Int {
        var slot = mix(color) and (capacity - 1)
        while (values[slot] >= 0) {
            if (keys[slot] == color) return values[slot]
            slot = (slot + 1) and (capacity - 1)
        }
        return -1
    }

    fun colors(): IntArray = order.copyOf(minOf(size, limit))

    private fun mix(value: Int): Int { val h = value * -0x61c88647; return h xor (h ushr 16) }
}

/**
 * Exact palette of an image read row by row, or null when it has more than [limit] colours
 * (photos usually exceed it within the first rows). Translucent entries sort first so the
 * PNG `tRNS` chunk stays short.
 */
internal fun paletteOf(width: Int, height: Int, limit: Int, row: (Int, IntArray) -> Unit): IntArray? {
    if (limit <= 0 || width <= 0 || height <= 0) return null
    val table = ColorTable(limit)
    val pixels = IntArray(width)
    for (y in 0 until height) {
        row(y, pixels)
        for (pixel in pixels) if (!table.add(pixel)) return null
    }
    return table.colors().sortedWith(compareBy<Int> { (it ushr 24) == 0xFF }).toIntArray()
}

/** Lossless indexed-colour PNG (palette + optional tRNS), streamed row by row. */
internal object IndexedPng {
    private val signature = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 13, 10, 26, 10)

    fun bitDepth(colors: Int) = when { colors <= 2 -> 1; colors <= 4 -> 2; colors <= 16 -> 4; else -> 8 }

    fun encode(width: Int, height: Int, palette: IntArray, out: OutputStream, row: (Int, IntArray) -> Unit) {
        require(width > 0 && height > 0 && palette.size in 1..256) { "Invalid indexed image." }
        val depth = bitDepth(palette.size)
        val table = ColorTable(palette.size).also { t -> palette.forEach { t.add(it) } }
        out.write(signature)
        chunk(out, "IHDR", ByteArrayOutputStream().apply {
            writeInt(width); writeInt(height); write(depth); write(3); write(0); write(0); write(0)
        }.toByteArray())
        chunk(out, "PLTE", ByteArray(palette.size * 3).also { bytes ->
            palette.forEachIndexed { i, c -> bytes[i * 3] = (c ushr 16).toByte(); bytes[i * 3 + 1] = (c ushr 8).toByte(); bytes[i * 3 + 2] = c.toByte() }
        })
        val translucent = palette.indexOfLast { (it ushr 24) != 0xFF }
        if (translucent >= 0) chunk(out, "tRNS", ByteArray(translucent + 1) { (palette[it] ushr 24).toByte() })
        val idat = ChunkStream(out, "IDAT")
        val deflater = Deflater(Deflater.BEST_COMPRESSION)
        DeflaterOutputStream(idat, deflater, 64 * 1024).use { stream ->
            val pixels = IntArray(width)
            val packed = ByteArray(1 + (width * depth + 7) / 8)
            for (y in 0 until height) {
                row(y, pixels)
                packed.fill(0)
                // Filter type 0: recommended for indexed images.
                for (x in 0 until width) {
                    val index = table.indexOf(pixels[x])
                    require(index >= 0) { "Pixel outside the palette." }
                    val bit = x * depth
                    val at = 1 + bit / 8
                    packed[at] = (packed[at].toInt() or (index shl (8 - depth - bit % 8))).toByte()
                }
                stream.write(packed)
            }
        }
        deflater.end()
        idat.flushChunk()
        chunk(out, "IEND", ByteArray(0))
    }

    private fun ByteArrayOutputStream.writeInt(value: Int) {
        write(value ushr 24); write(value ushr 16); write(value ushr 8); write(value)
    }

    internal fun chunk(out: OutputStream, type: String, data: ByteArray, length: Int = data.size) {
        val name = type.toByteArray(Charsets.US_ASCII)
        out.write(byteArrayOf((length ushr 24).toByte(), (length ushr 16).toByte(), (length ushr 8).toByte(), length.toByte()))
        out.write(name); out.write(data, 0, length)
        val crc = CRC32().apply { update(name); update(data, 0, length) }.value.toInt()
        out.write(byteArrayOf((crc ushr 24).toByte(), (crc ushr 16).toByte(), (crc ushr 8).toByte(), crc.toByte()))
    }

    /** Splits compressed image data into IDAT chunks without holding the whole stream. */
    private class ChunkStream(private val out: OutputStream, private val type: String) : OutputStream() {
        private val buffer = ByteArray(64 * 1024)
        private var used = 0
        override fun write(b: Int) { if (used == buffer.size) flushChunk(); buffer[used++] = b.toByte() }
        override fun write(b: ByteArray, off: Int, len: Int) {
            var offset = off; var remaining = len
            while (remaining > 0) {
                if (used == buffer.size) flushChunk()
                val count = minOf(remaining, buffer.size - used)
                System.arraycopy(b, offset, buffer, used, count)
                used += count; offset += count; remaining -= count
            }
        }
        // DeflaterOutputStream.close() closes this stream; the PNG continues after it.
        override fun close() = Unit
        fun flushChunk() { if (used > 0) { chunk(out, type, buffer, used); used = 0 } }
    }
}

/** Detects animation so animated WebP/PNG upload unchanged instead of losing frames. */
internal fun isAnimatedImage(contentType: String, header: ByteArray): Boolean {
    fun ascii(at: Int, text: String) = header.size >= at + text.length && text.indices.all { header[at + it] == text[it].code.toByte() }
    return when (AttachmentPolicy.normalizedType(contentType)) {
        "image/webp" -> ascii(0, "RIFF") && ascii(8, "WEBP") && ascii(12, "VP8X") && header.size > 20 && header[20].toInt() and 0x02 != 0
        "image/png" -> {
            // acTL must precede IDAT in an APNG; scan chunk headers.
            var at = 8
            while (at + 8 <= header.size) {
                val length = ((header[at].toInt() and 0xFF) shl 24) or ((header[at + 1].toInt() and 0xFF) shl 16) or
                    ((header[at + 2].toInt() and 0xFF) shl 8) or (header[at + 3].toInt() and 0xFF)
                if (ascii(at + 4, "acTL")) return true
                if (ascii(at + 4, "IDAT") || length < 0) return false
                at += 12 + length
            }
            false
        }
        else -> false
    }
}

/** Seconds since epoch at which a signed delivery URL stops working, if it carries `exp`. */
fun attachmentUrlExpiry(url: String?): Long? {
    if (url == null) return null
    val query = runCatching { URI(url).rawQuery }.getOrNull() ?: return null
    return query.split('&').firstNotNullOfOrNull { part ->
        part.takeIf { it.startsWith("exp=") }?.substring(4)?.toLongOrNull()?.takeIf { it > 0 }
    }
}

/** The freshest URLs for [attachment]: a re-signed pair wins only when it expires later. */
fun ChatAttachment.withFreshUrls(fresh: AttachmentUrls?): ChatAttachment {
    if (fresh == null || unavailable) return this
    val current = attachmentUrlExpiry(url)
    val candidate = attachmentUrlExpiry(fresh.url)
    if (url != null && current != null && candidate != null && candidate <= current) return this
    return copy(url = fresh.url, previewUrl = fresh.previewUrl)
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
            val expires = attachmentUrlExpiry(attachment.url) ?: return@mapNotNull null
            if (attachment.unavailable || expires - now > marginSeconds || requestedExpiry[attachment.id] == expires) return@mapNotNull null
            requestedExpiry[attachment.id] = expires
            attachment.id
        }.distinct()
    }

    fun afterLoadFailure(attachment: ChatAttachment, status: Int?): Boolean {
        if (attachment.unavailable || attachment.url == null) return false
        val expired = attachmentUrlExpiry(attachment.url)?.let { it <= nowSeconds() } == true
        if (status !in setOf(403, 404) && !expired) return false
        return failureRetried.add(attachment.id)
    }

    fun reset() { requestedExpiry.clear(); failureRetried.clear() }
}
