package chat.caper.android.voice

import chat.caper.android.model.Participant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class VoiceSoundTest {
    private fun call(phase: VoiceState.Phase, ids: List<String> = listOf("me"), deafened: Boolean = false, self: String? = "me") =
        VoiceState(phase = phase, selfId = self, deafened = deafened, participants = ids.map { Participant(it, it, muted = false, deafened = false, tracks = emptyList()) })

    @Test fun `chimes for your own join and leave, but not a cancelled join or a reconnect`() {
        assertEquals(VoiceSound.JOIN, voiceSound(call(VoiceState.Phase.CONNECTING, emptyList()), call(VoiceState.Phase.CONNECTED)))
        assertEquals(VoiceSound.LEAVE, voiceSound(call(VoiceState.Phase.CONNECTED), call(VoiceState.Phase.IDLE, emptyList(), self = null)))
        assertEquals(VoiceSound.LEAVE, voiceSound(call(VoiceState.Phase.RECONNECTING), call(VoiceState.Phase.FAILED)))
        assertNull(voiceSound(call(VoiceState.Phase.CONNECTING, emptyList()), call(VoiceState.Phase.IDLE, emptyList())))
        assertNull(voiceSound(call(VoiceState.Phase.CONNECTED), call(VoiceState.Phase.RECONNECTING)))
        assertNull(voiceSound(call(VoiceState.Phase.RECONNECTING), call(VoiceState.Phase.CONNECTED, listOf("me2"), self = "me2")))
    }

    @Test fun `chimes as others come and go, unless you are deafened`() {
        assertEquals(VoiceSound.JOIN, voiceSound(call(VoiceState.Phase.CONNECTED), call(VoiceState.Phase.CONNECTED, listOf("me", "ana"))))
        assertEquals(VoiceSound.LEAVE, voiceSound(call(VoiceState.Phase.CONNECTED, listOf("me", "ana")), call(VoiceState.Phase.CONNECTED)))
        assertNull(voiceSound(call(VoiceState.Phase.CONNECTED, listOf("me", "ana")), call(VoiceState.Phase.CONNECTED, listOf("me", "ana"))))
        assertNull(voiceSound(call(VoiceState.Phase.CONNECTED), call(VoiceState.Phase.CONNECTED, listOf("me", "ana"), deafened = true)))
    }
}
