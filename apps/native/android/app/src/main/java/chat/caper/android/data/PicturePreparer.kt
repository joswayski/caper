package chat.caper.android.data

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/**
 * Turns a picked image (a local copy, see [AttachmentPreparer.copy]) into the bytes a profile
 * picture or space icon uploads. A GIF (by magic bytes) keeps animating: byte for byte at any
 * size within 1024 × 1024, else shrunk by [GifShrink] to a 512 px square. Anything else the platform decodes is oriented,
 * center-cropped to a square of at most 512 px and encoded as lossy WebP, which also drops EXIF
 * and location metadata. AVIF and HEIC never upload as themselves.
 */
object PicturePreparer {
    suspend fun prepare(file: File): PreparedPicture = withContext(Dispatchers.Default) {
        val header = file.inputStream().use { stream ->
            val bytes = ByteArray(16)
            var read = 0
            while (read < bytes.size) { val count = stream.read(bytes, read, bytes.size - read); if (count < 0) break; read += count }
            bytes.copyOf(read)
        }
        gifSize(header)?.let { (width, height) ->
            if (gifUploadsUnchanged(width, height)) {
                return@withContext PreparedPicture(file.readBytes(), "image/gif", width, height)
            }
            val context = currentCoroutineContext()
            val shrunk = try {
                GifShrink.shrink(file.readBytes()) { !context.isActive }
            } catch (failure: GifShrink.Failure) {
                context.ensureActive()
                throw PictureException(PicturePolicy.GIF_UNREADABLE)
            }
            return@withContext PreparedPicture(shrunk.bytes, "image/gif", shrunk.edge, shrunk.edge)
        }
        val decoded = decode(file) ?: throw PictureException(PicturePolicy.UNUSABLE)
        currentCoroutineContext().ensureActive()
        val square = square(decoded) ?: throw PictureException(PicturePolicy.UNUSABLE)
        try {
            currentCoroutineContext().ensureActive()
            encode(square) ?: throw PictureException(PicturePolicy.UNUSABLE)
        } finally {
            square.recycle()
        }
    }

    /** Subsampled close to the target size, upright, in software memory so it can be cropped. */
    private fun decode(file: File): Bitmap? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) decodeWithImageDecoder(file) else decodeWithBitmapFactory(file)

    /** ImageDecoder (HEIC, AVIF on Android 12+, WebP, PNG, JPEG, BMP) applies EXIF orientation itself. */
    @RequiresApi(Build.VERSION_CODES.P)
    private fun decodeWithImageDecoder(file: File): Bitmap? = runCatching {
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            decoder.setTargetSampleSize(pictureSampleSize(info.size.width, info.size.height))
        }
    }.getOrNull()

    /** Android 8–8.1: BitmapFactory, then the EXIF rotation and mirroring applied by hand. */
    private fun decodeWithBitmapFactory(file: File): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        val options = BitmapFactory.Options().apply {
            inSampleSize = pictureSampleSize(bounds.outWidth, bounds.outHeight)
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        val decoded = runCatching { BitmapFactory.decodeFile(file.path, options) }.getOrNull() ?: return null
        val exif = runCatching { ExifInterface(file.path) }.getOrNull()
        val rotation = exif?.rotationDegrees ?: 0
        val flipped = exif?.isFlipped == true
        if (rotation == 0 && !flipped) return decoded
        // A mirrored orientation is a horizontal flip followed by the rotation.
        val matrix = Matrix().apply {
            if (flipped) postScale(-1f, 1f)
            postRotate(rotation.toFloat())
        }
        val oriented = runCatching { Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, matrix, true) }.getOrNull()
        if (oriented !== decoded) decoded.recycle()
        return oriented
    }

    /** The centered square, scaled to [pictureEdge]. Recycles [bitmap]. */
    private fun square(bitmap: Bitmap): Bitmap? {
        val crop = squareCrop(bitmap.width, bitmap.height)
        val edge = pictureEdge(bitmap.width, bitmap.height)
        val cropped = runCatching { Bitmap.createBitmap(bitmap, crop.left, crop.top, crop.side, crop.side) }.getOrNull()
        if (cropped !== bitmap) bitmap.recycle()
        if (cropped == null) return null
        if (cropped.width == edge && cropped.height == edge) return cropped
        val scaled = runCatching { Bitmap.createScaledBitmap(cropped, edge, edge, true) }.getOrNull()
        if (scaled !== cropped) cropped.recycle()
        return scaled
    }

    /** Lossy WebP within 1 MiB; PNG, then JPEG, only if this device cannot encode WebP. */
    private fun encode(square: Bitmap): PreparedPicture? {
        @Suppress("DEPRECATION")
        val webp = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP
        var encodedWebp = false
        for (quality in PicturePolicy.QUALITIES) {
            val bytes = compress(square, webp, quality) ?: break
            encodedWebp = true
            if (bytes.size <= PicturePolicy.STILL_MAX_BYTES) return PreparedPicture(bytes, "image/webp", square.width, square.height)
        }
        if (!encodedWebp) compress(square, Bitmap.CompressFormat.PNG, 100)?.takeIf { it.size <= PicturePolicy.STILL_MAX_BYTES }?.let {
            return PreparedPicture(it, "image/png", square.width, square.height)
        }
        // JPEG has no transparency: flatten onto the app's blackout.
        val opaque = if (square.hasAlpha()) Bitmap.createBitmap(square.width, square.height, Bitmap.Config.ARGB_8888).also { flat ->
            Canvas(flat).apply { drawColor(Color.rgb(0x0C, 0x0D, 0x0F)); drawBitmap(square, 0f, 0f, null) }
        } else square
        try {
            for (quality in PicturePolicy.QUALITIES) {
                val bytes = compress(opaque, Bitmap.CompressFormat.JPEG, quality) ?: return null
                if (bytes.size <= PicturePolicy.STILL_MAX_BYTES) return PreparedPicture(bytes, "image/jpeg", square.width, square.height)
            }
            return null
        } finally {
            if (opaque !== square) opaque.recycle()
        }
    }

    private fun compress(bitmap: Bitmap, format: Bitmap.CompressFormat, quality: Int): ByteArray? {
        val out = ByteArrayOutputStream()
        val written = runCatching { bitmap.compress(format, quality, out) }.getOrDefault(false)
        return out.toByteArray().takeIf { written && it.isNotEmpty() }
    }
}
