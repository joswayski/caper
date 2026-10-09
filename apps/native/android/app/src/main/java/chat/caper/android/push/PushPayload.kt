package chat.caper.android.push

import chat.caper.android.model.caperAvatarIndex

internal const val DIRECT_MESSAGES_CHANNEL = "direct_messages"
internal const val CHANNEL_MESSAGES_CHANNEL = "channel_messages"
internal const val MENTIONS_CHANNEL = "mentions"

/** Android notification channels, in the order Settings lists them. */
internal val pushChannels = listOf(
    DIRECT_MESSAGES_CHANNEL to "Direct messages",
    CHANNEL_MESSAGES_CHANNEL to "Channel messages",
    MENTIONS_CHANNEL to "Mentions",
)

/** Mentions use their own channel; every other kind, known or not, is a plain DM or channel message. */
internal fun notificationChannelFor(kind: String, direct: Boolean): String = when (kind) {
    "mention.user", "mention.everyone" -> MENTIONS_CHANNEL
    else -> if (direct) DIRECT_MESSAGES_CHANNEL else CHANNEL_MESSAGES_CHANNEL
}

/**
 * One FCM data message: a DM carries [conversationId], a channel message [spaceId] and
 * [channelId]. The server already filters mutes, levels, requests and blocks.
 */
internal data class PushPayload(
    val kind: String,
    val messageId: String,
    val conversationId: String?,
    val spaceId: String?,
    val channelId: String?,
    val title: String,
    val body: String,
    val sender: String,
    val senderId: String?,
    val senderAvatarId: Int?,
    /** `#channel (Space)` for a channel; null for a DM. */
    val conversationTitle: String?,
) {
    val direct: Boolean get() = conversationId != null

    /** One notification per DM or channel, tagged with its ID. */
    val tag: String get() = conversationId ?: checkNotNull(channelId)

    val channel: String get() = notificationChannelFor(kind, direct)

    /** Channel notifications list each sender under the channel's title; a DM is one person. */
    val groupConversation: Boolean get() = !direct

    companion object {
        private val externalId = Regex("^[A-Za-z0-9]{12}$")
        private val messageIdPattern = Regex("^[A-Za-z0-9]{15}$")

        /** Null for anything that is not a well-formed DM or channel message. */
        fun parse(data: Map<String, String>): PushPayload? {
            fun value(key: String) = data[key]?.trim()?.takeIf { it.isNotEmpty() }
            val messageId = value("messageId")?.takeIf(messageIdPattern::matches) ?: return null
            val conversationId = value("conversationId")
            val spaceId = value("spaceId")
            val channelId = value("channelId")
            if (conversationId != null) {
                if (!externalId.matches(conversationId) || spaceId != null || channelId != null) return null
            } else if (spaceId == null || channelId == null || !externalId.matches(spaceId) || !externalId.matches(channelId)) return null
            val direct = conversationId != null
            val conversationTitle = if (direct) null else value("conversationTitle")?.limit(MAX_NAME)
            val sender = value("sender")?.limit(MAX_NAME)
            val title = value("title")?.limit(MAX_TITLE)
                ?: if (direct) sender else listOfNotNull(sender, conversationTitle).joinToString(" · ").ifEmpty { null }
            return PushPayload(
                kind = value("kind").orEmpty(),
                messageId = messageId,
                conversationId = conversationId,
                spaceId = spaceId,
                channelId = channelId,
                title = title ?: "Caper",
                body = value("body")?.limit(MAX_BODY) ?: "Sent a message",
                sender = sender ?: title?.substringBefore(" · ") ?: "Caper",
                senderId = value("senderId"),
                senderAvatarId = caperAvatarIndex(value("senderAvatarId")?.toIntOrNull()),
                conversationTitle = conversationTitle,
            )
        }

        // The server sends 180-character bodies; these only bound unexpected input.
        private const val MAX_NAME = 200
        private const val MAX_TITLE = 400
        private const val MAX_BODY = 1_000

        private fun String.limit(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max)) + "…"
    }
}
