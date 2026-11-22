package chat.caper.android.data

/** Shell refreshes on a timer: rare while the account's live notifications arrive, often while they don't. */
internal fun shellPollIntervalMs(notificationsLive: Boolean): Long = if (notificationsLive) 60_000L else 15_000L

/** Hands out request tickets; only the newest one's answer may be applied, so a slow older reply never wins. */
internal class LatestRequest {
    private var issued = 0L
    fun next(): Long = ++issued
    fun isLatest(ticket: Long): Boolean = ticket == issued
}
