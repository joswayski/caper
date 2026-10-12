package chat.caper.android

/** How long a row counts as arriving: long enough for its entrance, short enough
 * that a row the list recomposes later (scrolled away and back) never replays it. */
internal const val ARRIVAL_MS = 1_000L

/**
 * Remembers which messages arrived while the reader watched the live end of a
 * list, so only those animate in (web's `chat/arrivals.ts`). The first render of
 * a channel or thread, older pages, jumps into history and newer pages loaded
 * towards the live end all appear in place. A pending send keeps its client
 * message ID once confirmed, so its confirmed row resumes the same entrance.
 */
internal class Arrivals(private val now: () -> Long = { System.nanoTime() / 1_000_000 }) {
    private class Seen(val key: String, val ids: Set<String>, val last: String?, val live: Boolean)

    private val lists = mutableMapOf<String, Seen>()
    private val arrived = mutableMapOf<String, Long>()

    /**
     * Call on every composition with the list's client message IDs, oldest first.
     * [live] is true when the list ends at the newest message. Both this and the
     * previous composition must be live, so a newer page loaded on the way back to
     * the live end does not count.
     */
    fun observe(scope: String, key: String, ids: List<String>, live: Boolean) {
        val previous = lists[scope]
        lists[scope] = Seen(key, ids.toSet(), ids.lastOrNull(), live)
        val last = previous?.last
        if (previous == null || previous.key != key || last == null || !previous.live || !live) return
        // Only rows after the previous newest row are arrivals; prepended history is not.
        val tail = ids.lastIndexOf(last)
        if (tail < 0) return
        val at = now()
        for (id in ids.subList(tail + 1, ids.size)) if (id !in previous.ids && id !in arrived) arrived[id] = at
        arrived.values.removeAll { at - it > ARRIVAL_MS }
    }

    /** Milliseconds since [id] arrived, or null when it did not arrive recently. */
    fun elapsed(id: String): Long? = arrived[id]?.let { now() - it }?.takeIf { it <= ARRIVAL_MS }

    fun has(id: String) = elapsed(id) != null
}
