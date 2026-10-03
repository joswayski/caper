package chat.caper.android

import android.graphics.Bitmap
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

    @Test fun ownershipPickerSearchAndSavingStates() {
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
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding)) {
                        Text("Reaction UI test fixture")
                        ReactionMessageRow(message, state.value,
                            { id, emoji, active -> submitted = Triple(id, emoji, active) }, { _, _ -> }, { _, _ -> })
                    }
                }
            }
        }
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsSelected().performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "👍", false), submitted) }
        compose.onNodeWithContentDescription("❤️ reaction, 1").assertIsNotSelected().performClick()
        compose.runOnIdle { assertEquals(Triple(message.id, "❤️", true), submitted) }
        capture("reaction-chips-test-fixture.png", compose.onRoot())
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
        compose.onNodeWithContentDescription("Add reaction").assertIsNotEnabled()
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsNotEnabled()
        compose.runOnIdle {
            state.value = state.value.copy(reactionSaves = mapOf("${message.id}:🚀" to ReactionSaveUi("🚀", true, false, "Simulated save failure")))
        }
        compose.onNodeWithText("Simulated save failure").assertIsDisplayed()
        compose.onNodeWithText("Retry").assertIsEnabled()
        compose.onNodeWithText("Dismiss").assertIsEnabled()
        capture("reaction-error-test-fixture.png", compose.onRoot())

        // Public previews retain chat identity so incoming reaction ownership and
        // counts remain readable, but no reaction mutation may be initiated.
        compose.onNodeWithContentDescription("Add reaction").performClick()
        compose.onNode(isDialog()).assertExists()
        compose.runOnIdle { state.value = state.value.copy(selectedChannel = joined.copy(joined = false)) }
        compose.onNode(isDialog()).assertDoesNotExist()
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsSelected().assertIsNotEnabled()
        compose.onNodeWithText("2").assertIsDisplayed()
        compose.onNodeWithContentDescription("Add reaction").assertIsNotEnabled()
        compose.onNodeWithText("Retry").assertIsNotEnabled()
        compose.onNodeWithText("Dismiss").assertIsEnabled()
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
