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

    @Test fun `the stored waveform is per-slice RMS on a decibel scale, smoothed and scaled to 255`() {
        // 128 samples: each slice covers two. Rising loudness gives rising levels.
        val samples = IntArray(128) { 200 + it * 250 }
        val waveform = VoiceMessages.waveform(samples).map { it.toInt() and 0xFF }
        assertEquals(64, waveform.size)
        assertEquals(255, waveform.max())
        assertTrue("rising input stays rising after smoothing", waveform.zipWithNext().all { (a, b) -> b >= a })
        // RMS, not peak: one loud click in a quiet slice moves it much less than a loud slice.
        val click = IntArray(640) { if (it == 5) 30_000 else 300 }.also { for (at in 600 until 640) it[at] = 30_000 }
        val levels = VoiceMessages.waveform(click).map { it.toInt() and 0xFF }
        assertTrue(levels[0] < levels[63])
        // dB, not linear: a tenth of the amplitude is well above a tenth of the height.
        val steps = VoiceMessages.waveform(IntArray(64) { if (it < 32) 3_000 else 30_000 }).map { it.toInt() and 0xFF }
        assertTrue(steps[10] > 255 / 2)
    }

    @Test fun `smoothing is a three-tap moving average and silence stays zero`() {
        assertArrayEquals(floatArrayOf(0.5f, 1f / 3, 0f), VoiceMessages.smooth(floatArrayOf(1f, 0f, 0f)), 0.0001f)
        assertArrayEquals(floatArrayOf(0f, 1f / 3, 1f / 3, 0.5f), VoiceMessages.smooth(floatArrayOf(0f, 0f, 1f, 0f)), 0.0001f)
        assertTrue(VoiceMessages.waveform(IntArray(500)).all { it.toInt() == 0 })
        assertTrue(VoiceMessages.waveform(IntArray(0)).all { it.toInt() == 0 })
        val short = VoiceMessages.waveform(intArrayOf(0, 20_000, 500))
        assertEquals("fewer samples than slices repeat the nearest", 64, short.size)
        assertEquals(255, short.maxOf { it.toInt() and 0xFF })
    }

    @Test fun `bars interpolate linearly across the width`() {
        assertArrayEquals(floatArrayOf(0f, 0.5f, 1f), VoiceMessages.interpolate(floatArrayOf(0f, 1f), 3), 0.0001f)
        assertArrayEquals(floatArrayOf(0f, 0.25f, 0.5f, 0.75f, 1f), VoiceMessages.interpolate(floatArrayOf(0f, 1f), 5), 0.0001f)
        assertArrayEquals(floatArrayOf(1f, 0f), VoiceMessages.interpolate(floatArrayOf(1f, 0.5f, 0f), 2), 0.0001f)
        val many = VoiceMessages.interpolate(FloatArray(64) { it / 63f }, 37)
        assertEquals(37, many.size)
        assertEquals(0f, many.first(), 0.0001f)
        assertEquals(1f, many.last(), 0.0001f)
        assertTrue("evenly spaced: neighbouring bars change gradually", many.toList().zipWithNext().all { (a, b) -> b - a in 0f..0.03f })
        assertArrayEquals(FloatArray(5) { 0.5f }, VoiceMessages.interpolate(floatArrayOf(0.5f), 5), 0f)
        assertArrayEquals("no waveform draws a flat line", FloatArray(20), VoiceMessages.interpolate(null, 20), 0f)
        assertEquals(0, VoiceMessages.interpolate(floatArrayOf(1f), 0).size)
    }

    @Test fun `the boundary bar is partly played`() {
        assertEquals(0f, VoiceMessages.playedFraction(0.5f, 0.6f, 0.4f), 0f)
        assertEquals(1f, VoiceMessages.playedFraction(0.5f, 0.6f, 0.7f), 0f)
        assertEquals(0.5f, VoiceMessages.playedFraction(0.5f, 0.6f, 0.55f), 0.0001f)
        assertEquals(1f, VoiceMessages.playedFraction(0.5f, 0.5f, 0.5f), 0f)
    }

    @Test fun `live levels attack fast, release slowly and add bars at a steady pace`() {
        val live = LiveLevels(capacity = 4)
        assertTrue("the first sample adds a bar", live.sample(1f, 0))
        assertEquals(0.6f, live.level, 0.0001f)
        assertFalse("samples between bars only move the level", live.sample(1f, 25))
        assertEquals(0.84f, live.level, 0.0001f)
        live.sample(0f, 50)
        assertEquals("release is slower than attack", 0.84f * 0.85f, live.level, 0.0001f)
        assertTrue(live.sample(0f, 70))
        assertEquals(70L, live.lastBarAt)
        // A late sample keeps the 70 ms cadence.
        assertTrue(live.sample(0f, 150))
        assertEquals(140L, live.lastBarAt)
        repeat(5) { live.sample(0.5f, 210L + it * 70) }
        assertEquals("only the newest bars are kept", 4, live.values.size)
        live.clear()
        assertTrue(live.values.isEmpty())
        assertNull(live.lastBarAt)
    }

    @Test fun `stopping stages by default and sends only with the setting on`() {
        assertFalse(VoiceMessages.SEND_ON_STOP_DEFAULT)
        assertEquals(VoiceMessages.StopAction.STAGE, VoiceMessages.stopAction(VoiceMessages.SEND_ON_STOP_DEFAULT))
        assertEquals(VoiceMessages.StopAction.SEND, VoiceMessages.stopAction(true))
        assertEquals("Send voice messages as soon as I stop recording", VoiceMessages.SEND_ON_STOP_LABEL)
    }

    @Test fun `one staged voice message per message blocks the mic first`() {
        assertEquals("Remove the voice message to record another", voiceMicBlockedReason(true, true, true, 26))
        assertEquals("Wait for your last message to send, then record.", voiceMicBlockedReason(false, true, false, 36))
        assertNull(voiceMicBlockedReason(false, false, true, 29))
        assertNotNull(voiceMicBlockedReason(false, false, true, 28))
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

    @Test fun `recordings under a second are discarded and ten minutes is the most`() {
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
