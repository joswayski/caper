package chat.caper.android

import android.graphics.Bitmap
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.model.Account
import chat.caper.android.ui.Blackout
import chat.caper.android.ui.CaperTheme
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ProfileUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun onboardingAndExistingAccountUseTheirWebActions() {
        val account = mutableStateOf(Account("new-account"))
        val busy = mutableStateOf(false)
        var submitted: Pair<String, String>? = null
        compose.setContent {
            CaperTheme {
                Scaffold(containerColor = Blackout) { padding ->
                    Box(Modifier.padding(padding)) {
                        ProfileScreen(account.value, busy.value, null, close = if (account.value.username == null) null else ({})) { username, name ->
                            submitted = username to name
                        }
                    }
                }
            }
        }
        compose.onNodeWithText("Choose how you show up.").assertExists()
        compose.onNodeWithText("Create your profile").assertDoesNotExist()
        compose.onNodeWithText("Save profile").assertDoesNotExist()
        compose.onNodeWithText("Finish account").assertIsNotEnabled()
        compose.onNodeWithText("Username").performTextInput("fixture_new")
        compose.onNodeWithText("Display name").performTextInput("Fixture New")
        compose.onNodeWithText("Finish account").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals("fixture_new" to "Fixture New", submitted) }

        val directory = File(requireNotNull(InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")))
        check(directory.mkdirs() || directory.isDirectory)
        val screenshot = compose.onRoot().captureToImage().asAndroidBitmap()
        try {
            File(directory, "profile-onboarding-test-fixture.png").outputStream().use { output ->
                check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, output))
            }
        } finally { screenshot.recycle() }

        compose.runOnIdle { busy.value = true }
        compose.onNodeWithText("Saving…").assertIsNotEnabled()
        compose.onNodeWithText("Finish account").assertDoesNotExist()
        compose.runOnIdle {
            busy.value = false
            account.value = Account("existing-account", "fixture_owner", "Fixture Owner")
        }
        compose.onNodeWithText("Edit profile").assertExists()
        compose.onNodeWithText("Finish account").assertDoesNotExist()
        compose.onNodeWithText("Save profile").assertIsEnabled().performClick()
        compose.runOnIdle { assertEquals("fixture_owner" to "Fixture Owner", submitted) }
    }
}
