package chat.caper.android.data

import chat.caper.android.model.ChatAttachment
import java.util.Base64
import kotlin.math.log10
import kotlin.math.roundToInt
import kotlin.math.sqrt
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Recorded voice messages (the voice-messages contract): limits, the uploaded waveform, the
 * transcript state and the clock labels. Pure Kotlin so the JVM tests cover it.
 */
object VoiceMessages {
    /** The server accepts 1..=600000 ms; recording stops at 10:00. */
    const val MAX_DURATION_MS = 600_000L
    /** Shorter recordings are discarded. */
    const val MIN_DURATION_MS = 1_000L
    /** A press shorter than this is a tap: it starts hands-free recording instead of a hold. */
    const val TAP_MS = 300L
    /** Bytes in the waveform this client uploads: one level per equal slice of the recording. */
    const val WAVEFORM_BYTES = 64
    /** What reply previews, pins and forward excerpts call a voice message instead of its file name. */
    const val LABEL = "Voice message"
    /** The reservation's file name and type; the server transcodes and renames it on `complete`. */
    const val UPLOAD_NAME = "voice-message.m4a"
    const val UPLOAD_TYPE = "audio/mp4"
    const val PERMISSION_ERROR = "Microphone access is needed to record voice messages"
    const val TOO_SHORT = "Voice messages need at least a second."
    const val STAGED_BLOCK = "Remove the voice message to record another"
    /** The per-device setting (off by default): send on stop instead of staging in the composer. */
    const val SEND_ON_STOP_LABEL = "Send voice messages as soon as I stop recording"
    const val SEND_ON_STOP_KEY = "voiceSendOnStop"
    const val SEND_ON_STOP_DEFAULT = false
    /** Live meter: input sampled every [LIVE_SAMPLE_MS], a new bar every [LIVE_BAR_MS]. */
    const val LIVE_SAMPLE_MS = 25L
    const val LIVE_BAR_MS = 70L

    /** What stopping a recording does: stage it in the composer (the default) or send it at once. */
    enum class StopAction { STAGE, SEND }

    fun stopAction(sendOnStop: Boolean): StopAction = if (sendOnStop) StopAction.SEND else StopAction.STAGE

    /**
     * 0..1 on a perceptual scale for an amplitude in 0..32767 (a MediaRecorder reading or an
     * RMS of them): -50 dBFS and below is silent, 0 dBFS is full.
     */
    fun meterLevel(amplitude: Double): Float {
        if (amplitude <= 0.0) return 0f
        val dbfs = 20 * log10(amplitude.coerceAtMost(32767.0) / 32767.0)
        return ((dbfs + 50) / 50).toFloat().coerceIn(0f, 1f)
    }

    fun meterLevel(amplitude: Int): Float = meterLevel(amplitude.toDouble())

    /**
     * The 64 uploaded levels for [samples] (input amplitudes in recording order, 0..32767): the
     * RMS of each of 64 equal slices, on the -50..0 dB scale, smoothed with a 3-tap moving
     * average, then scaled so the loudest slice is 255. Fewer samples than slices repeat the
     * nearest; silence stays all zero (players draw it as a flat line).
     */
    fun waveform(samples: IntArray, slices: Int = WAVEFORM_BYTES): ByteArray {
        if (samples.isEmpty()) return ByteArray(slices)
        val levels = FloatArray(slices) { slice ->
            val start = (slice.toLong() * samples.size / slices).toInt()
            val end = maxOf(start + 1, ((slice + 1).toLong() * samples.size / slices).toInt()).coerceAtMost(samples.size)
            var sum = 0.0
            for (index in start until end) { val value = samples[index].coerceAtLeast(0).toDouble(); sum += value * value }
            meterLevel(sqrt(sum / (end - start)))
        }
        val smoothed = smooth(levels)
        val loudest = smoothed.maxOrNull() ?: 0f
        return ByteArray(slices) { if (loudest <= 0f) 0 else (smoothed[it] / loudest * 255f).roundToInt().coerceIn(0, 255).toByte() }
    }

