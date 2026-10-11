package chat.caper.android

import androidx.compose.ui.graphics.Color
import chat.caper.android.model.AppUiState
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.ui.TextMuted
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ComposerCounterTest {
    @Test fun `counter tones match the web thresholds`() {
        assertEquals(TextMuted, counterTone(3499))
        assertEquals(Color(0xFFE4C76A), counterTone(3500))
        assertEquals(Color(0xFFEDA361), counterTone(3750))
        assertEquals(Color(0xFFFF827C), counterTone(3900))
    }

    @Test fun `attach control explains checking, unavailable and full like every client`() {
        val ready = AppUiState(uploadsEnabled = true, uploadsChecked = true, chatAuthorId = "me")
        assertEquals("Checking file uploads…", attachDisabledReason(AppUiState(chatAuthorId = "me")))
        assertEquals("Checking file uploads…", attachDisabledReason(ready.copy(messagesLoading = true)))
        assertEquals("File uploads are unavailable", attachDisabledReason(ready.copy(uploadsEnabled = false)))
        assertEquals("File uploads are unavailable", attachDisabledReason(ready.copy(messagesError = "Failed")))
        val full = List(10) { DraftAttachmentUi("$it", "a.txt", "file") }
        assertEquals("You can attach up to 10 files.", attachDisabledReason(ready.copy(drafts = full)))
        assertNull(attachDisabledReason(ready))
    }
}
