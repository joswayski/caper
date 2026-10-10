package chat.caper.android.data

import chat.caper.android.model.AttachmentState
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

/**
 * Pure attachment rules shared by the composer, uploader and timeline (mirrors web `uploads.ts`).
 * The device compresses before upload with the server's `compression` settings, following
 * docs/media.md "Client compression and previews"; the server only verifies stored bytes.
 */
object AttachmentPolicy {
    const val MAX_ATTACHMENTS = 10
    const val PREVIEW_MAX_BYTES = 512 * 1024L
    const val PREVIEW_QUALITY = 80
    /** Decoding enormous images can exhaust memory on phones; upload those as-is. */
    const val MAX_COMPRESS_PIXELS = 50_000_000L
    /** A lossy re-encode or size-only transcode must save at least this fraction to replace the original. */
    const val MIN_SAVING = 0.10
    /** Hardware encoders block below roughly this bitrate, whatever the resolution. */
    const val VIDEO_BITRATE_FLOOR_KBPS = 1500
    /** Videos up to this multiple of the target bitrate are already efficient and upload unchanged. */
    const val VIDEO_BITRATE_HEADROOM = 1.25
    private const val PIXELS_1080P = 1920L * 1080L

    /** Lossless sources (screenshots, UI, drawings): never encoded lossily. */
    private val losslessStills = setOf("image/png", "image/bmp", "image/x-ms-bmp", "image/tiff")
    /** Photos: re-encoded lossily when that saves enough. */
    private val photoStills = setOf("image/jpeg", "image/heic", "image/heif")

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

    // --- Stills --------------------------------------------------------------------------------

    /** How a picked still is treated. GIF, SVG, AVIF and animated images are [KEEP]: unchanged. */
    enum class StillClass { LOSSLESS, PHOTO, KEEP }

    /**
     * PNG, BMP, TIFF and lossless WebP are lossless sources; JPEG, HEIC/HEIF and lossy WebP are
     * photos. A WebP whose coding cannot be read from [header] counts as lossless, so it is never
     * blurred by a lossy re-encode.
     */
    fun stillClass(contentType: String, header: ByteArray): StillClass {
        val type = normalizedType(contentType)
        if (isAnimatedImage(type, header)) return StillClass.KEEP
        return when {
            type in losslessStills -> StillClass.LOSSLESS
            type in photoStills -> StillClass.PHOTO
            type == "image/webp" -> if (webpIsLossless(header) == false) StillClass.PHOTO else StillClass.LOSSLESS
            else -> StillClass.KEEP
        }
    }

    /** Longest edge scaled down to [edge] (never enlarged); a non-positive edge keeps the size. */
    fun fitWithin(width: Int, height: Int, edge: Int): Pair<Int, Int> {
        if (edge <= 0 || width <= 0 || height <= 0) return width to height
        val scale = minOf(1.0, edge.toDouble() / max(width, height))
        return max(1, (width * scale).roundToInt()) to max(1, (height * scale).roundToInt())
    }

    enum class LosslessEncoding { PALETTE_PNG, WEBP_LOSSLESS, PNG, KEEP }

    /**
     * Lossless stays lossless: an exact indexed PNG when the colours fit `paletteColors`, else
     * lossless WebP where the platform encodes it (API 30+), else the original. A source that
     * cannot be shown inline as it is (BMP, TIFF) becomes a plain PNG instead of a download.
     */
    fun losslessEncoding(distinctColors: Int?, settings: CompressionSettings, webpLosslessAvailable: Boolean, originalInline: Boolean): LosslessEncoding = when {
        settings.paletteColors > 0 && distinctColors != null && distinctColors <= settings.paletteColors -> LosslessEncoding.PALETTE_PNG
        webpLosslessAvailable -> LosslessEncoding.WEBP_LOSSLESS
        !originalInline -> LosslessEncoding.PNG
        else -> LosslessEncoding.KEEP
    }

    /** A lossless re-encode costs no quality: keep it whenever it is smaller, or the original cannot render inline. */
    fun keepLossless(originalType: String, originalSize: Long, encodedSize: Long): Boolean =
        encodedSize > 0 && (encodedSize < originalSize || kind(originalType) != "image")

