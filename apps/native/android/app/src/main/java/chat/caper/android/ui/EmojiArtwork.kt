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
    private var names: Map<String, String>? = null
    /** The catalog by ID, built once; the first entry wins, as a scan would find it. */
    private var entries: Map<String, EmojiEntry>? = null

    /**
     * The catalog, read once (MainActivity starts it off the main thread at launch). The shared
     * autocomplete searches the same bytes, and the name and ID lookups are built with it.
     */
    fun catalog(context: Context): List<EmojiEntry> = synchronized(this) {
        catalog ?: load(context.assets.open("catalog.json").use { it.readBytes() })
    }

    private fun load(bytes: ByteArray): List<EmojiEntry> = synchronized(this) {
        chat.caper.android.data.CaperMobile.emojiCatalog(bytes)
        Json { ignoreUnknownKeys = true }.decodeFromString<List<EmojiEntry>>(bytes.decodeToString()).also {
            catalog = it
            names = emojiNameIndex(it)
            entries = HashMap<String, EmojiEntry>().apply { it.forEach { entry -> putIfAbsent(entry.id, entry) } }
        }
    }

    /** The catalog's dash-separated name ("thumbs-up"), or null when it has none. */
    fun name(context: Context, emoji: String): String? {
        val index = synchronized(this) { catalog(context); names!! }
        return emojiName(emoji, index)
    }

    fun id(emoji: String): String {
        val points = emoji.codePoints().toArray()
        val zwj = points.contains(0x200d)
        return points.filter { zwj || it != 0xfe0f }.joinToString("-") { it.toString(16) }
    }

    fun bitmap(context: Context, emoji: String): Bitmap? {
        val id = id(emoji)
        crops.get(id)?.let { return it }
        val index = synchronized(this) {
            catalog(context); entries!!
        }
        val entry = index[id] ?: return null
        val sheet = pages.get(entry.sheet) ?: context.assets.open("sheet-${entry.sheet}.png").use {
            BitmapFactory.decodeStream(it)
        }.also { pages.put(entry.sheet, it) }
        return Bitmap.createBitmap(sheet, entry.x, entry.y, 64, 64).also {
            crops.put(id, it)
        }
    }
}

/**
 * Reactions are stored fully qualified ("❤️" is U+2764 U+FE0F) while the catalog
 * keys some emoji without U+FE0F, so names are matched with it removed on both
 * sides. Picker entries win over unqualified duplicates, and entries whose name
 * is only their code-point ID have no name.
 */
internal fun emojiNameIndex(catalog: List<EmojiEntry>): Map<String, String> {
    val index = HashMap<String, String>()
    catalog.filter { it.name.isNotBlank() && it.name != it.id }
        .sortedByDescending { it.selectable }
        .forEach { index.putIfAbsent(withoutVariationSelectors(it.emoji), it.name) }
    return index
}

internal fun emojiName(emoji: String, index: Map<String, String>): String? = index[withoutVariationSelectors(emoji)]

private fun withoutVariationSelectors(emoji: String) = emoji.replace("\uFE0F", "")

@Composable internal fun EmojiImage(emoji: String, description: String?, modifier: Modifier = Modifier) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val bitmap = remember(emoji) { EmojiArtwork.bitmap(context, emoji)?.asImageBitmap() }
    if (bitmap != null) Image(bitmap, description, modifier)
}
