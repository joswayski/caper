package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.AppUiState
import chat.caper.android.model.Channel
import chat.caper.android.model.Participant
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceDetail
import chat.caper.android.model.VisitUi
import chat.caper.android.ui.CaperTheme
import chat.caper.android.voice.VoiceState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Voice in the channel list: live rooms' rows grow a card in place, and only voice with no row in
 * the list (your call elsewhere, a visited channel's room) sits above the channels, with no heading.
 */
@RunWith(AndroidJUnit4::class)
class LiveNowUiTest {
    @get:Rule val compose = createComposeRule()

    private fun people(prefix: String, count: Int) = (1..count).map { Participant("$prefix$it", "$prefix $it", muted = false, deafened = false, tracks = emptyList()) }
    private fun channel(id: String) = Channel(id, "space", id, private = false)

    private val detail = SpaceDetail(Space("space", "Fixture Studio"), listOf("general", "design", "planning", "random", "lobby").map(::channel), members = emptyList())

    @Test fun listedVoiceLeavesNothingAboveTheChannels() {
        // Your call and the live rooms all have rows here: their cards are in the list, not above it.
        val state = AppUiState(
            selectedSpace = detail,
            voiceRosters = mapOf("design" to people("Maya", 4), "planning" to people("Alex", 2), "random" to people("Sam", 1)),
            voiceAvailability = detail.channels.associate { it.id to true },
        )
        val voice = mutableStateOf(VoiceState())
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveNowSection(state, voice.value, null, narrow = true, joinVoice = {}, prepareVoice = {}, openChannel = {}, openCall = {}, leaveVoice = {})
                }
            }
        }
        for (gone in listOf("Live now", "rooms", "more live", "Show less", "Maya 1", "Join")) compose.onNodeWithText(gone, substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Join voice in #design").assertDoesNotExist()

        compose.runOnIdle {
            voice.value = VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "general", spaceId = "space", channelName = "general",
                selfId = "me1", participants = people("me", 1))
        }
        compose.onNodeWithText("You’re in", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Leave voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Switch voice to #design").assertDoesNotExist()
    }

    @Test fun aCallInAnotherSpaceSitsAboveTheChannelsAndATapOpensItsChat() {
        var opened = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveNowSection(
                        AppUiState(selectedSpace = detail),
                        VoiceState(phase = VoiceState.Phase.RECONNECTING, channelId = "elsewhere", spaceId = "other", channelName = "standup",
                            spaceName = "Other Space", selfId = "me1", participants = people("me", 1) + people("Bea", 2), monitoring = true),
                        null, narrow = true, joinVoice = {}, prepareVoice = {}, openChannel = {}, openCall = { opened++ }, leaveVoice = { left++ },
                    )
                }
            }
        }
        compose.onNodeWithText("standup").assertExists()
        compose.onNodeWithText("Other Space").assertExists()
        compose.onNodeWithText("Reconnecting…").assertExists()
        compose.onNodeWithText("Live now").assertDoesNotExist()
        compose.onNodeWithText("rooms", substring = true).assertDoesNotExist()

        // Anywhere on the card opens the call's chat: its heading, its status, a face.
        compose.onNodeWithText("Reconnecting…", useUnmergedTree = true).performClick()
        compose.onNodeWithContentDescription("Bea 1").performClick()
        compose.runOnIdle { assertEquals(2, opened) }
        // Its controls keep their own actions; a disabled Mute (the Audio test holds the mic) does nothing.
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.onNodeWithContentDescription("Mute microphone").performClick()
        compose.runOnIdle { assertEquals(2, opened); assertEquals(1, left) }
    }

    @Test fun aVisitedChannelsLiveRoomSitsAboveTheChannelsAndJoinsWithoutOpeningIt() {
        val visited = Channel("report", "other", "support", private = false)
        val state = AppUiState(
            selectedSpace = detail, selectedChannel = visited,
            visit = VisitUi(channelId = "report", spaceId = "other", spaceName = "Other", channelName = "support", direct = false),
            voiceRosters = mapOf("report" to people("Bea", 2)), voiceAvailability = mapOf("report" to true),
            voiceSessionStartedAt = mapOf("report" to System.currentTimeMillis() - 65_000),
        )
        val joined = mutableListOf<String>()
        val opened = mutableListOf<String>()
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveNowSection(state, VoiceState(), null, narrow = true, joinVoice = { joined += it.id }, prepareVoice = {},
                        openChannel = { opened += it.id }, openCall = {}, leaveVoice = {})
                }
            }
        }
        // Today's room card: the name and timer, then who is there and Join.
        compose.onNodeWithText("support").assertExists()
        compose.onNodeWithContentDescription("Voice session duration").assertExists()
        compose.onNodeWithText("Bea 1 and Bea 2", useUnmergedTree = true).performClick()
        compose.runOnIdle { assertEquals(listOf("report"), opened) }
        compose.onNodeWithContentDescription("Join voice in #support").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(listOf("report"), joined); assertEquals(listOf("report"), opened) }
    }

    @Test fun aLiveRowsCardOpensItsChatExceptFromJoinAndTheRowsMenu() {
        val action = mutableStateOf(VoiceJoinAction(switch = false, joining = false, enabled = true, unavailableLabel = null))
        var opened = 0
        var joined = 0
        var menus = 0
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    RowRoomCard(
                        LiveRoom(channel("design"), people("Maya", 8)), action.value, System.currentTimeMillis() - 1_701_000,
                        open = { opened++ }, join = { joined++ }, prepare = {},
                    ) {
                        // A stand-in for the channel row: its name and count, then its ⋯ menu.
                        Row(Modifier.fillMaxWidth().height(48.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text("design", Modifier.weight(1f).padding(start = 9.dp))
                            LiveVoiceCount(8)
                            IconButton({ menus++ }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "design channel menu") }
                        }
                    }
                }
            }
        }
        // Today's room-card body under the row, with the room's timer between the names and Join.
        val names = compose.onNodeWithText("Maya 1, Maya 2 and 6 more", useUnmergedTree = true).getUnclippedBoundsInRoot()
        val timer = compose.onNodeWithContentDescription("Voice session duration", useUnmergedTree = true).getUnclippedBoundsInRoot()
        val join = compose.onNodeWithContentDescription("Join voice in #design").assertIsEnabled().getUnclippedBoundsInRoot()
        val row = compose.onNodeWithText("design", useUnmergedTree = true).getUnclippedBoundsInRoot()
        assertTrue(row.bottom <= names.top)
        assertTrue(names.right <= timer.left && timer.right <= join.left)
        compose.onNodeWithContentDescription("8 in voice").assertExists()

        // The row's name and the names line are the card's: each opens the chat.
        compose.onNodeWithText("design", useUnmergedTree = true).performClick()
        compose.onNodeWithText("Maya 1, Maya 2 and 6 more", useUnmergedTree = true).performClick()
        compose.runOnIdle { assertEquals(2, opened) }
        compose.onNodeWithContentDescription("design channel menu").performClick()
        compose.onNodeWithContentDescription("Join voice in #design").performClick()
        compose.runOnIdle { assertEquals(2, opened); assertEquals(1, joined); assertEquals(1, menus) }
        // A Join that can't be pressed yet doesn't open the chat either.
        compose.runOnIdle { action.value = VoiceJoinAction(switch = false, joining = false, enabled = false, unavailableLabel = "Checking voice availability…") }
        compose.onNodeWithContentDescription("Checking voice availability…").performClick()
        compose.runOnIdle { assertEquals(2, opened); assertEquals(1, joined) }
    }
}
