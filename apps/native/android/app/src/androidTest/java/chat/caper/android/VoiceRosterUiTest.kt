package chat.caper.android

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.Participant
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import chat.caper.android.ui.SurfaceSidebar
import chat.caper.android.ui.Text as TextColor
import chat.caper.android.voice.VoiceState
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class VoiceRosterUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun remoteControlsAreHiddenUntilAudioOpensAndSelfHasNoMenu() {
        val voice = mutableStateOf(VoiceState(
            phase = VoiceState.Phase.CONNECTED, channelId = "design", selfId = "self", muted = true,
            participants = listOf(
                Participant("self", "Fixture Owner", muted = false, deafened = false, tracks = emptyList()),
                Participant("remote", "Remote Voice", muted = true, deafened = true, tracks = emptyList()),
            ),
            participantVolumes = mapOf("remote" to 170),
        ))
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding).width(280.dp).background(SurfaceSidebar).testTag("voice-roster")) { VoiceRoster(voice.value) }
                }
            }
        }

        compose.onNodeWithText("Fixture Owner (you)").assertExists()
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Fixture Owner").assertDoesNotExist()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertExists()
        compose.onNodeWithText("IN VOICE", substring = true).assertDoesNotExist()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        compose.onNodeWithText("Deafened").assertDoesNotExist()
        compose.onNodeWithText("You muted Remote Voice").assertDoesNotExist()
        compose.onNodeWithText("You muted Fixture Owner").assertDoesNotExist()
        compose.onAllNodesWithContentDescription("Muted").assertCountEquals(2)
        compose.onAllNodesWithContentDescription("Deafened").assertCountEquals(1)
        capture("voice-roster-connected-closed.png", compose.onNodeWithTag("voice-roster"))

        compose.runOnIdle { voice.value = voice.value.copy(locallyMutedParticipants = setOf("self", "remote")) }
        compose.onNodeWithText("You muted Remote Voice").assertExists()
        compose.onNodeWithText("You muted Fixture Owner").assertDoesNotExist()
        capture("voice-roster-connected-locally-muted.png", compose.onNodeWithTag("voice-roster"))

        compose.runOnIdle { voice.value = voice.value.copy(locallyMutedParticipants = emptySet()) }
        compose.onNodeWithText("You muted Remote Voice").assertDoesNotExist()

        compose.onNodeWithContentDescription("Audio controls for Remote Voice").performClick()
        compose.onNodeWithText("User volume").assertExists()
        compose.onNodeWithText("170%").assertExists()
        compose.onNodeWithText("Mute").assertExists()
        compose.onNodeWithContentDescription("Remote Voice volume").assertExists()
        compose.onNodeWithContentDescription("Mute Remote Voice for me").assertExists()
        for (label in listOf("User volume", "Mute")) {
            compose.onNodeWithText(label).performSemanticsAction(SemanticsActions.GetTextLayoutResult) { getLayout ->
                val layouts = mutableListOf<TextLayoutResult>()
                assertTrue(getLayout(layouts))
                assertEquals(TextColor, layouts.single().layoutInput.style.color)
            }
        }
        capture("voice-roster-connected-open.png", compose.onNode(isPopup()))
    }

    @Test fun connectingCallDoesNotExposeRemoteAudioControls() {
        val voice = VoiceState(
            phase = VoiceState.Phase.CONNECTING, channelId = "design", selfId = "self",
            participants = listOf(Participant("remote", "Remote Voice", muted = false, deafened = false, tracks = emptyList())),
        )
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding).width(280.dp).background(SurfaceSidebar)) { VoiceRoster(voice) }
                }
            }
        }
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertDoesNotExist()
    }

    private fun capture(name: String, node: SemanticsNodeInteraction) {
        val directory = File(requireNotNull(InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")))
        check(directory.mkdirs() || directory.isDirectory)
        // Compose waits for the target window's draw commit, including Popup windows.
        val screenshot = node.captureToImage().asAndroidBitmap()
        try {
            File(directory, name).outputStream().use { output ->
                check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, output))
            }
        } finally { screenshot.recycle() }
    }
}
