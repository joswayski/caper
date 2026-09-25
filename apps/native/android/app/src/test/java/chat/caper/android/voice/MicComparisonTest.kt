package chat.caper.android.voice

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.*
import org.junit.Test

class MicComparisonTest {
    @Test fun `natural gain and enhanced capture remain distinct and bounded`() {
        val recording = MicComparison(3)
        val raw = ByteBuffer.allocateDirect(8).order(ByteOrder.LITTLE_ENDIAN)
        listOf(1000, -2000, 10000, 4000).forEachIndexed { index, sample -> raw.putShort(index * 2, sample.toShort()) }
        recording.appendNatural(raw, 4, 150)
        listOf(250, -600, 3600, 5500).forEachIndexed { index, sample -> raw.putShort(index * 2, sample.toShort()) }
        recording.appendEnhanced(raw, 4, true)
        assertArrayEquals(shortArrayOf(1500, -3000, 15000), recording.samples(false))
        assertArrayEquals(shortArrayOf(250, -600, 3600), recording.samples(true))
        assertEquals(3, recording.frames)
        assertArrayEquals(shortArrayOf(0, 0, 0), recording.playbackSamples(false, 0))
        assertArrayEquals(shortArrayOf(3000, -6000, 30000), recording.playbackSamples(false, 200))
        assertArrayEquals(shortArrayOf(125, -300, 1800), recording.playbackSamples(true, 50))
    }

    @Test fun `failed model gives silent enhanced comparison rather than raw replay`() {
        val recording = MicComparison(2)
        val buffer = ByteBuffer.allocateDirect(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putShort(0, 1234); buffer.putShort(2, (-4321).toShort())
        recording.appendNatural(buffer, 2, 100)
        recording.appendEnhanced(buffer, 2, false)
        assertArrayEquals(shortArrayOf(1234, -4321), recording.samples(false))
        assertArrayEquals(shortArrayOf(0, 0), recording.samples(true))
    }

    @Test fun `replay gain above unity saturates rather than wrapping`() {
        val recording = MicComparison(2)
        val buffer = ByteBuffer.allocateDirect(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putShort(0, 20000); buffer.putShort(2, (-20000).toShort())
        recording.appendNatural(buffer, 2, 100)
        recording.appendEnhanced(buffer, 2, true)
        assertArrayEquals(shortArrayOf(32767, -32768), recording.playbackSamples(false, 200))
    }

    @Test fun `silence detection matches the web threshold and level tracks the latest buffer`() {
        val recording = MicComparison(4)
        val buffer = ByteBuffer.allocateDirect(4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putShort(0, 32); buffer.putShort(2, (-32).toShort())
        recording.appendNatural(buffer, 2, 100)
        recording.appendEnhanced(buffer, 2, true)
        assertFalse(recording.hasSignal())
        assertEquals(32 / 32768f, recording.level, 1e-6f)
        buffer.putShort(0, 33); buffer.putShort(2, 0)
        recording.appendNatural(buffer, 2, 100)
        recording.appendEnhanced(buffer, 2, true)
        assertTrue(recording.hasSignal())
    }
}
