package chat.caper.android

import coil3.size.Scale
import okio.Buffer
import org.junit.Assert.*
import org.junit.Test

class AvifCompatTest {
    private fun header(brand: String) = Buffer().apply { write(byteArrayOf(0, 0, 0, 0x20)); writeUtf8("ftyp"); writeUtf8(brand); writeUtf8("rest of the box") }

    @Test fun `avif is recognised by its ftyp brand without consuming the source`() {
        val still = header("avif")
        assertTrue(isAvif(still))
        assertEquals("peek leaves the bytes for the decoder", 27L, still.size)
        assertTrue(isAvif(header("avis")))
        assertFalse(isAvif(header("heic")))
        assertFalse(isAvif(Buffer().writeUtf8("RIFF....WEBPVP8 ")))
        assertFalse(isAvif(Buffer().writeUtf8("short")))
    }

    @Test fun `decode size fits the request, never enlarges and is capped`() {
        assertEquals(400 to 300, avifTargetSize(4000, 3000, 400, 400, Scale.FIT))
        assertEquals(533 to 400, avifTargetSize(4000, 3000, 400, 400, Scale.FILL))
        assertEquals(100 to 50, avifTargetSize(100, 50, 1000, 1000, Scale.FIT))
        assertEquals(4096 to 3072, avifTargetSize(8000, 6000, null, null, Scale.FIT))
        assertEquals(200 to 150, avifTargetSize(4000, 3000, 200, null, Scale.FIT))
    }
}
