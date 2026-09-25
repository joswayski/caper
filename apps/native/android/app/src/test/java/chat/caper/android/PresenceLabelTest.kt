package chat.caper.android

import org.junit.Assert.assertEquals
import org.junit.Test

class PresenceLabelTest {
    @Test fun `labels match the web PresenceDot`() {
        assertEquals("Online", presenceLabel("online", live = true))
        assertEquals("Idle", presenceLabel("idle", live = true))
        assertEquals("Offline (last known; reconnecting)", presenceLabel("offline", live = false))
        assertEquals("Status unavailable", presenceLabel(null, live = true))
    }
}
