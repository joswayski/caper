package chat.caper.android

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.ChatAttachment
import chat.caper.android.ui.CaperTheme
import coil3.request.SuccessResult
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AttachmentUiTest {
    @get:Rule val compose = createComposeRule()

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
}
