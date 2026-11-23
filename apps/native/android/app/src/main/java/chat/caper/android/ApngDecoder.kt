package chat.caper.android

import android.content.res.Resources
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Rect
import android.graphics.drawable.AnimationDrawable
import android.graphics.drawable.BitmapDrawable
import android.os.Build
import chat.caper.android.data.isAnimatedImage
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.gif.AnimatedImageDecoder
import coil3.gif.GifDecoder
import coil3.request.Options
import coil3.size.pxOrElse
import java.io.ByteArrayOutputStream
import java.util.zip.CRC32
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/**
 * Decodes animations for [ImageRequest]s that animate: APNG here (neither Android nor Coil plays
 * it), then GIF and, on Android 9+, animated WebP through Coil. Anything else falls through to
 * the loader's still decoders.
 */
internal fun animatedDecoderFactory(): Decoder.Factory {
    val apng = ApngDecoder.Factory()
    val platform = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) AnimatedImageDecoder.Factory() else GifDecoder.Factory()
    return Decoder.Factory { result, options, loader -> apng.create(result, options, loader) ?: platform.create(result, options, loader) }
}

/**
 * Every composited APNG frame as an [AnimationDrawable], looping, with each frame's delay.
 * Each frame becomes a standalone PNG (the IHDR at the frame's size, the shared palette and
 * colour chunks, and its `IDAT`/`fdAT` data) that [BitmapFactory] decodes, then is blended onto
 * the canvas and disposed as the APNG spec says. Frames are scaled to cover the requested size
 * within [MAX_FRAMES] and [BYTE_BUDGET], as on Apple; beyond them it shows the first frame.
 */
internal class ApngDecoder(private val result: SourceFetchResult, private val options: Options) : Decoder {
    override suspend fun decode(): DecodeResult {
        val bytes = result.source.source().use { it.readByteArray() }
        val resources = options.context.resources
        val apng = Apng.parse(bytes)
        val frames = apng?.let { decodeFrames(it, resources) }
        if (frames != null) {
            val drawable = AnimationDrawable().apply {
                isOneShot = false
                frames.forEach { (bitmap, delay) -> addFrame(BitmapDrawable(resources, bitmap), delay) }
            }
            return DecodeResult(drawable.asImage(), isSampled = true)
        }
        // Not a playable APNG after all: its default image, as any PNG decoder shows it.
        val still = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: error("Unreadable PNG")
        return DecodeResult(still.asImage(), isSampled = false)
    }

    private suspend fun decodeFrames(apng: Apng, resources: Resources): List<Pair<Bitmap, Int>>? {
        if (apng.frames.size < 2 || apng.frames.size > MAX_FRAMES) return null
        // An undefined size (the viewer's original) keeps the full canvas.
        val requested = min(options.size.width.pxOrElse { Int.MAX_VALUE }, options.size.height.pxOrElse { Int.MAX_VALUE })
        // Cover the requested square, never enlarging; then shrink until every frame fits the budget.
        var scale = min(1.0, requested.toDouble() / min(apng.width, apng.height))
        val fit = sqrt(BYTE_BUDGET.toDouble() / (apng.frames.size * 4.0 * apng.width * apng.height))
        scale = min(scale, fit)
        val outWidth = max(1, (apng.width * scale).toInt())
        val outHeight = max(1, (apng.height * scale).toInt())
        if (min(outWidth, outHeight) < min(MIN_EDGE, min(apng.width, apng.height))) return null
        val canvasBitmap = Bitmap.createBitmap(apng.width, apng.height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(canvasBitmap)
        val source = Paint(Paint.FILTER_BITMAP_FLAG).apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.SRC) }
        val over = Paint(Paint.FILTER_BITMAP_FLAG)
        val clear = Paint().apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.CLEAR) }
        val output = ArrayList<Pair<Bitmap, Int>>(apng.frames.size)
        try {
            apng.frames.forEachIndexed { index, frame ->
                currentCoroutineContext().ensureActive()
                val image = BitmapFactory.decodeByteArray(frame.png, 0, frame.png.size) ?: return null
                val region = Rect(frame.x, frame.y, frame.x + frame.width, frame.y + frame.height)
                val previous = if (frame.dispose == DISPOSE_PREVIOUS && index > 0) canvasBitmap.copy(Bitmap.Config.ARGB_8888, false) else null
                canvas.drawBitmap(image, null, region, if (frame.blend == BLEND_SOURCE) source else over)
                image.recycle()
                output += Bitmap.createScaledBitmap(canvasBitmap, outWidth, outHeight, true) to frame.delayMillis
                when {
                    previous != null -> { canvas.drawBitmap(previous, 0f, 0f, source); previous.recycle() }
                    // A first frame's "previous" disposal clears, as the spec says.
                    frame.dispose != DISPOSE_NONE -> canvas.drawRect(region, clear)
                }
            }
        } catch (failure: Throwable) {
            output.forEach { it.first.recycle() }
            throw failure
        } finally {
            canvasBitmap.recycle()
        }
        return output
    }

    class Factory : Decoder.Factory {
        override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder? {
            val peek = result.source.source().peek()
            val header = ByteArray(HEADER_BYTES)
            var read = 0
            while (read < header.size) {
                val count = peek.read(header, read, header.size - read)
                if (count < 0) break
                read += count
            }
            return if (isAnimatedImage("image/png", header.copyOf(read))) ApngDecoder(result, options) else null
        }
    }

    private companion object {
        const val HEADER_BYTES = 64 * 1024
        const val MAX_FRAMES = 300
        const val BYTE_BUDGET = 16 * 1024 * 1024
        const val MIN_EDGE = 48
        const val DISPOSE_NONE = 0
        const val DISPOSE_PREVIOUS = 2
        const val BLEND_SOURCE = 0
    }
}

