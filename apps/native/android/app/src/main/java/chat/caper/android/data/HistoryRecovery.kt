package chat.caper.android.data

import chat.caper.android.model.ChatHistory
import chat.caper.android.model.ChatMessage
import java.math.BigInteger

/** Retains paginated history only when refresh proves that no unapplied event was a reaction or file update. */
internal fun recoverHistory(
    retained: List<ChatMessage>,
    retainedHasMore: Boolean,
    appliedCursor: String,
    refreshed: ChatHistory,
): ChatHistory {
    val refreshedFirst = refreshed.messages.firstOrNull() ?: return refreshed
    val applied = BigInteger(appliedCursor)
    val refreshedCursor = BigInteger(refreshed.cursor)
    val canRetain = refreshedCursor == applied || accountsForMissingEvents(refreshed.messages, applied, refreshedCursor)

    val retainedOlderPrefix = canRetain && retained.firstOrNull()?.let {
        BigInteger(it.seq) < BigInteger(refreshedFirst.seq)
    } == true
    val retainedById = retained.associateBy { it.id }
    val byId = LinkedHashMap<String, ChatMessage>()
    if (canRetain) byId.putAll(retainedById)
    // Refreshed metadata wins, but never regress a newer reaction snapshot already applied locally.
    refreshed.messages.forEach { fresh ->
        val old = retainedById[fresh.id]
        val oldReaction = old?.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
        val freshReaction = fresh.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
        val merged = if (old != null && oldReaction > freshReaction) {
            fresh.copy(reactions = old.reactions, reactionSeq = old.reactionSeq)
        } else fresh
        // Likewise for files: a newer `message.attachments` snapshot already applied stays.
        byId[fresh.id] = if (old != null) newerAttachments(merged, old) else merged
    }
    return refreshed.copy(
        messages = byId.values.sortedBy { BigInteger(it.seq) },
        hasMore = if (retainedOlderPrefix) retainedHasMore else refreshed.hasMore,
    )
}

private fun accountsForMissingEvents(messages: List<ChatMessage>, applied: BigInteger, cursor: BigInteger): Boolean {
    if (cursor < applied) return false
    var expected = applied + BigInteger.ONE
    messages.forEach { message ->
        val sequence = BigInteger(message.seq)
        if (sequence in expected..cursor) {
            if (sequence != expected) return false
            expected += BigInteger.ONE
        } else if (sequence > applied && sequence <= cursor) return false
    }
    return expected == cursor + BigInteger.ONE
}
