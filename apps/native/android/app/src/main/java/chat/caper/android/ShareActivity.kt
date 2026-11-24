package chat.caper.android

import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.core.content.pm.ShortcutManagerCompat
import chat.caper.android.data.ShareTarget
import chat.caper.android.data.Sharing

/**
 * What another app shared: files (photos, videos, any file), text and links, and the
 * conversation when it came from a Sharing Shortcut.
 */
internal data class IncomingShare(val uris: List<Uri>, val text: String, val target: ShareTarget?) {
    companion object {
        /** Caper's own hand-off from [ShareActivity] to [MainActivity]. */
        const val ACTION = "chat.caper.android.action.SHARE"

        /**
         * Reads a SEND / SEND_MULTIPLE intent (or [ACTION]). Only `content:` streams from other
         * apps are kept ([Sharing.acceptsStream]); null when nothing usable was shared.
         */
        fun from(intent: Intent, ownPackage: String): IncomingShare? {
            if (intent.action !in setOf(Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE, ACTION)) return null
            val streams = buildList {
                if (intent.action == Intent.ACTION_SEND) stream(intent)?.let(::add)
                else addAll(streams(intent))
                // Some apps put the files only in ClipData.
                if (isEmpty()) intent.clipData?.let { clip -> (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }.forEach(::add) }
            }
            val uris = streams.filter { Sharing.acceptsStream(it.scheme, it.authority, ownPackage) }.distinct()
            val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.trim().orEmpty()
            if (uris.isEmpty() && text.isEmpty()) return null
            return IncomingShare(uris, text, ShareTarget.fromShortcutId(intent.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID)))
        }

        @Suppress("DEPRECATION")
        private fun stream(intent: Intent): Uri? =
            if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            else intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri

        @Suppress("DEPRECATION")
        private fun streams(intent: Intent): List<Uri> =
            (if (Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
            else intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)).orEmpty().filterNotNull()
    }

    /** Carries this share to [MainActivity], passing on read access to every file. */
    fun handOff(activity: Activity): Intent = Intent(activity, MainActivity::class.java).apply {
        action = ACTION
        putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris))
        putExtra(Intent.EXTRA_TEXT, text)
        target?.let { putExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID, it.shortcutId) }
        if (uris.isNotEmpty()) {
            clipData = ClipData.newRawUri(null, uris.first()).also { clip -> uris.drop(1).forEach { clip.addItem(ClipData.Item(it)) } }
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    }
}

/**
 * The share sheet's entry: opens Caper in its own task (not inside the sharing app), where
 * [MainActivity] asks which conversation it goes to, then fills that composer.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) IncomingShare.from(intent, packageName)?.let { startActivity(it.handOff(this)) }
        finish()
    }
}
