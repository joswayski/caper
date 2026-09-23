package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class CallAttemptGateTest {
    @Test fun `logout stop prevents an in-flight join from becoming current`() {
        val gate = CallAttemptGate()
        val joining = gate.begin()
        gate.end()
        assertFalse(gate.isCurrent(joining))
    }

    @Test fun `stop invalidates an established attempt before replacement starts`() {
        val gate = CallAttemptGate()
        val established = gate.begin()
        assertTrue(gate.isCurrent(established))
        gate.end()
        assertFalse(gate.isCurrent(established))
        assertTrue(gate.isCurrent(gate.begin()))
    }
}
