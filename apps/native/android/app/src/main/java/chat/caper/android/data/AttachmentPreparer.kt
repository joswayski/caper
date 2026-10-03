package chat.caper.android.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import androidx.exifinterface.media.ExifInterface
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.Presentation
import androidx.media3.transformer.AudioEncoderSettings
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import chat.caper.android.model.CompressionSettings
import java.io.File
import java.io.FileOutputStream
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Name, type and a stable local copy of a picked document or photo. */
data class PickedFile(val source: File, val name: String, val contentType: String)

/**
 * Applies the server's compression policy on the device (web `prepareFile`): stills become exact
 * indexed PNGs or lossy WebP, videos are transcoded to H.264/AAC MP4, and previews are drawn.
 * The API verifies stored bytes independently; this only saves storage and bandwidth.
 */
class AttachmentPreparer(private val context: Context) {
    /** Copies the picked content into [directory] so its exact byte size is known. */
    suspend fun copy(uri: Uri, directory: File): PickedFile = withContext(Dispatchers.IO) {
        directory.mkdirs()
        val resolver = context.contentResolver
        val displayName = runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull()
        val name = (displayName ?: uri.lastPathSegment ?: "file").substringAfterLast('/').filterNot { it.isISOControl() }.trim().ifEmpty { "file" }.take(255)
        val extension = name.substringAfterLast('.', "").lowercase()
        val type = AttachmentPolicy.normalizedType(
            resolver.getType(uri) ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension),
        )
        val source = File(directory, "source")
        val input = resolver.openInputStream(uri) ?: throw UploadException("This file could not be read.")
        input.use { stream -> FileOutputStream(source).use { stream.copyTo(it, 64 * 1024) } }
        if (source.length() < 1) throw UploadException("This file is empty.")
        PickedFile(source, name, type)
    }

    suspend fun prepare(picked: PickedFile, settings: CompressionSettings, directory: File, transcodeProgress: (Float) -> Unit = {}): PreparedAttachment {
        val base = PreparedAttachment(picked.source, picked.name, picked.contentType, AttachmentPolicy.kind(picked.contentType), picked.source.length())
        return when {
            picked.contentType.startsWith("image/") -> withContext(Dispatchers.Default) { prepareImage(base, settings, directory) }
            picked.contentType.startsWith("video/") -> prepareVideo(base, settings, directory, transcodeProgress)
            base.kind == "audio" -> withContext(Dispatchers.IO) { base.copy(durationMs = metadata(base.file)?.durationMs) }
            else -> base
        }
    }

    // --- Stills -------------------------------------------------------------------------------

    private suspend fun prepareImage(base: PreparedAttachment, settings: CompressionSettings, directory: File): PreparedAttachment {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(base.file.path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return base
        val rotation = if (base.contentType == "image/jpeg" || base.contentType == "image/heic" || base.contentType == "image/heif") orientation(base.file) else 0
        val swapped = rotation == 90 || rotation == 270
        val width = if (swapped) bounds.outHeight else bounds.outWidth
        val height = if (swapped) bounds.outWidth else bounds.outHeight
        var prepared = base.copy(width = width, height = height)
        val header = base.file.inputStream().use { stream ->
            val bytes = ByteArray(64 * 1024)
            var read = 0
            while (read < bytes.size) { val count = stream.read(bytes, read, bytes.size - read); if (count < 0) break; read += count }
            bytes.copyOf(read)
        }
        val (targetWidth, targetHeight) = AttachmentPolicy.fitWithin(width, height, settings.imageMaxEdge)
        val stillAllowed = AttachmentPolicy.compressibleStill(base.contentType) && !isAnimatedImage(base.contentType, header) &&
            targetWidth.toLong() * targetHeight <= AttachmentPolicy.MAX_COMPRESS_PIXELS
        // GIF and AVIF stay unchanged but still get a preview when large.
        if (!stillAllowed && !AttachmentPolicy.needsPreview(prepared.kind, width, height, base.file.length(), settings.previewEdge)) return prepared
        val bitmap = decode(base.file, bounds, rotation, targetWidth, targetHeight) ?: return prepared
        try {
            currentCoroutineContext().ensureActive()
            if (stillAllowed) {
                val resized = bitmap.width != width || bitmap.height != height
                val palette = if (settings.paletteColors > 0) paletteOf(bitmap.width, bitmap.height, settings.paletteColors.coerceAtMost(256)) { y, row ->
                    bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
                } else null
                currentCoroutineContext().ensureActive()
                val encoding = AttachmentPolicy.stillEncoding(palette?.size, settings, resized, AttachmentPolicy.kind(base.contentType) == "image")
                val encoded = when (encoding) {
                    AttachmentPolicy.StillEncoding.PALETTE_PNG -> File(directory, "still.png").also { file ->
                        FileOutputStream(file).buffered().use { out ->
                            IndexedPng.encode(bitmap.width, bitmap.height, requireNotNull(palette), out) { y, row ->
                                bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
                            }
                        }
                    } to "image/png"
                    AttachmentPolicy.StillEncoding.LOSSY -> lossy(bitmap, settings.imageQuality.coerceIn(1, 100), File(directory, "still"))
                    AttachmentPolicy.StillEncoding.KEEP -> null
                }
                if (encoded != null && AttachmentPolicy.keepReencoded(base.contentType, base.file.length(), encoded.first.length())) {
                    // Re-encoding also drops EXIF metadata such as photo GPS coordinates.
                    prepared = prepared.copy(
                        file = encoded.first, contentType = encoded.second, kind = "image",
                        name = AttachmentPolicy.renamed(base.name, encoded.second), width = bitmap.width, height = bitmap.height,
                    )
                } else encoded?.first?.delete()
            }
            if (AttachmentPolicy.needsPreview(prepared.kind, prepared.width, prepared.height, prepared.file.length(), settings.previewEdge)) {
                preview(bitmap, settings.previewEdge, directory)?.let { prepared = prepared.copy(preview = it.first, previewContentType = it.second) }
            }
        } finally {
            bitmap.recycle()
        }
        return prepared
    }

    private fun orientation(file: File): Int = runCatching {
        when (ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
            ExifInterface.ORIENTATION_ROTATE_90, ExifInterface.ORIENTATION_TRANSPOSE -> 90
            ExifInterface.ORIENTATION_ROTATE_180 -> 180
            ExifInterface.ORIENTATION_ROTATE_270, ExifInterface.ORIENTATION_TRANSVERSE -> 270
            else -> 0
        }
    }.getOrDefault(0)

    /** Decodes with subsampling, applies EXIF rotation, then scales to exactly the target size. */
    private fun decode(file: File, bounds: BitmapFactory.Options, rotation: Int, targetWidth: Int, targetHeight: Int): Bitmap? {
        val (decodeWidth, decodeHeight) = if (rotation == 90 || rotation == 270) targetHeight to targetWidth else targetWidth to targetHeight
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= decodeWidth && bounds.outHeight / (sample * 2) >= decodeHeight) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample; inPreferredConfig = Bitmap.Config.ARGB_8888 }
        val decoded = runCatching { BitmapFactory.decodeFile(file.path, options) }.getOrNull() ?: return null
        if (rotation == 0 && decoded.width == targetWidth && decoded.height == targetHeight) return decoded
        val matrix = Matrix().apply {
            postRotate(rotation.toFloat())
            val rotatedWidth = if (rotation == 90 || rotation == 270) decoded.height else decoded.width
            val rotatedHeight = if (rotation == 90 || rotation == 270) decoded.width else decoded.height
            postScale(targetWidth.toFloat() / rotatedWidth, targetHeight.toFloat() / rotatedHeight)
        }
        val transformed = runCatching { Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, matrix, true) }.getOrNull()
        if (transformed !== decoded) decoded.recycle()
        return transformed
    }

    /** WebP where the platform can encode it, else JPEG flattened onto white. */
    private fun lossy(bitmap: Bitmap, quality: Int, stem: File): Pair<File, String>? {
        val webp = File(stem.path + ".webp")
        @Suppress("DEPRECATION")
        val format = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP
        if (runCatching { FileOutputStream(webp).use { bitmap.compress(format, quality, it) } }.getOrDefault(false) && webp.length() > 0) {
            return webp to "image/webp"
        }
        webp.delete()
        val jpeg = File(stem.path + ".jpg")
        val opaque = if (bitmap.hasAlpha()) Bitmap.createBitmap(bitmap.width, bitmap.height, Bitmap.Config.ARGB_8888).also { flat ->
            Canvas(flat).apply { drawColor(Color.WHITE); drawBitmap(bitmap, 0f, 0f, null) }
        } else bitmap
        val written = runCatching { FileOutputStream(jpeg).use { opaque.compress(Bitmap.CompressFormat.JPEG, quality, it) } }.getOrDefault(false)
        if (opaque !== bitmap) opaque.recycle()
        return if (written && jpeg.length() > 0) jpeg to "image/jpeg" else null.also { jpeg.delete() }
    }

    private fun preview(bitmap: Bitmap, edge: Int, directory: File): Pair<File, String>? {
        val (width, height) = AttachmentPolicy.fitWithin(bitmap.width, bitmap.height, edge)
        val scaled = if (width == bitmap.width && height == bitmap.height) bitmap else Bitmap.createScaledBitmap(bitmap, width, height, true)
        try {
            val result = lossy(scaled, AttachmentPolicy.PREVIEW_QUALITY, File(directory, "preview")) ?: return null
            return result.takeIf { it.first.length() in 1..AttachmentPolicy.PREVIEW_MAX_BYTES } ?: null.also { result.first.delete() }
        } finally {
            if (scaled !== bitmap) scaled.recycle()
        }
    }

    // --- Video and audio ----------------------------------------------------------------------

    private data class Metadata(val width: Int?, val height: Int?, val durationMs: Long?)

    private fun metadata(file: File): Metadata? = runCatching {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(file.path)
            val rotation = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
            val rawWidth = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull()?.takeIf { it > 0 }
            val rawHeight = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull()?.takeIf { it > 0 }
            val swapped = rotation == 90 || rotation == 270
            Metadata(
                if (swapped) rawHeight else rawWidth, if (swapped) rawWidth else rawHeight,
                retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()?.takeIf { it >= 0 },
            )
        } finally { retriever.release() }
    }.getOrNull()

    private fun poster(file: File, durationMs: Long?, edge: Int, directory: File): Pair<File, String>? = runCatching {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(file.path)
            // Just after the start, past common black first frames.
            val at = minOf(500L, (durationMs ?: 1000L) / 2) * 1000
            val frame = retriever.getFrameAtTime(at, MediaMetadataRetriever.OPTION_CLOSEST_SYNC) ?: return@runCatching null
            try { preview(frame, edge, directory) } finally { frame.recycle() }
        } finally { retriever.release() }
    }.getOrNull()

    private suspend fun prepareVideo(base: PreparedAttachment, settings: CompressionSettings, directory: File, progress: (Float) -> Unit): PreparedAttachment {
        val measured = withContext(Dispatchers.IO) { metadata(base.file) }
        var prepared = base.copy(width = measured?.width, height = measured?.height, durationMs = measured?.durationMs)
        val target = AttachmentPolicy.videoTargetHeight(measured?.width, measured?.height, settings)
        if (target != null && measured?.height != null) {
            val output = File(directory, "video.mp4")
            val transcoded = runCatching { transcode(base.file, output, target.takeIf { it < measured.height }, settings, progress) }
                .getOrElse { if (it is CancellationException) throw it; false }
            if (transcoded && AttachmentPolicy.keepTranscoded(base.contentType, base.file.length(), output.length())) {
                val after = withContext(Dispatchers.IO) { metadata(output) }
                prepared = prepared.copy(
                    file = output, contentType = "video/mp4", kind = "video", name = AttachmentPolicy.renamed(base.name, "video/mp4"),
                    width = after?.width ?: prepared.width, height = after?.height ?: prepared.height, durationMs = after?.durationMs ?: prepared.durationMs,
                )
            } else output.delete()
        }
        if (prepared.kind == "video") withContext(Dispatchers.IO) { poster(prepared.file, prepared.durationMs, settings.previewEdge, directory) }?.let {
            prepared = prepared.copy(preview = it.first, previewContentType = it.second)
        }
        return prepared
    }

    /** H.264 + AAC in MP4 with Media3 Transformer. Returns false when the device cannot export it. */
    @OptIn(UnstableApi::class)
    private suspend fun transcode(input: File, output: File, height: Int?, settings: CompressionSettings, progress: (Float) -> Unit): Boolean =
        // Transformer must be driven from a thread with a Looper.
        withContext(Dispatchers.Main) {
            output.delete()
            val done = CompletableDeferred<Boolean>()
            val encoders = DefaultEncoderFactory.Builder(context)
                .setRequestedVideoEncoderSettings(VideoEncoderSettings.Builder().setBitrate(settings.videoBitrateKbps * 1000).build())
                .setRequestedAudioEncoderSettings(AudioEncoderSettings.Builder().setBitrate(settings.audioBitrateKbps * 1000).build())
                .build()
            val transformer = Transformer.Builder(context)
                .setVideoMimeType(MimeTypes.VIDEO_H264)
                .setAudioMimeType(MimeTypes.AUDIO_AAC)
                .setEncoderFactory(encoders)
                .addListener(object : Transformer.Listener {
                    override fun onCompleted(composition: Composition, exportResult: ExportResult) { done.complete(true) }
                    override fun onError(composition: Composition, exportResult: ExportResult, exportException: ExportException) { done.complete(false) }
                })
                .build()
            val effects = if (height != null) Effects(emptyList(), listOf(Presentation.createForHeight(height))) else Effects.EMPTY
            transformer.start(EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(input))).setEffects(effects).build(), output.path)
            val poll = launch {
                val holder = ProgressHolder()
                while (true) {
                    if (transformer.getProgress(holder) == Transformer.PROGRESS_STATE_AVAILABLE) progress(holder.progress / 100f)
                    delay(500)
                }
            }
            try {
                done.await()
            } catch (cancelled: CancellationException) {
                transformer.cancel()
                output.delete()
                throw cancelled
            } finally {
                poll.cancel()
            }
        }
}
