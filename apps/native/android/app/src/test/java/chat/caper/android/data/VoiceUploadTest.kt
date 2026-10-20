package chat.caper.android.data

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
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

class VoiceUploadTest {
    private val server = MockWebServer()
    @get:Rule val folder = TemporaryFolder()

    @After fun close() { server.close() }

    private val id = "VoIcEmSg12345678"

    @Test fun `a recording reserves as voice with its duration and waveform, uploads unchanged, and keeps the completed description`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody("""{"id":"$id","kind":"audio",
            "upload":{"method":"PUT","url":"${server.url("/r2/original/$id?X-Amz-Signature=abc")}","headers":{"content-type":"audio/mp4"}}}"""))
        server.enqueue(MockResponse().setResponseCode(200))
        // The server transcodes and renames on complete; the client sends what it says.
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":2900,
            "durationMs":4200,"voice":true,"waveform":"AAEC","transcript":{"status":"pending"}}"""))
        val bytes = ByteArray(3000) { (it % 97).toByte() }
        val recording = folder.newFile(VoiceMessages.UPLOAD_NAME).apply { writeBytes(bytes) }
        val waveform = VoiceMessages.encodeWaveform(ByteArray(64) { 9 })
        val prepared = PreparedAttachment(recording, VoiceMessages.UPLOAD_NAME, VoiceMessages.UPLOAD_TYPE, "audio", recording.length(),
            durationMs = 4_200, voice = true, waveform = waveform)

        val attachment = AttachmentUploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001", prepared)

        val create = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertEquals(setOf("channelId", "filename", "contentType", "byteSize", "sourceByteSize", "durationMs", "voice", "waveform"), create.keys)
        assertTrue(create["voice"]!!.jsonPrimitive.boolean)
        assertEquals(waveform, create["waveform"]!!.jsonPrimitive.content)
        assertEquals(4_200L, create["durationMs"]!!.jsonPrimitive.long)
        assertEquals("audio/mp4", create["contentType"]!!.jsonPrimitive.content)
        assertEquals(3000L, create["byteSize"]!!.jsonPrimitive.long)
        assertArrayEquals("the recording is uploaded as recorded", bytes, server.takeRequest().body.readByteArray())
        assertEquals("/api/assets/$id/complete", server.takeRequest().path)
        assertEquals("Voice message.m4a", attachment.name)
        assertEquals(2900L, attachment.size)
        assertTrue(attachment.isVoiceMessage)
        assertEquals(VoiceTranscript.Pending, attachment.transcriptState)
    }

    @Test fun `regular files never send voice fields`() = runTest {
        server.enqueue(MockResponse().setResponseCode(201).setBody("""{"id":"$id","kind":"audio",
            "upload":{"method":"PUT","url":"${server.url("/r2/original/$id?X-Amz-Signature=abc")}","headers":{}}}"""))
        server.enqueue(MockResponse().setResponseCode(200))
        server.enqueue(MockResponse().setBody("""{"id":"$id","kind":"audio","contentType":"audio/mpeg","name":"song.mp3","size":10}"""))
        val song = folder.newFile("song.mp3").apply { writeBytes(ByteArray(10)) }
        // A waveform without the voice flag is ignored.
        AttachmentUploader(CaperApi(baseUrl = server.url("/").toString())).upload("account-secret", "channel00001",
            PreparedAttachment(song, "song.mp3", "audio/mpeg", "audio", 10, durationMs = 1_000, waveform = "AAAA"))
        val create = Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject
        assertFalse("voice" in create.keys)
        assertFalse("waveform" in create.keys)
    }
}
