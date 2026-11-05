package chat.caper.android.data

import kotlinx.serialization.Serializable

@Serializable data class ActivityItem(
    val id: String, val kind: String, val actorId: String, val actorName: String,
    val avatarId: Int? = null, val body: String, val createdAt: String, val unread: Boolean,
    val spaceId: String? = null, val spaceName: String? = null,
    val channelId: String? = null, val channelName: String? = null,
    val conversationId: String? = null, val messageId: String? = null,
    val threadRootId: String? = null, val emoji: String? = null,
) {
    val canReply: Boolean get() = kind in setOf("message", "mention", "reply", "reaction") &&
        !messageId.isNullOrEmpty() && !(conversationId ?: channelId).isNullOrEmpty()
    val draftKey: String get() = "inbox:" + listOf(id, conversationId ?: channelId.orEmpty(), messageId.orEmpty(), threadRootId.orEmpty())
        .joinToString("") { "${it.length}:$it" }
    val replyLabel: String get() = if (threadRootId != null) "Reply in thread" else if (conversationId != null) "Reply in DM" else "Reply in #${channelName ?: "channel"}"
}
@Serializable data class ActivityPage(val items: List<ActivityItem>, val nextCursor: String? = null, val unreadCount: Int)
@Serializable data class ActivityRead(val unreadCount: Int)

internal enum class ActivityFilter(val wire: String, val label: String) {
    All("all", "All"), Dms("dms", "DMs"), Mentions("mentions", "Mentions"),
    Threads("threads", "Threads"), Reactions("reactions", "Reactions"), Invitations("invitations", "Invitations")
}
internal data class ActivityInboxState(
    val items: List<ActivityItem> = emptyList(), val cursor: String? = null, val unreadCount: Int = 0,
    val filter: ActivityFilter = ActivityFilter.All, val unreadOnly: Boolean = true,
    val loading: Boolean = false, val error: String? = null,
    val expanded: Map<String, ActivityItem> = emptyMap(),
    /** The row whose open is in flight; a second open waits for it. */
    val opening: String? = null,
    /** Mark-reads and opens in flight; automatic polling waits for them. */
    val busy: Int = 0,
    /** The previous filter's rows, kept in place (dimmed and inert) until the new page arrives. */
    val stale: Boolean = false,
) {
    val visibleItems: List<ActivityItem> get() = (items + expanded.values.filter { pinned -> items.none { it.id == pinned.id } }).sortedByDescending { it.createdAt }
}
