package chat.caper.android

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
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
                ReleaseHighlight("TEST FIXTURE — More in update $it", "Another change in the same release."),
            ), version = if (it == 4L) "0.1.112" else null)
        }).validated("android", "4".repeat(40))
        var dismissed: Long? = null
        var opened: String? = null
        val uriHandler = object : UriHandler { override fun openUri(uri: String) { opened = uri } }
        compose.setContent { CompositionLocalProvider(LocalUriHandler provides uriHandler) { CaperTheme {
            WhatsNewDialog(WhatsNewUi(), snapshot, true, {}, { dismissed = snapshot.sequence })
        }
        } }
        val newer = compose.onNodeWithText("TEST FIXTURE — Update 4").assertIsDisplayed().fetchSemanticsNode().boundsInRoot
        val older = compose.onNodeWithText("TEST FIXTURE — Update 2").fetchSemanticsNode().boundsInRoot
        assertTrue(newer.top < older.top)
        compose.onAllNodesWithText("October 11, 2026").assertCountEquals(1)
        compose.onAllNodesWithText("Version 0.1.112").assertCountEquals(1)
        compose.onAllNodesWithText("2222222").assertCountEquals(1)
        assertTrue(compose.onNodeWithText("Version 0.1.112").fetchSemanticsNode().boundsInRoot.top < newer.top)
        compose.onNodeWithText("Version 0.1.112").performTouchInput { click() }
        compose.runOnIdle { assertEquals("https://github.com/joswayski/caper/commit/" + "4".repeat(40), opened) }
        compose.onNodeWithText("2222222").performScrollTo().performTouchInput { click() }
        compose.runOnIdle { assertEquals("https://github.com/joswayski/caper/commit/" + "2".repeat(40), opened) }
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
