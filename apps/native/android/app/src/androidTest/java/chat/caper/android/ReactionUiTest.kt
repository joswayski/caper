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
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.*
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CompletableDeferred
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
                            openReactors = { _, _ -> }) { target.value = it }
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
        compose.onNodeWithContentDescription("👍 reaction, 2").assertIsSelected().assertHasNoClickAction()
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

    @Test fun pressAndHoldShowsWhoReactedWithoutToggling() {
        val joined = Channel("channel00001", "space0000001", "general", private = false, joined = true)
        val state = mutableStateOf(AppUiState(chatAuthorId = "self", selectedChannel = joined))
        val message = mutableStateOf(ChatMessage(
            "message00000001", "channel00001", "1", ChatAuthor("other", "Fixture Author", false, avatarId = 719),
            ChatContent(1, "text", "TEST FIXTURE — who reacted, not a live conversation."),
            "2026-09-29T10:00:00Z", "fixture-client",
            listOf(MessageReaction("👍", listOf("self", "other")), MessageReaction("❤️", listOf("other"))),
            "2",
        ))
        val self = Reactor("self", "fixture_self", "Fixture Self", 12)
        val reactor = Reactor("other", "fixture_reactor", "Fixture Reactor", 719)
        val names = ReactorList(message.value.id, "2", listOf(ReactorGroup("👍", listOf(self, reactor)), ReactorGroup("❤️", listOf(reactor))))
        val gate = CompletableDeferred<Unit>()
        var failNext = false
        var loads = 0
        var submitted: Triple<String, String, Boolean>? = null
        val target = mutableStateOf<String?>(null)
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding)) {
                        Text("Who reacted UI test fixture")
                        ReactionMessageRow(message.value, state.value,
                            { id, emoji, active -> submitted = Triple(id, emoji, active) }, { _, _ -> }, { _, _ -> },
                            openReactors = { _, emoji -> target.value = emoji }) {}
                    }
                }
                target.value?.let { emoji ->
                    ReactorsSheet(message.value, emoji, "self", { shown ->
                        loads++
                        gate.await()
                        if (failNext) { failNext = false; throw IOException("Simulated reactor load failure") }
                        names.copy(reactionSeq = shown.reactionSeq ?: "0")
                    }) { target.value = null }
                }
            }
        }

        // Press and hold opens the sheet on the pressed emoji and never toggles it.
        compose.onNodeWithContentDescription("❤️ reaction, 1").performTouchInput { longClick() }
        compose.onNodeWithText("Reactions").assertIsDisplayed()
        compose.runOnIdle { assertEquals(null, submitted) }
        compose.onNodeWithContentDescription("❤️ 1").assertIsSelected()
        compose.onNodeWithContentDescription("👍 2").assertIsNotSelected()
        compose.onNodeWithText("Loading…").assertIsDisplayed()
        compose.onNodeWithContentDescription("1 person reacted with :red-heart:").assertIsDisplayed()
        compose.runOnIdle { gate.complete(Unit) }
        compose.onNodeWithText("Fixture Reactor").assertIsDisplayed()
        compose.onNodeWithText("@fixture_reactor").assertIsDisplayed()
        compose.onNodeWithText(":red-heart:").assertIsDisplayed()
        compose.onNodeWithContentDescription("Fixture Reactor reacted with :red-heart:").assertIsDisplayed()

        compose.onNodeWithContentDescription("👍 2").performClick().assertIsSelected()
        compose.onNodeWithText(":thumbs-up:").assertIsDisplayed()
        compose.onNodeWithText("Fixture Self").assertIsDisplayed()
        compose.onNodeWithContentDescription("You and Fixture Reactor reacted with :thumbs-up:").assertIsDisplayed()
        captureScreen("reaction-reactors-sheet-test-fixture.png")

        // A changed snapshot refetches; a removed selected emoji falls back to the first one.
        compose.runOnIdle {
            failNext = true
            message.value = message.value.copy(reactions = listOf(MessageReaction("❤️", listOf("other"))), reactionSeq = "3")
        }
        compose.onNodeWithContentDescription("❤️ 1").assertIsSelected()
        compose.onNodeWithText("Couldn’t load reactions").assertIsDisplayed()
        compose.onNodeWithText("Retry").performClick()
        compose.onNodeWithText("Fixture Reactor").assertIsDisplayed()
        compose.runOnIdle { assertEquals(3, loads) }
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        compose.onNodeWithText("Reactions").assertDoesNotExist()

        // TalkBack's custom action opens the same sheet; removing every reaction closes it.
        val chip = compose.onNodeWithContentDescription("❤️ reaction, 1").fetchSemanticsNode()
        compose.runOnIdle { chip.config[SemanticsActions.CustomActions].single { it.label == "Show who reacted" }.action() }
        compose.onNodeWithText("Reactions").assertIsDisplayed()
        compose.runOnIdle { message.value = message.value.copy(reactions = emptyList(), reactionSeq = "4") }
        compose.waitForIdle()
        compose.onNodeWithText("Reactions").assertDoesNotExist()

        // Read-only previews cannot toggle, but can still see who reacted.
        compose.runOnIdle {
            message.value = message.value.copy(reactions = listOf(MessageReaction("👍", listOf("other"))), reactionSeq = "5")
            state.value = state.value.copy(selectedChannel = joined.copy(joined = false))
        }
        compose.onNodeWithContentDescription("👍 reaction, 1").assertHasNoClickAction().performTouchInput { longClick() }
        compose.onNodeWithText("Reactions").assertIsDisplayed()
        compose.onNodeWithText("Fixture Reactor").assertIsDisplayed()
        compose.runOnIdle { assertEquals(null, submitted) }
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        compose.onNodeWithText("Reactions").assertDoesNotExist()
    }

    private fun assertClipboard(expected: String) {
        compose.runOnIdle {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val clipboard = context.getSystemService(ClipboardManager::class.java)
            assertEquals(expected, clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        }
    }

    private fun capture(name: String, node: SemanticsNodeInteraction) = save(name, node.captureToImage().asAndroidBitmap())

    /** The whole screen, including the bottom sheet's own window. */
    private fun captureScreen(name: String) {
        compose.waitForIdle()
        save(name, checkNotNull(InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()) { "Screenshot failed." })
    }

    private fun save(name: String, screenshot: Bitmap) {
        val directory = File(requireNotNull(InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")))
        check(directory.mkdirs() || directory.isDirectory)
        try {
            File(directory, name).outputStream().use { check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally { screenshot.recycle() }
    }
}
