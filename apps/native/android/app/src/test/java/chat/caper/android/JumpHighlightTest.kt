package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Test

class JumpHighlightTest {
    @Test fun holdsForASecondThenEasesOutOverFour() {
        assertEquals(1f, jumpHighlightStrength(0), 0f)
        assertEquals(1f, jumpHighlightStrength(1_000), 0f)
        // (1 - t)², t = (elapsed - 1s) / 4s: quick at first, settling gently, as every client.
        assertEquals(0.5625f, jumpHighlightStrength(2_000), 1e-6f)
        assertEquals(0.25f, jumpHighlightStrength(3_000), 1e-6f)
        assertEquals(0.0625f, jumpHighlightStrength(4_000), 1e-6f)
        assertEquals(0f, jumpHighlightStrength(5_000), 0f)
        assertEquals(0f, jumpHighlightStrength(9_000), 0f)
    }
}
