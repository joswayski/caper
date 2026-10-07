package chat.caper.android.data

import chat.caper.android.data.AttachmentPolicy.LosslessEncoding
import chat.caper.android.data.AttachmentPolicy.StillClass
import chat.caper.android.data.AttachmentPolicy.VideoFacts
import chat.caper.android.model.CompressionSettings
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import org.junit.Assert.*
import org.junit.Test

/** docs/media.md "Client compression and previews": the decision rules, without Android codecs. */
class CompressionPolicyTest {
    private val defaults = CompressionSettings()

    private fun bytes(vararg parts: Any): ByteArray = ByteArrayOutputStream().apply {
        parts.forEach { part -> when (part) { is String -> write(part.toByteArray(Charsets.US_ASCII)); is Int -> write(part); is ByteArray -> write(part) } }
    }.toByteArray()

    private fun le32(value: Int) = byteArrayOf(value.toByte(), (value ushr 8).toByte(), (value ushr 16).toByte(), (value ushr 24).toByte())
    private val lossyWebp = bytes("RIFF", le32(30), "WEBP", "VP8 ", le32(10), ByteArray(10))
    private val losslessWebp = bytes("RIFF", le32(30), "WEBP", "VP8L", le32(10), ByteArray(10))
    // Extended: an ICC chunk, an alpha chunk, then the image chunk.
    private val extendedLossy = bytes("RIFF", le32(60), "WEBP", "VP8X", le32(10), 0x30, ByteArray(9), "ICCP", le32(3), ByteArray(4), "ALPH", le32(2), ByteArray(2), "VP8 ", le32(4), ByteArray(4))
    private val extendedLossless = bytes("RIFF", le32(40), "WEBP", "VP8X", le32(10), 0x10, ByteArray(9), "VP8L", le32(4), ByteArray(4))

    @Test fun `lossless sources are classified apart from photos and kept files`() {
        assertEquals(StillClass.LOSSLESS, AttachmentPolicy.stillClass("image/png", ByteArray(0)))
        assertEquals(StillClass.LOSSLESS, AttachmentPolicy.stillClass("image/bmp", ByteArray(0)))
        assertEquals(StillClass.LOSSLESS, AttachmentPolicy.stillClass("image/webp", losslessWebp))
        assertEquals(StillClass.LOSSLESS, AttachmentPolicy.stillClass("image/webp", extendedLossless))
        assertEquals("unknown WebP coding is never treated as a photo", StillClass.LOSSLESS, AttachmentPolicy.stillClass("image/webp", ByteArray(4)))
        assertEquals(StillClass.PHOTO, AttachmentPolicy.stillClass("image/webp", lossyWebp))
        assertEquals(StillClass.PHOTO, AttachmentPolicy.stillClass("image/webp", extendedLossy))
        assertEquals(StillClass.PHOTO, AttachmentPolicy.stillClass("image/jpeg", ByteArray(0)))
        assertEquals(StillClass.PHOTO, AttachmentPolicy.stillClass("image/heic", ByteArray(0)))
        assertEquals(StillClass.PHOTO, AttachmentPolicy.stillClass("IMAGE/HEIF; x=1", ByteArray(0)))
        for (type in listOf("image/gif", "image/svg+xml", "image/avif")) assertEquals(type, StillClass.KEEP, AttachmentPolicy.stillClass(type, ByteArray(0)))
        val animated = bytes("RIFF", ByteArray(4), "WEBP", "VP8X", ByteArray(4), 0x02, ByteArray(9))
        assertEquals(StillClass.KEEP, AttachmentPolicy.stillClass("image/webp", animated))
    }

    @Test fun `lossless stays lossless - palette png, else lossless webp, else the original`() {
        assertEquals(LosslessEncoding.PALETTE_PNG, AttachmentPolicy.losslessEncoding(256, defaults, webpLosslessAvailable = true, originalInline = true))
        assertEquals("real screenshots have thousands of colours", LosslessEncoding.WEBP_LOSSLESS,
            AttachmentPolicy.losslessEncoding(null, defaults, webpLosslessAvailable = true, originalInline = true))
        assertEquals(LosslessEncoding.KEEP, AttachmentPolicy.losslessEncoding(null, defaults, webpLosslessAvailable = false, originalInline = true))
        assertEquals(LosslessEncoding.PNG, AttachmentPolicy.losslessEncoding(null, defaults, webpLosslessAvailable = false, originalInline = false))
        assertEquals(LosslessEncoding.WEBP_LOSSLESS, AttachmentPolicy.losslessEncoding(12, defaults.copy(paletteColors = 0), true, true))
        assertEquals(LosslessEncoding.WEBP_LOSSLESS, AttachmentPolicy.losslessEncoding(40, defaults.copy(paletteColors = 32), true, true))
        // Any saving keeps a lossless encode; a larger one never replaces an inline original.
        assertTrue(AttachmentPolicy.keepLossless("image/png", 1_000, 999))
        assertFalse(AttachmentPolicy.keepLossless("image/png", 1_000, 1_000))
        assertTrue(AttachmentPolicy.keepLossless("image/bmp", 1_000, 5_000))
        assertFalse(AttachmentPolicy.keepLossless("image/bmp", 1_000, 0))
    }

