package chat.caper.android.voice

import android.content.Context
import android.media.AudioFormat
import java.nio.ByteBuffer
import java.security.MessageDigest

/** The only path from AudioRecord's direct buffer to WebRTC's nativeDataIsRecorded.
 * Inference lives on the native worker, never on the capture callback. */
internal class AudioCapture private constructor(private val handle: Long) {
    private val gate = CapturePrivacyGate()
    private val natural = ByteBuffer.allocateDirect(8192)
    @Volatile var inputGain = 100
        private set
    @Volatile var strength = 25
        private set
    private var closed = false
    @Volatile private var comparison: MicComparison? = null
    @Volatile private var testActive = false
    val isTesting get() = testActive

    fun publication(enabled: Boolean) {
        gate.publish(enabled && !testActive, inputGain)
    }

    fun gain(value: Int) {
        val next = value.coerceIn(0, 200)
        gate.changedGain(inputGain, next)
        inputGain = next
    }

    fun processingStrength(value: Int) { strength = value.coerceIn(0, 100) }

    @Synchronized fun beginComparison() {
        publication(false)
        testActive = true
        comparison = MicComparison()
    }

    @Synchronized fun endComparison(): MicComparison? {
        publication(false)
        return comparison.also { comparison = null }
    }

    fun resumePublication() { testActive = false }

    // A short monitor guards native handle lifetime. No HTTP, I/O, or model
    // inference runs while holding it. Unsupported format is never published.
    @Synchronized fun onBuffer(buffer: ByteBuffer, format: Int, channels: Int, rate: Int, bytesRead: Int) {
        val current = gate.captureEpoch()
        if (closed || format != AudioFormat.ENCODING_PCM_16BIT || channels != 1 || rate != 48000 ||
            bytesRead <= 0 || bytesRead % 2 != 0 || bytesRead > buffer.capacity() || bytesRead > 8192) {
            silence(buffer, buffer.capacity())
            return
        }
        val mode = process(handle, buffer, natural, bytesRead / 2, inputGain, strength, current)
        if (mode == 0 || inputGain == 0) {
            silence(natural, bytesRead)
            silence(buffer, bytesRead)
        }
        comparison?.appendNatural(natural, bytesRead / 2, 100)
        comparison?.appendEnhanced(buffer, bytesRead / 2, mode != 0)
        if (!gate.mayPublish(current, inputGain, mode != 0)) silence(buffer, bytesRead)
        // Short reads must not expose stale AudioRecord bytes on subsequent callbacks.
        if (bytesRead < buffer.capacity()) for (index in bytesRead until buffer.capacity()) buffer.put(index, 0)
    }

    @Synchronized fun report(): LongArray = if (closed) longArrayOf() else diagnostics(handle)

    @Synchronized fun close() {
        publication(false)
        comparison = null
        testActive = false
        if (!closed) {
            closed = true
            // Joining the model worker can block through inference; stopping
            // an ongoing call must not hold Main or the WebRTC callback lock.
            Thread({ destroy(handle) }, "caper-audio-release").apply { isDaemon = true }.start()
        }
    }

    private fun silence(buffer: ByteBuffer, length: Int) { for (index in 0 until length) buffer.put(index, 0) }

    private external fun create(path: String, rate: Int): Long
    private external fun destroy(handle: Long)
    private external fun process(handle: Long, buffer: ByteBuffer, natural: ByteBuffer, frames: Int, gain: Int, strength: Int, epoch: Int): Int
    private external fun diagnostics(handle: Long): LongArray

    companion object {
        init { System.loadLibrary("caper_audio") }
        fun prepare(context: Context): AudioCapture {
            val target = context.noBackupFilesDir.resolve("dpdfnet8_48khz_hr.onnx")
            val expected = "7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631"
            if (!target.isFile || sha256(target.readBytes()) != expected) {
                val staging = context.noBackupFilesDir.resolve("dpdfnet8_48khz_hr.onnx.tmp")
                context.assets.open("dpdfnet8_48khz_hr.onnx").use { source -> staging.outputStream().use(source::copyTo) }
                check(sha256(staging.readBytes()) == expected) { "Bundled audio model failed validation." }
                check(staging.renameTo(target)) { "Cannot prepare local audio model." }
            }
            val bridge = AudioCapture(0)
            val handle = bridge.create(target.absolutePath, 48000)
            check(handle != 0L) { "Audio processing is unavailable." }
            return AudioCapture(handle)
        }
        private fun sha256(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes)
            .joinToString("") { "%02x".format(it) }
    }
}
