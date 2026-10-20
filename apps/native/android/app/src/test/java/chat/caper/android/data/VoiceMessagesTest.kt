package chat.caper.android.data

import chat.caper.android.model.AttachmentsUpdate
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.ChatContent
import chat.caper.android.model.ChatMessage
import java.util.Base64
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.*
import org.junit.Test

class VoiceMessagesTest {
    private val id = "VoIcEmSg12345678"

    @Test fun `waveform is 64 peaks scaled so the loudest slice is 255`() {
        // 128 samples: each slice covers two, and the peak of the pair wins.
        val samples = IntArray(128) { if (it % 2 == 0) it * 10 else 0 }
        val waveform = VoiceMessages.waveform(samples)
        assertEquals(64, waveform.size)
        assertEquals(255, waveform.last().toInt() and 0xFF)
        assertEquals(0, waveform.first().toInt() and 0xFF)
        // Linear: slice 32's peak (sample 64 → 640) against the loudest (sample 126 → 1260).
        assertEquals(640 * 255 / 1260, waveform[32].toInt() and 0xFF)
        val heights = waveform.map { it.toInt() and 0xFF }
        assertTrue("rising samples give rising peaks", heights.zipWithNext().all { (a, b) -> b >= a })
    }

    @Test fun `short and silent recordings still give 64 bytes`() {
        val short = VoiceMessages.waveform(intArrayOf(0, 1000, 500))
        assertEquals(64, short.size)
        assertEquals("fewer samples than slices repeat the nearest", 255, short[30].toInt() and 0xFF)
        assertEquals(0, short[0].toInt())
        assertTrue(VoiceMessages.waveform(IntArray(500)).all { it.toInt() == 0 })
        assertTrue(VoiceMessages.waveform(IntArray(0)).all { it.toInt() == 0 })
        assertEquals("negative readings count as silence", 0, VoiceMessages.waveform(intArrayOf(-5, 0)).maxOf { it.toInt() })
    }

    @Test fun `waveform encodes as padded standard base64 and decodes back`() {
        val bytes = ByteArray(64) { (it * 4).toByte() }
        val encoded = VoiceMessages.encodeWaveform(bytes)
        assertEquals(88, encoded.length)
        assertTrue(encoded.endsWith("=="))
        assertFalse(encoded.contains('\n'))
        assertArrayEquals(bytes, Base64.getDecoder().decode(encoded))
        val levels = VoiceMessages.decodeWaveform(encoded)!!
        assertEquals(64, levels.size)
        assertEquals(252 / 255f, levels.last(), 0.0001f)
        assertEquals(0f, levels.first(), 0f)
    }

    @Test fun `malformed or out of range waveforms draw the placeholder`() {
        assertNull(VoiceMessages.decodeWaveform(null))
        assertNull(VoiceMessages.decodeWaveform(""))
        assertNull(VoiceMessages.decodeWaveform("not base64!"))
        assertNull(VoiceMessages.decodeWaveform(Base64.getEncoder().encodeToString(ByteArray(257))))
        assertEquals(256, VoiceMessages.decodeWaveform(Base64.getEncoder().encodeToString(ByteArray(256)))!!.size)
        assertEquals(1, VoiceMessages.decodeWaveform(Base64.getEncoder().encodeToString(byteArrayOf(-1)))!!.size)
        val flat = VoiceMessages.resample(null, 20)
        assertEquals(20, flat.size)
        assertTrue(flat.all { it == flat.first() && it > 0f })
    }

    @Test fun `resampling keeps peaks when shrinking and repeats when stretching`() {
        val levels = floatArrayOf(0f, 1f, 0.2f, 0.4f)
        assertArrayEquals(floatArrayOf(1f, 0.4f), VoiceMessages.resample(levels, 2), 0f)
        assertArrayEquals(floatArrayOf(0f, 0f, 1f, 1f, 0.2f, 0.2f, 0.4f, 0.4f), VoiceMessages.resample(levels, 8), 0f)
        assertArrayEquals(levels, VoiceMessages.resample(levels, 4), 0f)
        assertEquals(37, VoiceMessages.resample(FloatArray(256) { it / 255f }, 37).size)
        assertEquals(1f, VoiceMessages.resample(FloatArray(256) { it / 255f }, 37).last(), 0f)
        assertEquals(0, VoiceMessages.resample(levels, 0).size)
        // Any server length (1..=256) fills any bar count.
        assertArrayEquals(FloatArray(5) { 0.5f }, VoiceMessages.resample(floatArrayOf(0.5f), 5), 0f)
    }

