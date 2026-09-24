package chat.caper.android.voice

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

class VoiceMuteIntentParityTest {
    @Test fun `explicit unmute updates actual local and signaling state`() = runTest {
        var applied = true to true
        val local = VoiceLocalMute(Mutex(), {}) { muted, deafened -> applied = muted to deafened }
        local.setDeafened(true)
        local.setMuted(false)
        assertEquals(false to false, applied)
        assertFalse(local.muted)
        assertFalse(local.deafened)
        local.setDeafened(true)
        local.setMuted(true) // An already-muted toggle does not replace saved intent.
        local.setDeafened(false)
        assertEquals(false to false, applied)
    }

    @Test fun `deafen saves prior mute only once and restores it on exit`() {
        val intent = VoiceMuteIntent(true)
        intent.setDeafened(true)
        intent.setDeafened(true)
        intent.setDeafened(false)
        assertTrue(intent.muted)
        assertFalse(intent.deafened)
        intent.setMuted(false)
        intent.setDeafened(false)
        assertFalse(intent.muted)
    }

    @Test fun `explicit unmute while deafened clears deafen and saved state`() {
        val intent = VoiceMuteIntent(true)
        intent.setDeafened(true)
        intent.setMuted(false)
        assertFalse(intent.muted)
        assertFalse(intent.deafened)
        intent.setDeafened(true)
        intent.setDeafened(false)
        assertFalse(intent.muted)
    }
}
