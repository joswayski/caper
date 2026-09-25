package chat.caper.android.voice

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.*
import org.junit.Test

class WavPcmTest {
    private fun chunk(id: String, body: ByteArray): ByteArray {
        val header = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)
        header.put(id.toByteArray(Charsets.US_ASCII)); header.putInt(body.size)
        return header.array() + body + if (body.size % 2 == 1) byteArrayOf(0) else byteArrayOf()
    }

    private fun wav(format: Short = 1, bits: Short = 16, samples: ShortArray): ByteArray {
        val fmt = ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN)
            .putShort(format).putShort(2).putInt(44100).putInt(44100 * 4).putShort(4).putShort(bits).array()
        val data = ByteBuffer.allocate(samples.size * 2).order(ByteOrder.LITTLE_ENDIAN).apply { samples.forEach { putShort(it) } }.array()
        val body = ByteArrayOutputStream().apply {
            write("WAVE".toByteArray()); write(chunk("fmt ", fmt)); write(chunk("LIST", "INFOISFT\u0003\u0000\u0000\u0000Lav".toByteArray()))
            write(chunk("data", data))
        }.toByteArray()
        return "RIFF".toByteArray() + ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(body.size).array() + body
    }

    @Test fun `reads 16-bit PCM after skipping metadata chunks`() {
        val parsed = parseWavPcm(wav(samples = shortArrayOf(1, -2, 300, -400)))
        assertEquals(2, parsed.channels)
        assertEquals(44100, parsed.sampleRate)
        assertArrayEquals(shortArrayOf(1, -2, 300, -400), parsed.samples)
    }

    @Test fun `rejects formats the speaker test cannot play`() {
        assertThrows(IllegalArgumentException::class.java) { parseWavPcm(wav(format = 3, samples = shortArrayOf(0, 0))) }
        assertThrows(IllegalArgumentException::class.java) { parseWavPcm("nope".toByteArray()) }
    }
}
