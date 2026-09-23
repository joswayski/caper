package chat.caper.android.voice

import chat.caper.android.data.ApiException
import java.io.IOException
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceControlRecoveryTest {
    @Test fun `bounded recovery retries outages but fails closed for revocation and bad requests`() {
        assertTrue(transientVoiceControlError(IOException("network changed")))
        assertTrue(transientVoiceControlError(ApiException(503, "unavailable")))
        assertTrue(transientVoiceControlError(ApiException(429, "slow down")))
        assertFalse(transientVoiceControlError(ApiException(401, "revoked")))
        assertFalse(transientVoiceControlError(ApiException(403, "revoked")))
        assertFalse(transientVoiceControlError(ApiException(400, "invalid")))
    }
}