/** An APNG split into standalone frame PNGs, with each frame's placement and timing. */
internal class Apng(val width: Int, val height: Int, val frames: List<Frame>) {
    class Frame(
        val png: ByteArray, val x: Int, val y: Int, val width: Int, val height: Int,
        val delayMillis: Int, val dispose: Int, val blend: Int,
    )

    companion object {
        private val SIGNATURE = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A)

        /** Null for anything but a well-formed APNG with at least one frame. */
        fun parse(bytes: ByteArray): Apng? {
            if (bytes.size < 8 || !bytes.copyOfRange(0, 8).contentEquals(SIGNATURE)) return null
            var ihdr: ByteArray? = null
            var animated = false
            val shared = ByteArrayOutputStream()
            val frames = ArrayList<Frame>()
            var control: ByteArray? = null
            var data = ByteArrayOutputStream()
            var seenImageData = false
            fun finish() {
                val fctl = control ?: return
                val header = ihdr ?: return
                val width = fctl.int(4)
                val height = fctl.int(8)
                val numerator = fctl.short(20)
                val denominator = fctl.short(22).takeIf { it != 0 } ?: 100
                val delay = numerator * 1000 / denominator
                frames += Frame(
                    png = standalone(header, width, height, shared.toByteArray(), data.toByteArray()),
                    x = fctl.int(12), y = fctl.int(16), width = width, height = height,
                    // Browsers play tiny delays at 100 ms.
                    delayMillis = if (delay < 20) 100 else delay,
                    dispose = fctl[24].toInt(), blend = fctl[25].toInt(),
                )
                control = null
                data = ByteArrayOutputStream()
            }
            var at = 8
            while (at + 12 <= bytes.size) {
                val length = bytes.int(at)
                if (length < 0 || at + 12 + length > bytes.size) return null
                val type = String(bytes, at + 4, 4, Charsets.US_ASCII)
                val body = bytes.copyOfRange(at + 8, at + 8 + length)
                when (type) {
                    "IHDR" -> ihdr = body
                    "acTL" -> animated = true
                    "fcTL" -> { finish(); if (body.size < 26) return null; control = body }
                    // The default image is the first frame only when an fcTL precedes it.
                    "IDAT" -> { seenImageData = true; if (control != null) data.write(body) }
                    "fdAT" -> if (body.size > 4) data.write(body, 4, body.size - 4)
                    "IEND" -> break
                    else -> if (!seenImageData) shared.write(chunk(type, body))
                }
                at += 12 + length
            }
            finish()
            val header = ihdr ?: return null
            if (!animated || frames.isEmpty() || header.size < 13) return null
            val width = header.int(0)
            val height = header.int(4)
            if (width !in 1..MAX_SIDE || height !in 1..MAX_SIDE) return null
            if (frames.any { it.width < 1 || it.height < 1 || it.x < 0 || it.y < 0 || it.x + it.width > width || it.y + it.height > height }) return null
            return Apng(width, height, frames)
        }

        private const val MAX_SIDE = 4096

        private fun standalone(ihdr: ByteArray, width: Int, height: Int, shared: ByteArray, data: ByteArray): ByteArray {
            val header = ihdr.copyOf().apply { putInt(0, width); putInt(4, height) }
            return ByteArrayOutputStream().apply {
                write(SIGNATURE)
                write(chunk("IHDR", header))
                write(shared)
                write(chunk("IDAT", data))
                write(chunk("IEND", ByteArray(0)))
            }.toByteArray()
        }

        private fun chunk(type: String, body: ByteArray): ByteArray {
            val kind = type.toByteArray(Charsets.US_ASCII)
            val crc = CRC32().apply { update(kind); update(body) }.value
            return ByteArrayOutputStream(body.size + 12).apply {
                write(ByteArray(4).apply { putInt(0, body.size) })
                write(kind)
                write(body)
                write(ByteArray(4).apply { putInt(0, crc.toInt()) })
            }.toByteArray()
        }

        private fun ByteArray.int(at: Int): Int =
            ((this[at].toInt() and 0xFF) shl 24) or ((this[at + 1].toInt() and 0xFF) shl 16) or
                ((this[at + 2].toInt() and 0xFF) shl 8) or (this[at + 3].toInt() and 0xFF)

        private fun ByteArray.short(at: Int): Int = ((this[at].toInt() and 0xFF) shl 8) or (this[at + 1].toInt() and 0xFF)

        private fun ByteArray.putInt(at: Int, value: Int) {
            this[at] = (value ushr 24).toByte(); this[at + 1] = (value ushr 16).toByte()
            this[at + 2] = (value ushr 8).toByte(); this[at + 3] = value.toByte()
        }
    }
}
