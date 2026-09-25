package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class MicComparisonBindingTest {
    @Test fun `old dialog cannot finish or resume replacement call even with reused attempt number`() {
        val oldService = Any()
        val replacement = Any()
        assertTrue(comparisonContextMatches(oldService, 7, oldService, 7))
        assertFalse(comparisonContextMatches(oldService, 7, replacement, 7))
        assertFalse(comparisonContextMatches(oldService, 7, oldService, 8))
        assertFalse(comparisonContextMatches(oldService, 7, null, null))
    }
}