    @Test fun `photos are re-encoded at quality and kept only when 10 percent smaller`() {
        assertTrue(AttachmentPolicy.reencodePhoto(defaults, resized = false, originalInline = true))
        val quality100 = defaults.copy(imageQuality = 100)
        assertFalse(AttachmentPolicy.reencodePhoto(quality100, resized = false, originalInline = true))
        assertTrue(AttachmentPolicy.reencodePhoto(quality100, resized = true, originalInline = true))
        assertTrue("HEIC is always converted", AttachmentPolicy.reencodePhoto(quality100, resized = false, originalInline = false))
        assertTrue(AttachmentPolicy.keepPhoto("image/jpeg", 1_000, 900))
        assertFalse(AttachmentPolicy.keepPhoto("image/jpeg", 1_000, 901))
        assertTrue(AttachmentPolicy.keepPhoto("image/heic", 1_000, 5_000))
        assertFalse(AttachmentPolicy.keepPhoto("image/heic", 1_000, 0))
        assertEquals("shot.png", AttachmentPolicy.renamed("shot.HEIC", "image/png"))
        assertEquals("photo.webp", AttachmentPolicy.renamed("photo.jpeg", "image/webp"))
        assertEquals("clip.mp4", AttachmentPolicy.renamed("clip.mov", "video/mp4"))
        assertEquals("doc.pdf", AttachmentPolicy.renamed("doc.pdf", "application/pdf"))
    }

    @Test fun `sizing never enlarges and previews follow edge and byte limits`() {
        assertEquals(4096 to 2304, AttachmentPolicy.fitWithin(8000, 4500, 4096))
        assertEquals(100 to 50, AttachmentPolicy.fitWithin(100, 50, 640))
        assertEquals(8000 to 4500, AttachmentPolicy.fitWithin(8000, 4500, 0))
        assertEquals(640 to 1, AttachmentPolicy.fitWithin(10_000, 10, 640))
        assertTrue(AttachmentPolicy.needsPreview("image", 641, 100, 1_000, 640))
        assertFalse(AttachmentPolicy.needsPreview("image", 640, 640, 512 * 1024, 640))
        assertTrue(AttachmentPolicy.needsPreview("image", 10, 10, 512 * 1024 + 1, 640))
        assertTrue(AttachmentPolicy.needsPreview("video", null, null, 1, 640))
        assertFalse(AttachmentPolicy.needsPreview("file", 4000, 4000, 9_999_999, 640))
        assertFalse(AttachmentPolicy.needsPreview("audio", null, null, 9_999_999, 640))
    }

    @Test fun `video max height bounds the short edge and never upscales`() {
        assertEquals(1080, AttachmentPolicy.videoTargetHeight(3840, 2160, defaults))
        assertEquals(1920, AttachmentPolicy.videoTargetHeight(2160, 3840, defaults))
        assertEquals("portrait 1080p keeps its size", 1920, AttachmentPolicy.videoTargetHeight(1080, 1920, defaults))
        assertEquals(720, AttachmentPolicy.videoTargetHeight(1280, 720, defaults))
        assertNull(AttachmentPolicy.videoTargetHeight(null, 2160, defaults))
        assertNull(AttachmentPolicy.videoTargetHeight(3840, 2160, defaults.copy(videoMaxHeight = 0)))
    }

    @Test fun `target bitrate scales with pixels above a floor`() {
        assertEquals(6000, AttachmentPolicy.videoTargetKbps(1920, 1080, defaults))
        assertEquals(6000, AttachmentPolicy.videoTargetKbps(1080, 1920, defaults))
        assertEquals(2667, AttachmentPolicy.videoTargetKbps(1280, 720, defaults))
        assertEquals("floor", 1500, AttachmentPolicy.videoTargetKbps(640, 360, defaults))
        assertEquals("the floor never exceeds the setting", 800, AttachmentPolicy.videoTargetKbps(320, 240, defaults.copy(videoBitrateKbps = 800)))
    }

