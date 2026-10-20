package chat.caper.android.data

import chat.caper.android.model.ChatEmbed
import chat.caper.android.model.ChatMessage
import chat.caper.android.model.embedsOf
import java.net.URI
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * `message.embeds` (docs/media.md "Link embeds"): the cards for one [revision] of a message,
 * sent unsequenced once they are ready. Malformed cards were already skipped.
 */
data class EmbedsUpdate(val channelId: String, val messageId: String, val revision: Int, val embeds: List<ChatEmbed>)

/** An unsequenced `message.embeds` event for [channelId]; malformed ones are null, dropped and never fatal. */
internal fun embedsUpdate(event: JsonObject, channelId: String): EmbedsUpdate? = runCatching {
    fun number(key: String) = (event[key] as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull
    fun string(key: String) = (event[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
    if (string("type") != "message.embeds" || number("schemaVersion") != 1L || string("channelId") != channelId) return null
    val messageId = string("messageId")?.takeIf { it.isNotEmpty() } ?: return null
    val revision = number("revision")?.takeIf { it in 1..Int.MAX_VALUE.toLong() } ?: return null
    val embeds = event["embeds"] as? JsonArray ?: return null
    EmbedsUpdate(channelId, messageId, revision.toInt(), embedsOf(embeds))
}.getOrNull()

/** The message with [embeds] when they belong to its current revision; any other revision, or a deleted message, is unchanged. */
internal fun withEmbeds(message: ChatMessage, revision: Int, embeds: List<ChatEmbed>): ChatMessage {
    if (message.revision != revision || message.deletedAt != null || message.content.embeds == embeds) return message
    return message.copy(content = message.content.copy(embeds = embeds))
}

internal fun withEmbeds(message: ChatMessage, update: EmbedsUpdate): ChatMessage =
    if (message.id != update.messageId || message.channelId != update.channelId) message else withEmbeds(message, update.revision, update.embeds)

/** A replay or refetch of the same revision may carry cards [message] lacks (it missed the event): take them. */
internal fun sameRevisionEmbeds(message: ChatMessage, candidate: ChatMessage): ChatMessage =
    if (candidate.id != message.id || candidate.content.embeds.isEmpty()) message
    else withEmbeds(message, candidate.revision, candidate.content.embeds)

/**
 * Keeps cards for a message that is not loaded yet, applied when it arrives. Bounded: the oldest
 * goes first, since a reload shows the stored cards anyway (never a resync).
 */
internal fun cacheUnseenEmbeds(unseen: LinkedHashMap<String, EmbedsUpdate>, update: EmbedsUpdate, limit: Int = 256) {
    unseen.remove(update.messageId)
    while (unseen.size >= limit) unseen.remove(unseen.keys.first())
    unseen[update.messageId] = update
}

/**
 * The first link in a draft that will get a card, for the composer preview: the API's rule, so
 * a link written `<like this>` gets none (web's `previewLink`).
 */
internal fun previewLink(draft: String): String? {
    val segments = linkSegments(draft)
    segments.forEachIndexed { index, segment ->
        val href = segment.href ?: return@forEachIndexed
        if (segments.getOrNull(index - 1)?.text?.endsWith("<") == true && segments.getOrNull(index + 1)?.text?.startsWith(">") == true) return@forEachIndexed
        return href
    }
    return null
}

/** The hosts whose player page (`/play/{provider}/{id}`) a web view may load. */
internal val EMBED_PLAYER_HOSTS = setOf("embed.caper.chat", "embed.staging.caper.chat")

/** A player block's `play` page, only over HTTPS from Caper's embed host; null otherwise. */
internal fun embedPlayUrl(play: String?): String? {
    val value = play ?: return null
    val uri = runCatching { URI(value) }.getOrNull() ?: return null
    if (uri.scheme != "https" || uri.rawUserInfo != null || (uri.port != -1 && uri.port != 443)) return null
    return value.takeIf { uri.host?.lowercase() in EMBED_PLAYER_HOSTS }
}

/** Whether a navigation inside the player web view stays on the player host; anything else opens in the browser. */
internal fun embedPlayerNavigation(url: String): Boolean = embedPlayUrl(url) != null

/** Web's `fit`: a display size that reserves space before media loads (at most 400 × 300); unknown sizes get the full width at 16:9. */
internal data class EmbedFrame(val width: Int, val aspectRatio: Float)

internal fun embedFrame(width: Double?, height: Double?, maxWidth: Int = 400, maxHeight: Int = 300): EmbedFrame {
    if (width == null || height == null || width <= 0 || height <= 0) return EmbedFrame(maxWidth, 16f / 9f)
    val scale = minOf(1.0, maxWidth / width, maxHeight / height)
    return EmbedFrame(maxOf(1, Math.round(width * scale).toInt()), (width / height).toFloat())
}
