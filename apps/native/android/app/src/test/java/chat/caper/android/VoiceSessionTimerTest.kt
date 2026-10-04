package chat.caper.android

import chat.caper.android.model.MediaSnapshot
import chat.caper.android.model.SpectatorSnapshot
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class VoiceSessionTimerTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test fun `snapshots remain compatible without session timestamp`() {
        assertNull(json.decodeFromString<MediaSnapshot>("""{"participants":[]}""").sessionStartedAt)
        assertNull(json.decodeFromString<SpectatorSnapshot>("""{"participants":[],"revision":1}""").sessionStartedAt)
        assertEquals(1234L, json.decodeFromString<MediaSnapshot>("""{"participants":[],"sessionStartedAt":1234}""").sessionStartedAt)
    }

    @Test fun `duration formatter clamps future and crosses hour boundary`() {
        assertEquals("00:00", formatVoiceSessionDuration(2_000, 1_000))
        assertEquals("00:59", formatVoiceSessionDuration(0, 59_999))
        assertEquals("01:00", formatVoiceSessionDuration(0, 60_000))
        assertEquals("59:59", formatVoiceSessionDuration(0, 3_599_999))
        assertEquals("1:00:00", formatVoiceSessionDuration(0, 3_600_000))
        assertEquals("12:03:04", formatVoiceSessionDuration(0, 43_384_000))
    }
}
