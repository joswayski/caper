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

    @Test fun linkOptionsWarnAndDispatchWithoutSpaceAdministration() {
        var regenerated = 0
        var settings = 0
        var deactivated = 0
        compose.setContent {
            CaperTheme { InviteLinkActions(true, { regenerated++ }, { settings++ }, { deactivated++ }) }
        }
        compose.onNodeWithText("Regenerate link").assertDoesNotExist()
        compose.onNodeWithContentDescription("More link options").performClick()
        compose.onNodeWithText("Previous link stops working. Resets to 30 days / 100 joins.").assertExists()
        listOf("Rename", "Delete space", "Manage space").forEach { compose.onNodeWithText(it).assertDoesNotExist() }
        compose.onNodeWithText("Regenerate link").performClick()
        compose.runOnIdle { assertEquals(1, regenerated); assertEquals(0, deactivated) }
        compose.onNodeWithContentDescription("More link options").performClick()
        compose.onNodeWithText("Link settings").performClick()
        compose.runOnIdle { assertEquals(1, settings) }
        compose.onNodeWithContentDescription("More link options").performClick()
        compose.onNodeWithText("Deactivate link").performClick()
        compose.runOnIdle { assertEquals(1, deactivated) }
    }

    @Test fun busyLinkOptionsCannotStartAnotherMutation() {
        compose.setContent {
            CaperTheme { InviteLinkActions(false, { error("busy") }, { error("busy") }, { error("busy") }) }
        }
        compose.onNodeWithContentDescription("More link options").assertIsNotEnabled()
        compose.onNodeWithText("Regenerate link").assertDoesNotExist()
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
