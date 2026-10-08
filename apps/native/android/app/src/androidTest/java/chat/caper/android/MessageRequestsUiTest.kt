package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.DirectConversation
import chat.caper.android.model.DirectPeer
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MessageRequestsUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun requestsRowExpandsAndBlockedRunsToggle() {
        // Labelled test data mirroring the parity fixture's request from Jordan.
        val request = DirectConversation("dm0000000003", DirectPeer("stranger0001", "jordan", "Jordan", 412), "1", "0", "incoming")
        var selected: DirectConversation? = null
        compose.setContent {
            CaperTheme {
                var open by androidx.compose.runtime.remember { mutableStateOf(false) }
                var revealed by androidx.compose.runtime.remember { mutableStateOf(false) }
                Column {
                    Text("Message requests UI test fixture")
                    MessageRequestsSection(listOf(request), null, open, { open = !open }) { selected = it }
                    BlockedRunRow(2, revealed) { revealed = !revealed }
                }
            }
        }

        compose.onNodeWithContentDescription("Message requests, 1").assertIsDisplayed()
        compose.onNodeWithText("@jordan").assertDoesNotExist()
        compose.onNodeWithContentDescription("Message requests, 1").performClick()
        compose.onNodeWithText("@jordan").assertIsDisplayed()
        compose.onNodeWithText("Jordan").performClick()
        compose.runOnIdle { assertEquals(request, selected) }

        compose.onNodeWithText("⊘ 2 blocked messages").assertIsDisplayed()
        compose.onNodeWithContentDescription("Show 2 blocked messages").performClick()
        compose.onNodeWithContentDescription("Hide 2 blocked messages").assertIsDisplayed()
    }
}
