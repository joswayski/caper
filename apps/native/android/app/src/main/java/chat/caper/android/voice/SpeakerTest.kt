package chat.caper.android.voice

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

internal class WavPcm(val channels: Int, val sampleRate: Int, val samples: ShortArray)

/** Reads 16-bit PCM WAV files such as web's bundled effects (which carry a LIST chunk). */
internal fun parseWavPcm(bytes: ByteArray): WavPcm {
    val data = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    fun tag(at: Int) = String(bytes, at, 4, Charsets.US_ASCII)
    require(bytes.size >= 12 && tag(0) == "RIFF" && tag(8) == "WAVE") { "Not a WAV file." }
    var offset = 12
    var channels = 0
    var rate = 0
    var bits = 0
    var format = 0
    while (offset + 8 <= bytes.size) {
        val id = tag(offset)
        val size = data.getInt(offset + 4)
        val body = offset + 8
        require(size >= 0 && body + size <= bytes.size || id == "data") { "Truncated WAV chunk." }
        when (id) {
            "fmt " -> {
                format = data.getShort(body).toInt()
                channels = data.getShort(body + 2).toInt()
                rate = data.getInt(body + 4)
                bits = data.getShort(body + 14).toInt()
            }
            "data" -> {
                require(format == 1 && bits == 16 && channels in 1..2 && rate > 0) { "Unsupported WAV format." }
                val length = size.coerceAtMost(bytes.size - body) / 2
                val samples = ShortArray(length) { data.getShort(body + it * 2) }
                return WavPcm(channels, rate, samples)
            }
        }
        offset = body + size + (size and 1)
    }
    error("WAV file has no audio data.")
}

/** Web's SpeakerTest: loops channel-join.wav at the speaker volume until canceled. */
internal object SpeakerTest {
    private var cached: WavPcm? = null

    suspend fun loop(context: Context, inCall: Boolean, volume: () -> Int) = withContext(Dispatchers.IO) {
        val clip = cached ?: parseWavPcm(context.assets.open("effects/channel-join.wav").use { it.readBytes() }).also { cached = it }
        val mask = if (clip.channels == 2) AudioFormat.CHANNEL_OUT_STEREO else AudioFormat.CHANNEL_OUT_MONO
        val size = AudioTrack.getMinBufferSize(clip.sampleRate, mask, AudioFormat.ENCODING_PCM_16BIT)
        check(size > 0) { "Audio output unavailable." }
        // During a call, play on the call's communication route like its audio.
        val attributes = AudioAttributes.Builder()
            .setUsage(if (inCall) AudioAttributes.USAGE_VOICE_COMMUNICATION else AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build()
        val track = AudioTrack.Builder().setAudioAttributes(attributes)
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(clip.sampleRate).setChannelMask(mask).build())
            .setBufferSizeInBytes(size.coerceAtLeast(8192))
            .setTransferMode(AudioTrack.MODE_STREAM).build()
        try {
            check(track.state == AudioTrack.STATE_INITIALIZED) { "Audio output unavailable." }
            track.play()
            val chunk = ShortArray(2048)
            while (true) {
                var offset = 0
                while (offset < clip.samples.size) {
                    currentCoroutineContext().ensureActive()
                    val count = (clip.samples.size - offset).coerceAtMost(chunk.size)
                    val gain = volume().coerceIn(0, 200)
                    for (index in 0 until count) chunk[index] = (clip.samples[offset + index] * gain / 100).coerceIn(-32768, 32767).toShort()
                    val written = track.write(chunk, 0, count, AudioTrack.WRITE_BLOCKING)
                    check(written > 0) { "Audio output unavailable." }
                    offset += written
                }
            }
        } finally {
            runCatching { track.pause(); track.flush() }
            track.release()
        }
    }
}
