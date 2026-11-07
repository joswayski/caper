package chat.caper.android

import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.data.RESENT_CODE_STATUS
import chat.caper.android.model.SessionScreen
import chat.caper.android.ui.CaperTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** What TalkBack hears on sign-in, as web's role="alert", role="status", <h1> and <label for>. */
@RunWith(AndroidJUnit4::class)
class SignInUiTest {
    @get:Rule val compose = createComposeRule()

    private fun announced(text: String, mode: LiveRegionMode) =
        hasText(text) and SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, mode)

    @Test fun emailStepNamesItsFieldAndAnnouncesItsErrorAtOnce() {
        compose.setContent { CaperTheme { LoginScreen(busy = false, error = "Sign-in is temporarily unavailable.", clearError = {}, submit = {}) } }
        compose.onNodeWithText("Welcome to Caper").assert(isHeading())
        compose.onNode(hasSetTextAction() and hasContentDescription("Email address")).assertExists()
        compose.onNode(announced("Sign-in is temporarily unavailable.", LiveRegionMode.Assertive)).assertExists()
    }

    @Test fun codeStepAnnouncesItsErrorAtOnceAndTheLastAttemptPolitely() {
        compose.setContent { CaperTheme {
            VerifyScreen(SessionScreen.Verify("challenge", "owner@example.test", attemptsRemaining = 1, resends = 1), busy = false,
                error = "That code is incorrect or expired.", clearError = {}, back = {}, submit = { _, _ -> }, resend = {})
        } }
        compose.onNodeWithText("Check your email.").assert(isHeading())
        compose.onNode(hasSetTextAction() and hasContentDescription("Sign-in code")).assertExists()
        compose.onNode(announced("That code is incorrect or expired.", LiveRegionMode.Assertive)).assertExists()
        compose.onNode(announced("One attempt left. Check the code carefully.", LiveRegionMode.Polite)).assertExists()
        // Web shows the new-code notice only while no error is shown.
        compose.onNodeWithText(RESENT_CODE_STATUS).assertDoesNotExist()
    }

    @Test fun aResentCodeIsAnnouncedPolitely() {
        compose.setContent { CaperTheme {
            VerifyScreen(SessionScreen.Verify("challenge", "owner@example.test", resends = 1), busy = false,
                error = null, clearError = {}, back = {}, submit = { _, _ -> }, resend = {})
        } }
        compose.onNode(announced(RESENT_CODE_STATUS, LiveRegionMode.Polite)).assertExists()
    }

    @Test fun noErrorMeansNoAlert() {
        compose.setContent { CaperTheme { LoginScreen(busy = false, error = null, clearError = {}, submit = {}) } }
        compose.onNode(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Assertive)).assertDoesNotExist()
    }
}
