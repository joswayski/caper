package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class CapturePrivacyGateTest {
    @Test fun `read crossing mute and reopen is private even when callback starts after reopen`() {
        val gate = CapturePrivacyGate()
        gate.publish(true, 175)
        assertEquals(0, gate.captureEpoch())
        assertTrue(gate.mayPublish(gate.captureEpoch(), 175, true))
        // AudioRecord.read starts now. A complete private interval happens
        // before its post-read callback; sampling only current state leaks it.
        gate.publish(false, 175)
        gate.publish(true, 175)
        val delayed = gate.captureEpoch()
        assertEquals(0, delayed)
        assertFalse(gate.mayPublish(delayed, 175, true))
        val fresh = gate.captureEpoch()
        assertTrue(gate.mayPublish(fresh, 175, true))
        gate.changedGain(175, 0)
        gate.changedGain(0, 125)
        assertEquals(0, gate.captureEpoch())
        assertTrue(gate.mayPublish(gate.captureEpoch(), 125, true))
    }

    @Test fun `mute comparison and reopened route cannot publish queued private audio`() {
        val gate = CapturePrivacyGate()
        gate.publish(true, 175)
        val first = gate.epoch()
        assertTrue(gate.mayPublish(first, 175, true))
        gate.publish(false, 175)
        assertFalse(gate.mayPublish(first, 175, true))
        val privateEpoch = gate.epoch()
        gate.publish(true, 175)
        assertFalse(gate.mayPublish(privateEpoch, 175, true))
        assertFalse(gate.mayPublish(gate.epoch(), 175, false))
        val beforeRoute = gate.epoch()
        gate.publish(true, 175)
        assertFalse(gate.mayPublish(beforeRoute, 175, true))
    }

    @Test fun `zero gain clears publication and restoration cannot leak old gain regime`() {
        val gate = CapturePrivacyGate()
        gate.publish(true, 150)
        val beforeZero = gate.epoch()
        gate.changedGain(150, 0)
        assertFalse(gate.mayPublish(beforeZero, 0, true))
        gate.changedGain(0, 60)
        assertFalse(gate.mayPublish(beforeZero, 60, true))
        assertTrue(gate.mayPublish(gate.epoch(), 60, true))
    }
}
