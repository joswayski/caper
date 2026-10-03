package chat.caper.android.data

import chat.caper.android.model.ChatAuthor
import java.util.UUID
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class AttachmentUploaderTest {
    private val server = MockWebServer()
    @get:Rule val folder = TemporaryFolder()

    @After fun close() { server.close() }

    private val id = "AbCdEfGh12345678"
    private val disposition = "attachment; filename=\"shot.png\"; filename*=UTF-8''shot.png"

    private fun reservation(preview: Boolean) = """{"id":"$id","kind":"image",
        "upload":{"method":"PUT","url":"${server.url("/r2/original/$id?X-Amz-Signature=abc")}","headers":{"content-type":"image/png","content-disposition":${JsonPrimitive(disposition)}}},
        ${if (preview) """"previewUpload":{"method":"PUT","url":"${server.url("/r2/preview/$id?X-Amz-Signature=def")}","headers":{"content-type":"image/webp"}},""" else ""}
        "storage":{"used":2048,"limit":1073741824}}"""

    private fun prepared(preview: Boolean): PreparedAttachment {
        val file = folder.newFile("shot.png").apply { writeBytes(ByteArray(1500) { it.toByte() }) }
        val small = if (preview) folder.newFile("preview.webp").apply { writeBytes(ByteArray(300) { 7 }) } else null
        return PreparedAttachment(file, "shot.png", "image/png", "image", sourceSize = 9000, width = 1920, height = 1080,
            preview = small, previewContentType = small?.let { "image/webp" })
    }

    @Test fun `reserve, put preview and original with exact headers, complete, then send ids`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation(preview = true)))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"image","contentType":"image/png","name":"shot.png","size":1500,"width":1920,"height":1080,"preview":{}}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val progress = mutableListOf<Float>()

        val attachment = AttachmentUploader(api).upload("account-secret", "channel00001", prepared(preview = true)) { progress += it }

        assertEquals(id, attachment.id)
        assertEquals(1f, progress.last())
        assertTrue(progress.zipWithNext().all { (a, b) -> b >= a })

        val create = server.takeRequest()
        assertEquals("POST", create.method)
        assertEquals("/api/assets", create.path)
        assertEquals("Bearer account-secret", create.headers["Authorization"])
        val body = Json.parseToJsonElement(create.body.readUtf8()).jsonObject
        assertEquals(setOf("channelId", "filename", "contentType", "byteSize", "sourceByteSize", "width", "height", "preview"), body.keys)
        assertEquals("channel00001", body["channelId"]!!.jsonPrimitive.content)
        assertEquals(1500L, body["byteSize"]!!.jsonPrimitive.long)
        assertEquals(9000L, body["sourceByteSize"]!!.jsonPrimitive.long)
        assertEquals(1920, body["width"]!!.jsonPrimitive.int)
        assertEquals("image/webp", body["preview"]!!.jsonObject["contentType"]!!.jsonPrimitive.content)
        assertEquals(300L, body["preview"]!!.jsonObject["byteSize"]!!.jsonPrimitive.long)

        val previewPut = server.takeRequest()
        assertEquals("PUT", previewPut.method)
        assertEquals("/r2/preview/$id?X-Amz-Signature=def", previewPut.path)
        assertEquals("image/webp", previewPut.headers["Content-Type"])
        assertEquals("300", previewPut.headers["Content-Length"])
        assertNull("no credentials go to storage", previewPut.headers["Authorization"])
        assertEquals(300L, previewPut.bodySize)

        val originalPut = server.takeRequest()
        assertEquals("/r2/original/$id?X-Amz-Signature=abc", originalPut.path)
        assertEquals("image/png", originalPut.headers["Content-Type"])
        assertEquals(disposition, originalPut.headers["Content-Disposition"])
        assertEquals("1500", originalPut.headers["Content-Length"])
        assertNull(originalPut.headers["Authorization"])
        assertNull(originalPut.headers["x-caper-chat-token"])
        assertArrayEquals(ByteArray(1500) { it.toByte() }, originalPut.body.readByteArray())

        val complete = server.takeRequest()
        assertEquals("POST", complete.method)
        assertEquals("/api/assets/$id/complete", complete.path)
        assertEquals("Bearer account-secret", complete.headers["Authorization"])

        // The message carries the ids; text may be empty for a file-only send.
        server.enqueue(MockResponse().setBody("""{"id":"message00001","channelId":"channel00001","seq":"9","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"","attachments":[{"id":"$id","kind":"image","contentType":"image/png","name":"shot.png","size":1500,"url":"https://cdn.caper.chat/original/$id?exp=1790000000&sig=s"}]},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000009"}"""))
        val sent = api.sendMessage("account-secret", "chat-secret", "channel00001", ChatAuthor("account00001", "Jose", false),
            UUID.fromString("00000000-0000-4000-8000-000000000009"), "", listOf(id))
        assertEquals(id, sent.content.attachments.single().id)
        val send = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertEquals(listOf(id), send["attachmentIds"]!!.jsonArray.map { it.jsonPrimitive.content })
    }

    @Test fun `no preview reservation means a single storage put and text-only sends omit ids`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation(preview = false)))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"image","contentType":"image/png","name":"shot.png","size":1500}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        AttachmentUploader(api).upload("account-secret", "channel00001", prepared(preview = false))
        val create = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertFalse("preview" in create.keys)
        assertEquals("/r2/original/$id?X-Amz-Signature=abc", server.takeRequest().path)
        assertEquals("/api/assets/$id/complete", server.takeRequest().path)

        server.enqueue(MockResponse().setBody("""{"id":"message00001","channelId":"channel00001","seq":"9","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000009"}"""))
        api.sendMessage("account-secret", "chat-secret", "channel00001", ChatAuthor("account00001", "Jose", false),
            UUID.fromString("00000000-0000-4000-8000-000000000009"), "hi")
        assertEquals("""{"clientMessageId":"00000000-0000-4000-8000-000000000009","text":"hi"}""", server.takeRequest().body.readUtf8())
    }

    @Test fun `storage full stops before any bytes are sent`() = runTest {
        server.enqueue(MockResponse().setResponseCode(413).setBody("""{"error":"storage limit reached","code":"storage_full"}"""))
        val error = runCatching {
            AttachmentUploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", prepared(preview = false))
        }.exceptionOrNull()
        assertEquals("You’ve used all of your file storage.", AttachmentPolicy.uploadErrorMessage(error!!))
        assertEquals(1, server.requestCount)
    }

    @Test fun `a refused storage put is reported and never completed`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation(preview = false)))
        server.enqueue(MockResponse().setResponseCode(403))
        val error = runCatching {
            AttachmentUploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", prepared(preview = false))
        }.exceptionOrNull()
        assertTrue(error is UploadException)
        assertEquals("Storage refused the upload (403).", AttachmentPolicy.uploadErrorMessage(error!!))
        assertEquals(2, server.requestCount)
    }

    @Test fun `usage and url refresh use bearer auth and ignore unrequested ids`() = runTest {
        server.enqueue(MockResponse().setBody("""{"used":1,"limit":2,"compression":{"imageQuality":80,"imageMaxEdge":2048,"paletteColors":64,"previewEdge":512,"videoMaxHeight":720,"videoBitrateKbps":2500,"audioBitrateKbps":96,"newSetting":1}}"""))
        server.enqueue(MockResponse().setBody("""{"urls":{"$id":{"url":"https://cdn.caper.chat/original/$id?exp=1790086400&sig=n","previewUrl":"https://cdn.caper.chat/preview/$id?exp=1790086400&sig=p"},"ZZZZZZZZZZZZZZZZ":{"url":"https://cdn.caper.chat/original/z?exp=1&sig=z"}}}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val usage = api.assetUsage("account-secret")
        assertEquals(80, usage.compression.imageQuality)
        assertEquals(64, usage.compression.paletteColors)
        assertEquals(720, usage.compression.videoMaxHeight)
        val urls = api.attachmentUrls("account-secret", listOf(id))
        assertEquals(setOf(id), urls.keys)
        assertEquals(1790086400L, attachmentUrlExpiry(urls.getValue(id).url))
        assertEquals("/api/assets/usage", server.takeRequest().path)
        val refresh = server.takeRequest()
        assertEquals("/api/assets/urls", refresh.path)
        assertEquals("Bearer account-secret", refresh.headers["Authorization"])
        assertEquals("""{"ids":["$id"]}""", refresh.body.readUtf8())
    }

    @Test fun `unconfigured uploads fail the usage check so the control stays hidden`() = runTest {
        server.enqueue(MockResponse().setResponseCode(503).setBody("""{"error":"uploads unavailable"}"""))
        val error = runCatching { CaperApi(baseUrl = server.url("/").toString()).assetUsage("account-secret") }.exceptionOrNull()
        assertEquals(503, (error as ApiException).status)
    }

    @Test fun `invalid asset ids never reach a url path`() {
        assertThrows(IllegalArgumentException::class.java) { "../../secret".assetPathId() }
        assertEquals(id, id.assetPathId())
    }
}