    private fun facts(width: Int, height: Int, mime: String = "video/avc", kbps: Long? = 5_000, hdr: Boolean = false, type: String = "video/mp4") =
        VideoFacts(width, height, mime, kbps?.times(1000), hdr, type)

    @Test fun `efficient h264 within limits uploads unchanged`() {
        assertNull(AttachmentPolicy.videoPlan(facts(1920, 1080, kbps = 7_500), defaults, toneMapSupported = true))
        assertNull(AttachmentPolicy.videoPlan(facts(1080, 1920, kbps = 7_500), defaults, toneMapSupported = true))
        assertNull("unknown bitrate is not a reason", AttachmentPolicy.videoPlan(facts(1280, 720, kbps = null), defaults, true))
        assertNull("disabled", AttachmentPolicy.videoPlan(facts(3840, 2160, mime = "video/hevc"), defaults.copy(videoMaxHeight = 0), true))
        assertNull("unknown size", AttachmentPolicy.videoPlan(VideoFacts(null, null, "video/hevc", null), defaults, true))
    }

    @Test fun `bitrate above 125 percent of target is a size-only transcode`() {
        val plan = AttachmentPolicy.videoPlan(facts(1920, 1080, kbps = 7_501), defaults, true)!!
        assertTrue(plan.sizeOnly); assertNull(plan.outputHeight); assertEquals(6000, plan.bitrateKbps); assertFalse(plan.toneMap)
        val small = AttachmentPolicy.videoPlan(facts(1280, 720, kbps = 4_000), defaults, true)!!
        assertEquals(2667, small.bitrateKbps)
        assertNull(AttachmentPolicy.videoPlan(facts(1280, 720, kbps = 3_300), defaults, true))
        // Size-only transcodes must save 10%; the audio track is never dropped.
        assertTrue(AttachmentPolicy.keepTranscoded(plan, 1_000, 900, audioBefore = true, audioAfter = true, outputHdr = false))
        assertFalse(AttachmentPolicy.keepTranscoded(plan, 1_000, 901, audioBefore = true, audioAfter = true, outputHdr = false))
        assertFalse(AttachmentPolicy.keepTranscoded(plan, 1_000, 100, audioBefore = true, audioAfter = false, outputHdr = false))
        assertTrue(AttachmentPolicy.keepTranscoded(plan, 1_000, 100, audioBefore = false, audioAfter = false, outputHdr = false))
    }

    @Test fun `4k and hevc transcode for compatibility whatever the size`() {
        val uhd = AttachmentPolicy.videoPlan(facts(3840, 2160, kbps = 40_000), defaults, true)!!
        assertEquals(1080, uhd.outputHeight); assertFalse(uhd.sizeOnly); assertEquals(6000, uhd.bitrateKbps)
        val portrait = AttachmentPolicy.videoPlan(facts(2160, 3840), defaults, true)!!
        assertEquals(1920, portrait.outputHeight)
        val hevc = AttachmentPolicy.videoPlan(facts(1920, 1080, mime = "video/hevc", kbps = 3_000), defaults, true)!!
        assertNull(hevc.outputHeight); assertFalse(hevc.sizeOnly)
        assertTrue(AttachmentPolicy.keepTranscoded(hevc, 1_000, 1_500, audioBefore = true, audioAfter = true, outputHdr = false))
        assertFalse(AttachmentPolicy.keepTranscoded(hevc, 1_000, 0, audioBefore = true, audioAfter = true, outputHdr = false))
        assertNotNull("VP9 WebM", AttachmentPolicy.videoPlan(facts(1280, 720, mime = "video/x-vnd.on2.vp9", kbps = 1_000, type = "video/webm"), defaults, true))
        assertNotNull("H.264 in a container that cannot play inline", AttachmentPolicy.videoPlan(facts(640, 360, kbps = 500, type = "video/3gpp"), defaults, true))
    }

    @Test fun `hdr is tone mapped where supported, otherwise the original uploads`() {
        val hdr = facts(3840, 2160, mime = "video/hevc", kbps = 50_000, hdr = true)
        val plan = AttachmentPolicy.videoPlan(hdr, defaults, toneMapSupported = true)!!
        assertTrue(plan.toneMap)
        assertNull("no washed-out SDR copy", AttachmentPolicy.videoPlan(hdr, defaults, toneMapSupported = false))
        assertFalse("an export that is still HDR is discarded", AttachmentPolicy.keepTranscoded(plan, 1_000, 100, true, true, outputHdr = true))
        assertNull("HDR alone is no reason to transcode", AttachmentPolicy.videoPlan(facts(1920, 1080, kbps = 6_000, hdr = true), defaults, true))
    }

