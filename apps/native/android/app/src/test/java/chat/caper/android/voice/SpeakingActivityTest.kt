package chat.caper.android.voice

import org.junit.Assert.assertEquals
import org.junit.Test

class SpeakingActivityTest {
    @Test fun speechThresholdReleaseAndMuteMatchWeb() {
        val lastLoud = mutableMapOf<String, Long>()
        assertEquals(setOf("maya"), SpeakingActivity.update(mapOf("maya" to 0.005, "alex" to 0.003), emptySet(), lastLoud, 1_000))
        assertEquals(setOf("maya"), SpeakingActivity.update(mapOf("maya" to 0.0), emptySet(), lastLoud, 1_150))
        assertEquals(emptySet<String>(), SpeakingActivity.update(mapOf("maya" to 0.0), emptySet(), lastLoud, 1_200))
        assertEquals(emptySet<String>(), SpeakingActivity.update(mapOf("maya" to 0.5), setOf("maya"), lastLoud, 2_000))
        SpeakingActivity.update(mapOf("alex" to 0.5), emptySet(), lastLoud, 3_000)
        assertEquals(emptySet<String>(), SpeakingActivity.update(emptyMap(), setOf("alex"), lastLoud, 3_050))
    }
}
