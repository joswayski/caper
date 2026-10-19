package chat.caper.android

import android.content.ContentValues
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.data.LibraryPermissions
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.ui.CaperTheme
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The attach drawer over a seeded library: labelled TEST FIXTURE images, removed afterwards. */
@RunWith(AndroidJUnit4::class)
class AttachDrawerUiTest {
    @get:Rule val compose = createComposeRule()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun seed(index: Int): Uri {
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "caper-test-fixture-$index.png")
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            if (Build.VERSION.SDK_INT >= 29) put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/CaperTestFixture")
        }
        val uri = requireNotNull(context.contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values))
        val bitmap = Bitmap.createBitmap(320, 320, Bitmap.Config.ARGB_8888)
        Canvas(bitmap).apply {
            drawColor(listOf(Color.rgb(99, 122, 67), Color.rgb(182, 77, 50), Color.rgb(52, 56, 59))[index % 3])
            drawText("TEST FIXTURE $index", 40f, 170f, Paint().apply { color = Color.WHITE; textSize = 32f })
        }
        context.contentResolver.openOutputStream(uri)!!.use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        bitmap.recycle()
        return uri
    }

    private fun save(name: String) {
        val directory = File(requireNotNull(InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")))
        check(directory.mkdirs() || directory.isDirectory)
        // The sheet is its own window, which Compose's node capture draws blank:
        // take the whole screen as the system shows it.
        compose.waitForIdle()
        val screenshot = requireNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot())
        try {
            File(directory, "$name.png").outputStream().use { check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally { screenshot.recycle() }
    }

    @Test fun tapsAttachInOrderAndAFullDrawerRefusesMore() {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        LibraryPermissions.requested(Build.VERSION.SDK_INT).forEach { automation.grantRuntimePermission(context.packageName, it) }
        val seeded = (0 until 3).map(::seed)
        try {
            val files = mutableStateOf(emptyList<DraftAttachmentUi>())
            compose.setContent {
                CaperTheme {
                    AttachDrawer(
                        AttachDrawerHost(files.value, remove = { key -> files.value = files.value.filter { it.key != key } }, canSend = true, send = {}),
                        add = { uris -> files.value += uris.map { DraftAttachmentUi(it.toString(), "photo.png", "image", pickedUri = it.toString()) } },
                        gallery = {}, files = {}, camera = {}, onDismiss = {},
                    )
                }
            }
            compose.waitUntil(15_000) { compose.onAllNodesWithTag("attach-drawer-tile", useUnmergedTree = true).fetchSemanticsNodes().size >= 3 }
            compose.onNodeWithText("Up to 10 files").assertExists()
            save("attach-drawer-open-test-fixture")
            val tiles = compose.onAllNodesWithTag("attach-drawer-tile", useUnmergedTree = true)
            tiles[1].performClick()
            tiles[0].performClick()
            compose.onNodeWithText("2 of 10 attached").assertExists()
            // Draft order follows taps, not the grid.
            tiles[1].assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Attached 1"))
            tiles[0].assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Attached 2"))
            save("attach-drawer-selected-test-fixture")
            tiles[1].performClick()
            compose.onNodeWithText("1 of 10 attached").assertExists()

            compose.runOnIdle {
                files.value = (0 until 10).map { DraftAttachmentUi("full-$it", "file.txt", "file") }
            }
            compose.onNodeWithText("10 of 10 attached").assertExists()
            compose.onNodeWithText("Remove a file to add another.").assertExists()
            tiles[2].performClick()
            compose.runOnIdle { assertEquals(10, files.value.size) }
            save("attach-drawer-full-test-fixture")
        } finally {
            seeded.forEach { context.contentResolver.delete(it, null, null) }
        }
    }
}
