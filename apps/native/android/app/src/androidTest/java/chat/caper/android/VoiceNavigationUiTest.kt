package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.ui.CaperTheme
import chat.caper.android.voice.VoiceState
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VoiceNavigationUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun callCardOpensItsChannelWhileLeaveRemainsIndependent() {
        var opened = 0
        var left = 0
        // A call in another space than the one being viewed names its space.
        val voice = VoiceState(phase = VoiceState.Phase.CONNECTED, spaceName = "Fixture Studio", channelName = "Planning")
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveCallCard(voice, null, otherSpaceName = "Fixture Studio", sessionStartedAt = null, narrow = false,
                        open = { opened++ }, leave = { left++ })
                }
            }
        }

        compose.onNodeWithText("Fixture Studio", useUnmergedTree = true).assertExists()
        // The whole card opens the call's chat: its channel, its space's name, its status.
        compose.onNodeWithText("Planning", useUnmergedTree = true).performClick()
        compose.onNodeWithText("Fixture Studio", useUnmergedTree = true).performClick()
        compose.onNodeWithText("0 people", useUnmergedTree = true).performClick()
        compose.runOnIdle { assertEquals(3, opened); assertEquals(0, left) }

        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(3, opened); assertEquals(1, left) }
    }
}
