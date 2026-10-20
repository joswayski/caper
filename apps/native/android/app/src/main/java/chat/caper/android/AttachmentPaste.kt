package chat.caper.android

import android.content.ClipboardManager
import android.net.Uri
import android.os.Bundle
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper
import android.view.inputmethod.InputContentInfo
import androidx.compose.foundation.text.contextmenu.builder.item
import androidx.compose.foundation.text.contextmenu.modifier.appendTextContextMenuComponents
import androidx.compose.foundation.text.contextmenu.modifier.filterTextContextMenuComponents
import androidx.compose.foundation.text.contextmenu.data.TextContextMenuKeys
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.ExperimentalComposeUiApi
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
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import kotlinx.coroutines.Job

private data object PasteFiles

/** Gboard's GIF/sticker/clipboard buttons use commitContent, not the Paste menu. */
@OptIn(ExperimentalComposeUiApi::class)
@Composable internal fun KeyboardAttachmentInput(
    add: ((Uri) -> Job?)?, content: @Composable () -> Unit,
) {
    val currentAdd by rememberUpdatedState(add)
    InterceptPlatformTextInput(
        interceptor = remember(add != null) {
            { request, next ->
                next.startInputMethod(PlatformTextInputMethodRequest { info ->
                    val connection = request.createInputConnection(info)
                    if (currentAdd == null) connection else {
                        // No image-only filter: keyboards may supply any document MIME type.
                        info.contentMimeTypes = arrayOf("*/*")
                        object : InputConnectionWrapper(connection, false) {
                            override fun commitContent(input: InputContentInfo, flags: Int, opts: Bundle?): Boolean {
                                val receive = currentAdd ?: return false
                                if (input.contentUri.scheme != "content") return false
                                val grant = flags and InputConnection.INPUT_CONTENT_GRANT_READ_URI_PERMISSION != 0
                                try {
                                    if (grant) input.requestPermission()
                                    val job = receive(input.contentUri)
                                    if (job == null) {
                                        if (grant) input.releasePermission()
                                        return false
                                    }
                                    // Keep both the grant and InputContentInfo alive across async copying,
                                    // navigation and keyboard dismissal. Completion also covers cancellation.
                                    job.invokeOnCompletion { if (grant) runCatching { input.releasePermission() } }
                                    return true
                                } catch (_: Exception) {
                                    if (grant) runCatching { input.releasePermission() }
                                    return false
                                }
                            }
                        }
                    }
                })
            }
        }, content = content,
    )
}

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
