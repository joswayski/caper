package chat.caper.android.voice

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MicrophoneRecoveryTest {
    @Test fun `the first loss reopens the input`() {
        assertTrue(MicrophoneRecovery().mayReopen(nowMs = 5_000))
    }

    @Test fun `a reopened input lost within ten seconds is not reopened again`() {
        val recovery = MicrophoneRecovery()
        recovery.reopened(nowMs = 5_000)
        assertFalse(recovery.mayReopen(nowMs = 5_000))
        assertFalse(recovery.mayReopen(nowMs = 14_999))
        assertTrue("it lasted the whole window", recovery.mayReopen(nowMs = 15_000))
    }

    @Test fun `each reopen starts its own window`() {
        val recovery = MicrophoneRecovery()
        recovery.reopened(nowMs = 0)
        recovery.reopened(nowMs = 20_000)
        assertFalse(recovery.mayReopen(nowMs = 25_000))
        assertTrue(recovery.mayReopen(nowMs = 30_000))
    }

    @Test fun `an input is gone once it has delivered no audio for three seconds`() {
        assertFalse(MicrophoneRecovery.stalled(capturedAtMs = 1_000, nowMs = 1_010))
        assertFalse(MicrophoneRecovery.stalled(capturedAtMs = 1_000, nowMs = 3_999))
        assertTrue(MicrophoneRecovery.stalled(capturedAtMs = 1_000, nowMs = 4_000))
    }
}
