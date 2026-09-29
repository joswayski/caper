package chat.caper.android

import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatDateDividerTest {
    private val losAngeles = ZoneId.of("America/Los_Angeles")

    @Test fun `local day boundaries use the device zone`() {
        assertTrue(sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T06:45:00Z", losAngeles))
        assertFalse(sameLocalDay("2026-09-29T06:30:00Z", "2026-09-29T07:30:00Z", losAngeles))
    }

    @Test fun `full date is localized`() {
        assertEquals("Monday, September 28, 2026", fullDateLabel("2026-09-29T06:30:00Z", losAngeles, Locale.US))
    }
}
