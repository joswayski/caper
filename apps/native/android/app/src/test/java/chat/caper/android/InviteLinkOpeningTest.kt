package chat.caper.android

import chat.caper.android.model.Account
import chat.caper.android.model.AppUiState
import chat.caper.android.model.SessionScreen
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InviteLinkOpeningTest {
    private val finished = Account("account00001", "mira", "Mira")

    @Test fun `opened links show once someone with a finished profile is home`() {
        assertTrue(opensInviteLinks(AppUiState(screen = SessionScreen.Home, account = finished)))
        assertTrue(opensInviteLinks(AppUiState(screen = SessionScreen.Spaces(finished), account = finished)))
    }

    @Test fun `opened links wait through sign-in, profile setup and launch loading`() {
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Loading)))
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.SignedOut)))
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Verify("challenge001", "mira@example.com"))))
        val unfinished = Account("account00001")
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Profile(unfinished), account = unfinished)))
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Home, account = finished.copy(displayName = null))))
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Home, account = finished, restoring = true)))
        assertFalse(opensInviteLinks(AppUiState(screen = SessionScreen.Home)))
    }
}
