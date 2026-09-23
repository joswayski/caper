package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class CallAttemptGateTest {
    @Test fun `space and channel revocation only match active call context`() {
        val active = VoiceState(
            phase = VoiceState.Phase.CONNECTED,
            channelId = "active-channel",
            spaceId = "active-space",
        )
        assertTrue(active.belongsToSpace("active-space"))
        assertTrue(active.belongsToChannel("active-channel"))
        assertFalse("deleting browsed space must preserve another active call", active.belongsToSpace("browsed-space"))
        assertFalse("revoking browsed channel must preserve another active call", active.belongsToChannel("browsed-channel"))
        assertFalse(VoiceState().belongsToSpace("active-space"))
    }

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
