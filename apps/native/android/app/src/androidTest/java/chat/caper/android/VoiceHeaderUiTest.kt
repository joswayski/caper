package chat.caper.android

import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.Participant
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
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

        compose.runOnIdle { action.value = HeaderVoiceAction.Joining }
        compose.onNodeWithContentDescription("Joining voice in #design").assertIsNotEnabled()

        compose.runOnIdle { action.value = HeaderVoiceAction.Leave }
        compose.onNodeWithContentDescription("Join voice").assertDoesNotExist()
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(2, joined); assertEquals(1, left) }
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
