package chat.caper.android

import chat.caper.android.data.*
import chat.caper.android.model.*
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
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

/**
 * The profile card's voice message: sent alone to the person's DM, the typed draft untouched; a
 * send that fails or isn't confirmed waits for Retry (same upload, same message ID once a send may
 * have arrived) or Discard. Real time throughout: cancels run under a 5 s timeout.
 */
class ProfileVoiceMessageTest {
    private val server = MockWebServer()
    @get:Rule val folder = TemporaryFolder()
    private val peer = "member000001"
    private val direct = DirectConversation("direct000001", DirectPeer(peer, "maya", "Maya"), "0", "0")
    private val author = ChatAuthor("account00001", "Jose", false)
    private val asset = "VoIcEmSg12345678"
    private val key = "voice-key"
    private val composer by lazy { ProfileMessageDrafts(CaperApi(baseUrl = server.url("/").toString())) }
    /** Statuses for the next message sends; 200 echoes the request's idempotency key. */
    private val sends = ConcurrentLinkedQueue<Int>()
    private var putStatus = 200

    @After fun close() { server.close() }

    private fun serve() {
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = when {
                request.path == "/api/assets" -> MockResponse().setResponseCode(201).setBody("""{"id":"$asset","kind":"audio",
                    "upload":{"method":"PUT","url":"${server.url("/r2/original/$asset?X-Amz-Signature=abc")}","headers":{"content-type":"audio/mp4"}}}""")
                request.method == "PUT" -> MockResponse().setResponseCode(putStatus)
                request.path == "/api/assets/$asset/complete" -> MockResponse().setBody("""{"id":"$asset","kind":"audio","contentType":"audio/mp4",
                    "name":"Voice message.m4a","size":1900,"durationMs":4200,"voice":true,"transcript":{"status":"pending"}}""")
                request.path == "/api/assets/$asset/cancel" -> MockResponse().setResponseCode(204)
                request.path == "/api/chat/channels/${direct.id}/messages" -> when (val status = sends.poll() ?: 200) {
                    200 -> {
                        val sent = Json.parseToJsonElement(request.body.clone().readUtf8()).jsonObject
                        MockResponse().setBody("""{"id":"message00001","channelId":"${direct.id}","seq":"1",
                            "author":{"id":"${author.id}","name":"Jose","isGuest":false},
                            "content":{"version":1,"type":"text","text":"","attachments":[{"id":"$asset","kind":"audio","contentType":"audio/mp4",
                                "name":"Voice message.m4a","size":1900,"durationMs":4200,"voice":true,"url":"https://cdn.caper.chat/original/$asset?exp=1&sig=s"}]},
                            "createdAt":"2026-10-20T12:00:00Z","clientMessageId":${sent.getValue("clientMessageId")}}""")
                    }
                    403 -> MockResponse().setResponseCode(403).setBody("""{"error":"not accepting DMs","code":"dm_not_accepted"}""")
                    else -> MockResponse().setResponseCode(status).setBody("""{"error":"try again"}""")
                }
                else -> MockResponse().setResponseCode(404)
            }
        }
    }

    private fun recording(): PreparedAttachment {
        val file = folder.newFile().apply { writeBytes(ByteArray(2048) { 3 }) }
        return PreparedAttachment(file, VoiceMessages.UPLOAD_NAME, VoiceMessages.UPLOAD_TYPE, "audio", file.length(),
            durationMs = 4_200, voice = true, waveform = VoiceMessages.encodeWaveform(ByteArray(64) { 1 }))
    }

    private fun stageText() {
        composer.text(peer, "Keep this typed text")
        val note = folder.newFile("note.txt").apply { writeText("staged") }
        composer.add(peer, listOf(DraftAttachmentUi("file-1", "note.txt", "file", compressing = false, localUrl = note.toURI().toString())))
        composer.ready(peer, "file-1", PreparedAttachment(note, "note.txt", "text/plain", "file", note.length()))
    }

    private val session: suspend () -> Pair<String, ChatAuthor> = { "chat-token" to author }

    private suspend fun sendVoice(voice: PreparedAttachment = recording(), blocked: Set<String> = emptySet(), session: suspend () -> Pair<String, ChatAuthor> = this.session) =
        composer.sendVoice(peer, peer, "maya", "account-token", key, voice, session, { listOf(direct) }, { blocked })

    private suspend fun retry() = composer.retryVoice(peer, peer, "maya", "account-token", session, { listOf(direct) }, { emptySet() })

    private fun request(): RecordedRequest = checkNotNull(server.takeRequest(5, TimeUnit.SECONDS)) { "Expected an API request." }

    private fun requests(): List<RecordedRequest> = generateSequence { server.takeRequest(200, TimeUnit.MILLISECONDS) }.toList()

    @Test fun `a voice message goes alone to the DM and the typed draft stays`() = runBlocking {
        serve()
        stageText()
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
        val body = Json.parseToJsonElement(request().body.readUtf8()).jsonObject
        assertEquals("", body.getValue("text").jsonPrimitive.content)
        assertEquals(listOf(asset), body.getValue("attachmentIds").jsonArray.map { it.jsonPrimitive.content })
        assertEquals("Keep this typed text", composer[peer].text)
        assertEquals(listOf("file-1"), composer[peer].files.map { it.key })
        assertNull(composer[peer].files.single().attachment)
        assertTrue(composer[peer].editable)
        assertNull(composer[peer].error)
        assertNull(composer[peer].voice)
        assertNull(composer.voiceKey(peer))
    }

    @Test fun `an unconfirmed send waits and Retry resends the same upload and message`() = runBlocking {
        serve()
        stageText()
        sends += 503
        assertNull(sendVoice())
        assertEquals(ProfileVoiceStatus(uncertain = true), composer[peer].voice)
        assertEquals("Voice message delivery isn’t confirmed. Retry resends the same message.", composer[peer].voice!!.message)
        assertEquals(key, composer.voiceKey(peer))
        assertEquals("Keep this typed text", composer[peer].text)
        assertTrue("typing and Send stay available", composer[peer].editable)
        val first = requests().last { it.path!!.endsWith("/messages") }
        assertNull("one waits at a time", sendVoice())

        val sent = retry()
        assertNotNull(sent)
        val again = requests()
        assertEquals("no new reservation or upload", listOf("/api/chat/channels/${direct.id}/messages"), again.map { it.path })
        val firstBody = Json.parseToJsonElement(first.body.readUtf8()).jsonObject
        val retryBody = Json.parseToJsonElement(again.single().body.readUtf8()).jsonObject
        assertEquals(firstBody.getValue("clientMessageId"), retryBody.getValue("clientMessageId"))
        assertEquals(firstBody.getValue("attachmentIds"), retryBody.getValue("attachmentIds"))
        assertNull(composer[peer].voice)
        assertNull(composer.voiceKey(peer))
        assertEquals("Keep this typed text", composer[peer].text)
    }

    @Test fun `a send that never left keeps the recording and Retry sends it`() = runBlocking {
        serve()
        putStatus = 500
        assertNull(sendVoice())
        assertEquals(ProfileVoiceStatus(uncertain = false), composer[peer].voice)
        assertEquals("Voice message didn’t send.", composer[peer].voice!!.message)
        assertTrue(requests().none { it.path!!.endsWith("/messages") })
        putStatus = 200
        assertNotNull(retry())
        assertNull(composer.voiceKey(peer))
        assertTrue("an empty draft goes away once sent", composer.state.value.isEmpty())
    }

    @Test fun `Discard cancels the upload and the typed draft stays`() = runBlocking {
        serve()
        stageText()
        sends += 500
        assertNull(sendVoice())
        requests()
        assertEquals(key, composer.discardVoice(peer, "account-token"))
        assertEquals("/api/assets/$asset/cancel", request().path)
        assertNull(composer[peer].voice)
        assertNull(composer.voiceKey(peer))
        assertEquals("Keep this typed text", composer[peer].text)
        assertNull(composer.discardVoice(peer, "account-token"))
    }

    @Test fun `signing out returns the waiting recording so its upload and copy go`() = runBlocking {
        serve()
        sends += 502
        assertNull(sendVoice())
        val dropped = composer.clear().single()
        assertEquals(key, dropped.key)
        assertEquals(asset, dropped.attachment?.id)
        assertNull(composer.voiceKey(peer))
    }

    @Test fun `a blocked person gets nothing and the recording is dropped`() = runBlocking {
        serve()
        stageText()
        assertNull(sendVoice(blocked = setOf(peer)))
        assertEquals(0, server.requestCount)
        assertEquals(directMessageError(DM_BLOCKED), composer[peer].error)
        assertNull(composer[peer].voice)
        assertNull(composer.voiceKey(peer))
        assertEquals("Keep this typed text", composer[peer].text)
        assertTrue(composer[peer].editable)
    }

    @Test fun `a refused send drops the recording, gives the upload back and shows the error`() = runBlocking {
        serve()
        sends += 403
        assertNull(sendVoice())
        assertEquals("/api/assets/$asset/cancel", requests().last().path)
        assertEquals(directMessageError(DM_NOT_ACCEPTED), composer[peer].error)
        assertNull(composer[peer].voice)
        assertNull(composer.voiceKey(peer))
        assertTrue(composer[peer].editable)
    }

    @Test fun `only a voice recording is sent this way`() = runBlocking {
        serve()
        val file = folder.newFile("song.mp3").apply { writeBytes(ByteArray(10)) }
        assertNull(sendVoice(PreparedAttachment(file, "song.mp3", "audio/mpeg", "audio", 10, durationMs = 1_000)))
        assertEquals(0, server.requestCount)
        assertTrue(composer.state.value.isEmpty())
    }
}
