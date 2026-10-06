package chat.caper.android

import android.content.ClipboardManager
import android.graphics.Bitmap
import android.view.KeyEvent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.*
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ReactionUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun longPressQuickReactionsClipboardAndRestrictions() {
        val joined = Channel("channel00001", "space0000001", "general", private = false, joined = true)
        val state = mutableStateOf(AppUiState(chatAuthorId = "self", selectedChannel = joined))
        val message = ChatMessage(
            "message00000001", "channel00001", "1", ChatAuthor("other", "Fixture Author", false, avatarId = 719),
            ChatContent(1, "text", "TEST FIXTURE — reaction chips, not a live conversation."),
            "2026-09-29T10:00:00Z", "fixture-client",
            listOf(MessageReaction("👍", listOf("self", "other")), MessageReaction("❤️", listOf("other")),
                MessageReaction("🎉", listOf("other")), MessageReaction("👀", listOf("other")), MessageReaction("🚀", listOf("other"))),
            "2",
        )
        var submitted: Triple<String, String, Boolean>? = null
        val target = mutableStateOf<ChatMessage?>(null)
        val picker = mutableStateOf(false)
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding)) {
                        Text("Reaction UI test fixture")
                        ReactionMessageRow(message, state.value,
                            { id, emoji, active -> submitted = Triple(id, emoji, active) }, { _, _ -> }, { _, _ -> },
                            { target.value = it })
                    }
                }
                target.value?.let {
                    MessageActionsSheet(it, state.value, { target.value = null },
                        { id, emoji, active -> submitted = Triple(id, emoji, active) },
                        { target.value = null; picker.value = true })
                }
                if (picker.value) EmojiPicker({ picker.value = false }) { emoji ->
                    picker.value = false
                    submitted = Triple(message.id, emoji, true)
                }
            }
        }
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsSelected().performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "👍", false), submitted) }
        compose.onNodeWithContentDescription("❤️ reaction, 1").assertIsNotSelected().performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "❤️", true), submitted) }
        capture("reaction-chips-test-fixture.png", compose.onRoot())
        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithText("Message actions").assertIsDisplayed()
        compose.onNodeWithContentDescription("👍 quick reaction").assertIsSelected().performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "👍", false), submitted) }

        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithContentDescription("Add reaction").performClick()
        capture("reaction-picker-test-fixture.png", compose.onNode(isDialog()))
        compose.onNode(hasSetTextAction()).performTextInput("definitely-no-such-emoji")
        compose.onNodeWithText("No emoji found.").assertIsDisplayed()
        capture("reaction-empty-test-fixture.png", compose.onNode(isDialog()))
        compose.onNode(hasSetTextAction()).performTextReplacement("rocket")
        compose.onNodeWithContentDescription("rockets").performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "🚀", true), submitted) }
        compose.onNodeWithText("Search emoji").assertDoesNotExist()
        compose.runOnIdle { state.value = state.value.copy(reactionSaves = mapOf("${message.id}:🚀" to ReactionSaveUi("🚀", true))) }
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsEnabled()
        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithContentDescription("👍 quick reaction").assertIsEnabled()
        compose.onNodeWithContentDescription("Add reaction").assertIsEnabled()
        compose.onNodeWithText("Copy text").assertIsEnabled()
        // The sheet holds window focus; Espresso.pressBack targets the unfocused activity root.
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        compose.onNodeWithText("Message actions").assertDoesNotExist()
        compose.runOnIdle {
            state.value = state.value.copy(reactionSaves = mapOf("${message.id}:🚀" to ReactionSaveUi("🚀", true, false, "Simulated save failure")))
        }
        compose.onNodeWithText("Simulated save failure").assertIsDisplayed()
        compose.onNodeWithText("Retry").assertIsEnabled()
        compose.onNodeWithText("Dismiss").assertIsEnabled()
        capture("reaction-error-test-fixture.png", compose.onRoot())

        // Public previews retain readable reaction ownership, but only expose copy actions.
        compose.runOnIdle { state.value = state.value.copy(selectedChannel = joined.copy(joined = false)) }
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsSelected().assertIsNotEnabled()
        compose.onNodeWithText("2").assertIsDisplayed()
        compose.onNodeWithText("Retry").assertIsNotEnabled()
        compose.onNodeWithText("Dismiss").assertIsEnabled()

        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithContentDescription("Add reaction").assertDoesNotExist()
        compose.onNodeWithContentDescription("👍 quick reaction").assertDoesNotExist()
        compose.onNodeWithText("Copy text").performClick()
        assertClipboard(message.content.text)
        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithText("Copy message ID").performClick()
        assertClipboard(message.id)

        compose.onNodeWithText(message.content.text).performTouchInput { longClick() }
        compose.onNodeWithText("Message actions").assertIsDisplayed()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.onNodeWithText("Message actions").assertDoesNotExist()
    }

    private fun assertClipboard(expected: String) {
        compose.runOnIdle {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val clipboard = context.getSystemService(ClipboardManager::class.java)
            assertEquals(expected, clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        }
    }

    private fun capture(name: String, node: SemanticsNodeInteraction) {
        val directory = File(requireNotNull(InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")))
        check(directory.mkdirs() || directory.isDirectory)
        val screenshot = node.captureToImage().asAndroidBitmap()
        try {
            File(directory, name).outputStream().use { check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally { screenshot.recycle() }
    }
}
