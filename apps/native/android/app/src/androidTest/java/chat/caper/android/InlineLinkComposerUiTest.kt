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
import org.junit.Assert.assertTrue
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
        // The preview stays under the text row inside the editor's own frame.
        val frame = editor.fetchSemanticsNode().boundsInRoot
        val preview = compose.onNodeWithTag("preview", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertTrue(preview.height > 0f && preview.top > frame.top && preview.bottom <= frame.bottom)
    }

    /** Whole drafts typed one key at a time, closing markers and the spaces after them included
     *  (shared/messages/composer-cases.json, "a whole draft typed"): each ends exactly as typed. */
    @Test fun typingWholeDraftsKeyByKeyKeepsThemExactly() {
        val drafts = listOf("**hello** world", "_soft_ and more", "**both _builds_** and after", "**a** _b_ **c** d",
            "**Done**, then _this_: ok.", "**_both_** plain", "**see** https://caper.chat/x and on",
            "~~struck~~ and **b** ~~x~~ y", "*star* and **bold** after", "**café** ☕ ok",
            "**Release check** for _the voice beta_: see https://example.com/docs/42?tab=voice#s-3 and " +
                "www.example.org/specs/17. @maya can you confirm **both _builds_** and *the narrow layout*? " +
                "**Done** when _history loading_ and **link previews** look right.")
        val value = mutableStateOf(TextFieldValue(""))
        val key = mutableStateOf(0)
        compose.setContent {
            CaperTheme {
                androidx.compose.runtime.key(key.value) {
                    EmojiTextField(value.value, { value.value = it }, Modifier.testTag("draft"))
                }
            }
        }
        for (draft in drafts) {
            compose.runOnIdle { value.value = TextFieldValue(""); key.value++ }
            val editor = compose.onNodeWithTag("draft")
            editor.performClick()
            var at = 0
            while (at < draft.length) {
                val next = at + Character.charCount(draft.codePointAt(at))
                editor.performTextInput(draft.substring(at, next))
                at = next
            }
            compose.runOnIdle { assertEquals(draft, value.value.text) }
        }
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
