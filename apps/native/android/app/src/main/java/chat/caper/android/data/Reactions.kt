package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ReactionUpdate
import java.math.BigInteger

/** Applies only strictly newer per-message snapshots; stream cursor ordering is handled separately. */
internal fun mergeReaction(message: ChatMessage, update: ReactionUpdate): ChatMessage {
    if (message.id != update.messageId || message.channelId != update.channelId) return message
    val next = update.seq.toBigIntegerOrNull() ?: return message
    val current = message.reactionSeq?.toBigIntegerOrNull() ?: BigInteger.valueOf(-1)
    if (next <= current) return message
    val unique = update.reactions.map { it.copy(authorIds = it.authorIds.distinct()) }.filter { it.authorIds.isNotEmpty() }
    return message.copy(reactions = unique, reactionSeq = update.seq)
}

internal fun mergeReactions(messages: List<ChatMessage>, updates: Map<String, ReactionUpdate>): List<ChatMessage> =
    messages.map { message -> updates[message.id]?.let { mergeReaction(message, it) } ?: message }
