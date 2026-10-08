package chat.caper.android

import android.view.KeyEvent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.*
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
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
            ChatContent(1, "text", "TEST FIXTURE — ask @maya or @fixture_owner, @everyone", listOf(
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
        compose.onNodeWithText("Message").assertIsEnabled().performClick()
        compose.onNodeWithText("Opening…").assertIsNotEnabled()
        compose.runOnIdle { assertEquals("maya", requested); fail?.invoke("That account was not found.") }
        compose.onNodeWithText("That account was not found.").assertIsDisplayed()
        compose.onNodeWithText("Message").assertIsEnabled()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        compose.waitForIdle()
        compose.onNodeWithText("Maya Fixture").assertDoesNotExist()

        compose.runOnIdle { text.config[SemanticsActions.CustomActions].last().action() }
        compose.onNodeWithText("You").assertIsDisplayed()
        compose.onNodeWithText("Message").assertDoesNotExist()
        compose.runOnIdle { assertEquals(0, actions) }
    }
}
