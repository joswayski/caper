package chat.caper.android

import android.graphics.Bitmap
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.decode.ImageSource
import coil3.fetch.SourceFetchResult
import coil3.request.Options
import coil3.size.Dimension
import coil3.size.Scale
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlinx.coroutines.runInterruptible
import okio.BufferedSource
import org.aomedia.avif.android.AvifDecoder

/**
 * AVIF for Android 8–11 (API 26–30). Android decodes AVIF itself from API 31, so this decoder
 * is registered only below that. It uses AOMedia's libavif JNI build (dav1d inside, about
 * 0.9 MB per ABI) and decodes straight into a bitmap of the requested size; libavif scales.
 */
internal class AvifCompatDecoder(private val source: ImageSource, private val options: Options) : Decoder {
    override suspend fun decode(): DecodeResult = runInterruptible {
        val bytes = source.use { it.source().readByteArray() }
        val buffer = ByteBuffer.allocateDirect(bytes.size).put(bytes)
        buffer.rewind()
        val info = AvifDecoder.Info()
        check(AvifDecoder.getInfo(buffer, bytes.size, info) && info.width > 0 && info.height > 0) { "Unreadable AVIF image." }
        val (width, height) = avifTargetSize(info.width, info.height, options.size.width.pixels(), options.size.height.pixels(), options.scale)
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        if (!AvifDecoder.decode(buffer, bytes.size, bitmap)) {
            bitmap.recycle()
            error("Unreadable AVIF image.")
        }
        DecodeResult(bitmap.asImage(), isSampled = width < info.width || height < info.height)
    }

    class Factory : Decoder.Factory {
        override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder? =
            if (result.mimeType == "image/avif" || isAvif(result.source.source())) AvifCompatDecoder(result.source, options) else null
    }
}

private fun Dimension.pixels(): Int? = (this as? Dimension.Pixels)?.px?.takeIf { it > 0 }

/** `ftyp` box with an `avif` (still) or `avis` (sequence) major brand. */
internal fun isAvif(source: BufferedSource): Boolean = runCatching {
    val header = source.peek().readByteArray(12)
    String(header, 4, 4, Charsets.US_ASCII) == "ftyp" && String(header, 8, 4, Charsets.US_ASCII) in setOf("avif", "avis")
}.getOrDefault(false)

/** Longest decoded edge, so a full-size photo cannot exhaust memory on an older phone. */
internal const val AVIF_MAX_EDGE = 4096

/** Fits the image to the requested size (never enlarging) and to [AVIF_MAX_EDGE]. */
internal fun avifTargetSize(width: Int, height: Int, targetWidth: Int?, targetHeight: Int?, scale: Scale): Pair<Int, Int> {
    val widthRatio = targetWidth?.let { it.toDouble() / width }
    val heightRatio = targetHeight?.let { it.toDouble() / height }
    val requested = when {
        widthRatio != null && heightRatio != null -> if (scale == Scale.FILL) max(widthRatio, heightRatio) else min(widthRatio, heightRatio)
        else -> widthRatio ?: heightRatio ?: 1.0
    }
    val ratio = minOf(1.0, requested, AVIF_MAX_EDGE.toDouble() / max(width, height))
    return max(1, (width * ratio).roundToInt()) to max(1, (height * ratio).roundToInt())
}
