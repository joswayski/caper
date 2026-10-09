package chat.caper.android

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import chat.caper.android.ui.TextMuted
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class ComposerCounterTest {
    @Test fun `counter tones match the web thresholds`() {
        assertEquals(TextMuted, counterTone(3499))
        assertEquals(Color(0xFFE4C76A), counterTone(3500))
        assertEquals(Color(0xFFEDA361), counterTone(3750))
        assertEquals(Color(0xFFFF827C), counterTone(3900))
    }

    @Test fun `draft cap keeps whole code points and a valid selection`() {
        val under = TextFieldValue("ab", TextRange(1))
        assertSame(under, capComposerDraft(under, limit = 2))
        val capped = capComposerDraft(TextFieldValue("ab😀c", TextRange(1, 5), TextRange(0, 2)), limit = 3)
        assertEquals("ab😀", capped.text)
        assertEquals(TextRange(1, 4), capped.selection)
        assertNull(capped.composition)
        assertEquals("ab", capComposerDraft(TextFieldValue("ab😀"), limit = 2).text)
    }
}
