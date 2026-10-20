package chat.caper.android

import chat.caper.android.data.*
import chat.caper.android.model.*
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import kotlinx.serialization.json.*
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.After
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class ProfileMessageDraftsTest {
    private val server = MockWebServer()
    @get:Rule val folder = TemporaryFolder()
    private val peer = "member000001"
    private val direct = DirectConversation("direct000001", DirectPeer(peer, "maya", "Maya"), "0", "0")
    private val author = ChatAuthor("account00001", "Jose", false)
    private val asset = "AbCdEfGh12345678"
    private val composer by lazy { ProfileMessageDrafts(CaperApi(baseUrl = server.url("/").toString())) }

    @After fun close() { server.close() }

    private fun file(person: String = peer, key: String = "file-1") {
        val bytes = folder.newFile(key).apply { writeText("profile file bytes") }
        val prepared = PreparedAttachment(bytes, "note.txt", "text/plain", "file", bytes.length())
        composer.add(person, listOf(DraftAttachmentUi(key, "note.txt", "file", compressing = false, localUrl = bytes.toURI().toString())))
        composer.ready(person, key, prepared)
    }

    private suspend fun send(directs: List<DirectConversation> = emptyList(), blocked: Set<String> = emptySet()) =
        composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { directs }, { blocked })

    private fun request(): RecordedRequest = checkNotNull(server.takeRequest(5, TimeUnit.SECONDS)) { "Expected an API request." }

    private fun sent(request: RecordedRequest): MockResponse {
        val body = Json.parseToJsonElement(request.body.clone().readUtf8()).jsonObject
        return MockResponse().setBody("""{"id":"message00001","channelId":"${direct.id}","seq":"1",
            "author":{"id":"${author.id}","name":"Jose","isGuest":false},
            "content":{"version":1,"type":"text","text":${body.getValue("text")}},
            "createdAt":"2026-10-11T12:00:00Z","clientMessageId":${body.getValue("clientMessageId")}}""")
    }

    @Test fun `picker media sends alone and preserves the profile draft`() = runTest {
        val gif = KlipyGif(kind = "sticker", slug = "wave", width = 100, height = 100, gif = "https://media.klipy.com/wave.gif")
        composer.text(peer, "Keep my text")
        file()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = sent(request)
        }
        val result = composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { listOf(direct) }, { emptySet() }, gif)
        assertNotNull(result)
        val payload = Json.parseToJsonElement(request().body.readUtf8()).jsonObject
        assertEquals(JsonPrimitive(""), payload["text"])
        assertEquals(gif.toJson(), payload["gif"])
        assertNull(payload["attachmentIds"])
        assertEquals(1, server.requestCount)
        assertEquals("Keep my text", composer[peer].text)
        assertEquals("file-1", composer[peer].files.single().key)
        assertTrue(composer[peer].editable)
    }

    @Test fun `unconfirmed picker media retries the same payload even with an unprepared draft file`() = runTest {
        val gif = KlipyGif(kind = "gif", slug = "wave", width = 100, height = 100, gif = "https://media.klipy.com/wave.gif")
        composer.add(peer, listOf(DraftAttachmentUi("unprepared", "photo.jpg", "image")))
        var attempts = 0
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse =
                if (++attempts == 1) MockResponse().setResponseCode(503) else sent(request)
        }
        assertNull(composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { listOf(direct) }, { emptySet() }, gif, "hello"))
        val original = request().body.readUtf8()
        assertTrue(composer[peer].uncertain)
        assertTrue(composer[peer].canSend)
        assertEquals("hello", composer[peer].pendingGifQuery)
        assertNull(send(listOf(direct), blocked = setOf(peer)))
        assertEquals(gif, composer[peer].pendingGif)
        assertEquals("hello", composer[peer].pendingGifQuery)
        assertNull(composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { listOf(direct) }, { emptySet() }, gif.copy(slug = "other")))
        assertNotNull(send(listOf(direct)))
        assertEquals(original, request().body.readUtf8())
        assertEquals("unprepared", composer[peer].files.single().key)
        assertTrue(composer[peer].editable)
        assertNull(composer[peer].pendingGif)
        assertEquals(2, server.requestCount)
    }

    @Test fun `picker media refuses blocked peers and incoming requests without sending`() = runTest {
        val gif = KlipyGif(kind = "gif", slug = "wave", width = 100, height = 100, gif = "https://media.klipy.com/wave.gif")
        assertNull(composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { listOf(direct) }, { setOf(peer) }, gif))
        assertNull(composer.send(peer, peer, "maya", "account-token", { "chat-token" to author }, { listOf(direct.copy(status = "incoming")) }, { emptySet() }, gif))
        assertEquals(0, server.requestCount)
        assertTrue(composer[peer].editable)
    }

    @Test fun `typing and local preparation keep separate drafts without creating a DM`() {
        composer.text(peer, "Maya draft 🙂")
        file()
        composer.text("jordan000001", "Jordan draft")
        assertEquals("Maya draft 🙂", composer[peer].text)
        assertEquals("note.txt", composer[peer].files.single().name)
        assertEquals("Jordan draft", composer["jordan000001"].text)
        assertEquals(0, server.requestCount)
        assertEquals(listOf("file-1"), composer.clear().map { it.key })
        composer.updateFile(peer, "file-1") { it.copy(progress = 1f) }
        assertTrue(composer.state.value.isEmpty())
    }

    @Test fun `blocks incoming requests and server privacy refusal keep text and files without uploading`() = runTest {
        composer.text(peer, "Keep this draft")
        file()
        assertTrue(runCatching { composer.upload(peer, peer, "maya", "account-token", { listOf(direct) }, { setOf(peer) }) }.isFailure)
        assertTrue(runCatching { composer.upload(peer, peer, "maya", "account-token", { listOf(direct.copy(status = "incoming")) }, { emptySet() }) }.isFailure)
        assertNull(send(blocked = setOf(peer)))
        assertEquals(directMessageError(DM_BLOCKED), composer[peer].error)
        assertNull(send(listOf(direct.copy(status = "incoming"))))
        assertEquals("Open conversation and accept this message request before replying.", composer[peer].error)
        assertEquals(0, server.requestCount)
        composer.clear()
        composer.text(peer, "Keep this draft")
        file(key = "privacy-file")
        server.enqueue(MockResponse().setBody("""{"conversations":[]}"""))
        server.enqueue(MockResponse().setResponseCode(403).setBody("""{"error":"not accepting DMs","code":"dm_not_accepted"}"""))
        assertNull(send())
        assertEquals(2, server.requestCount)
        assertEquals("GET", request().method)
        assertEquals("/api/dms", request().path)
        assertEquals(directMessageError(DM_NOT_ACCEPTED), composer[peer].error)
        assertEquals("Keep this draft", composer[peer].text)
        assertEquals("note.txt", composer[peer].files.single().name)
        assertTrue(composer[peer].editable)
    }

    @Test fun `an incoming request missing from local caches is refused and list errors never fall through to creation`() = runTest {
        composer.text(peer, "Keep the uncached request draft")
        file()
        server.enqueue(MockResponse().setBody("""{"conversations":[{"id":"${direct.id}","peer":{"id":"$peer","username":"maya","displayName":"Maya"},"lastSeq":"0","readSeq":"0","status":"incoming"}]}"""))
        assertNull(send())
        assertEquals("Open conversation and accept this message request before replying.", composer[peer].error)
        assertEquals(1, server.requestCount)
        val lookup = request()
        assertEquals("GET", lookup.method)
        assertEquals("/api/dms", lookup.path)
        assertEquals("Keep the uncached request draft", composer[peer].text)
        assertEquals("note.txt", composer[peer].files.single().name)

        server.enqueue(MockResponse().setResponseCode(503).setBody("""{"error":"DM list unavailable"}"""))
        assertNull(send())
        assertEquals(2, server.requestCount)
        assertEquals("GET", request().method)
        assertEquals("Keep the uncached request draft", composer[peer].text)
        assertTrue(composer[peer].editable)
    }

    @Test fun `eager resolution rejects a changed recipient before uploading`() = runTest {
        file()
        server.enqueue(MockResponse().setBody("""{"conversations":[]}"""))
        server.enqueue(MockResponse().setBody("""{"id":"${direct.id}","peer":{"id":"other0000001","username":"maya","displayName":"Maya"},"lastSeq":"0","readSeq":"0"}"""))
        val error = runCatching { composer.upload(peer, peer, "maya", "account-token", { emptyList() }, { emptySet() }) }.exceptionOrNull()
        assertEquals("Direct message recipient mismatch.", error?.message)
        assertEquals(2, server.requestCount)
        assertNull(composer[peer].files.single().attachment)
        assertTrue(composer[peer].editable)
    }

    @Test fun `eager uploads coalesce without sending and Send retries exactly the same message without reuploading`() = runTest {
        file()
        composer.text("jordan000001", "Unrelated draft")
        var attempts = 0
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = when (request.path) {
                "/api/dms" -> if (request.method == "GET") MockResponse().setBody("""{"conversations":[]}""")
                    else MockResponse().setBody("""{"id":"${direct.id}","peer":{"id":"$peer","username":"maya","displayName":"Maya"},"lastSeq":"0","readSeq":"0","status":"outgoing"}""")
                "/api/assets" -> MockResponse().setBody("""{"id":"$asset","kind":"file","upload":{"method":"PUT","url":"${server.url("/storage")}","headers":{"content-type":"text/plain"}},"storage":{"used":0,"limit":1000}}""")
                "/storage" -> {
                    entered.complete(Unit)
                    check(release.await(5, TimeUnit.SECONDS))
                    MockResponse()
                }
                "/api/assets/$asset/complete" -> MockResponse().setBody("""{"id":"$asset","kind":"file","contentType":"text/plain","name":"note.txt","size":18}""")
                "/api/chat/channels/${direct.id}/messages" -> if (++attempts == 1) MockResponse().setResponseCode(503).setBody("""{"error":"messages unavailable"}""") else sent(request)
                else -> MockResponse().setResponseCode(404)
            }
        }
        val firstUpload = launch { composer.upload(peer, peer, "maya", "account-token", { emptyList() }, { emptySet() }) }
        val secondUpload = launch { composer.upload(peer, peer, "maya", "account-token", { emptyList() }, { emptySet() }) }
        entered.await()
        assertEquals(0, attempts)
        assertTrue(composer[peer].editable)
        val sending = launch { assertNull(send()) }
        yield()
        assertTrue(composer[peer].sending)
        release.countDown()
        firstUpload.join(); secondUpload.join(); sending.join()
        assertEquals(6, server.requestCount)
        assertEquals(asset, composer[peer].files.single().attachment?.id)
        assertTrue(composer[peer].uncertain)
        composer.text(peer, "Must not replace the unknown send")
        assertFalse(composer.remove(peer, "file-1"))
        assertEquals("", composer[peer].text)
        assertEquals("GET", request().method)
        val paths = (1..4).map { request().path }
        assertEquals(listOf("/api/dms", "/api/assets", "/storage", "/api/assets/$asset/complete"), paths)
        val first = request().body.readUtf8()
        assertEquals(listOf(asset), Json.parseToJsonElement(first).jsonObject.getValue("attachmentIds").jsonArray.map { it.jsonPrimitive.content })
        composer.updateFile(peer, "file-1") { it.copy(uploadedAt = 0L) }
        assertNotNull(send())
        val retry = request()
        assertEquals("/api/chat/channels/${direct.id}/messages", retry.path)
        assertEquals(first, retry.body.readUtf8())
        assertEquals(7, server.requestCount)
        assertEquals(ProfileMessageDraft(), composer[peer])
        assertFalse(peer in composer.state.value)
        assertEquals("Unrelated draft", composer["jordan000001"].text)
    }

    @Test fun `definitive send rejection keeps draft editable and next send gets a new client id`() = runTest {
        var attempts = 0
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse =
                if (++attempts == 1) MockResponse().setResponseCode(403).setBody("""{"error":"not accepting DMs","code":"dm_not_accepted"}""") else sent(request)
        }
        composer.text(peer, "First attempt")
        assertNull(send(listOf(direct)))
        val first = Json.parseToJsonElement(request().body.readUtf8()).jsonObject
        assertTrue(composer[peer].editable)
        assertEquals("First attempt", composer[peer].text)
        composer.text(peer, "Corrected 🙂")
        val result = send(listOf(direct))
        assertEquals("Corrected 🙂", result?.second?.content?.text)
        val next = Json.parseToJsonElement(request().body.readUtf8()).jsonObject
        assertNotEquals(first.getValue("clientMessageId"), next.getValue("clientMessageId"))
        assertEquals("Corrected 🙂", next.getValue("text").jsonPrimitive.content)
    }

    @Test fun `upload failure keeps the file and retry resumes the same draft`() = runTest {
        composer.text(peer, "Caption")
        file()
        server.enqueue(MockResponse().setResponseCode(413).setBody("""{"error":"storage full","code":"storage_full"}"""))
        assertTrue(runCatching { composer.upload(peer, peer, "maya", "account-token", { listOf(direct) }, { emptySet() }) }.isFailure)
        assertEquals("You’ve used all of your file storage.", composer[peer].files.single().error)
        assertTrue(composer[peer].editable)
        server.enqueue(MockResponse().setResponseCode(413).setBody("""{"error":"storage full","code":"storage_full"}"""))
        assertNull(send(listOf(direct)))
        assertEquals(2, server.requestCount)
        assertEquals("You’ve used all of your file storage.", composer[peer].error)
        assertEquals("Caption", composer[peer].text)
        assertEquals("note.txt", composer[peer].files.single().name)
        assertTrue(composer[peer].editable)
        assertFalse(composer[peer].uncertain)
    }

    @Test(timeout = 10000) fun `expired uploads are cancelled and renewed before sending`() = runBlocking {
        file()
        composer.updateFile(peer, "file-1") {
            it.copy(attachment = ChatAttachment(asset, "file", "text/plain", "note.txt", 18), uploadedAt = 0L)
        }
        server.enqueue(MockResponse().setBody("{}"))
        server.enqueue(MockResponse().setResponseCode(413).setBody("""{"error":"storage full","code":"storage_full"}"""))
        assertTrue(runCatching { composer.upload(peer, peer, "maya", "account-token", { listOf(direct) }, { emptySet() }) }.isFailure)
        assertEquals("/api/assets/$asset/cancel", request().path)
        assertEquals("/api/assets", request().path)
        assertNull(composer[peer].files.single().attachment)
        assertEquals(2, server.requestCount)
        assertFalse(composer[peer].uncertain)
    }

    @Test fun `logout invalidates both resolving and queued eager uploads`() = runTest {
        file()
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                entered.complete(Unit)
                check(release.await(5, TimeUnit.SECONDS))
                return MockResponse().setBody("""{"conversations":[]}""")
            }
        }
        val resolving = launch { composer.upload(peer, peer, "maya", "account-token", { emptyList() }, { emptySet() }) }
        entered.await()
        val queued = launch { composer.upload(peer, peer, "maya", "account-token", { emptyList() }, { emptySet() }) }
        yield()
        composer.clear()
        release.countDown()
        resolving.join(); queued.join()
        assertTrue(composer.state.value.isEmpty())
        assertEquals(1, server.requestCount)
        assertEquals("GET", request().method)
    }

    @Test fun `logout during a send prevents late resolution or draft restoration`() = runTest {
        composer.text(peer, "Do not send after logout")
        file()
        val entered = CompletableDeferred<Unit>()
        val resume = CompletableDeferred<Unit>()
        val job = launch {
            composer.send(peer, peer, "maya", "account-token", {
                entered.complete(Unit); resume.await(); "chat-token" to author
            }, { emptyList() }, { emptySet() })
        }
        entered.await()
        assertTrue(composer[peer].sending)
        composer.clear()
        resume.complete(Unit)
        job.join()
        assertTrue(composer.state.value.isEmpty())
        assertEquals(0, server.requestCount)
    }
}
