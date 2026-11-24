package chat.caper.android

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import androidx.core.content.ContextCompat
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.createBitmap
import androidx.core.graphics.drawable.IconCompat
import androidx.core.graphics.drawable.toBitmap
import chat.caper.android.data.ShareTarget
import chat.caper.android.model.ForwardDestination
import chat.caper.android.push.CaperNotifications
import chat.caper.android.ui.SurfaceRaised
import chat.caper.android.ui.MessageText
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import androidx.compose.ui.graphics.toArgb

/**
 * Sharing Shortcuts: the most recent conversations as direct targets in the system share
 * sheet (Android 10+). Picking one opens [ShareActivity] with its ID, skipping the picker.
 * Launchers may also list them on Caper's long-press menu, where they open the conversation
 * (excluding them from the launcher would unpublish them before Android 13).
 */
internal object ShareShortcuts {
    private const val CATEGORY = "chat.caper.android.category.SHARE_TARGET"
    private const val COUNT = 8

    /** Replaces Caper's sharing shortcuts with [destinations]' first few (newest first). */
    suspend fun publish(context: Context, destinations: List<ForwardDestination>, avatars: Map<String, Int?>) = withContext(Dispatchers.Default) {
        val limit = minOf(COUNT, ShortcutManagerCompat.getMaxShortcutCountPerActivity(context))
        val recent = destinations.mapNotNull { destination -> ShareTarget.of(destination)?.let { it to destination } }.take(limit)
        val kept = recent.map { it.first.shortcutId }.toSet()
        val stale = ShortcutManagerCompat.getDynamicShortcuts(context).map { it.id }.filter { ShareTarget.fromShortcutId(it) != null && it !in kept }
        if (stale.isNotEmpty()) { ShortcutManagerCompat.removeDynamicShortcuts(context, stale); ShortcutManagerCompat.removeLongLivedShortcuts(context, stale) }
        // Each push becomes rank 0, so the newest goes last.
        recent.asReversed().forEach { (target, destination) ->
            val label = if (destination.direct) destination.name else "#${destination.name}"
            val person = androidx.core.app.Person.Builder().setName(destination.name).setKey(target.shortcutId).build()
            val shortcut = ShortcutInfoCompat.Builder(context, target.shortcutId)
                .setShortLabel(label)
                .setLongLabel(if (destination.direct) label else "$label · ${destination.spaceName}")
                .setIcon(IconCompat.createWithBitmap(icon(context, avatars[destination.id].takeIf { destination.direct }, if (destination.direct) destination.name.take(1) else "#")))
                .setCategories(setOf(CATEGORY))
                .setLongLived(true)
                .apply { if (destination.direct) setPerson(person) }
                // Opening the shortcut from the launcher opens the conversation.
                .setIntent(Intent(context, MainActivity::class.java).setAction(Intent.ACTION_VIEW).apply {
                    if (target.direct) putExtra(CaperNotifications.EXTRA_CONVERSATION_ID, target.channelId)
                    else putExtra(CaperNotifications.EXTRA_SPACE_ID, target.spaceId).putExtra(CaperNotifications.EXTRA_CHANNEL_ID, target.channelId)
                })
                .build()
            runCatching { ShortcutManagerCompat.pushDynamicShortcut(context, shortcut) }
        }
    }

    /** Signing out removes every conversation's shortcut. */
    fun clear(context: Context) {
        val ids = ShortcutManagerCompat.getDynamicShortcuts(context).map { it.id }.filter { ShareTarget.fromShortcutId(it) != null }
        if (ids.isEmpty()) return
        ShortcutManagerCompat.removeDynamicShortcuts(context, ids)
        ShortcutManagerCompat.removeLongLivedShortcuts(context, ids)
    }

    /** A DM's bundled avatar, or the channel `#` / the person's initial on a raised disc. */
    private fun icon(context: Context, avatarId: Int?, glyph: String): Bitmap {
        val size = 192
        avatarId?.let { caperAvatarResources.getOrNull(it) }?.let { ContextCompat.getDrawable(context, it) }?.let { return it.toBitmap(size, size) }
        val bitmap = createBitmap(size, size)
        val canvas = Canvas(bitmap)
        canvas.drawCircle(size / 2f, size / 2f, size / 2f, Paint(Paint.ANTI_ALIAS_FLAG).apply { color = SurfaceRaised.toArgb() })
        val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = MessageText.toArgb(); textSize = size * 0.5f; textAlign = Paint.Align.CENTER }
        canvas.drawText(glyph.uppercase(), size / 2f, size / 2f - (text.descent() + text.ascent()) / 2, text)
        return bitmap
    }
}
