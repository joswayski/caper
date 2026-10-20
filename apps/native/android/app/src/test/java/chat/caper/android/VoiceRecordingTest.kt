package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class VoiceRecordingTest {
    @Test fun `the mic explains why it cannot record`() {
        assertNull(voiceRecordBlockedReason(pendingMessage = false, inVoiceCall = false, sdk = 26))
        assertNull("Android 10+ shares the microphone with a call", voiceRecordBlockedReason(pendingMessage = false, inVoiceCall = true, sdk = 29))
        assertEquals(
            "Leave the voice channel to record a voice message on this Android version.",
            voiceRecordBlockedReason(pendingMessage = false, inVoiceCall = true, sdk = 28),
        )
        assertEquals("Wait for your last message to send, then record.", voiceRecordBlockedReason(pendingMessage = true, inVoiceCall = false, sdk = 36))
    }
}
