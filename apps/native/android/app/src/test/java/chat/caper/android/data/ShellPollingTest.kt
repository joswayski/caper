package chat.caper.android.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ShellPollingTest {
    @Test fun `timer polls slow to a minute while live notifications arrive`() {
        assertEquals(60_000L, shellPollIntervalMs(notificationsLive = true))
        assertEquals(15_000L, shellPollIntervalMs(notificationsLive = false))
    }

    @Test fun `an older DM list reply never replaces a newer one`() {
        val requests = LatestRequest()
        val older = requests.next()
        val newer = requests.next()
        // The newer request answers first and applies; the older one, answering late, is dropped.
        assertTrue(requests.isLatest(newer))
        assertFalse(requests.isLatest(older))
    }
}
