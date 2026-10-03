package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ReactionUpdate
import java.math.BigInteger

/** HTTP acknowledgements are snapshots, not proof that preceding stream events were applied. */
internal fun replayCursorAfterReaction(current: String?, update: ReactionUpdate, sequenced: Boolean): String? =
    if (sequenced) update.seq else current

internal fun ReactionUpdate.validated(expectedChannel: String, expectedMessage: String? = null): ReactionUpdate {
    require(type == "message.reactions" && schemaVersion == 1) { "Invalid reaction update type or schema." }
    require(channelId == expectedChannel && channelId.isNotEmpty()) { "Reaction channel mismatch." }
    require(messageId.isNotEmpty() && (expectedMessage == null || messageId == expectedMessage)) { "Reaction message mismatch." }
    val number = seq.toBigIntegerOrNull()
    require(number != null && number.signum() >= 0 && number.toString() == seq) { "Invalid reaction sequence." }
    require(reactions.all { reaction ->
        reaction.emoji.isNotEmpty() && reaction.authorIds.isNotEmpty() &&
            reaction.authorIds.all(String::isNotEmpty) && reaction.authorIds.size == reaction.authorIds.toSet().size
    }) { "Invalid reactions." }
    return this
}

/** Applies only strictly newer per-message snapshots; stream cursor ordering is handled separately. */
internal fun mergeReaction(message: ChatMessage, update: ReactionUpdate): ChatMessage {
    if (message.id != update.messageId || message.channelId != update.channelId) return message
    val next = update.seq.toBigIntegerOrNull() ?: return message
    val current = message.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
    if (next <= current) return message
    return message.copy(reactions = update.reactions, reactionSeq = update.seq)
}

/** Monotonically merges messages and consumes cached updates once their message appears. */
internal fun mergeMessages(
    loaded: List<ChatMessage>, incoming: List<ChatMessage>, unseen: MutableMap<String, ReactionUpdate>,
): List<ChatMessage> {
    val merged = linkedMapOf<String, ChatMessage>()
    (loaded + incoming).forEach { candidate ->
        val current = merged[candidate.id]
        merged[candidate.id] = if (current == null) candidate else {
            val candidateReaction = candidate.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
            val currentReaction = current.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
            if (candidateReaction > currentReaction) candidate else current
        }
    }
    return merged.values.map { message ->
        unseen.remove(message.id)?.let { mergeReaction(message, it) } ?: message
    }.sortedWith(compareBy { BigInteger(it.seq) })
}

/** Returns false when a new unseen message would exceed the bound and needs a resync. */
internal fun cacheUnseenReaction(
    unseen: MutableMap<String, ReactionUpdate>, update: ReactionUpdate, limit: Int = 256,
): Boolean {
    val old = unseen[update.messageId]
    if (old == null && unseen.size >= limit) return false
    val next = update.seq.toBigIntegerOrNull() ?: return true
    if (old == null || next > (old.seq.toBigIntegerOrNull() ?: BigInteger.valueOf(-1))) unseen[update.messageId] = update
    return true
}
