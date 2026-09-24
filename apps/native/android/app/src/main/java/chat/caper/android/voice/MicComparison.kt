package chat.caper.android.voice

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Local-only PCM. Neither array is sent, saved to disk, or included in UI state. */
internal class MicComparison(private val limit: Int = 30 * 48000) {
    private val natural = ShortArray(limit)
    private val enhanced = ShortArray(limit)
    private val playbackEpoch = AtomicLong()
    private val activeTrack = AtomicReference<AudioTrack?>()
    private val playbackLock = Any()
    var frames = 0
        private set

    fun appendNatural(buffer: ByteBuffer, count: Int, gain: Int) {
        val pcm = buffer.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val available = count.coerceAtMost(limit - frames)
        for (index in 0 until available) {
            natural[frames + index] = (pcm.getShort(index * 2) * gain / 100).coerceIn(-32768, 32767).toShort()
        }
    }

    fun appendEnhanced(buffer: ByteBuffer, count: Int, valid: Boolean) {
        val pcm = buffer.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val available = count.coerceAtMost(limit - frames)
        for (index in 0 until available) enhanced[frames + index] = if (valid) pcm.getShort(index * 2) else 0.toShort()
        frames += available
    }

    fun samples(enhancedVersion: Boolean): ShortArray =
        (if (enhancedVersion) enhanced else natural).copyOf(frames)

    fun playbackSamples(enhancedVersion: Boolean, outputGain: Int): ShortArray {
        val gain = outputGain.coerceIn(0, 200)
        return samples(enhancedVersion).map { (it.toInt() * gain / 100).coerceIn(-32768, 32767).toShort() }.toShortArray()
    }

    fun stopPlayback() {
        synchronized(playbackLock) {
            playbackEpoch.incrementAndGet()
            activeTrack.getAndSet(null)?.let { track ->
                runCatching { track.pause(); track.flush() }
            }
        }
    }

    suspend fun play(enhancedVersion: Boolean, outputGain: Int) = withContext(Dispatchers.IO) {
        if (frames == 0) return@withContext
        currentCoroutineContext().ensureActive()
        val epoch = playbackEpoch.incrementAndGet()
        val size = AudioTrack.getMinBufferSize(48000, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        require(size > 0)
        val track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(size.coerceAtLeast(4800))
            .setTransferMode(AudioTrack.MODE_STREAM).build()
        try {
            currentCoroutineContext().ensureActive()
            synchronized(playbackLock) {
                if (playbackEpoch.get() == epoch) {
                    activeTrack.set(track)
                    track.play()
                }
            }
            if (playbackEpoch.get() != epoch) return@withContext
            val samples = playbackSamples(enhancedVersion, outputGain)
            var offset = 0
            while (offset < frames && playbackEpoch.get() == epoch) {
                currentCoroutineContext().ensureActive()
                val written = track.write(samples, offset, (frames - offset).coerceAtMost(2400), AudioTrack.WRITE_BLOCKING)
                if (written <= 0) break
                offset += written
            }
            while (playbackEpoch.get() == epoch && track.playbackHeadPosition < offset) delay(20)
        } finally {
            if (activeTrack.compareAndSet(track, null)) runCatching { track.pause(); track.flush() }
            track.release()
        }
    }
}
