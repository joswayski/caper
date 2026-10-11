package chat.caper.android.push

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.os.Build
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.util.concurrent.TimeUnit

/** Bounded image/poster download and device-only PNG. Never fetches video bytes. */
internal object NotificationAttachment {
    private val client = OkHttpClient.Builder().callTimeout(5, TimeUnit.SECONDS)
        .followRedirects(false).followSslRedirects(false).build()

    fun load(context: Context, attachment: PushAttachment?): File? {
        attachment ?: return null
        val original = runCatching { File.createTempFile("caper-notification-original-", ".media", context.cacheDir) }.getOrNull() ?: return null
        return try {
            client.newCall(Request.Builder().url(attachment.url).build()).execute().use { response ->
                val body = response.body ?: return null
                if (response.code != 200 || body.contentLength() > attachment.size) return null
                body.byteStream().use { input -> original.outputStream().use { output ->
                    val buffer = ByteArray(8192)
                    var total = 0L
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        total += count
                        if (total > attachment.size) return null
                        output.write(buffer, 0, count)
                    }
                    if (total != attachment.size) return null
                } }
            }
            val bitmap = if (Build.VERSION.SDK_INT >= 28) ImageDecoder.decodeBitmap(ImageDecoder.createSource(original)) { decoder, info, _ ->
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val scale = minOf(1.0, 1024.0 / maxOf(info.size.width, info.size.height))
                decoder.setTargetSize(maxOf(1, (info.size.width * scale).toInt()), maxOf(1, (info.size.height * scale).toInt()))
            } else {
                val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(original.path, options)
                if (options.outWidth <= 0 || options.outHeight <= 0) return null
                options.inJustDecodeBounds = false
                options.inSampleSize = 1
                while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 1024) options.inSampleSize *= 2
                BitmapFactory.decodeFile(original.path, options)
            } ?: return null
            // Keep in-flight files outside the directory swept by notification cleanup.
            val output = File.createTempFile("caper-notification-preview-", ".png", context.cacheDir)
            try {
                output.outputStream().use { if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) error("No image") }
                output
            } catch (_: Exception) { output.delete(); null }
            finally { bitmap.recycle() }
        } catch (_: Exception) { null }
        finally { original.delete() }
    }
}
