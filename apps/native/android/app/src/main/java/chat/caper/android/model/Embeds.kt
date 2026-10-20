package chat.caper.android.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonTransformingSerializer
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull

/**
 * A link preview card (docs/media.md "Link embeds"), version 1. Every string is plain text and
 * every link is http(s); [embedOf] drops what is not, like web's `embedOf`. The shared parsing
 * cases are `shared/messages/embed-cases.json`.
 */
@Serializable data class ChatEmbed(val version: Int = 1, val url: String, val site: EmbedSite, val blocks: List<EmbedBlock>)
/** [domain] comes from the posted URL, never from the page; cards always show it. */
@Serializable data class EmbedSite(val name: String, val domain: String)
@Serializable data class EmbedImage(val url: String, val width: Double? = null, val height: Double? = null, val alt: String? = null)

/** Drawn in order; unknown or malformed blocks are skipped. */
@Serializable sealed interface EmbedBlock {
    @Serializable @SerialName("author")
    data class Author(val name: String, val handle: String? = null, val url: String? = null, val icon: String? = null) : EmbedBlock
    @Serializable @SerialName("title")
    data class Title(val text: String, val url: String? = null) : EmbedBlock
    @Serializable @SerialName("text")
    data class Text(val text: String) : EmbedBlock
    /** 1-4 images; [thumbnail] is one small image beside the text. */
    @Serializable @SerialName("images")
    data class Images(val items: List<EmbedImage>, val thumbnail: Boolean = false) : EmbedBlock
    /** A direct MP4/WebM; [gif] loops muted without controls. */
    @Serializable @SerialName("video")
    data class Video(
        val url: String, val poster: String? = null, val width: Double? = null, val height: Double? = null,
        val durationMs: Double? = null, val gif: Boolean = false,
    ) : EmbedBlock
    /** A provider's player; native apps load [play] (Caper's player page) after Play. */
    @Serializable @SerialName("player")
    data class Player(
        val provider: String, val id: String, val play: String? = null, val poster: String? = null,
        val width: Double? = null, val height: Double? = null, val start: Double? = null,
    ) : EmbedBlock
    @Serializable @SerialName("footer")
    data class Footer(val text: String? = null, val timestamp: String? = null) : EmbedBlock
}

private val EMBED_PROVIDERS = setOf("youtube", "vimeo", "twitch")

private fun JsonElement?.string(): String? = (this as? JsonPrimitive)?.takeIf { it.isString }?.content
private fun JsonElement?.text(): String? = string()?.takeIf { it.isNotEmpty() }
/** Absent, or a string (JSON `null` is neither, as on web). */
private fun JsonElement?.optionalText(): Boolean = this == null || string() != null

