package chat.caper.android.voice

/** Monotonic ownership gate preventing a stopped asynchronous join from becoming active. */
internal class CallAttemptGate {
    private var generation = 0L
    @Synchronized fun begin(): Long = ++generation
    @Synchronized fun end() { generation++ }
    @Synchronized fun isCurrent(value: Long) = value == generation
}
