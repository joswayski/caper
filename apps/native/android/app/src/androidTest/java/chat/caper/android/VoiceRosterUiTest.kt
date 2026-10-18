package chat.caper.android

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
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

/**
 * Your call card: a tap anywhere but its controls opens the chat, and a long press on a face (or its
 * accessibility action) opens the full list, with each person's local audio.
 */
@RunWith(AndroidJUnit4::class)
class VoiceRosterUiTest {
    @get:Rule val compose = createComposeRule()

    private var opened = 0
    private var left = 0

    private fun person(id: String, name: String, muted: Boolean = false, deafened: Boolean = false) =
        Participant(id, name, muted = muted, deafened = deafened, tracks = emptyList())

    /** Your call's card above the channels, at the wide sidebar's 280dp so the list opens as a popover beside it. */
    private fun showCard(voice: () -> VoiceState) {
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Column(Modifier.padding(padding).width(280.dp).background(SurfaceSidebar).testTag("live-call-card")) {
                        LiveCallCard(voice(), null, otherSpaceName = null, sessionStartedAt = null, narrow = false, open = { opened++ }, leave = { left++ })
                    }
                }
            }
        }
    }

    private fun SemanticsNodeInteraction.performCustomAction(label: String) {
        val actions = fetchSemanticsNode().config[SemanticsActions.CustomActions]
        compose.runOnIdle { actions.single { it.label == label }.action() }
    }

    @Test fun longPressingAFaceOpensTheListWithThatPersonsAudioWhileATapOpensTheChat() {
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
        compose.onNodeWithText("2 people").assertExists()
        compose.onNodeWithContentDescription("Fixture Owner (you)").assertExists()
        compose.onNodeWithContentDescription("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Unmute microphone").assertExists()
        compose.onNodeWithContentDescription("Deafen audio").assertExists()
        compose.onNodeWithContentDescription("Leave voice").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        capture("voice-roster-connected-card.png", compose.onNodeWithTag("live-call-card"))

        // A tap on a face is the card's: it opens the chat, not the list.
        compose.onNodeWithContentDescription("Remote Voice").performClick()
        compose.runOnIdle { assertEquals(1, opened) }
        compose.onNodeWithText("In #design · 2 people").assertDoesNotExist()

        // A long press on your own face opens everyone, with no audio controls open, and not the chat.
        compose.onNodeWithContentDescription("Fixture Owner (you)").performTouchInput { longClick() }
        compose.onNodeWithText("In #design · 2 people").assertExists()
        compose.runOnIdle { assertEquals(1, opened) }
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

        // A long press on another person's face opens the list with their volume already open.
        compose.onNodeWithContentDescription("Remote Voice").performTouchInput { longClick() }
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
        compose.onNodeWithText("Close").performClick()
        compose.runOnIdle { assertEquals(1, opened) }

        // TalkBack: each face's custom action opens the same list; its click opens the chat.
        compose.onNodeWithContentDescription("Remote Voice").performCustomAction("Audio controls for Remote Voice")
        compose.onNodeWithText("User volume").assertExists()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithContentDescription("Fixture Owner (you)").performCustomAction("Show everyone in voice")
        compose.onNodeWithText("In #design · 2 people").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithContentDescription("Remote Voice").performSemanticsAction(SemanticsActions.OnClick)
        compose.runOnIdle { assertEquals(2, opened); assertEquals(0, left) }
    }

    @Test fun nothingInTheCardComesOrGoesWithSpeaking() {
        val voice = mutableStateOf(VoiceState(
            phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self",
            participants = listOf(person("self", "Fixture Owner"), person("remote", "Maya")),
        ))
        showCard { voice.value }
        val quiet = compose.onNodeWithTag("live-call-card").getUnclippedBoundsInRoot()
        compose.runOnIdle { voice.value = voice.value.copy(speakingParticipants = setOf("remote")) }
        // The ring on Maya's face says she is speaking; no line of text does, so the card keeps its size.
        compose.onNodeWithContentDescription("Maya").assertExists()
        compose.onNodeWithText("speaking", substring = true, ignoreCase = true).assertDoesNotExist()
        assertEquals(quiet, compose.onNodeWithTag("live-call-card").getUnclippedBoundsInRoot())
    }

    @Test fun moreThanTenFoldIntoAPlusTileWhoseLongPressOpensEveryone() {
        val people = listOf(person("self", "Fixture Owner")) + (1..11).map { person("p$it", "Person $it") }
        showCard { VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self", participants = people) }

        // Two rows of five fit 280dp: nine faces and "+3".
        compose.onNodeWithContentDescription("Person 8").assertExists()
        compose.onNodeWithContentDescription("Person 9").assertDoesNotExist()
        compose.onNodeWithContentDescription("3 more in voice").performClick()
        compose.runOnIdle { assertEquals(1, opened) }
        compose.onNodeWithText("In #design · 12 people").assertDoesNotExist()
        compose.onNodeWithContentDescription("3 more in voice").performTouchInput { longClick() }
        compose.onNodeWithText("In #design · 12 people").assertExists()
        compose.onNodeWithText("User volume").assertDoesNotExist()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithContentDescription("3 more in voice").performCustomAction("Show everyone in voice")
        compose.onNodeWithText("In #design · 12 people").assertExists()
        compose.runOnIdle { assertEquals(1, opened) }
    }

    @Test fun joiningShowsCancelAndOffersNoRemoteAudio() {
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    LiveCallCard(
                        VoiceState(phase = VoiceState.Phase.CONNECTING, channelId = "design", channelName = "design", selfId = "self",
                            participants = listOf(person("remote", "Remote Voice"))),
                        null, otherSpaceName = null, sessionStartedAt = null, narrow = false, open = { opened++ }, leave = { left++ },
                    )
                }
            }
        }
        compose.onNodeWithText("Connecting…").assertExists()
        compose.onNodeWithText("Cancel").assertExists()
        compose.onNodeWithContentDescription("Remote Voice").performTouchInput { longClick() }
        compose.onNodeWithText("Remote Voice").assertExists()
        compose.onNodeWithContentDescription("Audio controls for Remote Voice").assertDoesNotExist()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithContentDescription("Cancel joining voice").performClick()
        compose.runOnIdle { assertEquals(1, left); assertEquals(0, opened) }
    }

    @Test fun yourCallsRowGrowsIntoTheCardAndOnlyItsControlsAndMenuKeepTheirOwnActions() {
        var menus = 0
        compose.setContent {
            CaperTheme {
                Column(Modifier.width(280.dp)) {
                    RowCallCard(
                        VoiceState(phase = VoiceState.Phase.CONNECTED, channelId = "design", channelName = "design", selfId = "self",
                            participants = listOf(person("self", "Fixture Owner"), person("remote", "Maya")), monitoring = true),
                        null, narrow = false, open = { opened++ }, leave = { left++ },
                    ) {
                        // A stand-in for the channel row: its name and your call's status, then its ⋯ menu.
                        Row(Modifier.fillMaxWidth().height(48.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text("design", Modifier.weight(1f).padding(start = 9.dp))
                            CallStatus(VoiceState(phase = VoiceState.Phase.CONNECTED, participants = listOf(person("self", "Fixture Owner"), person("remote", "Maya"))), null)
                            IconButton({ menus++ }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "design channel menu") }
                        }
                    }
                }
            }
        }
        // The row line, then the faces, then the controls.
        val row = compose.onNodeWithText("design", useUnmergedTree = true).getUnclippedBoundsInRoot()
        val face = compose.onNodeWithContentDescription("Maya").getUnclippedBoundsInRoot()
        val leave = compose.onNodeWithContentDescription("Leave voice").getUnclippedBoundsInRoot()
        assertTrue(row.bottom <= face.top && face.bottom <= leave.top)
        compose.onNodeWithText("2 people", useUnmergedTree = true).assertExists()

        compose.onNodeWithText("design", useUnmergedTree = true).performClick()
        compose.onNodeWithText("2 people", useUnmergedTree = true).performClick()
        compose.onNodeWithContentDescription("Maya").performClick()
        compose.runOnIdle { assertEquals(3, opened) }
        // The menu, Leave and the Mute and Deafen the Audio test holds never open the chat.
        compose.onNodeWithContentDescription("design channel menu").performClick()
        compose.onNodeWithContentDescription("Leave voice").performClick()
        compose.onNodeWithContentDescription("Mute microphone").performClick()
        compose.onNodeWithContentDescription("Deafen audio").performClick()
        compose.runOnIdle { assertEquals(3, opened); assertEquals(1, menus); assertEquals(1, left) }
        // A long press on a face opens the list (as a popover on wide layouts), not the chat.
        compose.onNodeWithContentDescription("Maya").performTouchInput { longClick() }
        compose.onNodeWithText("In #design · 2 people").assertExists()
        compose.runOnIdle { assertEquals(3, opened) }
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
