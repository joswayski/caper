package chat.caper.android.voice

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AudioEffect
import android.media.audiofx.NoiseSuppressor
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.nio.ByteOrder
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * A voice message recorded through the call's microphone processing to [output]: AAC-LC in
 * MPEG-4, mono, 48 kHz, 64 kbps (MediaCodec and MediaMuxer).
 *
 * AudioRecord (VOICE_COMMUNICATION at 48 kHz, with the platform echo canceller and noise
 * suppressor where WebRTC would enable them for a call) feeds [AudioCapture]'s local path: the
 * saved input gain, DPDFNet-8 HR (RNNoise fallback) and the voice DSP at the saved Voice
 * enhancement strength. The model loads beside capture, which waits in memory and then catches up
 * at the denoiser's pace, so the first words are kept. If processing can't load or fails part way,
 * the rest is recorded with the input gain alone. Nothing is published: no peer, track or gate.
 *
 * [start], [takePeak], [finish] and [cancel] run on Main; capture, processing and encoding run on
 * one worker, the model loads on another.
 */
internal class VoiceNoteCapture private constructor(
    private val output: File,
    private val record: AudioRecord,
    private val effects: List<AudioEffect>,
    private val encoder: MediaCodec,
    private val muxer: MediaMuxer,
    private val gain: Int,
    private val strength: Int,
) {
    private enum class Mode { PREPARING, PROCESSED, PLAIN }

    private val peak = AtomicInteger(0)
    private val prepared = CountDownLatch(1)
    private val done = CountDownLatch(1)
    @Volatile private var stopping = false
    @Volatile private var discarded = false
    @Volatile private var saved = false

    private val lock = Any()
    private var mode = Mode.PREPARING
    private var processor: AudioCapture? = null
    private var released = false

    // Worker thread only.
    private val backlog = ArrayDeque<ShortArray>()
    private var backlogFrames = 0
    private var capturedFrames = 0L
    private var encodedFrames = 0L
    private val processed = ShortArray(BLOCK_FRAMES * 4)
    private val envelope = PeakEnvelope(RATE / 1000 * VOICE_SLOT_MS)
    private val info = MediaCodec.BufferInfo()
    private var track = -1
    private var written = 0

    /** The loudest processed sample since the last call (0..32767), as `MediaRecorder.getMaxAmplitude()`. */
    fun takePeak(): Int = peak.getAndSet(0)

    /**
     * The saved audio's envelope, read after [finish]: its loudest sample per 25 ms of audio, so the
     * uploaded waveform lines up with what was recorded even while the model was still loading.
     */
    fun envelope(): IntArray = envelope.values()

    /** Stops, pushes the tail through and closes the file; false (file deleted) when nothing usable was written. */
    fun finish(): Boolean {
        stopping = true
        if (!done.await(FINISH_WAIT_MS, TimeUnit.MILLISECONDS)) {
            // The worker still releases the microphone and deletes the file when it gets there.
            discarded = true
            Log.w(TAG, "Voice message encoding did not finish in time")
            return false
        }
        return saved
    }

    /** Stops without keeping anything; the microphone closes within one read. */
    fun cancel() {
        discarded = true
        stopping = true
        done.await(CANCEL_WAIT_MS, TimeUnit.MILLISECONDS)
    }

    private fun begin(context: Context) {
        Thread({ prepare(context) }, "caper-voice-note-model").apply { isDaemon = true }.start()
        Thread({ loop() }, "caper-voice-note").apply { isDaemon = true }.start()
    }

    private fun prepare(context: Context) {
        val capture = try {
            val next = AudioCapture.prepare(context)
            next.gain(gain)
            next.processingStrength(strength)
            next
        } catch (error: Throwable) {
            Log.w(TAG, "Voice message processing unavailable; recording with input gain only", error)
            null
        }
        synchronized(lock) {
            if (capture != null && mode == Mode.PREPARING && !released) { processor = capture; mode = Mode.PROCESSED }
            else { capture?.close(); if (mode == Mode.PREPARING) mode = Mode.PLAIN }
        }
        prepared.countDown()
    }

    private fun loop() {
        try {
            val block = ShortArray(BLOCK_FRAMES)
            while (!stopping) {
                val read = record.read(block, 0, BLOCK_FRAMES, AudioRecord.READ_BLOCKING)
                check(read >= 0) { "Microphone read failed ($read)." }
                if (read == 0) continue
                backlog.addLast(block.copyOf(read))
                backlogFrames += read
                capturedFrames += read
                pump()
            }
            runCatching { record.stop() }
            if (!discarded) {
                flush()
                saved = close() && !discarded
            }
        } catch (error: Throwable) {
            Log.w(TAG, "Voice message recording failed", error)
        } finally {
            release()
            if (!saved || discarded) { saved = false; output.delete() }
            done.countDown()
        }
    }

    /** Moves captured audio on: through the processor once it's ready, as is (input gain only) without it. */
    private fun pump() {
        val (current, capture) = synchronized(lock) { mode to processor }
        when (current) {
            Mode.PREPARING -> if (backlogFrames > MAX_BACKLOG_FRAMES) plain("the model took too long to load")
            Mode.PLAIN -> while (backlog.isNotEmpty()) {
                val samples = backlog.removeFirst()
                backlogFrames -= samples.size
                VoiceNotePcm.applyGain(samples, samples.size, gain)
                emit(samples, samples.size)
            }
            Mode.PROCESSED -> {
                val processor = capture ?: return
                while (backlog.isNotEmpty()) {
                    val samples = backlog.first()
                    val queued = processor.queueLocal(samples, samples.size)
                    if (queued < 0) return plain("processing failed")
                    if (queued == 0) break
                    backlog.removeFirst()
                    backlogFrames -= samples.size
                }
                while (true) {
                    val count = processor.takeLocal(processed)
                    if (count < 0) return plain("processing failed")
                    if (count == 0) break
                    emit(processed, count)
                }
            }
        }
    }

    /** Waits briefly for a model still loading, then pushes the queued audio and the denoiser's last hop out. */
    private fun flush() {
        prepared.await(MODEL_WAIT_MS, TimeUnit.MILLISECONDS)
        val current = synchronized(lock) { mode }
        if (current == Mode.PREPARING) plain("the model was still loading")
        if (current == Mode.PROCESSED) {
            repeat(FLUSH_BLOCKS) { backlog.addLast(ShortArray(BLOCK_FRAMES)) }
            backlogFrames += FLUSH_BLOCKS * BLOCK_FRAMES
        }
        val target = capturedFrames + BLOCK_FRAMES
        val deadline = SystemClock.elapsedRealtime() + FLUSH_WAIT_MS
        while (true) {
            pump()
            if (synchronized(lock) { mode } != Mode.PROCESSED) { pump(); return }
            if (backlog.isEmpty() && encodedFrames >= target) return
            if (SystemClock.elapsedRealtime() > deadline) { plain("the denoiser fell behind"); pump(); return }
            Thread.sleep(2)
        }
    }

    /** Records the rest without the processor (input gain only); audio already inside it is lost. */
    private fun plain(reason: String) {
        synchronized(lock) {
            if (mode == Mode.PLAIN) return
            Log.w(TAG, "Voice message continues without processing: $reason")
            mode = Mode.PLAIN
            processor?.close()
            processor = null
        }
    }

    private fun emit(samples: ShortArray, count: Int) {
        val loudest = VoiceNotePcm.peak(samples, count)
        peak.accumulateAndGet(loudest) { current, next -> maxOf(current, next) }
        // The flush's trailing silence is not part of the message's shape.
        envelope.add(samples, (capturedFrames - encodedFrames).coerceIn(0L, count.toLong()).toInt())
        var offset = 0
        var stalls = 0
        while (offset < count) {
            val index = encoder.dequeueInputBuffer(10_000)
            if (index < 0) {
                check(++stalls < 100) { "The AAC encoder stopped accepting audio." }
                drain(end = false)
                continue
            }
            val buffer = checkNotNull(encoder.getInputBuffer(index)).apply { clear(); order(ByteOrder.nativeOrder()) }
            val frames = minOf(count - offset, buffer.remaining() / 2)
            buffer.asShortBuffer().put(samples, offset, frames)
            encoder.queueInputBuffer(index, 0, frames * 2, encodedFrames * 1_000_000L / RATE, 0)
            encodedFrames += frames
            offset += frames
            drain(end = false)
        }
    }

    private fun drain(end: Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 1_000
        while (true) {
            val index = encoder.dequeueOutputBuffer(info, if (end) 10_000L else 0L)
            when {
                index == MediaCodec.INFO_TRY_AGAIN_LATER -> if (!end || SystemClock.elapsedRealtime() > deadline) return
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> if (track < 0) {
                    track = muxer.addTrack(encoder.outputFormat)
                    muxer.start()
                }
                index >= 0 -> {
                    val data = encoder.getOutputBuffer(index)
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) info.size = 0
                    if (info.size > 0 && track >= 0 && data != null) {
                        data.position(info.offset)
                        data.limit(info.offset + info.size)
                        muxer.writeSampleData(track, data, info)
                        written++
                    }
                    encoder.releaseOutputBuffer(index, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
                }
            }
        }
    }

    /** Ends the AAC stream and finalizes the MP4; true when it holds audio. */
    private fun close(): Boolean {
        var stalls = 0
        var index = encoder.dequeueInputBuffer(10_000)
        while (index < 0 && ++stalls < 100) { drain(end = false); index = encoder.dequeueInputBuffer(10_000) }
        if (index < 0) return false
        encoder.queueInputBuffer(index, 0, 0, encodedFrames * 1_000_000L / RATE, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        drain(end = true)
        if (track < 0 || written == 0) return false
        muxer.stop()
        return true
    }

    private fun release() {
        runCatching { record.release() }
        effects.forEach { runCatching { it.release() } }
        runCatching { encoder.stop() }
        runCatching { encoder.release() }
        runCatching { muxer.release() }
        synchronized(lock) {
            released = true
            processor?.close()
            processor = null
        }
    }

    companion object {
        private const val TAG = "CaperVoice"
        private const val RATE = 48_000
        /** 10 ms reads, as the call's capture. */
        private const val BLOCK_FRAMES = RATE / 100
        /** Captured audio held while the model loads: past this, the message continues without it. */
        private const val MAX_BACKLOG_FRAMES = RATE * 3
        /** Silence behind the last words: the denoiser holds back one 10 ms hop plus its resampler sample. */
        private const val FLUSH_BLOCKS = 3
        private const val MODEL_WAIT_MS = 1_000L
        private const val FLUSH_WAIT_MS = 1_000L
        private const val FINISH_WAIT_MS = 3_500L
        private const val CANCEL_WAIT_MS = 300L
        /** One envelope value per live-meter sample ([chat.caper.android.data.VoiceMessages.LIVE_SAMPLE_MS]). */
        private const val VOICE_SLOT_MS = 25
        /** WebRTC leaves AOSP's software effects off and uses the device's own (`WebRtcAudioEffects`). */
        private val AOSP_ECHO_CANCELER: UUID = UUID.fromString("bb392ec0-8d4d-11e0-a896-0002a5d5c51b")
        private val AOSP_NOISE_SUPPRESSOR: UUID = UUID.fromString("c06c8400-8e06-11e0-9cb6-0002a5d5c51b")

        /**
         * Opens the microphone and the encoder and starts recording; throws (holding nothing) when
         * either can't start, so the caller can fall back to MediaRecorder. Main thread.
         */
        fun start(context: Context, output: File): VoiceNoteCapture {
            val preferences = context.getSharedPreferences("audio", Context.MODE_PRIVATE)
            val gain = preferences.getInt("inputGain", 100).coerceIn(0, 200)
            val strength = preferences.getInt("strength", 25).coerceIn(0, 100)
            var record: AudioRecord? = null
            var encoder: MediaCodec? = null
            var muxer: MediaMuxer? = null
            val effects = mutableListOf<AudioEffect>()
            try {
                check(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                    "Microphone permission is required for a voice message."
                }
                val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
                check(minimum > 0) { "Microphone format unavailable." }
                // 200 ms of headroom: encoding shares the reading thread.
                record = AudioRecord.Builder().setAudioSource(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
                    .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(RATE).setChannelMask(AudioFormat.CHANNEL_IN_MONO).build())
                    .setBufferSizeInBytes(minimum.coerceAtLeast(RATE / 5 * 2)).build()
                check(record.state == AudioRecord.STATE_INITIALIZED) { "Microphone unavailable." }
                effects += callEffects(record.audioSessionId)
                val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, RATE, 1).apply {
                    setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
                    setInteger(MediaFormat.KEY_BIT_RATE, 64_000)
                    setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, BLOCK_FRAMES * 4 * 2)
                }
                encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
                    configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                    start()
                }
                muxer = MediaMuxer(output.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
                record.startRecording()
                check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "Microphone unavailable." }
                return VoiceNoteCapture(output, record, effects, encoder, muxer, gain, strength)
                    .also { it.begin(context.applicationContext) }
            } catch (error: Throwable) {
                runCatching { record?.release() }
                effects.forEach { runCatching { it.release() } }
                runCatching { encoder?.release() }
                runCatching { muxer?.release() }
                output.delete()
                throw error
            }
        }

        /** The device's echo canceller and noise suppressor, as WebRTC's call capture enables them. */
        private fun callEffects(session: Int): List<AudioEffect> = buildList {
            val available = runCatching { AudioEffect.queryEffects()?.toList() }.getOrNull().orEmpty()
            fun device(type: UUID, aosp: UUID) = available.any { it.type == type && it.uuid != aosp }
            if (device(AudioEffect.EFFECT_TYPE_AEC, AOSP_ECHO_CANCELER))
                runCatching { AcousticEchoCanceler.create(session)?.also { it.setEnabled(true) } }.getOrNull()?.let(::add)
            if (device(AudioEffect.EFFECT_TYPE_NS, AOSP_NOISE_SUPPRESSOR))
                runCatching { NoiseSuppressor.create(session)?.also { it.setEnabled(true) } }.getOrNull()?.let(::add)
        }
    }
}

