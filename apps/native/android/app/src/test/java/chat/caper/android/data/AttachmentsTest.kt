package chat.caper.android.data

import chat.caper.android.model.AttachmentUrls
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.AttachmentState
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
        assertEquals("image", AttachmentPolicy.kind("image/avif"))
        assertEquals("file", AttachmentPolicy.kind("image/heic"))
        assertEquals("file", AttachmentPolicy.kind("image/svg+xml"))
        assertEquals("video", AttachmentPolicy.kind("video/quicktime"))
        assertEquals("audio", AttachmentPolicy.kind("audio/x-m4a"))
        assertEquals("file", AttachmentPolicy.kind("application/pdf"))
        assertEquals("application/octet-stream", AttachmentPolicy.normalizedType(null))
        assertEquals("application/octet-stream", AttachmentPolicy.normalizedType("garbage"))
    }

    @Test fun `byte sizes are formatted like the web`() {
        assertEquals("512 B", AttachmentPolicy.formatBytes(512))
        assertEquals("1.6 MB", AttachmentPolicy.formatBytes(1_677_722))
        assertEquals("143 KB", AttachmentPolicy.formatBytes(146_432))
        assertEquals("2.0 GB", AttachmentPolicy.formatBytes(2_147_483_648))
    }

    @Test fun `status parses with new fields and defaults to ready`() {
        val parsed = message(
            """{"version":1,"type":"text","text":"","attachments":[
                {"id":"AbCdEfGh12345678","kind":"image","contentType":"image/avif","name":"a.avif","size":1,"status":"ready","url":"https://cdn.caper.chat/original/a?exp=1&sig=s"},
                {"id":"BbCdEfGh12345678","kind":"video","contentType":"video/quicktime","name":"b.mov","size":1,"status":"processing","width":1920,"height":1080,"previewUrl":"https://cdn.caper.chat/preview/b?exp=1&sig=p"},
                {"id":"CbCdEfGh12345678","kind":"file","contentType":"image/heic","name":"c.heic","size":1,"status":"failed"},
                {"id":"DbCdEfGh12345678","kind":"video","contentType":"video/mp4","name":"d.mp4","size":1,"animated":true,"url":"https://cdn.caper.chat/original/d?exp=1&sig=s"},
                {"id":"EbCdEfGh12345678","kind":"file","contentType":"text/plain","name":"e.txt","size":1,"status":"someday"},
                {"id":"FbCdEfGh12345678","kind":"file","contentType":"text/plain","name":"f.txt","size":1,"status":7,"animated":"yes"}]}""",
        )
        val byId = parsed.content.attachments.associateBy { it.id }
        assertEquals(AttachmentState.READY, byId.getValue("AbCdEfGh12345678").state)
        assertEquals(AttachmentState.PROCESSING, byId.getValue("BbCdEfGh12345678").state)
        assertEquals("https://cdn.caper.chat/preview/b?exp=1&sig=p", byId.getValue("BbCdEfGh12345678").previewUrl)
        assertEquals(AttachmentState.FAILED, byId.getValue("CbCdEfGh12345678").state)
        val animated = byId.getValue("DbCdEfGh12345678")
        assertTrue(animated.animated)
        assertEquals("absent status means ready", AttachmentState.READY, animated.state)
        assertEquals("unknown status without a url waits", AttachmentState.PROCESSING, byId.getValue("EbCdEfGh12345678").state)
        assertFalse("a malformed new field drops only that file", "FbCdEfGh12345678" in byId)
    }

    @Test fun `fresh urls never make a processing file look ready`() {
        val processing = ChatAttachment(
            "AbCdEfGh12345678", "video", "video/mp4", "a.mp4", 10, status = "processing",
            previewUrl = "https://cdn.example/preview/a?exp=2000&sig=p",
        )
        val stale = AttachmentUrls("https://cdn.example/original/a?exp=3000&sig=a", "https://cdn.example/preview/a?exp=3000&sig=b")
        val refreshed = processing.withFreshUrls(stale)
        assertNull(refreshed.url)
        assertEquals(stale.previewUrl, refreshed.previewUrl)
        val ready = processing.copy(status = "ready", url = "https://cdn.example/original/a?exp=2000&sig=x")
        assertEquals(stale.url, ready.withFreshUrls(stale).url)
        // A preview-only refresh keeps a ready file's URL.
        assertEquals(ready.url, ready.withFreshUrls(AttachmentUrls(previewUrl = "https://cdn.example/preview/a?exp=4000&sig=c")).url)
        assertSame(ready, ready.withFreshUrls(AttachmentUrls()))
    }

    @Test fun `processing previews are refreshed near expiry too`() {
        val refresh = AttachmentUrlRefresh(nowSeconds = { 0 }, marginSeconds = 3_600)
        val processing = ChatAttachment("AbCdEfGh12345678", "video", "video/mp4", "a.mp4", 10, status = "processing", previewUrl = "https://cdn.example/preview/a?exp=100&sig=p")
        assertEquals(listOf("AbCdEfGh12345678"), refresh.expiring(listOf(processing)))
        assertTrue(refresh.afterLoadFailure(processing, 403))
    }

    @Test fun `upload errors use the web copy`() {
        assertEquals("You’ve used all of your file storage.", AttachmentPolicy.uploadErrorMessage(ApiException(413, "storage limit reached", "storage_full")))
        assertEquals("This file is too large to upload.", AttachmentPolicy.uploadErrorMessage(ApiException(413, "too large")))
        assertEquals("Uploading too quickly. Try again shortly.", AttachmentPolicy.uploadErrorMessage(ApiException(429, "slow down")))
        assertEquals("invalid file name", AttachmentPolicy.uploadErrorMessage(ApiException(400, "invalid file name")))
        assertEquals("Storage refused the upload (403).", AttachmentPolicy.uploadErrorMessage(UploadException("Storage refused the upload (403).")))
        assertEquals("This file could not be uploaded.", AttachmentPolicy.uploadErrorMessage(IllegalStateException("boom")))
    }
}
