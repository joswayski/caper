package chat.caper.android

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.Participant
import chat.caper.android.ui.CaperTheme
import chat.caper.android.ui.SurfaceSidebar
import chat.caper.android.voice.VoiceState
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VoiceRosterUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun remoteControlsAreHiddenUntilAudioOpensAndSelfHasNoMenu() {
        val voice = VoiceState(
            phase = VoiceState.Phase.CONNECTED, channelId = "design", selfId = "self", muted = true,
            participants = listOf(
                Participant("self", "Fixture Owner", muted = false, deafened = false, tracks = emptyList()),
                Participant("remote", "Remote Voice", muted = false, deafened = true, tracks = emptyList()),
            ),
            participantVolumes = mapOf("remote" to 170),
        )
        compose.setContent { CaperTheme { Column(Modifier.width(280.dp).background(SurfaceSidebar)) { VoiceRoster(voice) } } }

        compose.onNodeWithText("Fixture Owner (you)").assertExists()
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Fixture Owner").assertDoesNotExist()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertExists()
        compose.onNodeWithText("IN VOICE", substring = true).assertDoesNotExist()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        compose.onNodeWithText("Deafened").assertDoesNotExist()
        compose.onAllNodesWithContentDescription("Muted").assertCountEquals(1)
        compose.onAllNodesWithContentDescription("Deafened").assertCountEquals(1)
        capture("voice-roster-connected-closed.png")

        compose.onNodeWithContentDescription("Audio controls for Remote Voice").performClick()
        compose.onNodeWithText("User volume").assertExists()
        compose.onNodeWithText("170%").assertExists()
        compose.onNodeWithText("Mute").assertExists()
        compose.onNodeWithContentDescription("Remote Voice volume").assertExists()
        compose.onNodeWithContentDescription("Mute Remote Voice for me").assertExists()
        capture("voice-roster-connected-open.png")
    }

    @Test fun connectingCallDoesNotExposeRemoteAudioControls() {
        val voice = VoiceState(
            phase = VoiceState.Phase.CONNECTING, channelId = "design", selfId = "self",
            participants = listOf(Participant("remote", "Remote Voice", muted = false, deafened = false, tracks = emptyList())),
        )
        compose.setContent { CaperTheme { Column(Modifier.width(280.dp).background(SurfaceSidebar)) { VoiceRoster(voice) } } }
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertDoesNotExist()
    }

    private fun capture(name: String) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val directory = requireNotNull(instrumentation.targetContext.getExternalFilesDir("ui"))
        val screenshot = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        try {
            File(directory, name).outputStream().use { output ->
                check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, output))
            }
        } finally { screenshot.recycle() }
    }
}
