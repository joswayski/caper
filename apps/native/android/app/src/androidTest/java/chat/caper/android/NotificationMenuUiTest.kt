package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.data.OverrideChange
import chat.caper.android.model.NotificationOverride
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NotificationMenuUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun directMenuTurnsNotificationsOffAndMutes() {
        val changes = mutableListOf<OverrideChange>()
        compose.setContent {
            CaperTheme {
                var open by remember { mutableStateOf(true) }
                Column {
                    Text("Notification menu UI test fixture")
                    TextButton({ open = true }) { Text("Open options") }
                    DirectOptionsMenu(open, { open = false }, null, loaded = true) { changes += it }
                }
            }
        }

        compose.onNodeWithText("Turn off notifications").performClick()
        compose.runOnIdle { assertEquals(listOf(OverrideChange.Level("nothing")), changes) }
        compose.onNodeWithText("Open options").performClick()
        compose.onNodeWithText("Mute conversation").performClick()
        compose.onNodeWithText("For 15 minutes").assertIsDisplayed()
        compose.onNodeWithText("For 24 hours").assertIsDisplayed()
        compose.onNodeWithText("Until I turn it back on").performClick()
        compose.runOnIdle { assertEquals(OverrideChange.Mute("forever"), changes.last()) }
        compose.onNodeWithText("For 15 minutes").assertDoesNotExist()
    }

    @Test fun mutedDirectMenuOffersUnmuteAndTurnOn() {
        val changes = mutableListOf<OverrideChange>()
        val override = NotificationOverride(conversationId = "direct000001", level = "nothing", mutedUntil = "forever")
        compose.setContent { CaperTheme { DirectOptionsMenu(true, {}, override, loaded = true) { changes += it } } }

        compose.onNodeWithText("Turn on notifications").assertIsDisplayed()
        compose.onNodeWithText("Muted").assertIsDisplayed()
        compose.onNodeWithText("Unmute conversation").performClick()
        compose.runOnIdle { assertEquals(listOf(OverrideChange.Mute(null)), changes) }
    }

    @Test fun channelMenuNamesTheInheritedLevelAndTheSpaceMute() {
        val changes = mutableListOf<OverrideChange>()
        var closed = false
        compose.setContent {
            CaperTheme {
                var page by remember { mutableStateOf(NotificationMenuPage.Main) }
                DropdownMenu(true, {}) {
                    ScopeMenuContent(page, { page = it }, "channel", null, "mentions", { changes += it }, { closed = true; page = NotificationMenuPage.Main },
                        spaceMuted = true)
                }
            }
        }

        compose.onNodeWithText("Muted with the space").assertIsDisplayed()
        compose.onNodeWithText("Default (Only @mentions)").assertIsDisplayed()
        compose.onNodeWithText("Mute channel").performClick()
        compose.onNodeWithText("For 8 hours").assertIsDisplayed()
        compose.onNodeWithContentDescription("Back").performClick()
        compose.onNodeWithText("Notifications").performClick()
        compose.onNodeWithText("All messages").assertIsDisplayed()
        compose.onNodeWithText("Only @mentions").assertIsDisplayed()
        compose.onNodeWithText("Nothing").performClick()
        compose.runOnIdle {
            assertEquals(listOf(OverrideChange.Level("nothing")), changes)
            assertTrue(closed)
        }
    }
}
