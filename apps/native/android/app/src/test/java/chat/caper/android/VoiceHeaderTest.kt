package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Test

class VoiceHeaderTest {
    @Test fun `phone subtitle counts members and people in voice, omitting what does not apply`() {
        assertEquals("12 members · 3 in voice", headerSubtitle(12, 3))
        assertEquals("1 member", headerSubtitle(1, 0))
        assertEquals("2 in voice", headerSubtitle(null, 2))
        assertEquals("", headerSubtitle(null, 0))
    }
}
