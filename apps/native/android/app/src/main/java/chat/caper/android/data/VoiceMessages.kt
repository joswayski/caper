package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import java.util.Base64
import kotlin.math.log10
import kotlin.math.roundToInt
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Recorded voice messages (the voice-messages contract): limits, the uploaded waveform, the
 * transcript state and the clock labels. Pure Kotlin so the JVM tests cover it.
 */
object VoiceMessages {
    /** The server accepts 1..=600000 ms; recording stops and sends at 10:00. */
    const val MAX_DURATION_MS = 600_000L
    /** Shorter recordings are discarded like a tap ("Hold to record"). */
    const val MIN_DURATION_MS = 1_000L
    /** Bytes in the waveform this client uploads: one peak per equal slice of the recording. */
    const val WAVEFORM_BYTES = 64
    /** What reply previews, pins and forward excerpts call a voice message instead of its file name. */
    const val LABEL = "Voice message"
    /** The reservation's file name and type; the server transcodes and renames it on `complete`. */
    const val UPLOAD_NAME = "voice-message.m4a"
    const val UPLOAD_TYPE = "audio/mp4"
    const val HOLD_HINT = "Hold to record"
    const val PERMISSION_ERROR = "Microphone access is needed to record voice messages"

    /**
     * The 64 peaks of [samples] (raw amplitudes in recording order, any non-negative scale), each
     * the loudest sample of one of 64 equal slices, scaled linearly so this recording's loudest
     * slice is 255. Fewer samples than slices repeat the nearest sample; silence stays all zero.
     */
    fun waveform(samples: IntArray, slices: Int = WAVEFORM_BYTES): ByteArray {
        val peaks = IntArray(slices)
        if (samples.isNotEmpty()) for (slice in 0 until slices) {
            val start = (slice.toLong() * samples.size / slices).toInt()
            val end = maxOf(start + 1, ((slice + 1).toLong() * samples.size / slices).toInt()).coerceAtMost(samples.size)
            var peak = 0
            for (index in start until end) peak = maxOf(peak, samples[index].coerceAtLeast(0))
            peaks[slice] = peak
        }
        val loudest = peaks.maxOrNull() ?: 0
        return ByteArray(slices) { index ->
            if (loudest == 0) 0 else (peaks[index].toLong() * 255 / loudest).toInt().toByte()
        }
    }

    /** Standard base64 with padding and no line breaks, as `POST /api/assets` expects. */
    fun encodeWaveform(bytes: ByteArray): String = Base64.getEncoder().encodeToString(bytes)

    /** 0..1 heights from a description's `waveform` (1..=256 bytes), or null when absent or malformed. */
    fun decodeWaveform(encoded: String?): FloatArray? {
        if (encoded.isNullOrBlank()) return null
        val bytes = runCatching { Base64.getDecoder().decode(encoded.trim()) }.getOrNull() ?: return null
        if (bytes.isEmpty() || bytes.size > 256) return null
        return FloatArray(bytes.size) { (bytes[it].toInt() and 0xFF) / 255f }
    }

    /**
     * [count] bars from any number of [levels]: each bar is the loudest level it covers when
     * shrinking, or the nearest level when stretching. No levels draws a flat placeholder.
     */
    fun resample(levels: FloatArray?, count: Int, flat: Float = 0.12f): FloatArray {
        if (count <= 0) return FloatArray(0)
        if (levels == null || levels.isEmpty()) return FloatArray(count) { flat }
        return FloatArray(count) { bar ->
            val start = (bar.toLong() * levels.size / count).toInt()
            val end = ((bar + 1).toLong() * levels.size / count).toInt()
            if (end <= start) levels[start.coerceAtMost(levels.size - 1)]
            else (start until end).maxOf { levels[it] }
        }
    }

    /**
     * A live meter level (0..1) for one MediaRecorder `getMaxAmplitude()` reading (0..32767):
     * -50 dBFS and below is silent, so speech fills the bars without clipping.
     */
    fun meterLevel(amplitude: Int): Float {
        if (amplitude <= 0) return 0f
        val dbfs = 20 * log10(amplitude.coerceAtMost(32767) / 32767.0)
        return ((dbfs + 50) / 50).toFloat().coerceIn(0f, 1f)
    }

    /** `m:ss`, rounded down: the recording timer and the player's duration and remaining time. */
    fun clock(ms: Long): String {
        val seconds = ms.coerceAtLeast(0) / 1000
        return "%d:%02d".format(java.util.Locale.US, seconds / 60, seconds % 60)
    }

    /** What the player shows: the whole duration while idle, the time left while it plays or is paused part way. */
    fun playerClock(durationMs: Long?, positionMs: Long?, active: Boolean): String {
        val total = (durationMs ?: 0).coerceAtLeast(0)
        return if (active && positionMs != null) clock((total - positionMs).coerceAtLeast(0)) else clock(total)
    }

    /** What a finished hold does: under a second is a tap (nothing uploads), past 10:00 is clamped. */
    fun recordedDuration(elapsedMs: Long): Long? =
        if (elapsedMs < MIN_DURATION_MS) null else elapsedMs.coerceAtMost(MAX_DURATION_MS)

    /** Reply previews and other excerpts of a message with no text. */
    fun excerptLabel(attachments: List<ChatAttachment>, hasAttachments: Boolean = attachments.isNotEmpty()): String = when {
        attachments.any { it.isVoiceMessage } -> LABEL
        hasAttachments -> "Attachment"
        else -> "Message"
    }
}

/**
 * A voice message's transcript. `pending` shows "Transcribing…", `ready` with text shows it;
 * `failed`, an absent or malformed value, an unknown status and empty text all show nothing.
 */
sealed interface VoiceTranscript {
    data object None : VoiceTranscript
    data object Pending : VoiceTranscript
    data class Ready(val text: String) : VoiceTranscript

    companion object {
        fun of(element: JsonElement?): VoiceTranscript {
            val value = element as? JsonObject ?: return None
            val status = (value["status"] as? JsonPrimitive)?.takeIf { it.isString }?.content
            return when (status) {
                "pending" -> Pending
                "ready" -> (value["text"] as? JsonPrimitive)?.takeIf { it.isString }?.content
                    ?.takeIf { it.isNotBlank() }?.let { Ready(it.trim()) } ?: None
                else -> None
            }
        }
    }
}

/** Only `voice: true` audio uses the voice player; other audio files keep their file card. */
val ChatAttachment.isVoiceMessage: Boolean get() = voice && kind == "audio"

val ChatAttachment.transcriptState: VoiceTranscript get() = VoiceTranscript.of(transcript)

/** Rounds a 0..1 progress to the nearest whole millisecond of [durationMs]. */
internal fun seekPosition(fraction: Float, durationMs: Long): Long = (fraction.coerceIn(0f, 1f) * durationMs).roundToInt().toLong()
