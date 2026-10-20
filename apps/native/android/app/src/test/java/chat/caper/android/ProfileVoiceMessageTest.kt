package chat.caper.android

import chat.caper.android.data.*
import chat.caper.android.model.*
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
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

/** The profile card's voice message: sent alone to the person's DM, the typed draft untouched. */
class ProfileVoiceMessageTest {
    private val server = MockWebServer()
    @get:Rule val folder = TemporaryFolder()
    private val peer = "member000001"
    private val direct = DirectConversation("direct000001", DirectPeer(peer, "maya", "Maya"), "0", "0")
    private val author = ChatAuthor("account00001", "Jose", false)
    private val asset = "VoIcEmSg12345678"
    private val composer by lazy { ProfileMessageDrafts(CaperApi(baseUrl = server.url("/").toString())) }

    @After fun close() { server.close() }

    private fun recording(): PreparedAttachment {
        val file = folder.newFile(VoiceMessages.UPLOAD_NAME).apply { writeBytes(ByteArray(2048) { 3 }) }
        return PreparedAttachment(file, VoiceMessages.UPLOAD_NAME, VoiceMessages.UPLOAD_TYPE, "audio", file.length(),
            durationMs = 4_200, voice = true, waveform = VoiceMessages.encodeWaveform(ByteArray(64) { 1 }))
    }

    private fun stageText() {
        composer.text(peer, "Keep this typed text")
        val note = folder.newFile("note.txt").apply { writeText("staged") }
        composer.add(peer, listOf(DraftAttachmentUi("file-1", "note.txt", "file", compressing = false, localUrl = note.toURI().toString())))
        composer.ready(peer, "file-1", PreparedAttachment(note, "note.txt", "text/plain", "file", note.length()))
    }

    private suspend fun sendVoice(voice: PreparedAttachment = recording(), blocked: Set<String> = emptySet()) =
        composer.sendVoice(peer, peer, "maya", "account-token", voice, { "chat-token" to author }, { listOf(direct) }, { blocked })

    private fun request(): RecordedRequest = checkNotNull(server.takeRequest(5, TimeUnit.SECONDS)) { "Expected an API request." }

    private fun enqueueUpload() {
        server.enqueue(MockResponse().setResponseCode(201).setBody("""{"id":"$asset","kind":"audio",
            "upload":{"method":"PUT","url":"${server.url("/r2/original/$asset?X-Amz-Signature=abc")}","headers":{"content-type":"audio/mp4"}}}"""))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody("""{"id":"$asset","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":1900,
            "durationMs":4200,"voice":true,"transcript":{"status":"pending"}}"""))
    }

    @Test fun `a voice message goes alone to the DM and the typed draft stays`() = runTest {
        stageText()
        // Answers by path, echoing the client's idempotency key as the API does.
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = when {
                request.path == "/api/assets" -> MockResponse().setResponseCode(201).setBody("""{"id":"$asset","kind":"audio",
                    "upload":{"method":"PUT","url":"${server.url("/r2/original/$asset?X-Amz-Signature=abc")}","headers":{"content-type":"audio/mp4"}}}""")
                request.method == "PUT" -> MockResponse().setResponseCode(200)
                request.path == "/api/assets/$asset/complete" -> MockResponse().setBody("""{"id":"$asset","kind":"audio","contentType":"audio/mp4",
                    "name":"Voice message.m4a","size":1900,"durationMs":4200,"voice":true,"transcript":{"status":"pending"}}""")
                request.path == "/api/chat/channels/${direct.id}/messages" -> {
                    val sent = Json.parseToJsonElement(request.body.clone().readUtf8()).jsonObject
                    MockResponse().setBody("""{"id":"message00001","channelId":"${direct.id}","seq":"1",
                        "author":{"id":"${author.id}","name":"Jose","isGuest":false},
                        "content":{"version":1,"type":"text","text":"","attachments":[{"id":"$asset","kind":"audio","contentType":"audio/mp4",
                            "name":"Voice message.m4a","size":1900,"durationMs":4200,"voice":true,"url":"https://cdn.caper.chat/original/$asset?exp=1&sig=s"}]},
                        "createdAt":"2026-10-20T12:00:00Z","clientMessageId":${sent.getValue("clientMessageId")}}""")
                }
                else -> MockResponse().setResponseCode(404)
            }
        }

        val result = sendVoice()

        assertNotNull(result)
        assertEquals(direct.id, result!!.first.id)
        assertTrue(result.second.content.attachments.single().isVoiceMessage)
        val reserve = Json.parseToJsonElement(request().body.readUtf8()).jsonObject
        assertEquals(direct.id, reserve.getValue("channelId").jsonPrimitive.content)
        assertTrue(reserve.getValue("voice").jsonPrimitive.boolean)
        assertEquals(4_200L, reserve.getValue("durationMs").jsonPrimitive.long)
        assertNotNull(reserve["waveform"])
        assertEquals("PUT", request().method)
        assertEquals("/api/assets/$asset/complete", request().path)
        val send = request()
        assertEquals("/api/chat/channels/${direct.id}/messages", send.path)
        val body = Json.parseToJsonElement(send.body.readUtf8()).jsonObject
        assertEquals("", body.getValue("text").jsonPrimitive.content)
        assertEquals(listOf(asset), body.getValue("attachmentIds").jsonArray.map { it.jsonPrimitive.content })
        assertEquals("Keep this typed text", composer[peer].text)
        assertEquals(listOf("file-1"), composer[peer].files.map { it.key })
        assertNull(composer[peer].files.single().attachment)
        assertTrue(composer[peer].editable)
        assertNull(composer[peer].error)
    }

    @Test fun `a blocked person gets nothing and the draft stays`() = runTest {
        stageText()
        assertNull(sendVoice(blocked = setOf(peer)))
        assertEquals(0, server.requestCount)
        assertEquals(directMessageError(DM_BLOCKED), composer[peer].error)
        assertEquals("Keep this typed text", composer[peer].text)
        assertTrue(composer[peer].editable)
    }

    // Real time: the cancel runs under a 5 s timeout that virtual time would expire at once.
    @Test(timeout = 20_000) fun `a refused send gives the upload back and shows the error`() = runBlocking {
        enqueueUpload()
        server.enqueue(MockResponse().setResponseCode(403).setBody("""{"error":"not accepting DMs","code":"dm_not_accepted"}"""))
        server.enqueue(MockResponse().setResponseCode(204))
        assertNull(sendVoice())
        repeat(4) { request() }
        assertEquals("/api/assets/$asset/cancel", request().path)
        assertEquals(directMessageError(DM_NOT_ACCEPTED), composer[peer].error)
        assertTrue(composer[peer].editable)
    }

    @Test fun `only a voice recording is sent this way`() = runTest {
        val file = folder.newFile("song.mp3").apply { writeBytes(ByteArray(10)) }
        assertNull(sendVoice(PreparedAttachment(file, "song.mp3", "audio/mpeg", "audio", 10, durationMs = 1_000)))
        assertEquals(0, server.requestCount)
        assertTrue(composer.state.value.isEmpty())
    }
}
