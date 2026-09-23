package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class CallAttemptGateTest {
    @Test fun `late result from old call cannot update replacement call`() {
        val gate = CallAttemptGate()
        val oldCall = Any()
        val oldAttempt = gate.begin()
        gate.end()
        val newCall = Any()
        val newAttempt = gate.begin()

        assertFalse(callResultIsCurrent(oldCall, newCall, oldAttempt, gate))
        assertFalse(callResultIsCurrent(oldCall, oldCall, oldAttempt, gate))
        assertTrue(callResultIsCurrent(newCall, newCall, newAttempt, gate))
    }

    @Test fun `unmapped and locally muted tracks are never enabled`() {
        assertEquals(RemoteAudioPreference(0.0, false), remoteAudioPreference(false, false, false, 200, 200))
        assertEquals(RemoteAudioPreference(2.0, false), remoteAudioPreference(true, false, true, 200, 100))
        assertEquals(RemoteAudioPreference(1.5, true), remoteAudioPreference(true, false, false, 150, 100))
        assertFalse(remoteAudioPreference(true, true, false, 100, 100).enabled)
    }

    @Test fun `undeafen restores prior mute intent`() {
        val intent = VoiceMuteIntent(initiallyMuted = false)
        intent.setDeafened(true)
        assertTrue(intent.muted)
        intent.setDeafened(false)
        assertFalse(intent.muted)

        intent.setMuted(true)
        intent.setDeafened(true)
        intent.setDeafened(false)
        assertTrue(intent.muted)
    }

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
