package chat.caper.android.data

import chat.caper.android.model.AttachmentUrls
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.CompressionSettings
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import kotlinx.serialization.json.Json
import org.junit.Assert.*
import org.junit.Test

class AttachmentsTest {
    private fun message(content: String) = Json.decodeFromString<ChatMessage>(
        """{"id":"message00001","channelId":"channel00001","seq":"4","author":{"id":"account00001","name":"A","isGuest":false},"content":$content,"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}""",
    )

    private val image = """{"id":"AbCdEfGh12345678","kind":"image","contentType":"image/webp","name":"a.webp","size":143000,"width":1600,"height":900,"preview":{},"url":"https://cdn.caper.chat/original/AbCdEfGh12345678?exp=1790000000&sig=x","previewUrl":"https://cdn.caper.chat/preview/AbCdEfGh12345678?exp=1790000000&sig=y","futureField":1}"""

    @Test fun `malformed attachments are skipped without rejecting the message`() {
        val parsed = message(
            """{"version":1,"type":"text","text":"","attachments":[$image,
                {"kind":"image","contentType":"image/png","name":"no-id.png","size":1},
                {"id":"BbCdEfGh12345678","kind":"hologram","contentType":"x/y","name":"x","size":1},
                {"id":"CbCdEfGh12345678","kind":"file","contentType":"text/html","name":"x.html","size":"big"},
                {"id":"DbCdEfGh12345678","kind":"file","contentType":"text/html","name":"x.html","size":5,"url":"javascript:alert(1)"},
                {"id":"EbCdEfGh12345678","kind":"file","contentType":"application/pdf","name":"gone.pdf","size":5,"unavailable":true},
                7, null]}""",
        )
        assertEquals(listOf("AbCdEfGh12345678", "EbCdEfGh12345678"), parsed.content.attachments.map { it.id })
        assertNotNull(parsed.content.attachments.first().preview)
        assertTrue(parsed.content.attachments.last().unavailable)
        // A file-only message has empty text and still validates.
        assertEquals(parsed, parsed.validated("channel00001"))
    }

    @Test fun `non-array attachments and old messages parse as no files`() {
        assertTrue(message("""{"version":1,"type":"text","text":"hi","attachments":{"id":"x"}}""").content.attachments.isEmpty())
        assertTrue(message("""{"version":1,"type":"text","text":"hi","attachments":null}""").content.attachments.isEmpty())
        assertTrue(message("""{"version":1,"type":"text","text":"hi"}""").content.attachments.isEmpty())
    }

    @Test fun `signed url expiry is read from exp`() {
        assertEquals(1790000000L, attachmentUrlExpiry("https://cdn.caper.chat/original/x?exp=1790000000&sig=abc"))
        assertEquals(1790000000L, attachmentUrlExpiry("https://cdn.caper.chat/original/x?sig=abc&exp=1790000000"))
        assertNull(attachmentUrlExpiry("https://cdn.caper.chat/original/x?sig=abc"))
        assertNull(attachmentUrlExpiry("https://cdn.caper.chat/original/x?exp=soon"))
        assertNull(attachmentUrlExpiry(null))
    }

    private fun attachment(id: String = "AbCdEfGh12345678", exp: Long? = 2_000, unavailable: Boolean = false) = ChatAttachment(
        id, "image", "image/png", "a.png", 10,
        url = exp?.let { "https://cdn.example/original/$id?exp=$it&sig=s" }, previewUrl = exp?.let { "https://cdn.example/preview/$id?exp=$it&sig=p" },
        unavailable = unavailable,
    )

    @Test fun `fresh urls replace older signatures only`() {
        val current = attachment(exp = 2_000)
        val newer = AttachmentUrls("https://cdn.example/original/n?exp=3000&sig=a", "https://cdn.example/preview/n?exp=3000&sig=b")
        val older = AttachmentUrls("https://cdn.example/original/o?exp=1000&sig=a")
        assertEquals(newer.url, current.withFreshUrls(newer).url)
        assertEquals(newer.previewUrl, current.withFreshUrls(newer).previewUrl)
        assertSame(current, current.withFreshUrls(older))
        assertSame(current, current.withFreshUrls(null))
        assertEquals(newer.url, attachment(exp = null).withFreshUrls(newer).url)
        val gone = attachment(unavailable = true)
        assertSame(gone, gone.withFreshUrls(newer))
    }

    @Test fun `refresh near expiry once per signature`() {
        var now = 0L
        val refresh = AttachmentUrlRefresh(nowSeconds = { now }, marginSeconds = 3_600)
        val soon = attachment("AbCdEfGh12345678", exp = 3_000)
        val later = attachment("BbCdEfGh12345678", exp = 10_000)
        assertEquals(listOf("AbCdEfGh12345678"), refresh.expiring(listOf(soon, later)))
        assertTrue("already requested for this exp", refresh.expiring(listOf(soon, later)).isEmpty())
        now = 7_000
        assertEquals(listOf("BbCdEfGh12345678"), refresh.expiring(listOf(soon, later)))
        // A re-signed URL with a new expiry can be refreshed again in a day or two.
        now = 95_000
        assertEquals(listOf("AbCdEfGh12345678"), refresh.expiring(listOf(attachment("AbCdEfGh12345678", exp = 96_000))))
        assertTrue(refresh.expiring(listOf(attachment(exp = null), attachment(exp = 1, unavailable = true))).isEmpty())
    }

