package chat.caper.android.voice

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** Explicit prejoin capture; no peer, sender or media capability exists here. */
internal class PrejoinMicTest private constructor(private val capture: AudioCapture, private val recorder: AudioRecord) {
    private val running = AtomicBoolean(true)
    private val finishing = AtomicBoolean(false)
    private val finished = CompletableDeferred<MicComparison?>()
    private var job: Job? = null

    fun start() {
        capture.beginComparison()
        job = CoroutineScope(Dispatchers.IO).launch {
            val buffer = ByteBuffer.allocateDirect(960)
            try {
                recorder.startRecording()
                val until = System.nanoTime() + 30_000_000_000L
                while (running.get() && System.nanoTime() < until) {
                    val read = recorder.read(buffer, 960, AudioRecord.READ_BLOCKING)
                    if (read <= 0) break
                    capture.onBuffer(buffer, AudioFormat.ENCODING_PCM_16BIT, 1, 48000, read)
                }
            } finally { running.set(false) }
        }
    }

    suspend fun finish(): MicComparison? = withContext(NonCancellable + Dispatchers.IO) {
        if (!finishing.compareAndSet(false, true)) return@withContext finished.await()
        try {
            stopNow()
            // Release the recorder even if a blocking read ignores stop(); a
            // canceled UI scope must not strand the microphone or DSP worker.
            runCatching { recorder.release() }
            withTimeoutOrNull(2_000) { job?.join() }
            capture.endComparison().also { finished.complete(it) }
        } catch (error: Throwable) {
            finished.completeExceptionally(error)
            throw error
        } finally { capture.close() }
    }

    fun stopNow() {
        running.set(false)
        runCatching { recorder.stop() }
    }

    fun level(): Float = capture.inputLevel
    fun gain(value: Int) { capture.gain(value) }
    fun processingStrength(value: Int) { capture.processingStrength(value) }

    companion object {
        suspend fun prepare(context: Context): PrejoinMicTest = withContext(Dispatchers.IO) {
            check(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                "Microphone permission is required for a mic test."
            }
            val capture = AudioCapture.prepare(context.applicationContext)
            try {
                val preferences = context.getSharedPreferences("audio", Context.MODE_PRIVATE)
                capture.gain(preferences.getInt("inputGain", 100))
                capture.processingStrength(preferences.getInt("strength", 25))
                val size = AudioRecord.getMinBufferSize(48000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
                check(size > 0) { "Microphone format unavailable." }
                val recorder = AudioRecord.Builder().setAudioSource(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
                    .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_IN_MONO).build())
                    .setBufferSizeInBytes(size.coerceAtLeast(1920)).build()
                check(recorder.state == AudioRecord.STATE_INITIALIZED) { recorder.release(); "Microphone unavailable." }
                PrejoinMicTest(capture, recorder).also { it.start() }
            } catch (error: Throwable) { capture.close(); throw error }
        }
    }
}
