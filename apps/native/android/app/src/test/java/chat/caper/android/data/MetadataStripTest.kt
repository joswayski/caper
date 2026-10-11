package chat.caper.android.data

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.zip.CRC32
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Originals keep their media bytes but lose location and camera metadata. */
class MetadataStripTest {
    @get:Rule val folder = TemporaryFolder()

    private fun ByteArray.indexOf(needle: ByteArray): Int =
        (0..size - needle.size).firstOrNull { at -> needle.indices.all { this[at + it] == needle[it] } } ?: -1
    private fun ByteArray.contains(text: String) = indexOf(text.toByteArray(Charsets.ISO_8859_1)) >= 0

    // --- MP4 / MOV --------------------------------------------------------------------------------

    private fun box(type: String, vararg children: ByteArray): ByteArray {
        val body = children.fold(ByteArray(0)) { all, part -> all + part }
        val size = body.size + 8
        return byteArrayOf((size ushr 24).toByte(), (size ushr 16).toByte(), (size ushr 8).toByte(), size.toByte()) +
            type.toByteArray(Charsets.ISO_8859_1) + body
    }

    private fun largeBox(type: String, body: ByteArray): ByteArray {
        val size = body.size + 16L
        return byteArrayOf(0, 0, 0, 1) + type.toByteArray(Charsets.ISO_8859_1) +
            ByteArray(8) { (size ushr (56 - it * 8)).toByte() } + body
    }

    private val samples = ByteArray(4_000) { (it * 7 + 3).toByte() }
    private val location = "+37.3349-122.0090/"

    private fun movie(moovLast: Boolean, largeMdat: Boolean = false): ByteArray {
        val ftyp = box("ftyp", "qt  ".toByteArray(), ByteArray(4), "qt  ".toByteArray())
        val mdat = if (largeMdat) largeBox("mdat", samples) else box("mdat", samples)
        val udta = box("udta", box("©xyz", byteArrayOf(0, 18, 0x15, 0xC7.toByte()), location.toByteArray()))
        val meta = box("meta", box("hdlr", ByteArray(8), "mdta".toByteArray()),
            box("keys", ByteArray(8), "com.apple.quicktime.location.ISO6709".toByteArray()), box("ilst", location.toByteArray()))
        val trak = box("trak", box("tkhd", ByteArray(84)), box("udta", box("name", "Back Camera".toByteArray())), box("mdia", ByteArray(16)))
        val moov = box("moov", box("mvhd", ByteArray(100)), trak, udta, meta)
        return if (moovLast) ftyp + mdat + moov else ftyp + moov + mdat
    }

    private fun stripMovie(bytes: ByteArray): ByteArray {
        val input = folder.newFile().apply { writeBytes(bytes) }
        val output = folder.newFile().apply { delete() }
        assertTrue(MetadataStrip.mp4(input, output))
        return output.readBytes()
    }

    @Test fun `mov location boxes are neutralised with identical samples and size`() {
        for ((moovLast, large) in listOf(true to false, false to false, true to true)) {
            val original = movie(moovLast, large)
            assertTrue(original.contains(location))
            val stripped = stripMovie(original)
            assertEquals(original.size, stripped.size)
            assertFalse(stripped.contains(location))
            assertFalse(stripped.contains("©xyz"))
            assertFalse(stripped.contains("com.apple.quicktime.location"))
            assertFalse(stripped.contains("Back Camera"))
            assertFalse(stripped.contains("udta"))
            assertFalse(stripped.contains("meta"))
            // Sample bytes sit at the same offset, unchanged; the track structure is kept.
            val at = original.indexOf(samples)
            assertEquals(at, stripped.indexOf(samples))
            assertTrue(stripped.contains("tkhd") && stripped.contains("mdia") && stripped.contains("mvhd"))
        }
    }

    @Test fun `files without metadata or not iso bmff are left alone`() {
        val plain = box("ftyp", ByteArray(8)) + box("moov", box("mvhd", ByteArray(100))) + box("mdat", samples)
        val output = folder.newFile().apply { delete() }
        assertFalse(MetadataStrip.mp4(folder.newFile().apply { writeBytes(plain) }, output))
        assertFalse(MetadataStrip.mp4(folder.newFile().apply { writeBytes(ByteArray(64) { 0x1A }) }, output))
        assertFalse(output.exists())
    }

    // --- JPEG -------------------------------------------------------------------------------------

    private fun segment(marker: Int, payload: ByteArray): ByteArray {
        val length = payload.size + 2
        return byteArrayOf(0xFF.toByte(), marker.toByte(), (length ushr 8).toByte(), length.toByte()) + payload
    }

    /** Little-endian Exif with Orientation and a GPS IFD pointer plus GPS latitude text. */
    private fun exif(orientation: Int): ByteArray {
        val out = ByteArrayOutputStream()
        out.write("Exif\u0000\u0000".toByteArray(Charsets.ISO_8859_1))
        out.write(byteArrayOf('I'.code.toByte(), 'I'.code.toByte(), 42, 0, 8, 0, 0, 0))
        out.write(byteArrayOf(2, 0))
        out.write(byteArrayOf(0x12, 0x01, 3, 0, 1, 0, 0, 0, orientation.toByte(), 0, 0, 0))
        out.write(byteArrayOf(0x25, 0x88.toByte(), 4, 0, 1, 0, 0, 0, 38, 0, 0, 0)) // GPSInfo IFD at 38
        out.write(byteArrayOf(0, 0, 0, 0))
        out.write("GPS 37.3349N 122.0090W".toByteArray(Charsets.ISO_8859_1))
        return out.toByteArray()
    }

