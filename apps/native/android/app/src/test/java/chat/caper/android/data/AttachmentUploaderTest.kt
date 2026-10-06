package chat.caper.android.data

import chat.caper.android.model.AttachmentState
import chat.caper.android.model.ChatAuthor
import java.io.ByteArrayInputStream
import java.io.InputStream
import java.util.UUID
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Test

class AttachmentUploaderTest {
    private val server = MockWebServer()

    @After fun close() { server.close() }

    private val id = "AbCdEfGh12345678"
    private val original = ByteArray(150_000) { (it * 31).toByte() }

    private class Bytes(
        private val bytes: ByteArray, override val name: String = "IMG_0001.HEIC",
        override val contentType: String = "image/heic", override val size: Long = bytes.size.toLong(),
    ) : UploadSource {
        var opened = 0
        override fun open(): InputStream { opened++; return ByteArrayInputStream(bytes) }
    }

    private fun reservation() = """{"id":"$id","kind":"file",
        "upload":{"method":"PUT","url":"${server.url("/s3/incoming/$id?X-Amz-Signature=abc")}","headers":{"content-type":"image/heic"}},
        "storage":{"used":2048,"limit":10737418240}}"""

    private val processing = """{"id":"$id","kind":"file","contentType":"image/heic","name":"IMG_0001.HEIC","size":150000,"status":"processing"}"""

    private fun uploader(api: CaperApi) = AttachmentUploader(api, completeRetryDelaysMs = listOf(1, 1, 1))

