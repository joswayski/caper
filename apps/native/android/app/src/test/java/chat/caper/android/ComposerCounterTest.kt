package chat.caper.android

import androidx.compose.ui.graphics.Color
import chat.caper.android.ui.TextMuted
import org.junit.Assert.assertEquals
import org.junit.Test

class ComposerCounterTest {
    @Test fun `counter tones match the web thresholds`() {
        assertEquals(TextMuted, counterTone(3499))
        assertEquals(Color(0xFFE4C76A), counterTone(3500))
        assertEquals(Color(0xFFEDA361), counterTone(3750))
        assertEquals(Color(0xFFFF827C), counterTone(3900))
    }
}
