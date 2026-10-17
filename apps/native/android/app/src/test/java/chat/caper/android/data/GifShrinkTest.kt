package chat.caper.android.data

import java.io.ByteArrayOutputStream
import kotlin.math.abs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class GifShrinkTest {
    private val palette = intArrayOf(0x637A43, 0xF1EDE4, 0xB64D32, 0x0C0D0F)

    private class Frame(val delay: Int, val pixel: (Int, Int) -> Int)

    /**
     * An independent test writer: a 4-colour GIF whose LZW stream clears every two pixels, so it
     * barely compresses and large fixtures stay cheap to build. [transparent] makes colour 3
     * transparent.
     */
    private fun fixture(width: Int, height: Int, frames: List<Frame>, transparent: Boolean = false): ByteArray {
        val out = ByteArrayOutputStream()
        fun word(value: Int) { out.write(value and 0xFF); out.write(value shr 8) }
        out.write("GIF89a".toByteArray())
        word(width); word(height)
        out.write(byteArrayOf(0xF1.toByte(), 0, 0))
        for (colour in palette) { out.write(colour shr 16); out.write((colour shr 8) and 0xFF); out.write(colour and 0xFF) }
        out.write(byteArrayOf(0x21, 0xFF.toByte(), 11)); out.write("NETSCAPE2.0".toByteArray()); out.write(byteArrayOf(3, 1, 0, 0, 0))
        for (frame in frames) {
            out.write(byteArrayOf(0x21, 0xF9.toByte(), 4, if (transparent) 0x09 else 0x04))
            word(frame.delay)
            out.write(byteArrayOf(3, 0, 0x2C))
            word(0); word(0); word(width); word(height)
            out.write(byteArrayOf(0, 2))
            val codes = ArrayList<Int>()
            var count = 0
            for (y in 0 until height) for (x in 0 until width) {
                if (count++ % 2 == 0) codes.add(4)
                codes.add(frame.pixel(x, y))
            }
            codes.add(5)
            val data = ArrayList<Int>()
            var accumulator = 0
            var bits = 0
            for (code in codes) {
                accumulator = accumulator or (code shl bits)
                bits += 3
                while (bits >= 8) { data.add(accumulator and 0xFF); accumulator = accumulator shr 8; bits -= 8 }
            }
            if (bits > 0) data.add(accumulator)
            for (block in data.chunked(255)) { out.write(block.size); for (value in block) out.write(value) }
            out.write(0)
        }
        out.write(0x3B)
        return out.toByteArray()
    }

    /** Quadrants that rotate each frame, so every frame differs. */
    private fun quadrants(width: Int, height: Int, count: Int, delay: Int = 7) = List(count) { frame ->
        Frame(delay) { x, y -> ((if (x < width / 2) 0 else 1) + (if (y < height / 2) 0 else 2) + frame) % 4 }
    }

    private class Decoded(val width: Int, val frames: List<IntArray>, val delays: List<Int>) {
        fun at(frame: Int, x: Int, y: Int) = frames[frame][y * width + x]
    }

    private fun frames(bytes: ByteArray): Decoded {
        val frames = ArrayList<IntArray>()
        val delays = ArrayList<Int>()
        GifShrink.decode(bytes) { canvas, delay, _ -> frames.add(canvas.copyOf()); delays.add(delay) }
        return Decoded((bytes[6].toInt() and 0xFF) or ((bytes[7].toInt() and 0xFF) shl 8), frames, delays)
    }

    private fun near(actual: Int, expected: Int, tolerance: Int = 10) = actual ushr 24 == 0xFF &&
        (0..2).all { shift -> abs(((actual shr (shift * 8)) and 0xFF) - ((expected shr (shift * 8)) and 0xFF)) <= tolerance }

    @Test
    fun decoderCompositesEveryFrameWithItsDelay() {
        val decoded = frames(fixture(8, 6, quadrants(8, 6, 3, 12)))
        assertEquals(listOf(12, 12, 12), decoded.delays)
        assertEquals(0xFF000000.toInt() or palette[0], decoded.at(0, 0, 0))
        assertEquals(0xFF000000.toInt() or palette[0], decoded.at(1, 7, 5))
        assertEquals(0xFF000000.toInt() or palette[3], decoded.at(2, 7, 0))
    }

    @Test
    fun wideGifShrinksTo512WithEveryFrameAndItsTiming() {
        val input = fixture(1000, 800, quadrants(1000, 800, 14))
        val shrunk = GifShrink.shrink(input)
        assertEquals(512, shrunk.edge)
        assertTrue("${shrunk.bytes.size} bytes", shrunk.bytes.size < 100_000)
        assertEquals(512 to 512, gifSize(shrunk.bytes))
        val decoded = frames(shrunk.bytes)
        assertEquals(14, decoded.frames.size)
        assertTrue(decoded.delays.all { it == 7 })
        for (frame in 0 until 14) {
            assertTrue(near(decoded.at(frame, 10, 10), palette[frame % 4]))
            assertTrue(near(decoded.at(frame, 500, 10), palette[(1 + frame) % 4]))
            assertTrue(near(decoded.at(frame, 10, 500), palette[(2 + frame) % 4]))
            assertTrue(near(decoded.at(frame, 500, 500), palette[(3 + frame) % 4]))
        }
    }

    @Test
    fun gifsUnder512KeepTheirShortEdge() {
        val shrunk = GifShrink.shrink(fixture(400, 300, quadrants(400, 300, 3)))
        assertEquals(300, shrunk.edge)
        assertEquals(3, frames(shrunk.bytes).frames.size)
    }

    @Test
    fun transparentPixelsStayTransparentInEveryFrame() {
        val input = fixture(300, 300, listOf(Frame(4) { x, _ -> if (x < 150) 3 else 1 }, Frame(4) { x, _ -> if (x < 150) 1 else 3 }), true)
        val shrunk = GifShrink.shrink(input)
        assertEquals(300, shrunk.edge)
        val decoded = frames(shrunk.bytes)
        assertEquals(0, decoded.at(0, 10, 150) ushr 24)
        assertTrue(near(decoded.at(0, 290, 150), palette[1]))
        assertTrue(near(decoded.at(1, 10, 150), palette[1]))
        assertEquals(0, decoded.at(1, 290, 150) ushr 24)
    }

    @Test(expected = GifShrink.Failure::class)
    fun nonGifBytesFail() {
        GifShrink.shrink(byteArrayOf(1, 2, 3))
    }

    @Test
    fun truncatedGifsStillDecodeWhatArrived() {
        val input = fixture(40, 40, quadrants(40, 40, 2))
        assertTrue(frames(input.copyOf(input.size - 200)).frames.isNotEmpty())
    }
}
