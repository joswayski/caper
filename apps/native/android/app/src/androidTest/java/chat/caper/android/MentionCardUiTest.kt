package chat.caper.android

import android.view.KeyEvent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.data.CaperApi
import chat.caper.android.model.*
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MentionCardUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun personPillsOpenACardWithMessageOrYou() {
        val me = Account("owner000000a", "fixture_owner", "Fixture Owner")
        val channel = Channel("channel00001", "space0000001", "general", private = false)
        val space = SpaceDetail(Space("space0000001", "Space"), listOf(channel), listOf(
            Member("owner000000a", "fixture_owner", "Fixture Owner", true), Member("maya0000000a", "maya", "Maya Fixture", false),
        ))
        val state = AppUiState(account = me, chatAuthorId = me.id, selectedSpace = space, selectedChannel = channel)
        val message = ChatMessage(
            "message00001", channel.id, "1", ChatAuthor("alex0000000a", "Alex", false),
            ChatContent(1, "text", "TEST FIXTURE — ask @maya or @fixture_owner, @everyone", mentions = listOf(
                MessageMention("user", "maya0000000a", "maya"), MessageMention("user", "owner000000a", "fixture_owner"), MessageMention("everyone"),
            )),
            "2026-10-06T12:00:00Z", "00000000-0000-4000-8000-000000000001",
        )
        val target = mutableStateOf<MessageMention?>(null)
        var actions = 0
        var requested: String? = null
        var fail: ((String) -> Unit)? = null
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding)) {
                        Text("Mention card UI test fixture")
                        ReactionMessageRow(message, state, { _, _, _ -> }, { _, _ -> }, { _, _ -> },
                            openReactors = { _, _ -> }, openMention = { target.value = it }) { actions++ }
                    }
                }
                target.value?.let { mention ->
                    val card = mentionCard(mention.id, mention.username.orEmpty(), state)
                    key(mention) {
                        MentionCardSheet(card, { _, failed -> requested = card.username; fail = failed }) { target.value = null }
                    }
                }
            }
        }

        // TalkBack reaches each person pill through a labelled custom action; @everyone has none.
        val text = compose.onNodeWithText("TEST FIXTURE", substring = true).fetchSemanticsNode()
        val labels = text.config[SemanticsActions.CustomActions].map { it.label }
        assertEquals(listOf("Open profile for Maya Fixture", "Open profile for Fixture Owner"), labels)

        compose.runOnIdle { text.config[SemanticsActions.CustomActions].first().action() }
        compose.onNodeWithText("Maya Fixture").assertIsDisplayed()
        compose.onNodeWithText("@maya").assertIsDisplayed()
        compose.onNodeWithContentDescription("Open conversation").assertIsEnabled().performClick()
        compose.onNodeWithContentDescription("Opening…").assertIsNotEnabled()
        compose.runOnIdle { assertEquals("maya", requested); fail?.invoke("That account was not found.") }
        compose.onNodeWithText("That account was not found.").assertIsDisplayed()
        compose.onNodeWithContentDescription("Open conversation").assertIsEnabled()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        compose.onNodeWithText("Maya Fixture").assertDoesNotExist()

        compose.runOnIdle { text.config[SemanticsActions.CustomActions].last().action() }
        compose.onNodeWithText("You").assertIsDisplayed()
        compose.onNodeWithContentDescription("Open conversation").assertDoesNotExist()
        compose.runOnIdle { assertEquals(0, actions) }
    }

    @Test fun profileComposerKeepsTextAcrossEmojiAndReopenAndOnlyExplicitSendSends() {
        val drafts = ProfileMessageDrafts(CaperApi())
        val visible = mutableStateOf(true)
        var sends = 0
        var picking = false
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding)) {
                        val state = drafts.state.collectAsState().value
                        if (visible.value) ProfileMessageComposer(
                            "Maya", state["maya"] ?: ProfileMessageDraft(), writable = true, uploadsEnabled = false,
                            text = { drafts.text("maya", it) }, add = {}, paste = {}, remove = {}, send = { sends++ },
                            pickerInteraction = { picking = it },
                        )
                    }
                }
            }
        }
        compose.onNodeWithContentDescription("Send to @Maya").assertIsNotEnabled()
        compose.onNodeWithText("Message @Maya").performTextInput("Keep this draft")
        compose.onNodeWithContentDescription("Emojis, GIFs and stickers").performClick()
        compose.onNodeWithText("Insert emoji").assertIsDisplayed()
        compose.runOnIdle { assertTrue(picking) }
        compose.onNodeWithText("Search emoji").performTextInput("grinning-face")
        compose.onNodeWithContentDescription("grinning-face").performClick()
        compose.onNodeWithText("Keep this draft😀").assertIsDisplayed()
        compose.runOnIdle { assertEquals(false, picking); assertEquals(0, sends); visible.value = false }
        compose.waitForIdle()
        compose.runOnIdle { visible.value = true }
        compose.onNodeWithText("Keep this draft😀").assertIsDisplayed()
        compose.onNodeWithContentDescription("Attach files").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Send to @Maya").performClick()
        compose.runOnIdle { assertEquals(1, sends) }
    }

    @Test fun profileSheetDismissesWithDraftsButProtectsPickersAndSending() {
        val visible = mutableStateOf(true)
        val draft = mutableStateOf(ProfileMessageDraft(text = "Keep this draft"))
        val picking = mutableStateOf(false)
        var closed = 0
        compose.setContent {
            CaperTheme {
                if (visible.value) ProfileBottomSheet("Protected profile", draft.value, picking.value, {
                    closed++; visible.value = false
                }) { Text("Profile sheet test") }
            }
        }
        fun back() {
            // Reopening recomposes on the next frame. Back sent before the sheet's
            // window exists reaches the activity instead and finishes it.
            compose.waitForIdle()
            compose.waitUntil(5_000) { compose.onAllNodesWithText("Profile sheet test").fetchSemanticsNodes().isNotEmpty() }
            InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
            compose.waitForIdle()
        }
        back()
        compose.onNodeWithContentDescription("Close profile").assertDoesNotExist()
        compose.runOnIdle {
            assertEquals(1, closed)
            assertEquals("Keep this draft", draft.value.text)
            visible.value = true
            draft.value = ProfileMessageDraft(files = listOf(DraftAttachmentUi("file", "note.txt", "file")))
        }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Close sheet").performClick()
        compose.onNodeWithContentDescription("Close profile").assertDoesNotExist()
        compose.runOnIdle {
            assertEquals(2, closed)
            assertEquals("note.txt", draft.value.files.single().name)
            visible.value = true
            picking.value = true
        }
        back()
        compose.onNodeWithContentDescription("Close profile").assertIsDisplayed()
        compose.runOnIdle { picking.value = false; draft.value = ProfileMessageDraft(sending = true) }
        back()
        compose.onNodeWithContentDescription("Close profile").assertIsNotEnabled()
        compose.runOnIdle { assertEquals(2, closed); draft.value = ProfileMessageDraft(text = "Explicit close keeps this") }
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.runOnIdle {
            assertEquals(3, closed)
            assertEquals("Explicit close keeps this", draft.value.text)
            visible.value = true
            draft.value = ProfileMessageDraft()
        }
        back()
        compose.onNodeWithContentDescription("Close profile").assertDoesNotExist()
        compose.runOnIdle { assertEquals(4, closed) }
    }
}
