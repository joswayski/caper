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

/**
 * Mentions use their own channel, and so does `feedback` (a report for the Caper team, which is
 * treated like a direct mention); invitations and message requests have theirs; every other
 * kind, known or not, is a plain DM or channel message.
 */
internal fun notificationChannelFor(kind: String, direct: Boolean): String = when (kind) {
    "mention.user", "mention.everyone", "feedback" -> MENTIONS_CHANNEL
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
    /** The thread the message replies in, when it is a thread reply (any message kind). */
    val threadRootId: String? = null,
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
            // A retraction is never shown, whatever else it carries ([PushRetraction]).
            if (kind == PushRetraction.KIND) return null
            val invitation = kind == "space.invitation" || kind == "channel.invitation"
            val request = kind == "direct.request"
            val voice = kind == "voice.started"
            val consent = invitation || request
            val notificationId = if (consent || voice) value("notificationId")?.takeIf { Regex("^[1-9][0-9]*$").matches(it) } ?: return null else null
            val messageId = if (voice) "voice:$notificationId" else if (consent) "${if (invitation) "invitation" else "request"}:$notificationId" else value("messageId")?.takeIf(messageIdPattern::matches) ?: return null
            val conversationId = value("conversationId")
            val spaceId = value("spaceId")
            val channelId = value("channelId")
            if (voice && conversationId != null) return null
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
                threadRootId = if (consent || voice) null else value("threadRootId")?.takeIf(messageIdPattern::matches),
            )
        }

        // The server sends 180-character bodies; these only bound unexpected input.
        private const val MAX_NAME = 200
        private const val MAX_TITLE = 400
        private const val MAX_BODY = 1_000

        private fun String.limit(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max)) + "…"
    }
}

/**
 * A `message.deleted` data message: take [messageId]'s line back out of its DM's or channel's
 * notification. It deliberately carries `deletedMessageId`, never `messageId`, so older app
 * versions drop it.
 */
internal data class PushRetraction(val messageId: String, val conversationId: String?, val spaceId: String?, val channelId: String?) {
    /** The tag of the conversation notification that may show the message, as [PushPayload.tag]. */
    val tag: String get() = conversationId ?: checkNotNull(channelId)

    companion object {
        const val KIND = "message.deleted"
        private val externalId = Regex("^[A-Za-z0-9]{12}$")
        private val messageIdPattern = Regex("^[A-Za-z0-9]{15}$")

        /** Null for anything but a well-formed DM or channel retraction. */
        fun parse(data: Map<String, String>): PushRetraction? {
            fun value(key: String) = data[key]?.trim()?.takeIf { it.isNotEmpty() }
            if (value("kind") != KIND) return null
            val messageId = value("deletedMessageId")?.takeIf(messageIdPattern::matches) ?: return null
            val conversationId = value("conversationId")
            val spaceId = value("spaceId")
            val channelId = value("channelId")
            if (conversationId != null) {
                if (!externalId.matches(conversationId) || spaceId != null || channelId != null) return null
            } else if (spaceId == null || channelId == null || !externalId.matches(spaceId) || !externalId.matches(channelId)) return null
            return PushRetraction(messageId, conversationId, spaceId, channelId)
        }
    }
}

/**
 * Which line of a conversation notification shows [messageId]. Lines posted by this version
 * record their message ID; for lines from older versions the notification's message ID list
 * ([postedIds], oldest first) counts only when it lines up with the lines one to one. Null
 * leaves the notification as it is.
 */
internal fun retractedLine(lineIds: List<String?>, postedIds: List<String>, messageId: String): Int? {
    lineIds.indexOf(messageId).takeIf { it >= 0 }?.let { return it }
    if (postedIds.size != lineIds.size || lineIds.indices.any { lineIds[it] != null && lineIds[it] != postedIds[it] }) return null
    return postedIds.indexOf(messageId).takeIf { it >= 0 && lineIds[it] == null }
}

/**
 * Message IDs retracted in the last [ttlMs]: a retraction can overtake its original (or the
 * original can arrive again), and that original must never be shown.
 */
internal class RecentRetractions(private val nowMs: () -> Long = System::currentTimeMillis, private val ttlMs: Long = 30 * 60_000L, private val max: Int = 200) {
    private val retracted = LinkedHashMap<String, Long>()

    @Synchronized fun add(messageId: String) {
        prune()
        retracted.remove(messageId)
        retracted[messageId] = nowMs()
        while (retracted.size > max) retracted.remove(retracted.keys.first())
    }

    @Synchronized operator fun contains(messageId: String): Boolean { prune(); return messageId in retracted }

    private fun prune() {
        val now = nowMs()
        retracted.entries.removeAll { now - it.value > ttlMs }
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
