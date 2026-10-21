package chat.caper.android.voice

import org.junit.Assert.*
import org.junit.Test

class VoiceNotePcmTest {
    @Test fun `peak is the loudest magnitude on getMaxAmplitude's scale`() {
        assertEquals(0, VoiceNotePcm.peak(shortArrayOf(), 0))
        assertEquals(1200, VoiceNotePcm.peak(shortArrayOf(10, -1200, 900), 3))
        // -32768 has no positive twin; the meter's scale tops out at 32767.
        assertEquals(32767, VoiceNotePcm.peak(shortArrayOf(Short.MIN_VALUE), 1))
        // Only the samples read count, not stale ones after them.
        assertEquals(5, VoiceNotePcm.peak(shortArrayOf(5, 30000), 1))
    }

    @Test fun `input gain alone scales and clamps audio recorded without the processor`() {
        val samples = shortArrayOf(1000, -1000, 30000, -30000)
        VoiceNotePcm.applyGain(samples, samples.size, 100)
        assertArrayEquals(shortArrayOf(1000, -1000, 30000, -30000), samples)
        VoiceNotePcm.applyGain(samples, samples.size, 200)
        assertArrayEquals(shortArrayOf(2000, -2000, Short.MAX_VALUE, Short.MIN_VALUE), samples)
        VoiceNotePcm.applyGain(samples, 2, 50)
        assertArrayEquals(shortArrayOf(1000, -1000, Short.MAX_VALUE, Short.MIN_VALUE), samples)
        VoiceNotePcm.applyGain(samples, samples.size, 0)
        assertArrayEquals(ShortArray(4), samples)
    }

    @Test fun `the envelope keeps the loudest sample per slot of audio, partial last slot included`() {
        val envelope = PeakEnvelope(slot = 3)
        assertArrayEquals(intArrayOf(), envelope.values())
        envelope.add(shortArrayOf(1, -7, 3, 2), 4)
        assertArrayEquals(intArrayOf(7, 2), envelope.values())
        // Slots span calls, and only [count] samples are read.
        envelope.add(shortArrayOf(9, Short.MIN_VALUE, 4, 100), 2)
        assertArrayEquals(intArrayOf(7, 32767), envelope.values())
        envelope.add(ShortArray(1200), 1200)
        assertEquals(2 + 400, envelope.values().size)
    }

    @Test fun `processed recording is used when it starts`() {
        var reason: Throwable? = null
        val started = startWithFallback({ "processed" }, { error("plain must not start") }) { reason = it }
        assertEquals("processed", started)
        assertNull(reason)
    }

    @Test fun `a processed recording that cannot start falls back to the plain recorder and says why`() {
        val failure = UnsatisfiedLinkError("caper_audio")
        var reason: Throwable? = null
        val started = startWithFallback({ throw failure }, { "plain" }) { reason = it }
        assertEquals("plain", started)
        assertSame(failure, reason)
    }

    @Test fun `when both recorders fail the plain recorder's error reaches the caller`() {
        val plain = IllegalStateException("Microphone unavailable.")
        val thrown = assertThrows(IllegalStateException::class.java) {
            startWithFallback<String>({ throw RuntimeException("processed") }, { throw plain }) {}
        }
        assertSame(plain, thrown)
    }
}
