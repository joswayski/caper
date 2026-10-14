package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.Member
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class InvitePeopleUiTest {
    @get:Rule val compose = createComposeRule()
    private val member = Member("member000001", "mira", "Mira", false)

    @Test fun inviteOnlyKeepsUsernameInvitationsWithoutCurrentMemberRemoval() {
        var invited: String? = null
        compose.setContent {
            CaperTheme {
                Column {
                    InviteManager(listOf(member), emptyList(), false,
                        { username, done, _ -> invited = username; done() },
                        { error("Invite-only must not remove members") }, {}, false, emptyList(), inviteOnly = true)
                }
            }
        }
        compose.onNodeWithText("Members").assertDoesNotExist()
        compose.onNodeWithText("Mira").assertDoesNotExist()
        compose.onNodeWithText("Remove").assertDoesNotExist()
        compose.onNodeWithText("Exact username").performTextInput("jordan")
        compose.onNode(hasText("Invite") and hasClickAction()).performClick()
        compose.runOnIdle { assertEquals("jordan", invited) }
        compose.onNodeWithText("Invitation sent. They must accept before joining.").assertExists()
    }

    @Test fun managementStillOffersCurrentMemberRemoval() {
        var removed: Member? = null
        compose.setContent {
            CaperTheme {
                Column {
                    InviteManager(listOf(member), emptyList(), false, { _, _, _ -> },
                        { removed = it }, {}, false, emptyList())
                }
            }
        }
        compose.onNodeWithText("Members").assertExists()
        compose.onNodeWithText("Mira").assertExists()
        compose.onNodeWithText("Remove").performClick()
        compose.runOnIdle { assertEquals(member, removed) }
    }
}