    @Test fun `transcript states follow the contract and unknown shapes show nothing`() {
        fun parse(json: String?) = VoiceTranscript.of(json?.let { Json.parseToJsonElement(it) })
        assertEquals(VoiceTranscript.Pending, parse("""{"status":"pending"}"""))
        assertEquals(VoiceTranscript.Ready("Yo what's going on"), parse("""{"status":"ready","text":"Yo what's going on"}"""))
        assertEquals(VoiceTranscript.Ready("trimmed"), parse("""{"status":"ready","text":"  trimmed\n"}"""))
        assertEquals(VoiceTranscript.None, parse("""{"status":"ready","text":"   "}"""))
        assertEquals(VoiceTranscript.None, parse("""{"status":"ready"}"""))
        assertEquals(VoiceTranscript.None, parse("""{"status":"ready","text":7}"""))
        assertEquals(VoiceTranscript.None, parse("""{"status":"failed"}"""))
        assertEquals(VoiceTranscript.None, parse("""{"status":"summarising","text":"later"}"""))
        assertEquals(VoiceTranscript.None, parse("""{"text":"no status"}"""))
        assertEquals(VoiceTranscript.None, parse(""""pending""""))
        assertEquals(VoiceTranscript.None, parse("null"))
        assertEquals(VoiceTranscript.None, parse(null))
    }

    @Test fun `clocks round down and the player counts down while active`() {
        assertEquals("0:00", VoiceMessages.clock(0))
        assertEquals("0:00", VoiceMessages.clock(-40))
        assertEquals("0:04", VoiceMessages.clock(4_200))
        assertEquals("0:59", VoiceMessages.clock(59_999))
        assertEquals("1:05", VoiceMessages.clock(65_000))
        assertEquals("10:00", VoiceMessages.clock(VoiceMessages.MAX_DURATION_MS))
        assertEquals("0:04", VoiceMessages.playerClock(4_200, null, active = false))
        assertEquals("0:04", VoiceMessages.playerClock(4_200, 1_000, active = false))
        assertEquals("0:03", VoiceMessages.playerClock(4_200, 1_000, active = true))
        assertEquals("0:00", VoiceMessages.playerClock(4_200, 9_000, active = true))
        assertEquals("0:00", VoiceMessages.playerClock(null, null, active = false))
        assertEquals(2_100L, seekPosition(0.5f, 4_200))
        assertEquals(4_200L, seekPosition(3f, 4_200))
        assertEquals(0L, seekPosition(-1f, 4_200))
    }

    @Test fun `holds under a second are taps and ten minutes is the most`() {
        assertNull(VoiceMessages.recordedDuration(0))
        assertNull(VoiceMessages.recordedDuration(999))
        assertEquals(1_000L, VoiceMessages.recordedDuration(1_000))
        assertEquals(4_200L, VoiceMessages.recordedDuration(4_200))
        assertEquals(600_000L, VoiceMessages.recordedDuration(600_049))
    }

    @Test fun `meter levels map decibels into the bar height`() {
        assertEquals(0f, VoiceMessages.meterLevel(0), 0f)
        assertEquals(0f, VoiceMessages.meterLevel(-3), 0f)
        assertEquals(1f, VoiceMessages.meterLevel(32_767), 0f)
        assertEquals(1f, VoiceMessages.meterLevel(90_000), 0f)
        assertEquals(0f, VoiceMessages.meterLevel(50), 0f)
        val quiet = VoiceMessages.meterLevel(1_000)
        val loud = VoiceMessages.meterLevel(10_000)
        assertTrue(quiet in 0.01f..0.99f && loud > quiet)
    }

    private val voiceJson = """{"id":"$id","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":31337,
        "durationMs":4200,"url":"https://cdn.caper.chat/original/$id?exp=1&sig=s","voice":true,
        "waveform":"${VoiceMessages.encodeWaveform(ByteArray(64) { it.toByte() })}","transcript":{"status":"pending"}}"""

    @Test fun `voice fields parse and odd ones never drop the file`() {
        val lenient = Json { ignoreUnknownKeys = true }
        val parsed = lenient.decodeFromString(ChatAttachment.serializer(), voiceJson)
        assertTrue(parsed.voice)
        assertTrue(parsed.isVoiceMessage)
        assertEquals(VoiceTranscript.Pending, parsed.transcriptState)
        assertEquals(64, VoiceMessages.decodeWaveform(parsed.waveform)!!.size)
        val plain = parsed.copy(voice = false, waveform = null, transcript = null)
        assertFalse("regular audio keeps its file card", plain.isVoiceMessage)
        assertFalse(parsed.copy(kind = "file").isVoiceMessage)
        val message = Json.decodeFromString<ChatMessage>(
            """{"id":"message00001","channelId":"channel00001","seq":"4","author":{"id":"account00001","name":"A","isGuest":false},
            "content":{"version":1,"type":"text","text":"","attachments":[
                ${voiceJson.replace(""""transcript":{"status":"pending"}""", """"transcript":"oops"""")},
                ${voiceJson.replace(id, "VoIcEmSg22345678").replace(""""transcript":{"status":"pending"}""", """"transcript":{"status":"ready","text":"hi","confidence":0.9}""")}
            ]},"createdAt":"2026-10-03T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}""",
        )
        assertEquals(2, message.content.attachments.size)
        assertEquals(VoiceTranscript.None, message.content.attachments[0].transcriptState)
        assertEquals(VoiceTranscript.Ready("hi"), message.content.attachments[1].transcriptState)
        assertEquals(VoiceMessages.LABEL, VoiceMessages.excerptLabel(message.content.attachments))
    }

    @Test fun `a message attachments event moves a transcript from pending to ready and keeps the voice fields`() {
        val lenient = Json { ignoreUnknownKeys = true }
        val pending = lenient.decodeFromString(ChatAttachment.serializer(), voiceJson)
        val message = ChatMessage(
            "message00000001", "channel00001", "3", ChatAuthor("author", "A", false),
            ChatContent(1, "text", "", listOf(pending)), "now", "client", attachmentsSeq = "4",
        )
        val event = Json.parseToJsonElement(
            """{"type":"message.attachments","schemaVersion":1,"channelId":"channel00001","seq":"9","messageId":"message00000001",
            "attachments":[${voiceJson.replace(""""transcript":{"status":"pending"}""", """"transcript":{"status":"ready","text":"Yo what's going on"}""")}]}""",
        ).jsonObject
        assertEquals("9", attachmentsSequence(event, "channel00001"))
        val update = lenient.decodeFromJsonElement(AttachmentsUpdate.serializer(), event)
        val merged = mergeAttachments(message, update)
        val file = merged.content.attachments.single()
        assertTrue(file.voice)
        assertEquals(pending.waveform, file.waveform)
        assertEquals(4_200L, file.durationMs)
        assertEquals(VoiceTranscript.Ready("Yo what's going on"), file.transcriptState)
        assertNotEquals("the message changes, so the timeline recomposes", message, merged)
        assertSame("a replayed pending snapshot cannot undo the transcript", merged,
            mergeAttachments(merged, AttachmentsUpdate("message.attachments", 1, "channel00001", "8", message.id, listOf(pending))))
    }

    @Test fun `excerpts say voice message instead of the file name`() {
        val voice = ChatAttachment(id, "audio", "audio/mp4", "Voice message.m4a", 10, voice = true)
        val song = ChatAttachment(id, "audio", "audio/mpeg", "song.mp3", 10)
        assertEquals("Voice message", VoiceMessages.excerptLabel(listOf(voice)))
        assertEquals("Attachment", VoiceMessages.excerptLabel(listOf(song)))
        assertEquals("Attachment", VoiceMessages.excerptLabel(emptyList(), hasAttachments = true))
        assertEquals("Message", VoiceMessages.excerptLabel(emptyList()))
    }
}
