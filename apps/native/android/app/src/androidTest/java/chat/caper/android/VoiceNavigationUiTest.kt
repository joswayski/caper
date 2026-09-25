package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onNodeWithContentDescription
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

    @Test fun connectedDockOpensChannelWhileLeaveRemainsIndependent() {
        var opened = 0
        var left = 0
        val voice = VoiceState(phase = VoiceState.Phase.CONNECTED, spaceName = "Fixture Studio", channelName = "Planning")
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    ConnectedVoiceContext(voice, { opened++ }, { left++ })
                }
            }
        }

        compose.onNodeWithContentDescription("Open voice channel").performClick()
        compose.runOnIdle { assertEquals(1, opened); assertEquals(0, left) }

        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.runOnIdle { assertEquals(1, opened); assertEquals(1, left) }
    }
}
