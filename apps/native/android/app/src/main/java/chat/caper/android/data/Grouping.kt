package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import java.time.Duration
import java.time.Instant
import java.time.ZoneId

/** A message stays in its author's run while it is at most this long after the row above. */
internal val GROUP_WINDOW: Duration = Duration.ofMinutes(5)

/**
 * Whether a message row is grouped (compact: no avatar, no name/time header) under [previous],
 * the message row directly above it in the same list (a timeline, or a thread's root and replies).
 *
 * [previous] is null when nothing is above it, or the row above is not a message: a blocked-messages
 * placeholder, for example. A pending send passes its author's id. Grouping is presentation only.
 *
 * - The same author, at most five minutes apart and on the same local day (no date divider between).
 * - [threadContext]: a "Replied to a thread" broadcast keeps its full header with its context line.
 * - A thread's root never starts a group: its first reply ([threadRootId] is the row above) keeps its header.
 * - The jump target ([jumpTarget], the highlighted message) keeps the row below it separate.
 */
internal fun groupsWithPrevious(
    previous: ChatMessage?,
    authorId: String?,
    createdAt: String,
    threadRootId: String? = null,
    threadContext: Boolean = false,
    jumpTarget: String? = null,
    zoneId: ZoneId = ZoneId.systemDefault(),
): Boolean {
    if (previous == null || authorId == null || previous.author.id != authorId) return false
    if (threadContext || previous.id == threadRootId || previous.id == jumpTarget) return false
    val before = instant(previous.createdAt) ?: return false
    val after = instant(createdAt) ?: return false
    if (Duration.between(before, after).abs() > GROUP_WINDOW) return false
    return before.atZone(zoneId).toLocalDate() == after.atZone(zoneId).toLocalDate()
}

/** [current]'s row under [previous]; in the channel timeline a reply is a broadcast with a context line. */
internal fun groupsWithPrevious(
    previous: ChatMessage?,
    current: ChatMessage,
    inThread: Boolean,
    jumpTarget: String? = null,
    zoneId: ZoneId = ZoneId.systemDefault(),
): Boolean = groupsWithPrevious(
    previous, current.author.id, current.createdAt, current.threadRootId,
    threadContext = !inThread && current.threadRootId != null, jumpTarget, zoneId,
)

private fun instant(value: String): Instant? = runCatching { Instant.parse(value) }.getOrNull()
