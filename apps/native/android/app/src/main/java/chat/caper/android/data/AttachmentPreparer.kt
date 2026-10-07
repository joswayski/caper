package chat.caper.android.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.annotation.OptIn
import androidx.exifinterface.media.ExifInterface
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.Presentation
import androidx.media3.transformer.AudioEncoderSettings
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.EditedMediaItemSequence
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import chat.caper.android.data.AttachmentPolicy.LosslessEncoding
import chat.caper.android.data.AttachmentPolicy.StillClass
import chat.caper.android.model.CompressionSettings
import java.io.File
import java.io.FileOutputStream
import java.util.Locale
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
 * Applies the server's compression settings on the device (docs/media.md "Client compression
 * and previews", web `prepareFile`). Lossless stills stay lossless, photos become lossy WebP
 * only when that saves 10%, videos are transcoded to H.264/AAC MP4 only when needed (HDR tone
 * mapped to SDR), and previews are drawn. Whenever a rule cannot be met the original uploads.
 */
class AttachmentPreparer(private val context: Context) {
    /** Copies the picked content into [directory] so its exact byte size is known. */
    suspend fun copy(uri: Uri, directory: File): PickedFile = withContext(Dispatchers.IO) {
        directory.mkdirs()
        val resolver = context.contentResolver
        val displayName = runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst() && !cursor.isNull(0)) cursor.getString(0) else null
            }
        }.getOrNull()
        val name = (displayName ?: uri.lastPathSegment ?: "file").substringAfterLast('/').filterNot { it.isISOControl() }.trim().ifEmpty { "file" }.take(255)
        val extension = name.substringAfterLast('.', "").lowercase(Locale.ROOT)
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
        val prepared = when {
            picked.contentType.startsWith("image/") -> withContext(Dispatchers.Default) { prepareImage(base, settings, directory) }
            picked.contentType.startsWith("video/") -> prepareVideo(base, settings, directory, transcodeProgress)
            base.kind == "audio" -> withContext(Dispatchers.IO) { base.copy(durationMs = metadata(base.file)?.durationMs) }
            else -> base
        }
        return if (prepared.file == base.file) withContext(Dispatchers.IO) { stripOriginal(prepared, directory) } else prepared
    }

    /**
     * A file uploading as its original still drops location and camera metadata, losslessly:
     * JPEG Exif/XMP (orientation kept), PNG text/Exif chunks, MP4/MOV `udta`/`meta` boxes.
     */
    private fun stripOriginal(prepared: PreparedAttachment, directory: File): PreparedAttachment {
        val type = AttachmentPolicy.normalizedType(prepared.contentType)
        val output = File(directory, "clean")
        val stripped = when {
            type == "image/jpeg" -> MetadataStrip.jpeg(prepared.file, output)
            type == "image/png" -> MetadataStrip.png(prepared.file, output)
            type.startsWith("video/") -> MetadataStrip.mp4(prepared.file, output)
            else -> false
        }
        return if (stripped && output.length() > 0) prepared.copy(file = output) else prepared.also { output.delete() }
    }

    // --- Stills -------------------------------------------------------------------------------

    private suspend fun prepareImage(base: PreparedAttachment, settings: CompressionSettings, directory: File): PreparedAttachment {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(base.file.path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return base
        val header = base.file.inputStream().use { stream ->
            val bytes = ByteArray(64 * 1024)
            var read = 0
            while (read < bytes.size) { val count = stream.read(bytes, read, bytes.size - read); if (count < 0) break; read += count }
            bytes.copyOf(read)
        }
        val still = AttachmentPolicy.stillClass(base.contentType, header)
        val rotation = if (still == StillClass.KEEP) 0 else orientation(base.file)
        val swapped = rotation == 90 || rotation == 270
        val width = if (swapped) bounds.outHeight else bounds.outWidth
        val height = if (swapped) bounds.outWidth else bounds.outHeight
        var prepared = base.copy(width = width, height = height)
        val (targetWidth, targetHeight) = AttachmentPolicy.fitWithin(width, height, settings.imageMaxEdge)
        val encodable = still != StillClass.KEEP && targetWidth.toLong() * targetHeight <= AttachmentPolicy.MAX_COMPRESS_PIXELS &&
            width.toLong() * height <= AttachmentPolicy.MAX_COMPRESS_PIXELS * 4
        // GIF, SVG, AVIF and animations stay unchanged but still get a preview when large.
        if (!encodable && !AttachmentPolicy.needsPreview(prepared.kind, width, height, base.file.length(), settings.previewEdge)) return prepared
        val resized = targetWidth != width || targetHeight != height
        // Lossless sources decode unpremultiplied when nothing transforms them, so translucent
        // pixels keep their exact values through the palette or lossless WebP.
        val exact = still == StillClass.LOSSLESS && rotation == 0 && !resized
        val bitmap = decode(base.file, bounds, rotation, targetWidth, targetHeight, exact) ?: return prepared
        try {
            currentCoroutineContext().ensureActive()
            val inline = AttachmentPolicy.kind(base.contentType) == "image"
            val encoded = if (!encodable) null else when (still) {
                StillClass.LOSSLESS -> encodeLossless(bitmap, settings, inline, directory)
                    ?.takeIf { AttachmentPolicy.keepLossless(base.contentType, base.file.length(), it.first.length()) }
                StillClass.PHOTO -> if (AttachmentPolicy.reencodePhoto(settings, resized, inline)) {
                    lossy(bitmap, settings.imageQuality.coerceIn(1, 100), File(directory, "still"))
                        ?.takeIf { AttachmentPolicy.keepPhoto(base.contentType, base.file.length(), it.first.length()) }
                } else null
                StillClass.KEEP -> null
            }
            directory.listFiles { file -> file.name.startsWith("still") && file != encoded?.first }?.forEach { it.delete() }
            if (encoded != null) {
                // Re-encoding applied the orientation and dropped EXIF metadata such as GPS.
                prepared = prepared.copy(
                    file = encoded.first, contentType = encoded.second, kind = "image",
                    name = AttachmentPolicy.renamed(base.name, encoded.second), width = bitmap.width, height = bitmap.height,
                )
            }
            if (AttachmentPolicy.needsPreview(prepared.kind, prepared.width, prepared.height, prepared.file.length(), settings.previewEdge)) {
                preview(bitmap, settings.previewEdge, directory)?.let { prepared = prepared.copy(preview = it.first, previewContentType = it.second) }
            }
        } finally {
            bitmap.recycle()
        }
        return prepared
    }

    /** Indexed PNG when the colours fit, else lossless WebP (API 30+), else a PNG for non-inline sources. */
    private suspend fun encodeLossless(bitmap: Bitmap, settings: CompressionSettings, inline: Boolean, directory: File): Pair<File, String>? {
        val palette = if (settings.paletteColors > 0) paletteOf(bitmap.width, bitmap.height, settings.paletteColors.coerceAtMost(256)) { y, row ->
            bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
        } else null
        currentCoroutineContext().ensureActive()
        return when (AttachmentPolicy.losslessEncoding(palette?.size, settings, Build.VERSION.SDK_INT >= Build.VERSION_CODES.R, inline)) {
            LosslessEncoding.PALETTE_PNG -> File(directory, "still.png").let { file ->
                runCatching {
                    FileOutputStream(file).buffered().use { out ->
                        IndexedPng.encode(bitmap.width, bitmap.height, requireNotNull(palette), out) { y, row ->
                            bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
                        }
                    }
                }.getOrNull()?.let { file to "image/png" }
            }
            LosslessEncoding.WEBP_LOSSLESS -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                compress(bitmap, Bitmap.CompressFormat.WEBP_LOSSLESS, 100, File(directory, "still.webp"))?.let { it to "image/webp" }
            } else null
            LosslessEncoding.PNG -> compress(bitmap, Bitmap.CompressFormat.PNG, 100, File(directory, "still.png"))?.let { it to "image/png" }
            LosslessEncoding.KEEP -> null
        }
    }

    private fun compress(bitmap: Bitmap, format: Bitmap.CompressFormat, quality: Int, file: File): File? {
        val written = runCatching { FileOutputStream(file).use { bitmap.compress(format, quality, it) } }.getOrDefault(false)
        return if (written && file.length() > 0) file else null.also { file.delete() }
    }

    private fun orientation(file: File): Int = runCatching {
        when (ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
            ExifInterface.ORIENTATION_ROTATE_90, ExifInterface.ORIENTATION_TRANSPOSE -> 90
            ExifInterface.ORIENTATION_ROTATE_180 -> 180
            ExifInterface.ORIENTATION_ROTATE_270, ExifInterface.ORIENTATION_TRANSVERSE -> 270
            else -> 0
        }
    }.getOrDefault(0)

    /**
     * Decodes with subsampling, applies EXIF rotation, then scales to exactly the target size.
     * [exact] decodes unpremultiplied and untouched (lossless sources needing no transform).
     */
    private fun decode(file: File, bounds: BitmapFactory.Options, rotation: Int, targetWidth: Int, targetHeight: Int, exact: Boolean): Bitmap? {
        val (decodeWidth, decodeHeight) = if (rotation == 90 || rotation == 270) targetHeight to targetWidth else targetWidth to targetHeight
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= decodeWidth && bounds.outHeight / (sample * 2) >= decodeHeight) sample *= 2
        val options = BitmapFactory.Options().apply {
            inSampleSize = sample; inPreferredConfig = Bitmap.Config.ARGB_8888
            if (exact) inPremultiplied = false
        }
        val decoded = runCatching { BitmapFactory.decodeFile(file.path, options) }.getOrNull() ?: return null
        if (rotation == 0 && decoded.width == targetWidth && decoded.height == targetHeight) return decoded
        val rotatedWidth = if (rotation == 90 || rotation == 270) decoded.height else decoded.width
        val rotatedHeight = if (rotation == 90 || rotation == 270) decoded.width else decoded.height
        val scaling = rotatedWidth != targetWidth || rotatedHeight != targetHeight
        val matrix = Matrix().apply {
            postRotate(rotation.toFloat())
            postScale(targetWidth.toFloat() / rotatedWidth, targetHeight.toFloat() / rotatedHeight)
        }
        val transformed = runCatching { Bitmap.createBitmap(premultiplied(decoded), 0, 0, decoded.width, decoded.height, matrix, scaling) }.getOrNull()
        if (transformed !== decoded) decoded.recycle()
        return transformed
    }

    /** Canvas cannot draw unpremultiplied pixels with alpha; returns a drawable bitmap (maybe [bitmap] itself). */
    private fun premultiplied(bitmap: Bitmap): Bitmap {
        if (bitmap.isPremultiplied || !bitmap.hasAlpha()) return bitmap
        val copy = Bitmap.createBitmap(bitmap.width, bitmap.height, Bitmap.Config.ARGB_8888)
        val row = IntArray(bitmap.width)
        for (y in 0 until bitmap.height) {
            bitmap.getPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
            copy.setPixels(row, 0, bitmap.width, 0, y, bitmap.width, 1)
        }
        return copy
    }

    /** Lossy WebP where the platform can encode it, else JPEG flattened onto white. */
    private fun lossy(bitmap: Bitmap, quality: Int, stem: File): Pair<File, String>? {
        @Suppress("DEPRECATION")
        val format = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP
        compress(bitmap, format, quality, File(stem.path + ".webp"))?.let { return it to "image/webp" }
        val drawable = premultiplied(bitmap)
        val opaque = if (drawable.hasAlpha()) Bitmap.createBitmap(drawable.width, drawable.height, Bitmap.Config.ARGB_8888).also { flat ->
            Canvas(flat).apply { drawColor(Color.WHITE); drawBitmap(drawable, 0f, 0f, null) }
        } else drawable
        val jpeg = compress(opaque, Bitmap.CompressFormat.JPEG, quality, File(stem.path + ".jpg"))
        if (opaque !== bitmap) opaque.recycle()
        if (drawable !== bitmap && drawable !== opaque) drawable.recycle()
        return jpeg?.let { it to "image/jpeg" }
    }

    private fun preview(bitmap: Bitmap, edge: Int, directory: File): Pair<File, String>? {
        val (width, height) = AttachmentPolicy.fitWithin(bitmap.width, bitmap.height, edge)
        val drawable = premultiplied(bitmap)
        val scaled = if (width == drawable.width && height == drawable.height) drawable else Bitmap.createScaledBitmap(drawable, width, height, true)
        if (drawable !== bitmap && drawable !== scaled) drawable.recycle()
        try {
            val result = lossy(scaled, AttachmentPolicy.PREVIEW_QUALITY, File(directory, "preview")) ?: return null
            return result.takeIf { it.first.length() in 1..AttachmentPolicy.PREVIEW_MAX_BYTES } ?: null.also { result.first.delete() }
        } finally {
            if (scaled !== bitmap) scaled.recycle()
        }
    }

    // --- Video and audio ----------------------------------------------------------------------

    private data class Metadata(val width: Int?, val height: Int?, val durationMs: Long?, val bitrate: Long?)

    private fun metadata(file: File): Metadata? = runCatching {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(file.path)
            val rotation = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
            val rawWidth = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull()?.takeIf { it > 0 }
            val rawHeight = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull()?.takeIf { it > 0 }
            val swapped = rotation == 90 || rotation == 270
            val durationMs = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()?.takeIf { it >= 0 }
            val bitrate = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_BITRATE)?.toLongOrNull()?.takeIf { it > 0 }
                ?: durationMs?.takeIf { it > 0 }?.let { file.length() * 8_000 / it }
            Metadata(if (swapped) rawHeight else rawWidth, if (swapped) rawWidth else rawHeight, durationMs, bitrate)
        } finally { retriever.release() }
    }.getOrNull()

    /** Track facts the container declares: video codec, HDR transfer, and whether there is audio. */
    private data class Tracks(val videoMime: String?, val hdr: Boolean, val audioMime: String?)

    private fun tracks(file: File): Tracks? = runCatching {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(file.path)
            var video: MediaFormat? = null
            var audio: MediaFormat? = null
            for (index in 0 until extractor.trackCount) {
                val format = extractor.getTrackFormat(index)
                val mime = format.getString(MediaFormat.KEY_MIME) ?: continue
                if (mime.startsWith("video/") && video == null) video = format
                if (mime.startsWith("audio/") && audio == null) audio = format
            }
            val videoMime = video?.getString(MediaFormat.KEY_MIME)?.lowercase(Locale.ROOT)
            val transfer = video?.takeIf { it.containsKey(MediaFormat.KEY_COLOR_TRANSFER) }?.getInteger(MediaFormat.KEY_COLOR_TRANSFER)
            val hdr = transfer == MediaFormat.COLOR_TRANSFER_ST2084 || transfer == MediaFormat.COLOR_TRANSFER_HLG ||
                videoMime == MediaFormat.MIMETYPE_VIDEO_DOLBY_VISION
            Tracks(videoMime, hdr, audio?.getString(MediaFormat.KEY_MIME)?.lowercase(Locale.ROOT))
        } finally { extractor.release() }
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

    @OptIn(UnstableApi::class)
    private suspend fun prepareVideo(base: PreparedAttachment, settings: CompressionSettings, directory: File, progress: (Float) -> Unit): PreparedAttachment {
        val (measured, tracks) = withContext(Dispatchers.IO) { metadata(base.file) to tracks(base.file) }
        var prepared = base.copy(width = measured?.width, height = measured?.height, durationMs = measured?.durationMs)
        val facts = AttachmentPolicy.VideoFacts(measured?.width, measured?.height, tracks?.videoMime, measured?.bitrate, tracks?.hdr == true, base.contentType)
        // OpenGL HDR-to-SDR tone mapping needs API 29; without it HDR uploads unchanged.
        val plan = if (tracks?.videoMime != null) AttachmentPolicy.videoPlan(facts, settings, Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) else null
        if (plan != null) {
            val output = File(directory, "video.mp4")
            val result = runCatching { transcode(base.file, output, plan, settings, tracks?.audioMime, progress) }
                .getOrElse { if (it is CancellationException) throw it; null }
            val after = if (result != null && output.length() > 0) withContext(Dispatchers.IO) { tracks(output) } else null
            val outputHdr = after?.hdr == true || result?.colorInfo?.colorTransfer.let { it == C.COLOR_TRANSFER_ST2084 || it == C.COLOR_TRANSFER_HLG }
            val keep = result != null && after != null && after.videoMime == MimeTypes.VIDEO_H264 &&
                AttachmentPolicy.keepTranscoded(plan, base.file.length(), output.length(), tracks?.audioMime != null, after.audioMime != null, outputHdr)
            if (keep) {
                val measuredAfter = withContext(Dispatchers.IO) { metadata(output) }
                prepared = prepared.copy(
                    file = output, contentType = "video/mp4", kind = "video", name = AttachmentPolicy.renamed(base.name, "video/mp4"),
                    width = measuredAfter?.width ?: prepared.width, height = measuredAfter?.height ?: prepared.height,
                    durationMs = measuredAfter?.durationMs ?: prepared.durationMs,
                )
            } else output.delete()
        }
        if (prepared.kind == "video") withContext(Dispatchers.IO) { poster(prepared.file, prepared.durationMs, settings.previewEdge, directory) }?.let {
            prepared = prepared.copy(preview = it.first, previewContentType = it.second)
        }
        return prepared
    }

    /**
     * H.264 + AAC in MP4 with Media3 Transformer on the device's (hardware) encoders. Requesting a
     * video bitrate forces a real re-encode even for H.264 input; AAC audio is passed through
     * unchanged and other audio is encoded at `audioBitrateKbps`. HDR input is tone mapped to SDR
     * with OpenGL. Returns null when the device cannot export it, so the original is kept.
     */
    @OptIn(UnstableApi::class)
    private suspend fun transcode(
        input: File, output: File, plan: AttachmentPolicy.VideoPlan, settings: CompressionSettings, audioMime: String?, progress: (Float) -> Unit,
    ): ExportResult? =
        // Transformer must be driven from a thread with a Looper.
        withContext(Dispatchers.Main) {
            output.delete()
            val done = CompletableDeferred<ExportResult?>()
            val encoders = DefaultEncoderFactory.Builder(context)
                .setRequestedVideoEncoderSettings(VideoEncoderSettings.Builder().setBitrate(plan.bitrateKbps * 1000).build())
                .apply {
                    if (audioMime != null && audioMime != MimeTypes.AUDIO_AAC) {
                        setRequestedAudioEncoderSettings(AudioEncoderSettings.Builder().setBitrate(settings.audioBitrateKbps * 1000).build())
                    }
                }
                .build()
            val transformer = Transformer.Builder(context)
                .setVideoMimeType(MimeTypes.VIDEO_H264)
                .setAudioMimeType(MimeTypes.AUDIO_AAC)
                .setEncoderFactory(encoders)
                .addListener(object : Transformer.Listener {
                    override fun onCompleted(composition: Composition, exportResult: ExportResult) { done.complete(exportResult) }
                    override fun onError(composition: Composition, exportResult: ExportResult, exportException: ExportException) { done.complete(null) }
                })
                .build()
            val effects = plan.outputHeight?.let { Effects(emptyList(), listOf(Presentation.createForHeight(it))) } ?: Effects.EMPTY
            val item = EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(input))).setEffects(effects).build()
            val composition = Composition.Builder(EditedMediaItemSequence.Builder(item).build())
                .setHdrMode(if (plan.toneMap) Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL else Composition.HDR_MODE_KEEP_HDR)
                .build()
            transformer.start(composition, output.path)
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
