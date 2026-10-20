package chat.caper.android

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import chat.caper.android.ui.CaperTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ComposerMediaPickerUiTest {
    @get:Rule val compose = createComposeRule()

    @Test fun oneButtonDefaultsToEmojisAndKeepsUnavailableMediaTabs() {
        compose.setContent {
            CaperTheme {
                EmojiInsertButton(true) {}
            }
        }
        compose.onNodeWithContentDescription("Emojis, GIFs and stickers").performClick()
        compose.onNodeWithText("Search emoji").assertIsDisplayed()
        compose.onNodeWithText("Emojis").assertIsSelected()
        compose.onNodeWithText("GIFs").performClick().assertIsSelected()
        compose.onNodeWithText("GIFs and stickers are unavailable right now.").assertIsDisplayed()
        compose.onNodeWithText("Stickers").performClick().assertIsSelected()
        compose.onNodeWithText("GIFs and stickers are unavailable right now.").assertIsDisplayed()
        compose.onNodeWithText("Emojis").performClick()
        compose.onNodeWithText("Search emoji").assertIsDisplayed()
        compose.onNodeWithText("Close").performClick()
        compose.onNodeWithText("Search emoji").assertDoesNotExist()
    }

    @Test fun disabledComposerDoesNotOpenPicker() {
        compose.setContent { CaperTheme { EmojiInsertButton(false) {} } }
        compose.onNodeWithContentDescription("Emojis, GIFs and stickers").assertIsNotEnabled().performClick()
        compose.onNodeWithText("Search emoji").assertDoesNotExist()
    }
}
