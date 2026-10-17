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
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
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

/** Your call card's faces and the full list they open, with each person's local audio. */
@RunWith(AndroidJUnit4::class)
class VoiceRosterUiTest {
    @get:Rule val compose = createComposeRule()

    private fun person(id: String, name: String, muted: Boolean = false, deafened: Boolean = false) =
        Participant(id, name, muted = muted, deafened = deafened, tracks = emptyList())

    private fun showCard(voice: () -> VoiceState) {
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    // The wide sidebar's 280dp, so the list opens as a popover beside it.
                    Column(Modifier.padding(padding).width(280.dp).background(SurfaceSidebar).testTag("live-call-card")) {
                        LiveCallCard(voice(), null, otherSpaceName = null, sessionStartedAt = null, narrow = false, openCall = {}, leave = {})
                    }
                }
            }
        }
    }

    @Test fun facesOpenTheListWithThatPersonsAudioAndYoursHasNone() {
        val voice = mutableStateOf(VoiceState(
            phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self", muted = true,
            participants = listOf(
                person("remote", "Remote Voice", muted = true, deafened = true),
                person("self", "Fixture Owner"),
            ),
            participantVolumes = mapOf("remote" to 170),
        ))
        showCard { voice.value }

        // You first, then join order; your mute shows pressed here as in the account bar.
        compose.onNodeWithText("You're in · 2").assertExists()
        compose.onNodeWithContentDescription("Fixture Owner (you)").assertExists()
        compose.onNodeWithContentDescription("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Unmute microphone").assertExists()
        compose.onNodeWithContentDescription("Deafen audio").assertExists()
        compose.onNodeWithContentDescription("Leave voice").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        capture("voice-roster-connected-card.png", compose.onNodeWithTag("live-call-card"))

        // Your own face opens everyone, with no audio controls open.
        compose.onNodeWithContentDescription("Fixture Owner (you)").performClick()
        compose.onNodeWithText("In #design · 2 people").assertExists()
        compose.onNodeWithText("Fixture Owner (you)").assertExists()
        compose.onNodeWithText("Deafened").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Fixture Owner").assertDoesNotExist()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        capture("voice-roster-connected-closed.png", compose.onNode(isPopup()))

        compose.runOnIdle { voice.value = voice.value.copy(locallyMutedParticipants = setOf("self", "remote")) }
        compose.onNodeWithText("You muted Remote Voice").assertExists()
        compose.onNodeWithText("You muted Fixture Owner").assertDoesNotExist()
        capture("voice-roster-connected-locally-muted.png", compose.onNode(isPopup()))
        compose.runOnIdle { voice.value = voice.value.copy(locallyMutedParticipants = emptySet()) }
        compose.onNodeWithText("You muted Remote Voice").assertDoesNotExist()

        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithText("In #design · 2 people").assertDoesNotExist()

        // Another person's face opens the list with their volume already open.
        compose.onNodeWithContentDescription("Remote Voice").performClick()
        compose.onNodeWithText("User volume").assertExists()
        compose.onNodeWithText("170%").assertExists()
        compose.onNodeWithText("Mute for me").assertExists()
        compose.onNodeWithText("Only changes what you hear.").assertExists()
        compose.onNodeWithContentDescription("Remote Voice volume").assertExists()
        compose.onNodeWithContentDescription("Mute Remote Voice for me").assertExists()
        for (label in listOf("User volume", "Mute for me")) {
            compose.onNodeWithText(label).performSemanticsAction(SemanticsActions.GetTextLayoutResult) { getLayout ->
                val layouts = mutableListOf<TextLayoutResult>()
                assertTrue(getLayout(layouts))
                assertEquals(TextColor, layouts.single().layoutInput.style.color)
            }
        }
        capture("voice-roster-connected-open.png", compose.onNode(isPopup()))
        // Audio closes again from the same row.
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").performClick()
        compose.onNodeWithText("User volume").assertDoesNotExist()
    }

    @Test fun moreThanTenFoldIntoAPlusTileThatOpensEveryone() {
        val people = listOf(person("self", "Fixture Owner")) + (1..11).map { person("p$it", "Person $it") }
        showCard { VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self", participants = people) }

        // Two rows of five fit 280dp: nine faces and "+3".
        compose.onNodeWithContentDescription("Person 8").assertExists()
        compose.onNodeWithContentDescription("Person 9").assertDoesNotExist()
        compose.onNodeWithContentDescription("3 more in voice").performClick()
        compose.onNodeWithText("In #design · 12 people").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
    }

    @Test fun joiningShowsCancelAndOffersNoRemoteAudio() {
        var left = 0
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveCallCard(
                        VoiceState(phase = VoiceState.Phase.CONNECTING, channelId = "design", channelName = "design", selfId = "self",
                            participants = listOf(person("remote", "Remote Voice"))),
                        null, otherSpaceName = null, sessionStartedAt = null, narrow = false, openCall = {}, leave = { left++ },
                    )
                }
            }
        }
        compose.onNodeWithText("Connecting…").assertExists()
        compose.onNodeWithText("Cancel").assertExists()
        compose.onNodeWithContentDescription("Remote Voice").performClick()
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertDoesNotExist()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithContentDescription("Cancel joining voice").performClick()
        compose.runOnIdle { assertEquals(1, left) }
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
