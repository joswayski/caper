package chat.caper.android.voice

import android.media.AudioFormat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.sin
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Real bundled JNI/model execution, generated PCM only: no AudioRecord,
 * AudioTrack, permission request, peer, network, or physical device capture. */
@RunWith(AndroidJUnit4::class)
class NativeAudioProcessingTest {
    @Test fun modelComparisonAndZeroGainNeverPublishPrivatePCM() {
        val capture = AudioCapture.prepare(InstrumentationRegistry.getInstrumentation().targetContext)
        val buffer = ByteBuffer.allocateDirect(960).order(ByteOrder.LITTLE_ENDIAN)
        try {
            assertEquals("Pinned ONNX model must initialize before fallback is needed", 1L, capture.report()[0])
            capture.gain(150)
            capture.processingStrength(0)
            capture.publication(true)
            capture.beginComparison()
            repeat(150) { hop ->
                stimulus(buffer, hop)
                capture.onBuffer(buffer, AudioFormat.ENCODING_PCM_16BIT, 1, 48000, 960)
                assertEquals("comparison must send exact silence", 0, peak(buffer))
                Thread.sleep(10)
            }
            val warm = capture.endComparison()!!
            assertTrue("worker must process actual hops", capture.report()[1] > 80)
            val natural = warm.samples(false)
            val enhanced = warm.samples(true)
            assertTrue("comparison must retain positive processed PCM", natural.any { abs(it.toInt()) > 1 })
            assertEquals(natural.size, enhanced.size)
            assertTrue("strength zero bypasses contour, not denoise",
                natural.indices.all { abs(natural[it].toInt() - enhanced[it].toInt()) <= 1 })

            // The ring and model are warm with nonzero audio. Zero gain must
            // clear both local versions, not only newly submitted model input.
            capture.gain(0)
            capture.processingStrength(100)
            capture.beginComparison()
            repeat(20) { hop ->
                stimulus(buffer, hop + 150)
                capture.onBuffer(buffer, AudioFormat.ENCODING_PCM_16BIT, 1, 48000, 960)
                assertEquals(0, peak(buffer))
                Thread.sleep(10)
            }
            val zero = capture.endComparison()!!
            assertEquals(20 * 480, zero.frames)
            assertTrue(zero.samples(false).all { it == 0.toShort() })
            assertTrue(zero.samples(true).all { it == 0.toShort() })

            capture.gain(170)
            capture.resumePublication()
            capture.publication(true)
            var publishedPeak = 0
            repeat(100) { hop ->
                stimulus(buffer, hop + 170)
                capture.onBuffer(buffer, AudioFormat.ENCODING_PCM_16BIT, 1, 48000, 960)
                if (hop == 0) assertEquals("read-entry epoch fence", 0, peak(buffer))
                publishedPeak = maxOf(publishedPeak, peak(buffer))
                Thread.sleep(10)
            }
            assertTrue("fresh processed audio resumes after reopening", publishedPeak > 1)
        } finally { capture.close() }
    }

    private fun stimulus(buffer: ByteBuffer, hop: Int) {
        for (index in 0 until 480) {
            val time = (hop * 480 + index) / 48000.0
            val pitch = 2 * PI * (165 * time + 3 * sin(2 * PI * 2 * time))
            val envelope = 0.3 + 0.7 * abs(sin(2 * PI * 3 * time))
            val value = envelope * (11000 * sin(pitch) + 4200 * sin(3 * pitch) + 2500 * sin(7 * pitch))
            buffer.putShort(index * 2, value.toInt().toShort())
        }
    }

    private fun peak(buffer: ByteBuffer) = (0 until 480).maxOf { abs(buffer.getShort(it * 2).toInt()) }
}