/** [VoiceNoteCapture]'s PCM arithmetic, apart for unit tests. */
internal object VoiceNotePcm {
    /** The loudest sample's magnitude on `MediaRecorder.getMaxAmplitude()`'s 0..32767 scale. */
    fun peak(samples: ShortArray, count: Int): Int {
        var loudest = 0
        for (index in 0 until count.coerceAtMost(samples.size)) loudest = maxOf(loudest, abs(samples[index].toInt()))
        return loudest.coerceAtMost(32767)
    }

    /** The input volume preference (0..200%) alone, for audio recorded without the processor. */
    fun applyGain(samples: ShortArray, count: Int, gain: Int) {
        if (gain == 100) return
        val scale = gain.coerceIn(0, 200) / 100f
        for (index in 0 until count.coerceAtMost(samples.size)) {
            samples[index] = (samples[index] * scale).roundToInt().coerceIn(-32768, 32767).toShort()
        }
    }
}

/**
 * The loudest magnitude (0..32767) per [slot] samples of audio, as `getMaxAmplitude()` read every
 * slot would see it; a last, partial slot counts too.
 */
internal class PeakEnvelope(private val slot: Int) {
    private var values = IntArray(512)
    private var size = 0
    private var loudest = 0
    private var filled = 0

    fun add(samples: ShortArray, count: Int) {
        for (index in 0 until count.coerceAtMost(samples.size)) {
            loudest = maxOf(loudest, abs(samples[index].toInt()).coerceAtMost(32767))
            if (++filled == slot) {
                if (size == values.size) values = values.copyOf(size * 2)
                values[size++] = loudest
                loudest = 0
                filled = 0
            }
        }
    }

    fun values(): IntArray = if (filled == 0) values.copyOf(size) else values.copyOf(size + 1).also { it[size] = loudest }
}

/** Starts [processed]; when it throws, reports why through [fellBack] and starts [plain] instead. */
internal inline fun <T> startWithFallback(processed: () -> T, plain: () -> T, fellBack: (Throwable) -> Unit): T =
    try { processed() } catch (error: Throwable) { fellBack(error); plain() }