    @Test fun `size labels show the compression saving`() {
        assertEquals("1.6 MB → 143 KB", AttachmentPolicy.sizeLabel(1_677_722, 146_432))
        assertEquals("143 KB", AttachmentPolicy.sizeLabel(146_432, 146_432))
        assertEquals("143 KB", AttachmentPolicy.sizeLabel(146_432, null))
    }

    @Test fun `animated webp and apng are detected so they upload unchanged`() {
        val animatedWebp = bytes("RIFF", ByteArray(4), "WEBP", "VP8X", ByteArray(4), 0x02, ByteArray(9))
        assertTrue(isAnimatedImage("image/webp", animatedWebp))
        assertFalse(isAnimatedImage("image/webp", extendedLossless))
        val ihdr = bytes(0, 0, 0, 13, "IHDR", ByteArray(13), ByteArray(4))
        val actl = bytes(0, 0, 0, 8, "acTL", ByteArray(8), ByteArray(4))
        val idat = bytes(0, 0, 0, 0, "IDAT", ByteArray(4))
        val signature = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 13, 10, 26, 10)
        assertTrue(isAnimatedImage("image/png", signature + ihdr + actl + idat))
        assertFalse(isAnimatedImage("image/png", signature + ihdr + idat + actl))
        assertFalse(isAnimatedImage("image/jpeg", animatedWebp))
    }

    @Test fun `palette counting stops past the limit and sorts translucent colours first`() {
        val pixels = intArrayOf(0xFF102030.toInt(), 0x00000000, 0xFF102030.toInt(), 0x80FF0000.toInt())
        val palette = paletteOf(2, 2, 256) { y, row -> System.arraycopy(pixels, y * 2, row, 0, 2) }!!
        assertEquals(3, palette.size)
        assertTrue(palette.take(2).all { (it ushr 24) != 0xFF })
        val gradient = IntArray(300) { 0xFF000000.toInt() or it }
        assertNull(paletteOf(300, 1, 256) { _, row -> System.arraycopy(gradient, 0, row, 0, 300) })
        assertEquals(256, paletteOf(256, 1, 256) { _, row -> System.arraycopy(gradient, 0, row, 0, 256) }!!.size)
        assertNull(paletteOf(2, 2, 0) { _, _ -> })
    }

    private fun roundTrip(width: Int, height: Int, pixels: IntArray) {
        val palette = requireNotNull(paletteOf(width, height, 256) { y, row -> System.arraycopy(pixels, y * width, row, 0, width) })
        val png = ByteArrayOutputStream().also { out ->
            IndexedPng.encode(width, height, palette, out) { y, row -> System.arraycopy(pixels, y * width, row, 0, width) }
        }.toByteArray()
        // The JDK's own PNG decoder (reflection: unit tests compile against android.jar).
        val decoded = requireNotNull(Class.forName("javax.imageio.ImageIO").getMethod("read", java.io.InputStream::class.java).invoke(null, ByteArrayInputStream(png)))
        val type = decoded.javaClass
        assertEquals(width, type.getMethod("getWidth").invoke(decoded)); assertEquals(height, type.getMethod("getHeight").invoke(decoded))
        val getRgb = type.getMethod("getRGB", Int::class.javaPrimitiveType, Int::class.javaPrimitiveType)
        for (y in 0 until height) for (x in 0 until width) {
            val expected = pixels[y * width + x]
            val actual = getRgb.invoke(decoded, x, y) as Int
            if ((expected ushr 24) == 0) assertEquals(0, actual ushr 24) else assertEquals("pixel $x,$y", expected, actual)
        }
    }

    @Test fun `indexed png is lossless at every bit depth including alpha`() {
        roundTrip(5, 3, IntArray(15) { if (it % 2 == 0) 0xFF000000.toInt() else 0xFFFFFFFF.toInt() })
        roundTrip(9, 2, IntArray(18) { intArrayOf(0xFFFF0000.toInt(), 0xFF00FF00.toInt(), 0xFF0000FF.toInt(), 0x00000000)[it % 4] })
        roundTrip(7, 7, IntArray(49) { 0xFF000000.toInt() or (it * 4111 and 0xFFFFFF) }.also { it[3] = 0x40123456 })
        roundTrip(300, 200, IntArray(60_000) { 0xFF000000.toInt() or ((it % 200) * 0x010101) })
        assertEquals(1, IndexedPng.bitDepth(2)); assertEquals(4, IndexedPng.bitDepth(16)); assertEquals(8, IndexedPng.bitDepth(17))
    }
}
