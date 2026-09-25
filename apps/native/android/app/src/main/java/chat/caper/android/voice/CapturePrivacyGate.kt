package chat.caper.android.voice

import java.util.concurrent.atomic.AtomicInteger

/** Epochs distinguish queued model PCM from a previous mute, comparison, or route. */
internal class CapturePrivacyGate {
    private val generation = AtomicInteger(1)
    @Volatile private var intended = false
    @Volatile private var enabled = false
    private var previousCallbackEpoch = 0

    @Synchronized fun publish(value: Boolean, gain: Int) {
        enabled = false
        intended = value
        generation.incrementAndGet()
        enabled = value && gain > 0
    }

    @Synchronized fun changedGain(previous: Int, next: Int) {
        if ((previous == 0) != (next == 0)) {
            enabled = false
            generation.incrementAndGet()
        }
        enabled = intended && next > 0
    }

    fun epoch() = generation.get()

    // WebRTC invokes this callback AFTER AudioRecord.read. The preceding
    // callback is our conservative read-entry fence: any intervening change
    // makes this buffer private, even if publication has already reopened.
    // Only the single capture thread calls this method.
    fun captureEpoch(): Int {
        val current = generation.get()
        val captured = if (current == previousCallbackEpoch) current else 0
        previousCallbackEpoch = current
        return captured
    }

    fun mayPublish(processedEpoch: Int, gain: Int, processingSucceeded: Boolean): Boolean =
        enabled && gain > 0 && processingSucceeded && generation.get() == processedEpoch
}
