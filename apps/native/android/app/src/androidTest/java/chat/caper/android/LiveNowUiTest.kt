package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.Participant
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import chat.caper.android.ui.CaperTheme
import chat.caper.android.voice.VoiceState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The sidebar's Live now: your call first, then live rooms busiest first, folding past two. */
@RunWith(AndroidJUnit4::class)
class LiveNowUiTest {
    @get:Rule val compose = createComposeRule()

    private fun people(prefix: String, count: Int) = (1..count).map { Participant("$prefix$it", "$prefix $it", muted = false, deafened = false, tracks = emptyList()) }
    private fun channel(id: String) = Channel(id, "space", id, private = false)

    private val detail = SpaceDetail(Space("space", "Fixture Studio"), listOf("general", "design", "planning", "random", "lobby").map(::channel), members = emptyList())

    @Test fun liveRoomsFoldPastTwoAndJoinWithoutOpeningTheChat() {
        val state = AppUiState(
            selectedSpace = detail,
            voiceRosters = mapOf("design" to people("Maya", 4), "planning" to people("Alex", 2), "random" to people("Sam", 1), "lobby" to people("Jo", 1)),
            voiceAvailability = detail.channels.associate { it.id to true },
        )
        val voice = mutableStateOf(VoiceState())
        val joined = mutableListOf<String>()
        val opened = mutableListOf<String>()
        compose.setContent {
            CaperTheme {
                // Scrolls like the sidebar, so every room can be reached on a small screen.
                Column(Modifier.width(280.dp).verticalScroll(rememberScrollState())) {
                    LiveNowSection(state, voice.value, null, narrow = true, joinVoice = { joined += it.id }, prepareVoice = {},
                        openChannel = { opened += it.id }, openCall = {}, leaveVoice = {})
                }
            }
        }

        compose.onNodeWithText("Live now").assertExists()
        compose.onNodeWithText("4 rooms").assertExists()
        // Busiest first: #design (4), #planning (2); the rest fold, by name.
        compose.onNodeWithText("Maya 1, Maya 2 and 2 more").assertExists()
        compose.onNodeWithText("Alex 1 and Alex 2").assertExists()
        compose.onNodeWithText("2 more live").assertExists()
        compose.onNodeWithText("#lobby, #random").assertExists()
        compose.onNodeWithContentDescription("Join voice in #lobby").assertDoesNotExist()
        val design = compose.onNodeWithContentDescription("Join voice in #design").getUnclippedBoundsInRoot()
        val planning = compose.onNodeWithContentDescription("Join voice in #planning").getUnclippedBoundsInRoot()
        assertTrue(design.bottom <= planning.top)

        compose.onNodeWithContentDescription("Join voice in #design").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(listOf("design"), joined); assertEquals(emptyList<String>(), opened) }
        compose.onNodeWithText("planning").performClick()
        compose.runOnIdle { assertEquals(listOf("planning"), opened) }

        compose.onNodeWithText("2 more live").performScrollTo().performClick()
        compose.onNodeWithText("Show less").assertExists()
        compose.onNodeWithContentDescription("Join voice in #lobby").assertExists()
        compose.onNodeWithContentDescription("Join voice in #random").assertExists()
        compose.onNodeWithText("Show less").performScrollTo().performClick()
        compose.onNodeWithContentDescription("Join voice in #lobby").assertDoesNotExist()

        // In a call elsewhere: your card leads, the others offer Switch.
        compose.runOnIdle {
            voice.value = VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "general", spaceId = "space", channelName = "general",
                selfId = "me", participants = people("me", 1))
        }
        compose.onNodeWithText("You're in · 1").assertExists()
        compose.onNodeWithText("5 rooms").assertExists()
        compose.onAllNodesWithText("Switch").assertCountEquals(2)
        compose.onNodeWithContentDescription("Switch voice to #design").assertIsEnabled()
        compose.onNodeWithContentDescription("Leave voice").assertExists()
        val card = compose.onNodeWithText("You're in · 1").getUnclippedBoundsInRoot()
        val firstRoom = compose.onNodeWithContentDescription("Switch voice to #design").getUnclippedBoundsInRoot()
        assertTrue(card.bottom <= firstRoom.top)
    }

    @Test fun absentWhileNobodyIsInVoiceAndYouAreNotInACall() {
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveNowSection(AppUiState(selectedSpace = detail), VoiceState(), null, narrow = true, joinVoice = {}, prepareVoice = {},
                        openChannel = {}, openCall = {}, leaveVoice = {})
                }
            }
        }
        compose.onNodeWithText("Live now").assertDoesNotExist()
    }

    @Test fun aCallInAnotherSpaceStillShowsWithItsSpaceName() {
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveNowSection(
                        AppUiState(selectedSpace = detail),
                        VoiceState(phase = VoiceState.Phase.RECONNECTING, channelId = "elsewhere", spaceId = "other", channelName = "standup", spaceName = "Other Space"),
                        null, narrow = true, joinVoice = {}, prepareVoice = {}, openChannel = {}, openCall = {}, leaveVoice = {},
                    )
                }
            }
        }
        compose.onNodeWithText("Live now").assertExists()
        compose.onNodeWithText("standup").assertExists()
        compose.onNodeWithText("Other Space").assertExists()
        compose.onNodeWithText("Reconnecting…").assertExists()
        compose.onNodeWithText("rooms", substring = true).assertDoesNotExist()
    }
}