    /**
     * Photos are re-encoded lossily at `imageQuality` (100 disables that) unless they had to
     * shrink to `imageMaxEdge` or cannot be shown inline as they are (HEIC is always converted).
     */
    fun reencodePhoto(settings: CompressionSettings, resized: Boolean, originalInline: Boolean): Boolean =
        settings.imageQuality in 1..99 || resized || !originalInline

    enum class PhotoFormat { AVIF, WEBP }

    /**
     * Photos become AVIF at `avifQuality` only when the server asks for exactly `avif` (older
     * servers omit `imageFormat`) and this device has the encoder; otherwise, and whenever an AVIF
     * encode fails, lossy WebP (JPEG below that) at `imageQuality`.
     */
    fun photoFormat(settings: CompressionSettings, avifAvailable: Boolean): PhotoFormat =
        if (settings.imageFormat == "avif" && avifAvailable) PhotoFormat.AVIF else PhotoFormat.WEBP

    /** libavif quality 1–100; out-of-range values clamp. */
    fun avifQuality(settings: CompressionSettings): Int = settings.avifQuality.coerceIn(1, 100)

    /** A lossy photo replaces the original only when at least 10% smaller, or the original cannot render inline. */
    fun keepPhoto(originalType: String, originalSize: Long, encodedSize: Long): Boolean =
        encodedSize > 0 && (kind(originalType) != "image" || encodedSize <= originalSize * (1 - MIN_SAVING))

