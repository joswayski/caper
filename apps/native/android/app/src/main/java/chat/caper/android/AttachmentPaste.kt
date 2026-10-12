package chat.caper.android

import android.content.ClipboardManager
import android.net.Uri
import androidx.compose.foundation.text.contextmenu.builder.item
import androidx.compose.foundation.text.contextmenu.modifier.appendTextContextMenuComponents
import androidx.compose.foundation.text.contextmenu.modifier.filterTextContextMenuComponents
import androidx.compose.foundation.text.contextmenu.data.TextContextMenuKeys
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext

private data object PasteFiles

/** Value-based text fields only paste text; keep their selection/mention behavior and add file paste. */
@Composable internal fun Modifier.attachmentPaste(enabled: Boolean, add: (List<Uri>) -> Unit): Modifier {
    val clipboard = LocalContext.current.getSystemService(ClipboardManager::class.java)
    fun files(): List<Uri> {
        val clip = clipboard.primaryClip ?: return emptyList()
        return (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
            .filter { it.scheme == "content" }.distinct()
    }
    fun paste(): Boolean {
        if (!enabled) return false
        val uris = files()
        if (uris.isEmpty()) return false
        add(uris)
        return true
    }
    return this
        .onPreviewKeyEvent { event ->
            event.type == KeyEventType.KeyDown &&
                ((event.key == Key.V && (event.isCtrlPressed || event.isMetaPressed)) ||
                    (event.key == Key.Insert && event.isShiftPressed)) && paste()
        }
        .appendTextContextMenuComponents {
            if (enabled && files().isNotEmpty()) {
                item(PasteFiles, "Paste") { paste(); close() }
            }
        }
        .filterTextContextMenuComponents {
            // A file provider may also advertise a text URI. Never paste its path instead of its bytes.
            it.key != TextContextMenuKeys.PasteKey || !enabled || files().isEmpty()
        }
}
