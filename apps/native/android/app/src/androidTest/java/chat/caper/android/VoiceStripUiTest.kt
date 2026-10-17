package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.Participant
import chat.caper.android.ui.CaperTheme
import chat.caper.android.voice.VoiceState
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Phones' strip under the chat header, and the channel rows' voice ends (the header itself carries no voice). */
@RunWith(AndroidJUnit4::class)
class VoiceStripUiTest {
    @get:Rule val compose = createComposeRule()

    private fun person(id: String, name: String) = Participant(id, name, muted = false, deafened = false, tracks = emptyList())

    @Test fun callStripMutesInPlaceAndOtherwiseOpensBrowse() {
        val voice = mutableStateOf(VoiceState(
            phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self",
            participants = listOf(person("remote", "Maya"), person("self", "Fixture Owner")), speakingParticipants = setOf("remote"),
        ))
        var opened = 0
        var toggled = 0
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(390.dp)) {
                    VoiceCallStrip(voice.value, null, sessionStartedAt = null, openBrowse = { opened++ }, toggleMute = {
                        toggled++
                        voice.value = voice.value.copy(muted = !voice.value.muted)
                    })
                }
            }
        }

        // The channel, faces and speaker read as one summary of the strip.
        compose.onNodeWithContentDescription("In voice in #design. Maya is speaking", substring = true).assertExists()
        // Mute is its own 44dp control: it toggles without opening Browse.
        compose.onNodeWithContentDescription("Mute microphone").assertIsEnabled().assertWidthIsAtLeast(44.dp).assertHeightIsAtLeast(44.dp).performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(0, opened) }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsEnabled()
        // Anything else opens Browse, where the call card is.
        compose.onNodeWithContentDescription("In voice in #design", substring = true).performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(1, opened) }
        // The strip has no Leave or Deafen: those live on the call card and the account bar.
        compose.onNodeWithContentDescription("Leave voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Deafen audio").assertDoesNotExist()

        // Like the account bar's, Mute waits while the Audio test holds the microphone.
        compose.runOnIdle { voice.value = voice.value.copy(monitoring = true) }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsNotEnabled()

        compose.runOnIdle { voice.value = voice.value.copy(phase = VoiceState.Phase.CONNECTING, speakingParticipants = emptySet()) }
        compose.onNodeWithContentDescription("In voice in #design. Connecting…", substring = true).assertExists()
        compose.onNodeWithContentDescription("is speaking", substring = true).assertDoesNotExist()
    }

    @Test fun joinStripCountsWhoIsTalkingAndJoinsThisChannel() {
        val action = mutableStateOf(VoiceJoinAction(switch = false, joining = false, enabled = true, unavailableLabel = null))
        var joined = 0
        var prepared = 0
        val people = listOf(person("a", "Maya"), person("b", "Alex"), person("c", "Sam"), person("d", "Jo"))
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(390.dp)) {
                    VoiceJoinStrip("design", people, action.value, join = { joined++ }, prepare = { prepared++ })
                }
            }
        }

        compose.onNodeWithText("4 talking").assertExists()
        compose.onNodeWithText("Join").assertExists()
        compose.onNodeWithContentDescription("Join voice in #design").assertIsEnabled().assertHeightIsAtLeast(40.dp).performClick()
        compose.runOnIdle { assertEquals(1, joined); assertEquals(1, prepared) }

        compose.runOnIdle { action.value = action.value.copy(joining = true, enabled = false) }
        compose.onNodeWithText("Joining…").assertExists()
        compose.onNodeWithContentDescription("Joining voice in #design").assertIsNotEnabled()

        compose.runOnIdle { action.value = VoiceJoinAction(switch = false, joining = false, enabled = false, unavailableLabel = "Checking voice availability…") }
        compose.onNodeWithContentDescription("Checking voice availability…").assertIsNotEnabled()
        compose.runOnIdle { assertEquals(1, joined) }
    }

    @Test fun quietRowsKeepAOneTapJoinAndLiveRowsCountWhoIsInVoice() {
        val action = mutableStateOf(VoiceJoinAction(switch = false, joining = false, enabled = true, unavailableLabel = null))
        var joined = 0
        compose.setContent {
            CaperTheme {
                Row {
                    RowVoiceJoin("general", action.value, join = { joined++ }, prepare = {})
                    LiveVoiceCount(4)
                }
            }
        }

        compose.onNodeWithContentDescription("Join voice in #general").assertIsEnabled()
            .assertWidthIsAtLeast(44.dp).assertHeightIsAtLeast(44.dp).performClick()
        compose.runOnIdle { assertEquals(1, joined) }
        // A live row's count is a label, not a control.
        compose.onNodeWithContentDescription("4 in voice").assertExists()

        compose.runOnIdle { action.value = action.value.copy(switch = true) }
        compose.onNodeWithContentDescription("Switch voice to #general").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(2, joined) }

        compose.runOnIdle { action.value = action.value.copy(switch = false, joining = true, enabled = false) }
        compose.onNodeWithContentDescription("Joining voice in #general").assertIsNotEnabled()
    }
}