/** ASCII case-insensitively, like web's `/^https?:\/\//i`. */
internal fun isWebLink(value: String): Boolean {
    fun at(prefix: String) = value.length >= prefix.length &&
        prefix.indices.all { val c = value[it]; (if (c in 'A'..'Z') c + ('a' - 'A') else c) == prefix[it] }
    return at("https://") || at("http://")
}
private fun JsonElement?.link(): String? = string()?.takeIf(::isWebLink)
private fun JsonElement?.optionalLink(): Boolean = this == null || link() != null
private fun JsonElement?.size(): Double? = (this as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull?.takeIf { it.isFinite() && it > 0 }
private fun JsonElement?.optionalSize(): Boolean = this == null || size() != null

/** A block safe to draw, or null for unknown or malformed blocks (skipped). */
private fun embedBlock(value: JsonElement): EmbedBlock? {
    val item = value as? JsonObject ?: return null
    return when (item["type"].string()) {
        "author" -> {
            val name = item["name"].text()
            if (name == null || !item["handle"].optionalText() || !item["url"].optionalLink() || !item["icon"].optionalLink()) null
            else EmbedBlock.Author(name, item["handle"].string(), item["url"].link(), item["icon"].link())
        }
        "title" -> item["text"].text()?.takeIf { item["url"].optionalLink() }?.let { EmbedBlock.Title(it, item["url"].link()) }
        "text" -> item["text"].text()?.let { EmbedBlock.Text(it) }
        "images" -> {
            val items = (item["items"] as? JsonArray)?.mapNotNull { entry ->
                val image = entry as? JsonObject ?: return@mapNotNull null
                val url = image["url"].link()
                if (url == null || !image["width"].optionalSize() || !image["height"].optionalSize() || !image["alt"].optionalText()) null
                else EmbedImage(url, image["width"].size(), image["height"].size(), image["alt"].string())
            }
            if (items.isNullOrEmpty()) null
            else EmbedBlock.Images(items.take(4), (item["thumbnail"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull == true)
        }
        "video" -> {
            val url = item["url"].link()
            if (url == null || !item["poster"].optionalLink() || listOf("width", "height", "durationMs").any { !item[it].optionalSize() }) null
            else EmbedBlock.Video(
                url, item["poster"].link(), item["width"].size(), item["height"].size(), item["durationMs"].size(),
                (item["gif"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull == true,
            )
        }
        "player" -> {
            val provider = item["provider"].string()?.takeIf { it in EMBED_PROVIDERS }
            val id = item["id"].text()
            if (provider == null || id == null || !item["poster"].optionalLink() || listOf("width", "height", "start").any { !item[it].optionalSize() }) null
            // `play` is checked where it is loaded (see `embedPlayUrl`).
            else EmbedBlock.Player(provider, id, item["play"].string(), item["poster"].link(), item["width"].size(), item["height"].size(), item["start"].size())
        }
        "footer" -> {
            val text = item["text"]; val timestamp = item["timestamp"]
            if ((text.text() == null && timestamp.text() == null) || !text.optionalText() || !timestamp.optionalText()) null
            else EmbedBlock.Footer(text.string(), timestamp.string())
        }
        else -> null
    }
}

/** A card safe to draw, or null: version 1, an http(s) link, a named site and at least one usable block. */
fun embedOf(value: JsonElement?): ChatEmbed? {
    val embed = value as? JsonObject ?: return null
    val version = (embed["version"] as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull
    val url = embed["url"].link()
    val site = embed["site"] as? JsonObject
    val blocks = embed["blocks"] as? JsonArray
    if (version != 1.0 || url == null || site == null || blocks == null) return null
    val name = site["name"].text() ?: return null
    val domain = site["domain"].text() ?: return null
    return blocks.mapNotNull(::embedBlock).takeIf { it.isNotEmpty() }?.let { ChatEmbed(1, url, EmbedSite(name, domain), it) }
}

/** Cards safe to draw from a `content.embeds` (or event) array; malformed ones are skipped, never the message. */
fun embedsOf(value: JsonElement?): List<ChatEmbed> = (value as? JsonArray)?.mapNotNull(::embedOf).orEmpty()

/** Mirrors the web's `embedsOf`: one bad card never breaks a history page or frame. */
object TolerantEmbedsSerializer : KSerializer<List<ChatEmbed>> {
    private val delegate = ListSerializer(ChatEmbed.serializer())
    override val descriptor: SerialDescriptor = delegate.descriptor
    override fun serialize(encoder: Encoder, value: List<ChatEmbed>) = delegate.serialize(encoder, value)
    override fun deserialize(decoder: Decoder): List<ChatEmbed> {
        val input = decoder as? JsonDecoder ?: return delegate.deserialize(decoder)
        return embedsOf(input.decodeJsonElement())
    }
}

/** Only a JSON `true` is true; anything else is false rather than a rejected message. */
object TolerantFlagSerializer : JsonTransformingSerializer<Boolean>(Boolean.serializer()) {
    override fun transformDeserialize(element: JsonElement): JsonElement =
        JsonPrimitive((element as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull == true)
}
