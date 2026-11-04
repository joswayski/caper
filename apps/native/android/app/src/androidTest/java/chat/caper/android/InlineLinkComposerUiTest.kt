package chat.caper.android

import android.content.ClipboardManager
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import chat.caper.android.ui.CaperTheme
import chat.caper.android.ui.EmojiTextField
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Real Compose editor regressions; requires an Android device, not JVM mocks. */
@RunWith(AndroidJUnit4::class)
@OptIn(ExperimentalTestApi::class)
class InlineLinkComposerUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun formattedLinksCopyVisibleTextAndReplaceSelectionInSourceCoordinates() {
        val source = "🙂 **https://Example.org/Path?q=Case** _@maya_ www.b.org"
        val value = mutableStateOf(TextFieldValue(source))
        compose.setContent {
            CaperTheme {
                EmojiTextField(value.value, { value.value = it }, Modifier.testTag("draft"),
                    visualTransformation = MentionVisualTransformation(MentionSource(listOf(MentionCandidate("maya", "Maya")), true)),
                    below = { androidx.compose.material3.Text("Link preview fixture", Modifier.testTag("preview")) })
            }
        }
        val editor = compose.onNodeWithTag("draft")
        editor.performClick().performTextInputSelection(TextRange(0, source.length))
        editor.performSemanticsAction(SemanticsActions.CopyText) { it() }
        compose.runOnIdle {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            assertEquals("🙂 https://Example.org/Path?q=Case @maya www.b.org",
                context.getSystemService(ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString())
        }
        val start = source.indexOf("Example")
        editor.performTextInputSelection(TextRange(start, start + "Example".length))
        editor.performTextInput("Caper")
        compose.runOnIdle {
            assertEquals(source.replace("Example", "Caper"), value.value.text)
            assertEquals(TextRange(start + 5), value.value.selection)
            assertFalse(value.value.text.contains(DraftLinkPrefix))
        }
        compose.onNodeWithTag("preview").assertIsDisplayed()
    }

    @Test fun defaultTransformationKeepsFormattingAndNativeUndoAtUrlBoundary() {
        val source = "**https://a.co** www.b.org"
        val value = mutableStateOf(TextFieldValue(source))
        compose.setContent {
            CaperTheme {
                EmojiTextField(value.value, { value.value = it }, Modifier.testTag("draft"))
            }
        }
        val editor = compose.onNodeWithTag("draft")
        editor.performClick().performTextInputSelection(TextRange(3))
        editor.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle {
            assertEquals("**ttps://a.co** www.b.org", value.value.text)
            assertEquals(TextRange(2), value.value.selection)
        }
        editor.performKeyInput { keyDown(Key.CtrlLeft); pressKey(Key.Z); keyUp(Key.CtrlLeft) }
        compose.runOnIdle { assertEquals(source, value.value.text) }
    }
}