    /** 3-tap moving average; the ends average the neighbours they have. */
    fun smooth(levels: FloatArray): FloatArray = FloatArray(levels.size) { index ->
        val from = maxOf(0, index - 1)
        val to = minOf(levels.size - 1, index + 1)
        var sum = 0f
        for (at in from..to) sum += levels[at]
        sum / (to - from + 1)
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
     * [count] evenly spaced bars across any number of [levels] by linear interpolation, so
     * neighbouring bars change gradually. No levels draws a flat line (zero; the renderer's
     * minimum height).
     */
    fun interpolate(levels: FloatArray?, count: Int): FloatArray {
        if (count <= 0) return FloatArray(0)
        if (levels == null || levels.isEmpty()) return FloatArray(count)
        if (levels.size == 1 || count == 1) return FloatArray(count) { if (count == 1) levels.average().toFloat() else levels[0] }
        return FloatArray(count) { bar ->
            val position = bar.toFloat() * (levels.size - 1) / (count - 1)
            val low = position.toInt().coerceAtMost(levels.size - 2)
            val fraction = position - low
            levels[low] + (levels[low + 1] - levels[low]) * fraction
        }
    }

    /**
     * How much of the bar spanning [start]..[end] (fractions of the width) is played at
     * [progress]: 0 before it, 1 past it, partial on the boundary bar.
     */
    fun playedFraction(start: Float, end: Float, progress: Float): Float =
        if (end <= start) (if (progress >= end) 1f else 0f) else ((progress - start) / (end - start)).coerceIn(0f, 1f)

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

    /** What a finished recording keeps: under a second is discarded, past 10:00 is clamped. */
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
 * The live recording meter: levels arrive every [VoiceMessages.LIVE_SAMPLE_MS] and follow the
 * input with a fast attack and a slower release; every [VoiceMessages.LIVE_BAR_MS] the current
 * level becomes a new bar, so bars scroll in at a constant rate without jitter.
 */
class LiveLevels(private val capacity: Int = 64, private val attack: Float = 0.6f, private val release: Float = 0.15f) {
    private val bars = ArrayDeque<Float>()
    var level = 0f
        private set
    /** When the newest bar was added (ms, on the caller's clock); null before the first. */
    var lastBarAt: Long? = null
        private set

    val values: List<Float> get() = bars.toList()

    /** Feeds one input level (0..1) at [nowMs]; true when that added a bar. */
    fun sample(input: Float, nowMs: Long): Boolean {
        val target = input.coerceIn(0f, 1f)
        level += (target - level) * if (target > level) attack else release
        val last = lastBarAt
        if (last != null && nowMs - last < VoiceMessages.LIVE_BAR_MS) return false
        bars.addLast(level)
        while (bars.size > capacity) bars.removeFirst()
        // Keep a constant cadence even when a sample arrives late.
        lastBarAt = if (last == null) nowMs else last + VoiceMessages.LIVE_BAR_MS * ((nowMs - last) / VoiceMessages.LIVE_BAR_MS)
        return true
    }

    fun clear() { bars.clear(); level = 0f; lastBarAt = null }
}

/**
 * Why the composer's mic can't record now, or null. A staged voice message comes first (one
 * per message), then an unsent message, then a voice call before Android 10, where a second
 * capture can't run beside the call's microphone.
 */
fun voiceMicBlockedReason(stagedVoice: Boolean, pendingMessage: Boolean, inVoiceCall: Boolean, sdk: Int): String? = when {
    stagedVoice -> VoiceMessages.STAGED_BLOCK
    pendingMessage -> "Wait for your last message to send, then record."
    inVoiceCall && sdk < 29 -> "Leave the voice channel to record a voice message on this Android version."
    else -> null
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
