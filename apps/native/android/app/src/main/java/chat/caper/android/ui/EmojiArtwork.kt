package chat.caper.android.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable data class EmojiEntry(
    val id: String, val emoji: String, val name: String, val keywords: String, val category: String,
    val selectable: Boolean, val sheet: Int, val x: Int, val y: Int,
)

internal object EmojiArtwork {
    private val crops = LruCache<String, Bitmap>(192)
    private val pages = LruCache<Int, Bitmap>(4)
    private var catalog: List<EmojiEntry>? = null

    fun catalog(context: Context): List<EmojiEntry> = synchronized(this) {
        catalog ?: context.assets.open("catalog.json").bufferedReader().use {
            Json { ignoreUnknownKeys = true }.decodeFromString<List<EmojiEntry>>(it.readText())
        }.also { catalog = it }
    }

    fun id(emoji: String): String {
        val points = emoji.codePoints().toArray()
        val zwj = points.contains(0x200d)
        return points.filter { zwj || it != 0xfe0f }.joinToString("-") { it.toString(16) }
    }

    fun bitmap(context: Context, emoji: String): Bitmap? {
        val id = id(emoji)
        crops.get(id)?.let { return it }
        val entry = catalog(context).firstOrNull { it.id == id } ?: return null
        val sheet = pages.get(entry.sheet) ?: context.assets.open("sheet-${entry.sheet}.png").use {
            BitmapFactory.decodeStream(it)
        }.also { pages.put(entry.sheet, it) }
        return Bitmap.createBitmap(sheet, entry.x, entry.y, 64, 64).also {
            crops.put(id, it)
        }
    }
}

@Composable internal fun EmojiImage(emoji: String, description: String?, modifier: Modifier = Modifier) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val bitmap = remember(emoji) { EmojiArtwork.bitmap(context, emoji)?.asImageBitmap() }
    if (bitmap != null) Image(bitmap, description, modifier)
}
