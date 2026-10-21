package chat.caper.android

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.model.ComposerFocusRequest
import chat.caper.android.ui.CaperTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ReplyFocusUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun explicitRequestFocusesOncePreservesDraftAndCanRepeat() {
        var request by mutableStateOf<ComposerFocusRequest?>(null)
        var enabled by mutableStateOf(false)
        var consumed = 0
        var clearFocus: () -> Unit = {}
        compose.setContent {
            val focus = LocalFocusManager.current
            clearFocus = { focus.clearFocus(force = true) }
            var draft by remember { mutableStateOf(TextFieldValue("Existing draft")) }
            CaperTheme {
                Column {
                    SuggestingComposer(draft, { draft = it }, MentionSource(emptyList(), false), "root", {},
                        focusRequest = request, focusEnabled = enabled,
                        consumeFocus = { consumed++; request = null }) { field, change, actions ->
                        OutlinedTextField(draft, change, field, label = { Text("Reply") }, enabled = enabled, keyboardActions = actions)
                    }
                }
            }
        }
        compose.onNodeWithText("Existing draft").assertIsNotFocused()
        compose.runOnIdle { request = ComposerFocusRequest("channel", "root", "first") }
        compose.runOnIdle { assertEquals(0, consumed); enabled = true }
        compose.waitUntil { consumed == 1 }
        compose.onNodeWithText("Existing draft").assertIsFocused()
        compose.runOnIdle { clearFocus() }
        compose.onNodeWithText("Existing draft").assertIsNotFocused()
        compose.runOnIdle { request = ComposerFocusRequest("channel", "root", "second") }
        compose.waitUntil { consumed == 2 }
        compose.onNodeWithText("Existing draft").assertIsFocused()
    }
}