    fun renamed(name: String, contentType: String): String {
        val extension = when (contentType) {
            "image/webp" -> "webp"; "image/avif" -> "avif"; "image/jpeg" -> "jpg"; "image/png" -> "png"; "video/mp4" -> "mp4"
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

    // --- Video ---------------------------------------------------------------------------------

    /** What the device measured about a picked video. [mimeType] is the video track's codec type. */
    data class VideoFacts(
        val width: Int?,
        val height: Int?,
        val mimeType: String?,
        /** Overall bits per second (container bitrate, or size over duration). */
        val bitrate: Long?,
        /** HLG or PQ (HDR10) transfer, or Dolby Vision. */
        val hdr: Boolean = false,
        val contentType: String = "video/mp4",
    )

    /**
     * A needed transcode to H.264/AAC MP4. [outputHeight] is the display height when the video
     * must shrink (null keeps its size); [sizeOnly] marks a transcode run only for bitrate, which
     * must then save 10%; [toneMap] asks for HDR to SDR tone mapping.
     */
    data class VideoPlan(val outputHeight: Int?, val bitrateKbps: Int, val sizeOnly: Boolean, val toneMap: Boolean)

    /**
     * Output (display) height bounded on the SHORT edge by `videoMaxHeight` ("1080p"):
     * 3840x2160 -> 1080, 2160x3840 -> 1920, 1080x1920 keeps 1920. Never upscales; null when
     * disabled or the size is unknown.
     */
    fun videoTargetHeight(width: Int?, height: Int?, settings: CompressionSettings): Int? {
        if (settings.videoMaxHeight <= 0 || width == null || height == null || width <= 0 || height <= 0) return null
        val short = minOf(width, height)
        if (short <= settings.videoMaxHeight) return height
        val scaled = (height.toLong() * settings.videoMaxHeight / short).toInt()
        return maxOf(2, scaled - scaled % 2)
    }

    /** `videoBitrateKbps` at 1080p, scaled by pixel count, never below the hardware-encoder floor. */
    fun videoTargetKbps(width: Int, height: Int, settings: CompressionSettings): Int {
        val scaled = (settings.videoBitrateKbps.toDouble() * width.toLong() * height / PIXELS_1080P).roundToInt()
        return max(scaled, minOf(VIDEO_BITRATE_FLOOR_KBPS, settings.videoBitrateKbps))
    }

    /**
     * Transcode only when needed: the short edge is above `videoMaxHeight`, the codec is not
     * H.264, the container cannot play inline, or the bitrate is above 1.25x the target. HDR
     * that this device cannot tone map uploads unchanged (null) rather than washed out.
     */
    fun videoPlan(facts: VideoFacts, settings: CompressionSettings, toneMapSupported: Boolean): VideoPlan? {
        val width = facts.width ?: return null
        val height = facts.height ?: return null
        val outputHeight = videoTargetHeight(width, height, settings) ?: return null
        val shrink = outputHeight < height
        val outputWidth = if (shrink) (width.toLong() * outputHeight / height).toInt() else width
        val target = videoTargetKbps(outputWidth, outputHeight, settings)
        val incompatible = (facts.mimeType != null && facts.mimeType != "video/avc") || kind(facts.contentType) != "video"
        val heavy = facts.bitrate != null && facts.bitrate > target * 1000L * VIDEO_BITRATE_HEADROOM
        if (!shrink && !incompatible && !heavy) return null
        if (facts.hdr && !toneMapSupported) return null
        return VideoPlan(outputHeight.takeIf { shrink }, target, sizeOnly = !shrink && !incompatible, toneMap = facts.hdr)
    }

    /**
     * Keep a finished transcode unless it lost the audio track, is still HDR (tone mapping did
     * not happen), or was run only for size and saved less than 10%.
     */
    fun keepTranscoded(plan: VideoPlan, originalSize: Long, transcodedSize: Long, audioBefore: Boolean, audioAfter: Boolean, outputHdr: Boolean): Boolean = when {
        transcodedSize <= 0 -> false
        audioBefore && !audioAfter -> false
        outputHdr -> false
        plan.sizeOnly -> transcodedSize <= originalSize * (1 - MIN_SAVING)
        else -> true
    }

    /** "1.6 MB → 143 KB" once compression saved space, otherwise the stored size. */
    fun sizeLabel(sourceSize: Long, storedSize: Long?): String =
        if (storedSize != null && storedSize < sourceSize) "${formatBytes(sourceSize)} → ${formatBytes(storedSize)}"
        else formatBytes(storedSize ?: sourceSize)

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

private fun ByteArray.ascii(at: Int, text: String) = size >= at + text.length && text.indices.all { this[at + it] == text[it].code.toByte() }

/** Detects animation so animated WebP/PNG upload unchanged instead of losing frames. */
internal fun isAnimatedImage(contentType: String, header: ByteArray): Boolean = when (AttachmentPolicy.normalizedType(contentType)) {
    "image/webp" -> header.ascii(0, "RIFF") && header.ascii(8, "WEBP") && header.ascii(12, "VP8X") && header.size > 20 && header[20].toInt() and 0x02 != 0
    "image/png" -> {
        // acTL must precede IDAT in an APNG; scan chunk headers.
        var at = 8
        var animated = false
        while (at + 8 <= header.size) {
            val length = ((header[at].toInt() and 0xFF) shl 24) or ((header[at + 1].toInt() and 0xFF) shl 16) or
                ((header[at + 2].toInt() and 0xFF) shl 8) or (header[at + 3].toInt() and 0xFF)
            if (header.ascii(at + 4, "acTL")) { animated = true; break }
            if (header.ascii(at + 4, "IDAT") || length < 0) break
            at += 12 + length
        }
        animated
    }
    else -> false
}

/**
 * Whether a still WebP is lossless (`VP8L`) or lossy (`VP8 `), read from its RIFF chunks;
 * null when [header] does not reach the image chunk.
 */
internal fun webpIsLossless(header: ByteArray): Boolean? {
    if (!header.ascii(0, "RIFF") || !header.ascii(8, "WEBP")) return null
    var at = 12
    while (at + 8 <= header.size) {
        if (header.ascii(at, "VP8L")) return true
        if (header.ascii(at, "VP8 ")) return false
        val length = (header[at + 4].toLong() and 0xFF) or ((header[at + 5].toLong() and 0xFF) shl 8) or
            ((header[at + 6].toLong() and 0xFF) shl 16) or ((header[at + 7].toLong() and 0xFF) shl 24)
        val next = at + 8 + length + (length and 1)
        if (next > Int.MAX_VALUE) return null
        at = next.toInt()
    }
    return null
}

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
