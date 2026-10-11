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

        // Phones keep one face and drop the count; the summary still names everyone.
        compose.runOnIdle { compact.value = true }
        compose.onNodeWithContentDescription("Maya, Alex, Sam and 1 more are in voice").assertExists()
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

    @Test fun phoneHeaderMutesBeforeTheActionDuringACall() {
        val muted = mutableStateOf(false)
        val monitoring = mutableStateOf(false)
        var toggled = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                Row {
                    HeaderVoice(listOf(person("a", "Maya")), HeaderVoiceAction.Leave, "design", enabled = true, unavailableLabel = null, compact = true,
                        onJoin = {}, onPrepare = {}, onLeave = { left++ },
                        mute = { HeaderMuteButton(muted.value, monitoring.value) { toggled++; muted.value = !muted.value } })
                }
            }
        }

        compose.onNodeWithContentDescription("Mute microphone").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(0, left) }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsEnabled()
        // The mic sits before the action, as on web.
        val mic = compose.onNodeWithContentDescription("Unmute microphone").getUnclippedBoundsInRoot()
        val leave = compose.onNodeWithContentDescription("Leave voice").getUnclippedBoundsInRoot()
        assertTrue(mic.right <= leave.left)

        // Like the account bar's, mute waits while the Audio test holds the microphone.
        compose.runOnIdle { monitoring.value = true }
        compose.onNodeWithContentDescription("Unmute microphone").assertIsNotEnabled()

        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(1, left) }
    }

    @Test fun conversationWithoutVoiceOffersTheCallsMuteAndLeave() {
        var toggled = 0
        var left = 0
        compose.setContent {
            CaperTheme {
                HeaderVoiceElsewhere({ HeaderMuteButton(muted = false, monitoring = false) { toggled++ } }) { left++ }
            }
        }

        compose.onNodeWithContentDescription("Join voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Mute microphone").performClick()
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(1, toggled); assertEquals(1, left) }
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
