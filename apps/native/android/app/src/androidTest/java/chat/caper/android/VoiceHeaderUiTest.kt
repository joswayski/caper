package chat.caper.android

import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.Participant
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VoiceHeaderUiTest {
    @get:Rule val compose = createComposeRule()

    private fun person(id: String, name: String) = Participant(id, name, muted = false, deafened = false, tracks = emptyList())

    @Test fun headerJoinsSwitchesAndLeavesWithOneAction() {
        val action = mutableStateOf(HeaderVoiceAction.Join)
        val people = mutableStateOf(emptyList<Participant>())
        val compact = mutableStateOf(false)
        var joined = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                Row {
                    HeaderVoice(people.value, action.value, "design", enabled = true, unavailableLabel = null, compact = compact.value,
                        onJoin = { joined++ }, onPrepare = {}, onLeave = { left++ })
                }
            }
        }

        // An empty channel shows only the action.
        compose.onNodeWithContentDescription("is in voice", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Join voice").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(1, joined) }

        compose.runOnIdle {
            people.value = listOf(person("a", "Maya"), person("b", "Alex"), person("c", "Sam"), person("d", "Jo"))
            action.value = HeaderVoiceAction.Switch
        }
        compose.onNodeWithContentDescription("Maya, Alex, Sam and 1 more are in voice").assertExists()
        compose.onNodeWithContentDescription("Switch voice to #design").performClick()
        compose.runOnIdle { assertEquals(2, joined) }

        // Phones drop the faces and count; the title's subtitle says how many are in voice.
        compose.runOnIdle { compact.value = true }
        compose.onNodeWithContentDescription("are in voice", substring = true).assertDoesNotExist()
        // Phones show icons only; each label stays as the accessible name.
        compose.onNodeWithText("Switch here").assertDoesNotExist()
        compose.onNodeWithContentDescription("Switch voice to #design").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(3, joined) }

        compose.runOnIdle { action.value = HeaderVoiceAction.Joining }
        compose.onNodeWithContentDescription("Joining voice in #design").assertIsNotEnabled()

        compose.runOnIdle { action.value = HeaderVoiceAction.Leave }
        compose.onNodeWithContentDescription("Join voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.onNodeWithText("Leave").assertDoesNotExist()
        compose.runOnIdle { assertEquals(3, joined); assertEquals(1, left) }
    }

    @Test fun phoneHeaderMutesAndDeafensBeforeTheActionDuringACall() {
        val muted = mutableStateOf(false)
        val deafened = mutableStateOf(false)
        val monitoring = mutableStateOf(false)
        var toggled = 0
        var deafenToggled = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                Row {
                    HeaderVoice(listOf(person("a", "Maya")), HeaderVoiceAction.Leave, "design", enabled = true, unavailableLabel = null, compact = true,
                        onJoin = {}, onPrepare = {}, onLeave = { left++ },
                        audioControls = {
                            HeaderMuteButton(muted.value, monitoring.value) { toggled++; muted.value = !muted.value }
                            HeaderDeafenButton(deafened.value, monitoring.value) { deafenToggled++; deafened.value = !deafened.value }
                        })
                }
            }
        }

        compose.onNodeWithContentDescription("Mute microphone").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(0, left) }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsEnabled()
        compose.onNodeWithContentDescription("Deafen audio").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(1, deafenToggled); assertEquals(1, toggled) }
        compose.onNodeWithContentDescription("Undeafen audio").assertIsEnabled()
        // Mic, then deafen, then disconnect, as on web.
        val mic = compose.onNodeWithContentDescription("Unmute microphone").getUnclippedBoundsInRoot()
        val deafen = compose.onNodeWithContentDescription("Undeafen audio").getUnclippedBoundsInRoot()
        val leave = compose.onNodeWithContentDescription("Leave voice").getUnclippedBoundsInRoot()
        assertTrue(mic.right <= deafen.left)
        assertTrue(deafen.right <= leave.left)

        // Like the account bar's, mute and deafen wait while the Audio test holds the microphone.
        compose.runOnIdle { monitoring.value = true }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Undeafen audio").assertIsNotEnabled()

        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(1, deafenToggled); assertEquals(1, left) }
    }

    @Test fun conversationWithoutVoiceOffersTheCallsMuteDeafenAndLeave() {
        var toggled = 0
        var deafenToggled = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                HeaderVoiceElsewhere({
                    HeaderMuteButton(muted = false, monitoring = false) { toggled++ }
                    HeaderDeafenButton(deafened = false, monitoring = false) { deafenToggled++ }
                }) { left++ }
            }
        }

        compose.onNodeWithContentDescription("Join voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Mute microphone").performClick()
        compose.onNodeWithContentDescription("Deafen audio").performClick()
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(1, deafenToggled); assertEquals(1, left) }
    }

    @Test fun unavailableVoiceNamesWhyJoinIsDisabled() {
        compose.setContent {
            CaperTheme {
                HeaderVoice(emptyList(), HeaderVoiceAction.Join, "general", enabled = false,
                    unavailableLabel = "Checking voice availability…", compact = false, onJoin = {}, onPrepare = {}, onLeave = {})
            }
        }
        compose.onNodeWithContentDescription("Checking voice availability…").assertIsNotEnabled()
    }
}
