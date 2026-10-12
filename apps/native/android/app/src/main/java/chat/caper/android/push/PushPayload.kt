package chat.caper.android.push

import chat.caper.android.model.caperAvatarIndex

internal const val DIRECT_MESSAGES_CHANNEL = "direct_messages"
internal const val CHANNEL_MESSAGES_CHANNEL = "channel_messages"
internal const val MENTIONS_CHANNEL = "mentions"
internal const val INVITATIONS_CHANNEL = "invitations"
internal const val MESSAGE_REQUESTS_CHANNEL = "message_requests"

/** Android notification channels, in the order Settings lists them. */
internal val pushChannels = listOf(
    DIRECT_MESSAGES_CHANNEL to "Direct messages",
    CHANNEL_MESSAGES_CHANNEL to "Channel messages",
    MENTIONS_CHANNEL to "Mentions",
    INVITATIONS_CHANNEL to "Invitations",
    MESSAGE_REQUESTS_CHANNEL to "Message requests",
)

/** Mentions use their own channel; every other kind, known or not, is a plain DM or channel message. */
internal fun notificationChannelFor(kind: String, direct: Boolean): String = when (kind) {
    "mention.user", "mention.everyone" -> MENTIONS_CHANNEL
    "space.invitation", "channel.invitation" -> INVITATIONS_CHANNEL
    "direct.request" -> MESSAGE_REQUESTS_CHANNEL
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
    val attachment: PushAttachment? = null,
    val notificationId: String? = null,
) {
    val direct: Boolean get() = conversationId != null
    val invitation: Boolean get() = kind == "space.invitation" || kind == "channel.invitation"
    val consent: Boolean get() = invitation || kind == "direct.request"

    /** One notification per DM or channel, tagged with its ID. */
    val tag: String get() = if (consent) "${if (invitation) "invitation" else "request"}:$notificationId" else conversationId ?: checkNotNull(channelId)

    val channel: String get() = notificationChannelFor(kind, direct)

    /** Channel notifications list each sender under the channel's title; a DM is one person. */
    val groupConversation: Boolean get() = !direct

    companion object {
        private val externalId = Regex("^[A-Za-z0-9]{12}$")
        private val messageIdPattern = Regex("^[A-Za-z0-9]{15}$")

        /** Null for anything that is not a well-formed DM or channel message. */
        fun parse(data: Map<String, String>): PushPayload? {
            fun value(key: String) = data[key]?.trim()?.takeIf { it.isNotEmpty() }
            val kind = value("kind").orEmpty()
            val invitation = kind == "space.invitation" || kind == "channel.invitation"
            val request = kind == "direct.request"
            val consent = invitation || request
            val notificationId = if (consent) value("notificationId")?.takeIf { Regex("^[1-9][0-9]*$").matches(it) } ?: return null else null
            val messageId = if (consent) "${if (invitation) "invitation" else "request"}:$notificationId" else value("messageId")?.takeIf(messageIdPattern::matches) ?: return null
            val conversationId = value("conversationId")
            val spaceId = value("spaceId")
            val channelId = value("channelId")
            if (invitation) {
                if (conversationId != null || spaceId == null || !externalId.matches(spaceId)) return null
                if (kind == "space.invitation" && channelId != null) return null
                if (kind == "channel.invitation" && (channelId == null || !externalId.matches(channelId))) return null
            } else if (conversationId != null) {
                if (!externalId.matches(conversationId) || spaceId != null || channelId != null) return null
            } else if (request || spaceId == null || channelId == null || !externalId.matches(spaceId) || !externalId.matches(channelId)) return null
            val direct = conversationId != null
            val conversationTitle = if (direct) null else value("conversationTitle")?.limit(MAX_NAME)
            val sender = value("sender")?.limit(MAX_NAME)
            val title = value("title")?.limit(MAX_TITLE)
                ?: if (direct) sender else listOfNotNull(sender, conversationTitle).joinToString(" · ").ifEmpty { null }
            return PushPayload(
                kind = kind,
                messageId = messageId,
                notificationId = notificationId,
                conversationId = conversationId,
                spaceId = spaceId,
                channelId = channelId,
                title = title ?: "Caper",
                body = value("body")?.limit(MAX_BODY) ?: "Sent a message",
                sender = sender ?: title?.substringBefore(" · ") ?: "Caper",
                senderId = value("senderId"),
                senderAvatarId = caperAvatarIndex(value("senderAvatarId")?.toIntOrNull()),
                conversationTitle = conversationTitle,
                attachment = if (consent) null else PushAttachment.parse(data),
            )
        }

        // The server sends 180-character bodies; these only bound unexpected input.
        private const val MAX_NAME = 200
        private const val MAX_TITLE = 400
        private const val MAX_BODY = 1_000

        private fun String.limit(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max)) + "…"
    }
}

/** One signed image original or existing video poster, never a public device file. */
internal data class PushAttachment(val url: String, val contentType: String, val size: Long) {
    companion object {
        const val MAX_BYTES = 50_000_000L
        fun parse(data: Map<String, String>): PushAttachment? {
            val raw = data["attachmentUrl"] ?: return null
            val url = runCatching { java.net.URI(raw) }.getOrNull() ?: return null
            if (url.scheme != "https" || url.host == null || url.rawUserInfo != null || url.rawFragment != null ||
                !Regex("^/(assets|original|preview)/[A-Za-z0-9]{1,64}$").matches(url.path)) return null
            val type = data["attachmentContentType"] ?: return null
            if (type !in listOf("image/png", "image/jpeg", "image/gif", "image/webp", "image/avif")) return null
            val size = data["attachmentSize"]?.toLongOrNull()?.takeIf { it in 1..MAX_BYTES } ?: return null
            return PushAttachment(raw, type, size)
        }
    }
}