    private val icc = "ICC_PROFILE\u0000".toByteArray(Charsets.ISO_8859_1) + ByteArray(20) { 9 }
    private val scan = segment(0xDA, ByteArray(10) { 1 }) + ByteArray(500) { (it % 250).toByte() } + byteArrayOf(0xFF.toByte(), 0xD9.toByte())

    private fun jpeg(vararg segments: ByteArray) =
        byteArrayOf(0xFF.toByte(), 0xD8.toByte()) + segments.fold(ByteArray(0)) { all, part -> all + part } + scan

    @Test fun `jpeg loses gps and xmp but keeps pixels, icc and orientation`() {
        val jfif = segment(0xE0, "JFIF\u0000".toByteArray(Charsets.ISO_8859_1) + ByteArray(9))
        val xmp = segment(0xE1, "http://ns.adobe.com/xap/1.0/\u0000<x:xmpmeta GPSLatitude=\"37\"/>".toByteArray(Charsets.ISO_8859_1))
        val dqt = segment(0xDB, ByteArray(65) { 2 })
        val original = jpeg(jfif, segment(0xE1, exif(6)), xmp, segment(0xE2, icc), segment(0xED, "Photoshop 3.0".toByteArray()), segment(0xEE, "Adobe".toByteArray() + ByteArray(7)), dqt)
        val stripped = MetadataStrip.jpeg(original)!!
        assertFalse(stripped.contains("GPS"))
        assertFalse(stripped.contains("xmpmeta"))
        assertFalse(stripped.contains("Photoshop"))
        assertTrue(stripped.contains("ICC_PROFILE") && stripped.contains("Adobe") && stripped.contains("JFIF"))
        // Entropy-coded data (and everything from SOS on) is byte for byte the same.
        assertArrayEquals(scan, stripped.copyOfRange(stripped.size - scan.size, stripped.size))
        assertTrue(stripped.indexOf(dqt) > 0)
        // JFIF stays first; then a minimal Exif carrying only Orientation 6.
        assertEquals(2, stripped.indexOf(jfif))
        val app1 = 2 + jfif.size
        assertEquals(0xE1, stripped[app1 + 1].toInt() and 0xFF)
        val length = ((stripped[app1 + 2].toInt() and 0xFF) shl 8) or (stripped[app1 + 3].toInt() and 0xFF)
        assertEquals(6, MetadataStrip.exifOrientation(stripped.copyOfRange(app1 + 4, app1 + 2 + length)))
        // Still a JPEG stream (SOI first).
        assertEquals(0xD8, stripped[1].toInt() and 0xFF)
    }

    @Test fun `upright jpeg gets no exif at all and clean jpeg is left alone`() {
        val stripped = MetadataStrip.jpeg(jpeg(segment(0xE1, exif(1))))!!
        assertFalse(stripped.contains("Exif"))
        assertArrayEquals(byteArrayOf(0xFF.toByte(), 0xD8.toByte()) + scan, stripped)
        assertNull(MetadataStrip.jpeg(jpeg(segment(0xDB, ByteArray(65)))))
        assertNull(MetadataStrip.jpeg(ByteArray(10)))
        assertEquals(3, MetadataStrip.exifOrientation(MetadataStrip.orientationSegment(3).copyOfRange(4, MetadataStrip.orientationSegment(3).size)))
    }

    // --- PNG --------------------------------------------------------------------------------------

    private fun chunk(type: String, data: ByteArray): ByteArray {
        val out = ByteArrayOutputStream()
        IndexedPng.chunk(out, type, data)
        return out.toByteArray()
    }

    @Test fun `png text and exif chunks are removed, image chunks untouched`() {
        val pixels = IntArray(12) { if (it % 3 == 0) 0xFF112233.toInt() else 0xFFFFFFFF.toInt() }
        val palette = paletteOf(4, 3, 256) { y, row -> System.arraycopy(pixels, y * 4, row, 0, 4) }!!
        val png = ByteArrayOutputStream().also { IndexedPng.encode(4, 3, palette, it) { y, row -> System.arraycopy(pixels, y * 4, row, 0, 4) } }.toByteArray()
        val iend = png.size - 12
        val tagged = png.copyOfRange(0, 33) + chunk("eXIf", "MM\u0000*GPS".toByteArray()) + chunk("tEXt", "Location\u000037.3,-122.0".toByteArray()) +
            png.copyOfRange(33, iend) + chunk("iTXt", "XML:com.adobe.xmp\u0000\u0000\u0000\u0000\u0000<gps/>".toByteArray()) +
            chunk("zTXt", "Comment\u0000\u0000x".toByteArray()) + png.copyOfRange(iend, png.size)
        val stripped = MetadataStrip.png(tagged)!!
        assertArrayEquals(png, stripped)
        assertNull("nothing to remove", MetadataStrip.png(png))
        assertNull(MetadataStrip.png(ByteArray(20)))
        // Still a valid image to the JDK decoder.
        assertNotNull(Class.forName("javax.imageio.ImageIO").getMethod("read", java.io.InputStream::class.java).invoke(null, ByteArrayInputStream(stripped)))
        assertTrue(CRC32().apply { update(stripped) }.value != 0L)
    }
}
