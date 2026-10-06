package chat.caper.android.data

import chat.caper.android.model.Reactor
import chat.caper.android.model.ReactorList

internal fun ReactorList.validated(expectedMessage: String): ReactorList {
    require(messageId == expectedMessage) { "Reaction message mismatch." }
    val number = reactionSeq.toBigIntegerOrNull()
    require(number != null && number.signum() >= 0 && number.toString() == reactionSeq) { "Invalid reaction sequence." }
    require(reactions.map { it.emoji }.toSet().size == reactions.size && reactions.all { group ->
        group.emoji.isNotEmpty() && group.authors.isNotEmpty() && group.authors.all { it.id.isNotEmpty() } &&
            group.authors.map { it.id }.toSet().size == group.authors.size
    }) { "Invalid reactions." }
    return this
}

/** Who reacted, reused per message while its snapshot's reaction revision is unchanged. */
internal class ReactorCache(private val limit: Int = 64) {
    private val entries = object : LinkedHashMap<String, ReactorList>(16, .75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ReactorList>?): Boolean = size > limit
    }

    /** Snapshots without a revision have never had a reaction; the API reports those as "0". */
    @Synchronized fun get(messageId: String, reactionSeq: String?): ReactorList? =
        entries[messageId]?.takeIf { it.reactionSeq == (reactionSeq ?: "0") }

    @Synchronized fun put(list: ReactorList) { entries[list.messageId] = list }

    @Synchronized fun clear() { entries.clear() }
}

/** The name every client shows for a reactor: display name, then username, then "Someone". */
internal fun reactorName(reactor: Reactor): String =
    reactor.displayName?.takeIf { it.isNotBlank() } ?: reactor.username?.takeIf { it.isNotBlank() } ?: "Someone"

/** ":thumbs-up:" from the emoji catalog, or the glyph itself when no name is known. */
internal fun reactionEmojiLabel(emojiName: String?, emoji: String): String =
    emojiName?.takeIf { it.isNotBlank() }?.let { ":$it:" } ?: emoji

/**
 * Shared wording: "You, Alice A, Bob B and 2 others reacted with :thumbs-up:".
 * The signed-in person moves to the front as "You"; at most three names are listed.
 */
internal fun reactionSummary(authors: List<Reactor>, selfId: String?, emojiName: String?, emoji: String): String {
    val self = selfId?.let { id -> authors.firstOrNull { it.id == id } }
    val names = buildList {
        if (self != null) add("You")
        authors.forEach { if (it !== self) add(reactorName(it)) }
    }
    if (names.isEmpty()) return fallbackReactionSummary(emptyList(), selfId, emojiName, emoji)
    val label = reactionEmojiLabel(emojiName, emoji)
    val shown = names.take(3)
    val others = names.size - shown.size
    val people = when {
        others > 0 -> shown.joinToString(", ") + " and $others " + if (others == 1) "other" else "others"
        shown.size == 1 -> shown.single()
        else -> shown.dropLast(1).joinToString(", ") + " and " + shown.last()
    }
    return "$people reacted with $label"
}

/** Before names load (or when loading fails), only the snapshot's public IDs are known. */
internal fun fallbackReactionSummary(authorIds: List<String>, selfId: String?, emojiName: String?, emoji: String): String {
    val label = reactionEmojiLabel(emojiName, emoji)
    if (selfId != null && authorIds == listOf(selfId)) return "You reacted with $label"
    val count = authorIds.size
    return "$count ${if (count == 1) "person" else "people"} reacted with $label"
}
