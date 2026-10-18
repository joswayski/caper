package chat.caper.android

import android.graphics.Bitmap
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import coil3.compose.AsyncImage
import coil3.request.allowHardware
import coil3.request.crossfade
import coil3.request.transformations
import coil3.size.Size
import coil3.transform.Transformation

/** Small CPU-only poster blur, including on API 26. Never mutates Coil's cached source bitmap. */
private object SpoilerBlur : Transformation() {
    override val cacheKey = "spoiler-cpu-24-radius12-v2"

    override suspend fun transform(input: Bitmap, size: Size): Bitmap {
        val scale = minOf(1f, 24f / maxOf(input.width, input.height))
        val width = maxOf(1, (input.width * scale).toInt())
        val height = maxOf(1, (input.height * scale).toInt())
        val small = Bitmap.createScaledBitmap(input, width, height, true)
        val pixels = IntArray(width * height)
        small.getPixels(pixels, 0, width, 0, 0, width, height)
        if (small !== input) small.recycle()
        val output = IntArray(pixels.size)
        for (y in 0 until height) for (x in 0 until width) {
            var a = 0; var r = 0; var g = 0; var b = 0; var count = 0
            for (dy in -12..12) for (dx in -12..12) {
                val pixel = pixels[(y + dy).coerceIn(0, height - 1) * width + (x + dx).coerceIn(0, width - 1)]
                a += pixel ushr 24; r += pixel ushr 16 and 255
                g += pixel ushr 8 and 255; b += pixel and 255; count++
            }
            output[y * width + x] = (a / count shl 24) or (r / count shl 16) or (g / count shl 8) or (b / count)
        }
        return Bitmap.createBitmap(output, width, height, Bitmap.Config.ARGB_8888)
    }
}

/** Only a transformed, static bitmap can reach this painter; failures leave the neutral surface. */
@Composable internal fun SpoilerThumbnail(data: String?, cacheKey: String, onError: (Throwable) -> Unit = {}) {
    if (data == null) return
    val context = LocalContext.current
    val request = remember(data, cacheKey) {
        AttachmentImages.request(context, data, "$cacheKey:${SpoilerBlur.cacheKey}").newBuilder()
            .size(192).allowHardware(false).crossfade(false).transformations(SpoilerBlur).build()
    }
    // A new painter when the mask/source changes cannot retain an earlier unmasked drawable.
    key(request) {
        AsyncImage(model = request, contentDescription = null, imageLoader = AttachmentImages.loader(context),
            onError = { onError(it.result.throwable) },
            modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
    }
}
