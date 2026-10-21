package chat.caper.android

import android.content.ClipDescription
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputContentInfo
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.ChatAttachment
import chat.caper.android.ui.CaperTheme
import chat.caper.android.ui.EmojiTextField
import coil3.request.SuccessResult
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AttachmentUiTest {
    @get:Rule val compose = createComposeRule()

    @OptIn(ExperimentalComposeUiApi::class)
    @Test fun keyboardAcceptsGifAndDocumentsWithoutReplacingText() {
        val draft = mutableStateOf(TextFieldValue("Keep suffix", TextRange(5)))
        val available = mutableStateOf(true)
        val received = mutableListOf<Uri>()
        val jobs = mutableListOf<Job>()
        var request: PlatformTextInputMethodRequest? = null
        compose.setContent {
            // Capture the real composer's request at the system boundary, after its interceptor.
            InterceptPlatformTextInput(interceptor = { input, _ -> request = input; awaitCancellation() }) {
                CaperTheme {
                    EmojiTextField(draft.value, { draft.value = it }, Modifier.testTag("composer"),
                        keyboardAttachment = if (available.value) { uri ->
                            received.add(uri)
                            Job().also { jobs.add(it) }
                        } else null)
                }
            }
        }
        compose.onNodeWithTag("composer").performClick()
        compose.waitUntil(5_000) { request != null }
        compose.runOnIdle { draft.value = draft.value.copy(selection = TextRange(5)) }
        try {
            compose.runOnIdle {
                val info = EditorInfo()
                val connection = requireNotNull(request).createInputConnection(info)
                assertTrue(ClipDescription("file", arrayOf("application/pdf")).hasMimeType(requireNotNull(info.contentMimeTypes).single()))
                val types = listOf("image/gif", "application/pdf", "application/zip", "application/x-custom-file")
                val expected = types.indices.map { Uri.parse("content://test.keyboard/file/$it") }
                types.zip(expected).forEach { (type, uri) ->
                    assertTrue(connection.commitContent(InputContentInfo(uri, ClipDescription("file", arrayOf(type)), null), 0, null))
                }
                assertEquals(expected, received)
                assertEquals("Keep suffix", draft.value.text)
                connection.commitText("text ", 1)
            }
            compose.runOnIdle {
                assertEquals("Keep text suffix", draft.value.text)
                available.value = false
            }
            compose.runOnIdle {
                val info = EditorInfo()
                val connection = requireNotNull(request).createInputConnection(info)
                assertTrue(info.contentMimeTypes.isNullOrEmpty())
                assertFalse(connection.commitContent(InputContentInfo(Uri.parse("content://test.keyboard/blocked"),
                    ClipDescription("blocked", arrayOf("image/gif")), null), 0, null))
                assertEquals(4, received.size)
            }
        } finally {
            jobs.forEach { it.cancel() }
        }
    }

    @Test fun viewerHideClosesAndConcealsTheOriginatingAttachment() {
        val file = ChatAttachment("viewer-spoiler", "image", "image/png", "secret.png", 10,
            spoiler = true, width = 240, height = 180, url = "https://cdn.invalid/secret")
        compose.setContent {
            CaperTheme { MessageAttachments(listOf(file), pending = false, onLoadFailed = { _, _ -> }) }
        }
        compose.onNodeWithText("Tap to reveal").performClick()
        compose.onNodeWithContentDescription("secret.png").performClick()
        compose.onNode(hasText("Hide spoiler") and hasAnyAncestor(isDialog())).performClick()
        compose.onNodeWithContentDescription("Close viewer").assertDoesNotExist()
        compose.onAllNodesWithText("Tap to reveal").assertCountEquals(1)
        compose.onNodeWithContentDescription("secret.png").assertDoesNotExist()
    }

    @Test fun hidingOneSpoilerPreservesOtherRevealsAndConcealsItsFilename() {
        val files = listOf("first", "second").map { id ->
            ChatAttachment(id, "file", "text/plain", "$id-secret.txt", 10, spoiler = true,
                url = "https://cdn.invalid/$id")
        }
        compose.setContent {
            CaperTheme { MessageAttachments(files, pending = false, onLoadFailed = { _, _ -> }) }
        }
        compose.onNodeWithText("first-secret.txt").assertDoesNotExist()
        compose.onAllNodesWithText("Tap to reveal").onFirst().performClick()
        compose.onAllNodesWithText("Tap to reveal").onFirst().performClick()
        compose.onAllNodesWithText("Hide spoiler").assertCountEquals(2)
        compose.onAllNodesWithText("Hide spoiler").onFirst().performClick()
        compose.onNodeWithText("first-secret.txt").assertDoesNotExist()
        compose.onAllNodesWithText("Hide spoiler").assertCountEquals(1)
        compose.onAllNodesWithText("Tap to reveal").assertCountEquals(1)
    }

    @Test fun confirmationRetainsPixelsUntilTheServerPreviewIsReady() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val id = UUID.randomUUID().toString()
        val local = File(context.cacheDir, "$id.png")
        fun writeImage(color: Int) {
            val bitmap = Bitmap.createBitmap(960, 720, Bitmap.Config.ARGB_8888)
            bitmap.eraseColor(color)
            local.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
        writeImage(Color.RED)
        val attachment = mutableStateOf(ChatAttachment(id, "image", "image/png", "photo.png", 1,
            width = 240, height = 180, url = local.toURI().toString()))
        val row = mutableStateOf("pending")
        compose.setContent {
            CaperTheme {
                Box(Modifier.width(240.dp).testTag("attachment")) {
                    // Acknowledgement replaces the pending row's composition in the real timeline.
                    key(row.value) {
                        MessageAttachments(listOf(attachment.value), pending = row.value == "pending", onLoadFailed = { _, _ -> })
                    }
                }
            }
        }
        fun pixel(): Int {
            val image = compose.onNodeWithTag("attachment").captureToImage().asAndroidBitmap()
            return image.getPixel(image.width / 8, image.height / 4)
        }
        try {
            compose.waitUntil(5_000) { pixel() == Color.RED }
            assertTrue(local.delete())
            compose.runOnIdle {
                attachment.value = attachment.value.copy(status = "processing", url = null)
                row.value = "confirmed"
            }
            assertEquals("processing without a URL must retain the local preview", Color.RED, pixel())

            // A different server result is already decoded, but the newly mounted row has not seen it.
            // Using blue ensures a stale local cache hit cannot accidentally satisfy this assertion.
            writeImage(Color.BLUE)
            val loaded = runBlocking {
                AttachmentImages.loader(context).execute(AttachmentImages.request(context, local, "$id:display"))
            }
            assertTrue(loaded is SuccessResult)
            assertTrue(local.delete())
            compose.runOnIdle {
                attachment.value = attachment.value.copy(status = "ready", url = "https://cdn.invalid/$id")
            }
            compose.waitUntil(5_000) { pixel() == Color.BLUE }
            compose.runOnIdle { attachment.value = attachment.value.copy(unavailable = true) }
            assertNotEquals("removed files never show cached pixels", Color.BLUE, pixel())
            compose.runOnIdle { attachment.value = attachment.value.copy(unavailable = false, status = "failed") }
            assertNotEquals("processing failures still show the failure card", Color.BLUE, pixel())
        } finally {
            local.delete()
        }
    }

    @Test fun draftTilesUseCornerButtonsAndThePencilMenuForFilesWithoutAPreview() {
        val removed = mutableListOf<String>()
        val files = mutableStateOf(listOf(chat.caper.android.model.DraftAttachmentUi(
            "pdf", "notes.pdf", "file", compressing = false, attachment = null, storedSize = 10,
        )))
        val enabled = mutableStateOf(true)
        compose.setContent {
            CaperTheme {
                DraftAttachmentStrip(files.value, { removed += it }, enabled = enabled.value, uploadOnSend = true,
                    toggleSpoiler = { key -> files.value = files.value.map { if (it.key == key) it.copy(spoiler = !it.spoiler) else it } })
            }
        }
        compose.onNodeWithContentDescription("Edit notes.pdf").performClick()
        compose.onNodeWithText("Mark as spoiler").performClick()
        // A spoiler shows a crossed-out eye, not a text badge.
        compose.onNodeWithText("Spoiler").assertDoesNotExist()
        compose.onNodeWithContentDescription("Edit spoiler attachment").performClick()
        compose.onNodeWithText("Remove spoiler").assertExists()
        compose.onNodeWithText("Remove attachment").performClick()
        compose.runOnIdle { assertEquals(listOf("pdf"), removed) }
        compose.runOnIdle { enabled.value = false }
        compose.onNodeWithContentDescription("Remove spoiler attachment").assertDoesNotExist()
        compose.onNodeWithContentDescription("Edit spoiler attachment").assertDoesNotExist()
    }
}
