package chat.caper.android.voice

/**
 * Web's lost-microphone rule (`recoverMicrophone`): a call whose input stops delivering audio
 * reopens the system's current input once and keeps publishing. An input reopened that way and
 * lost again within [WINDOW_MS] is not reopened; the call mutes instead.
 */
internal class MicrophoneRecovery {
    private var reopenedAt: Long? = null

    /** Whether a loss at [nowMs] may reopen the input. */
    fun mayReopen(nowMs: Long): Boolean = reopenedAt?.let { nowMs - it >= WINDOW_MS } ?: true

    /** A loss was answered with an input that delivers audio. */
    fun reopened(nowMs: Long) { reopenedAt = nowMs }

    companion object {
        const val WINDOW_MS = 10_000L
        /** No audio from the input for this long means it is gone: Android reroutes an unplugged one well within it. */
        const val STALL_MS = 3_000L
        /** How long a reopened input has to deliver its first audio. */
        const val REOPEN_WAIT_MS = 1_500L
        const val DISCONNECTED = "Your microphone disconnected. Connect a microphone or choose another input and try again."

        /** [capturedAtMs] is when the input last delivered audio. */
        fun stalled(capturedAtMs: Long, nowMs: Long): Boolean = nowMs - capturedAtMs >= STALL_MS
    }
}