    @Test fun `failed loads refresh once on 403 or 404 and never for other errors`() {
        val refresh = AttachmentUrlRefresh(nowSeconds = { 0 })
        val file = attachment(exp = 100_000)
        assertFalse(refresh.afterLoadFailure(file, 500))
        assertFalse(refresh.afterLoadFailure(file, null))
        assertTrue(refresh.afterLoadFailure(file, 403))
        assertFalse("retry once only", refresh.afterLoadFailure(file, 404))
        assertTrue(refresh.afterLoadFailure(attachment("BbCdEfGh12345678", exp = 100_000), 404))
        assertFalse(refresh.afterLoadFailure(attachment("CbCdEfGh12345678", unavailable = true), 403))
        // An already expired URL is worth one refresh whatever the error.
        assertTrue(AttachmentUrlRefresh(nowSeconds = { 200_000 }).afterLoadFailure(file, null))
        refresh.reset()
        assertTrue(refresh.afterLoadFailure(file, 403))
    }

    @Test fun `kinds mirror the api allowlist`() {
        assertEquals("image", AttachmentPolicy.kind("image/PNG; charset=binary"))
        assertEquals("file", AttachmentPolicy.kind("image/heic"))
        assertEquals("file", AttachmentPolicy.kind("image/svg+xml"))
        assertEquals("video", AttachmentPolicy.kind("video/quicktime"))
        assertEquals("audio", AttachmentPolicy.kind("audio/x-m4a"))
        assertEquals("file", AttachmentPolicy.kind("application/pdf"))
        assertEquals("application/octet-stream", AttachmentPolicy.normalizedType(null))
        assertTrue(AttachmentPolicy.compressibleStill("image/heic"))
        assertFalse(AttachmentPolicy.compressibleStill("image/gif"))
        assertFalse(AttachmentPolicy.compressibleStill("image/avif"))
        assertFalse(AttachmentPolicy.compressibleStill("image/svg+xml"))
    }

    @Test fun `palette fits become lossless png, photos lossy, quality 100 keeps originals`() {
        val defaults = CompressionSettings()
        assertEquals(AttachmentPolicy.StillEncoding.PALETTE_PNG, AttachmentPolicy.stillEncoding(256, defaults, resized = false, originalInline = true))
        assertEquals(AttachmentPolicy.StillEncoding.LOSSY, AttachmentPolicy.stillEncoding(null, defaults, resized = false, originalInline = true))
        assertEquals(AttachmentPolicy.StillEncoding.LOSSY, AttachmentPolicy.stillEncoding(12, defaults.copy(paletteColors = 0), resized = false, originalInline = true))
        assertEquals(AttachmentPolicy.StillEncoding.LOSSY, AttachmentPolicy.stillEncoding(40, defaults.copy(paletteColors = 32), resized = false, originalInline = true))
        val lossless = defaults.copy(imageQuality = 100)
        assertEquals(AttachmentPolicy.StillEncoding.KEEP, AttachmentPolicy.stillEncoding(null, lossless, resized = false, originalInline = true))
        assertEquals(AttachmentPolicy.StillEncoding.LOSSY, AttachmentPolicy.stillEncoding(null, lossless, resized = true, originalInline = true))
        assertEquals(AttachmentPolicy.StillEncoding.LOSSY, AttachmentPolicy.stillEncoding(null, lossless, resized = false, originalInline = false))
    }

    @Test fun `re-encoded files are kept only when 10 percent smaller or the original is not inline`() {
        assertTrue(AttachmentPolicy.keepReencoded("image/png", 1_000, 900))
        assertFalse(AttachmentPolicy.keepReencoded("image/png", 1_000, 901))
        assertTrue(AttachmentPolicy.keepReencoded("image/heic", 1_000, 5_000))
        assertTrue(AttachmentPolicy.keepTranscoded("video/mp4", 1_000, 999))
        assertFalse(AttachmentPolicy.keepTranscoded("video/mp4", 1_000, 1_000))
        assertTrue(AttachmentPolicy.keepTranscoded("video/x-matroska", 1_000, 2_000))
        assertFalse(AttachmentPolicy.keepTranscoded("video/x-matroska", 1_000, 0))
        assertEquals("shot.png", AttachmentPolicy.renamed("shot.HEIC", "image/png"))
        assertEquals("photo.webp", AttachmentPolicy.renamed("photo.jpeg", "image/webp"))
        assertEquals("clip.mp4", AttachmentPolicy.renamed("clip.mov", "video/mp4"))
        assertEquals("noext.jpg", AttachmentPolicy.renamed("noext", "image/jpeg"))
        assertEquals("doc.pdf", AttachmentPolicy.renamed("doc.pdf", "application/pdf"))
    }

