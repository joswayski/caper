package chat.caper.android.data

import chat.caper.android.model.ChatHistory
import chat.caper.android.model.ChatMessage
import java.math.BigInteger

/** Retains paginated history only when the refreshed page reaches the applied event cursor. */
internal fun recoverHistory(
    retained: List<ChatMessage>,
    retainedHasMore: Boolean,
    appliedCursor: String,
    refreshed: ChatHistory,
): ChatHistory {
    val refreshedFirst = refreshed.messages.firstOrNull() ?: return refreshed
    if (BigInteger(refreshedFirst.seq) > BigInteger(appliedCursor) + BigInteger.ONE) return refreshed

    val retainedOlderPrefix = retained.firstOrNull()?.let {
        BigInteger(it.seq) < BigInteger(refreshedFirst.seq)
    } == true
    val byId = LinkedHashMap<String, ChatMessage>()
    retained.forEach { byId[it.id] = it }
    // Refreshed server state wins if an ID overlaps retained history.
    refreshed.messages.forEach { byId[it.id] = it }
    return refreshed.copy(
        messages = byId.values.sortedBy { BigInteger(it.seq) },
        hasMore = if (retainedOlderPrefix) retainedHasMore else refreshed.hasMore,
    )
}
