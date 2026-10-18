package chat.caper.android

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.data.PlatformRelease
import chat.caper.android.data.ReleaseHighlight
import chat.caper.android.data.WhatsNewSnapshot
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class WhatsNewUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun catchUpShowsNewestFirstAndDoneDismissesTheDisplayedSnapshot() {
        val snapshot = WhatsNewSnapshot(1, "android", "4".repeat(40), 4, listOf(2L, 4L).map {
            PlatformRelease(it, it.toString().repeat(40), "2026-10-11T19:00:00Z", listOf(
                ReleaseHighlight("TEST FIXTURE — Update $it", "Local release-notes UI preview."),
            ))
        }).validated("android", "4".repeat(40))
        var dismissed: Long? = null
        compose.setContent { CaperTheme {
            WhatsNewDialog(WhatsNewUi(), snapshot, true, {}, { dismissed = snapshot.sequence })
        } }
        val newer = compose.onNodeWithText("TEST FIXTURE — Update 4").assertIsDisplayed().fetchSemanticsNode().boundsInRoot
        val older = compose.onNodeWithText("TEST FIXTURE — Update 2").assertIsDisplayed().fetchSemanticsNode().boundsInRoot
        assertTrue(newer.top < older.top)
        compose.onAllNodesWithText("2026-10-11").assertCountEquals(1)
        compose.onNodeWithText("Got it").performClick()
        compose.runOnIdle { assertEquals(4L, dismissed) }
    }

    @Test fun settingsShowsHonestErrorAndRetry() {
        var retried = false
        compose.setContent { CaperTheme {
            WhatsNewDialog(WhatsNewUi(error = "TEST FIXTURE — unavailable"), null, false, { retried = true }, {})
        } }
        compose.onNodeWithText("TEST FIXTURE — unavailable").assertIsDisplayed()
        compose.onNodeWithText("Retry").performClick()
        compose.runOnIdle { assertTrue(retried) }
    }
}