    @Test fun `sizing never enlarges and previews follow edge and byte limits`() {
        assertEquals(4096 to 2304, AttachmentPolicy.fitWithin(8000, 4500, 4096))
        assertEquals(640 to 1138, AttachmentPolicy.fitWithin(1080, 1920, 1138))
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
        val defaults = CompressionSettings()
        assertEquals(1080, AttachmentPolicy.videoTargetHeight(3840, 2160, defaults))
        assertEquals(1920, AttachmentPolicy.videoTargetHeight(2160, 3840, defaults))
        assertEquals("portrait 1080p keeps its size", 1920, AttachmentPolicy.videoTargetHeight(1080, 1920, defaults))
        assertEquals(720, AttachmentPolicy.videoTargetHeight(1280, 720, defaults))
        assertEquals(1080, AttachmentPolicy.videoTargetHeight(1920, 1080, defaults))
        assertEquals("even for H.264", 1080, AttachmentPolicy.videoTargetHeight(1101, 1101, defaults))
        assertEquals(720, AttachmentPolicy.videoTargetHeight(2160, 3840, defaults.copy(videoMaxHeight = 405)))
        assertNull(AttachmentPolicy.videoTargetHeight(null, 2160, defaults))
        assertNull(AttachmentPolicy.videoTargetHeight(3840, 2160, defaults.copy(videoMaxHeight = 0)))
    }

    @Test fun `size labels show the compression saving`() {
        assertEquals("512 B", AttachmentPolicy.formatBytes(512))
        assertEquals("1.6 MB", AttachmentPolicy.formatBytes(1_677_722))
        assertEquals("143 KB", AttachmentPolicy.formatBytes(146_432))
        assertEquals("1.6 MB → 143 KB", AttachmentPolicy.sizeLabel(1_677_722, 146_432))
        assertEquals("143 KB", AttachmentPolicy.sizeLabel(146_432, 146_432))
        assertEquals("143 KB", AttachmentPolicy.sizeLabel(146_432, null))
    }

    @Test fun `upload errors use the web copy`() {
        assertEquals("You’ve used all of your file storage.", AttachmentPolicy.uploadErrorMessage(ApiException(413, "storage limit reached", "storage_full")))
        assertEquals("This file is too large to upload.", AttachmentPolicy.uploadErrorMessage(ApiException(413, "too large")))
        assertEquals("Uploading too quickly. Try again shortly.", AttachmentPolicy.uploadErrorMessage(ApiException(429, "slow down")))
        assertEquals("invalid file name", AttachmentPolicy.uploadErrorMessage(ApiException(400, "invalid file name")))
        assertEquals("Storage refused the upload (403).", AttachmentPolicy.uploadErrorMessage(UploadException("Storage refused the upload (403).")))
        assertEquals("This file could not be uploaded.", AttachmentPolicy.uploadErrorMessage(IllegalStateException("boom")))
    }

    @Test fun `animated webp and apng are detected so they upload unchanged`() {
        fun bytes(vararg parts: Any): ByteArray = ByteArrayOutputStream().apply {
            parts.forEach { part -> when (part) { is String -> write(part.toByteArray(Charsets.US_ASCII)); is Int -> write(part); is ByteArray -> write(part) } }
        }.toByteArray()
        val animatedWebp = bytes("RIFF", ByteArray(4), "WEBP", "VP8X", ByteArray(4), 0x02, ByteArray(9))
        val stillWebp = bytes("RIFF", ByteArray(4), "WEBP", "VP8X", ByteArray(4), 0x10, ByteArray(9))
        assertTrue(isAnimatedImage("image/webp", animatedWebp))
        assertFalse(isAnimatedImage("image/webp", stillWebp))
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
        assertEquals(0xFF102030.toInt(), palette.last())
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
            // Fully transparent pixels carry no colour information in the PNG palette sense.
            if ((expected ushr 24) == 0) assertEquals(0, actual ushr 24) else assertEquals("pixel $x,$y", expected, actual)
        }
    }

    @Test fun `indexed png is lossless at every bit depth including alpha`() {
        roundTrip(5, 3, IntArray(15) { if (it % 2 == 0) 0xFF000000.toInt() else 0xFFFFFFFF.toInt() })
        roundTrip(9, 2, IntArray(18) { intArrayOf(0xFFFF0000.toInt(), 0xFF00FF00.toInt(), 0xFF0000FF.toInt(), 0x00000000)[it % 4] })
        roundTrip(7, 7, IntArray(49) { 0xFF000000.toInt() or (it * 4111 and 0xFFFFFF) }.also { it[3] = 0x40123456 })
        roundTrip(300, 200, IntArray(60_000) { 0xFF000000.toInt() or ((it % 200) * 0x010101) })
        assertEquals(1, IndexedPng.bitDepth(2)); assertEquals(2, IndexedPng.bitDepth(4)); assertEquals(4, IndexedPng.bitDepth(16)); assertEquals(8, IndexedPng.bitDepth(17))
    }
}