    @Test fun `reserve, put the unchanged original with exactly the presigned headers, complete, then send ids`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody(processing))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val progress = mutableListOf<Float>()
        val source = Bytes(original)

        val attachment = uploader(api).upload("account-secret", "channel00001", source, maxUploadBytes = 2L shl 30) { progress += it }

        assertEquals(id, attachment.id)
        assertEquals(AttachmentState.PROCESSING, attachment.state)
        assertNull(attachment.url)
        assertEquals(1f, progress.last())
        assertTrue(progress.zipWithNext().all { (a, b) -> b >= a })

        val create = server.takeRequest()
        assertEquals("POST", create.method)
        assertEquals("/api/assets", create.path)
        assertEquals("Bearer account-secret", create.headers["Authorization"])
        val body = Json.parseToJsonElement(create.body.readUtf8()).jsonObject
        // Exactly the contract fields: no client-side compression metadata or preview.
        assertEquals(setOf("channelId", "filename", "contentType", "byteSize"), body.keys)
        assertEquals("channel00001", body["channelId"]!!.jsonPrimitive.content)
        assertEquals("IMG_0001.HEIC", body["filename"]!!.jsonPrimitive.content)
        assertEquals("image/heic", body["contentType"]!!.jsonPrimitive.content)
        assertEquals(150_000L, body["byteSize"]!!.jsonPrimitive.long)

        val put = server.takeRequest()
        assertEquals("PUT", put.method)
        assertEquals("/s3/incoming/$id?X-Amz-Signature=abc", put.path)
        assertEquals("image/heic", put.headers["Content-Type"])
        assertEquals("150000", put.headers["Content-Length"])
        assertNull("not chunked", put.headers["Transfer-Encoding"])
        assertNull("no credentials go to storage", put.headers["Authorization"])
        assertNull(put.headers["x-caper-chat-token"])
        assertNull(put.headers["Cookie"])
        assertArrayEquals("the original bytes, unchanged", original, put.body.readByteArray())

        val complete = server.takeRequest()
        assertEquals("POST", complete.method)
        assertEquals("/api/assets/$id/complete", complete.path)
        assertEquals("Bearer account-secret", complete.headers["Authorization"])

        // The message carries the ids; text may be empty for a file-only send.
        server.enqueue(MockResponse().setBody("""{"id":"message00001","channelId":"channel00001","seq":"9","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"","attachments":[$processing]},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000009"}"""))
        val sent = api.sendMessage("account-secret", "chat-secret", "channel00001", ChatAuthor("account00001", "Jose", false),
            UUID.fromString("00000000-0000-4000-8000-000000000009"), "", listOf(id))
        assertEquals(AttachmentState.PROCESSING, sent.content.attachments.single().state)
        val send = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertEquals(listOf(id), send["attachmentIds"]!!.jsonArray.map { it.jsonPrimitive.content })
    }

    @Test fun `unknown types are declared as octet-stream and text-only sends omit ids`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody(processing))
        val api = CaperApi(baseUrl = server.url("/").toString())
        uploader(api).upload("account-secret", "channel00001", Bytes(ByteArray(10), name = "notes", contentType = "not a type"))
        val create = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertEquals("application/octet-stream", create["contentType"]!!.jsonPrimitive.content)
        server.takeRequest(); server.takeRequest()

        server.enqueue(MockResponse().setBody("""{"id":"message00001","channelId":"channel00001","seq":"9","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000009"}"""))
        api.sendMessage("account-secret", "chat-secret", "channel00001", ChatAuthor("account00001", "Jose", false),
            UUID.fromString("00000000-0000-4000-8000-000000000009"), "hi")
        assertEquals("""{"clientMessageId":"00000000-0000-4000-8000-000000000009","text":"hi"}""", server.takeRequest().body.readUtf8())
    }

    @Test fun `complete retries while storage has not seen the upload`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"upload not found"}"""))
        server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"upload not found"}"""))
        server.enqueue(MockResponse().setBody(processing))
        val attachment = uploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", Bytes(original))
        assertEquals(id, attachment.id)
        assertEquals(5, server.requestCount)
        repeat(2) { server.takeRequest() }
        repeat(3) { assertEquals("/api/assets/$id/complete", server.takeRequest().path) }
    }

    @Test fun `complete gives up after its retries and other errors are not retried`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        repeat(4) { server.enqueue(MockResponse().setResponseCode(409).setBody("""{"error":"upload not found"}""")) }
        val api = CaperApi(baseUrl = server.url("/").toString())
        val stuck = runCatching { uploader(api).upload("account-secret", "channel00001", Bytes(original)) }.exceptionOrNull()
        assertEquals(409, (stuck as ApiException).status)
        assertEquals(6, server.requestCount)

        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setResponseCode(422).setBody("""{"error":"size mismatch"}"""))
        val mismatch = runCatching { uploader(api).upload("account-secret", "channel00001", Bytes(original)) }.exceptionOrNull()
        assertEquals("The file changed while uploading. Try again.", AttachmentPolicy.uploadErrorMessage(mismatch!!))
        assertEquals(9, server.requestCount)
    }

    @Test fun `files over maxUploadBytes and empty files are refused before reserving`() = runTest {
        val api = CaperApi(baseUrl = server.url("/").toString())
        val tooLarge = runCatching { uploader(api).upload("account-secret", "channel00001", Bytes(original), maxUploadBytes = 100_000) }.exceptionOrNull()
        assertEquals("Files can be up to 98 KB.", AttachmentPolicy.uploadErrorMessage(tooLarge!!))
        val empty = runCatching { uploader(api).upload("account-secret", "channel00001", Bytes(ByteArray(0))) }.exceptionOrNull()
        assertEquals("This file is empty.", AttachmentPolicy.uploadErrorMessage(empty!!))
        assertEquals(0, server.requestCount)
    }

    @Test fun `storage full stops before any bytes are sent`() = runTest {
        server.enqueue(MockResponse().setResponseCode(413).setBody("""{"error":"storage limit reached","code":"storage_full"}"""))
        val source = Bytes(original)
        val error = runCatching {
            uploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", source)
        }.exceptionOrNull()
        assertEquals("You’ve used all of your file storage.", AttachmentPolicy.uploadErrorMessage(error!!))
        assertEquals(1, server.requestCount)
        assertEquals(0, source.opened)
    }

    @Test fun `a refused storage put is reported and never completed`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(403))
        val error = runCatching {
            uploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", Bytes(original))
        }.exceptionOrNull()
        assertTrue(error is UploadException)
        assertEquals("Storage refused the upload (403).", AttachmentPolicy.uploadErrorMessage(error!!))
        assertEquals(2, server.requestCount)
    }

    @Test fun `a source shorter than its declared size fails instead of sending a wrong length`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody(reservation()))
        server.enqueue(MockResponse().setResponseCode(200))
        val error = runCatching {
            uploader(CaperApi(baseUrl = server.url("/").toString()))
                .upload("account-secret", "channel00001", Bytes(ByteArray(10), size = 20))
        }.exceptionOrNull()
        assertTrue(error is UploadException)
    }

    @Test fun `usage and url refresh use bearer auth and ignore unrequested ids`() = runTest {
        server.enqueue(MockResponse().setBody("""{"used":1,"limit":2,"maxUploadBytes":2147483648,"compression":{"imageQuality":80}}"""))
        server.enqueue(MockResponse().setBody("""{"urls":{"$id":{"url":"https://cdn.caper.chat/original/$id?exp=1790086400&sig=n","previewUrl":"https://cdn.caper.chat/preview/$id?exp=1790086400&sig=p"},"BbCdEfGh12345678":{"previewUrl":"https://cdn.caper.chat/preview/b?exp=1790086400&sig=q"},"CbCdEfGh12345678":{},"ZZZZZZZZZZZZZZZZ":{"url":"https://cdn.caper.chat/original/z?exp=1&sig=z"}}}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val usage = api.assetUsage("account-secret")
        assertEquals(2147483648L, usage.maxUploadBytes)
        val urls = api.attachmentUrls("account-secret", listOf(id, "BbCdEfGh12345678", "CbCdEfGh12345678"))
        assertEquals("processing files may get only a preview", setOf(id, "BbCdEfGh12345678"), urls.keys)
        assertEquals(1790086400L, attachmentUrlExpiry(urls.getValue(id).url))
        assertNull(urls.getValue("BbCdEfGh12345678").url)
        assertEquals("/api/assets/usage", server.takeRequest().path)
        val refresh = server.takeRequest()
        assertEquals("/api/assets/urls", refresh.path)
        assertEquals("Bearer account-secret", refresh.headers["Authorization"])
        assertEquals("""{"ids":["$id","BbCdEfGh12345678","CbCdEfGh12345678"]}""", refresh.body.readUtf8())
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
