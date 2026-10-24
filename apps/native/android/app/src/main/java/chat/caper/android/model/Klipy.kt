package chat.caper.android.model

import java.net.URI
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.nullable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * A message's `content.gif` (docs/media.md "GIFs and stickers (KLIPY)"): KLIPY's media URLs exactly
 * as KLIPY returned them, loaded straight from KLIPY. At least one of [gif] and [webp] is present.
 */
@Serializable data class KlipyGif(
    val provider: String = "klipy",
    /** `gif` or `sticker`. */
    val kind: String,
    val slug: String,
    val title: String = "",
    val width: Int,
    val height: Int,
    val gif: String? = null,
    val webp: String? = null,
    val mp4: String? = null,
    val still: String? = null,
) {
    val sticker: Boolean get() = kind == "sticker"
    /** The accessible label, and what a quote of a captionless GIF reads. */
    val label: String get() = title.ifEmpty { if (sticker) "Sticker" else "GIF" }

    /** Viewer-only external image, never a Caper asset to sign or refresh. API 26–27 prefer GIF. */
    fun mediaViewerAttachment(preferWebp: Boolean): ChatAttachment? {
        val url = (if (preferWebp) webp ?: gif else gif ?: webp) ?: return null
        val format = if (url == webp) "webp" else "gif"
        return ChatAttachment(
            id = "klipy:$url", kind = "image", contentType = "image/$format", name = "$label.$format", size = 0,
            width = width, height = height, url = url, previewUrl = still,
        )
    }

    /** The send body's `gif`, without absent URLs (they would change the retry hash). */
    fun toJson(): JsonObject = buildJsonObject {
        put("provider", provider); put("kind", kind); put("slug", slug); put("title", title)
        put("width", width); put("height", height)
        gif?.let { put("gif", it) }; webp?.let { put("webp", it) }; mp4?.let { put("mp4", it) }; still?.let { put("still", it) }
    }
}

/** `GET /api/klipy`: KLIPY's client key (shipped to devices by design) and this account's opaque customer id. */
@Serializable data class KlipyConfig(val appKey: String, val customerId: String) {
    val valid: Boolean get() = appKey.length in 1..200 && appKey.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' || it == '_' } &&
        customerId.length == 32 && customerId.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
}

private const val KLIPY_MAX_URL = 2048
private const val KLIPY_MAX_TEXT = 200
internal const val KLIPY_MAX_EDGE = 4096

/** KLIPY media only: HTTPS on klipy.com or a subdomain, no credentials, port or fragment (`klipy.rs` `media_url`). */
fun klipyMediaUrl(value: String): Boolean {
    if (value.toByteArray(Charsets.UTF_8).size > KLIPY_MAX_URL) return false
    val uri = runCatching { URI(value) }.getOrNull() ?: return false
    val host = uri.host?.lowercase(java.util.Locale.ROOT) ?: return false
    // Like the WHATWG parser the API and web use, the default port is no port at all.
    return uri.scheme.equals("https", ignoreCase = true) && uri.rawUserInfo == null && (uri.port == -1 || uri.port == 443) &&
        uri.rawFragment == null && (host == "klipy.com" || host.endsWith(".klipy.com"))
}

internal fun klipySlug(value: String): Boolean =
    value.length in 1..KLIPY_MAX_TEXT && value.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' || it == '_' }

internal fun klipyTitle(value: String): Boolean =
    value.codePointCount(0, value.length) <= KLIPY_MAX_TEXT && value.codePoints().noneMatch { Character.isISOControl(it) }

/** A whole JSON number from 1 to 4096; strings, fractions and other values are not. */
internal fun klipyEdge(element: JsonElement?): Int? =
    (element as? JsonPrimitive)?.takeIf { !it.isString && it !is JsonNull }?.content?.toIntOrNull()?.takeIf { it in 1..KLIPY_MAX_EDGE }

/**
 * The `content.gif` to render, or null when it breaks the shared rules (`shared/klipy/cases.json`
 * `messages`): the message then shows its text alone and is never rejected. Unknown fields are ignored.
 */
fun klipyGifOf(element: JsonElement?): KlipyGif? {
    val fields = element as? JsonObject ?: return null
    fun string(key: String): String? = (fields[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
    // Absent or null URLs are simply missing; anything else must be a valid KLIPY URL.
    fun url(key: String): Result<String?> = when (val value = fields[key]) {
        null, JsonNull -> Result.success(null)
        else -> (value as? JsonPrimitive)?.takeIf { it.isString && klipyMediaUrl(it.content) }?.let { Result.success(it.content) }
            ?: Result.failure(IllegalArgumentException(key))
    }
    if (string("provider") != "klipy") return null
    val kind = string("kind")?.takeIf { it == "gif" || it == "sticker" } ?: return null
    val slug = string("slug")?.takeIf(::klipySlug) ?: return null
    val title = if (fields.containsKey("title")) string("title")?.takeIf(::klipyTitle) ?: return null else ""
    val width = klipyEdge(fields["width"]) ?: return null
    val height = klipyEdge(fields["height"]) ?: return null
    val gif = url("gif").getOrElse { return null }
    val webp = url("webp").getOrElse { return null }
    val mp4 = url("mp4").getOrElse { return null }
    val still = url("still").getOrElse { return null }
    if (gif == null && webp == null) return null
    return KlipyGif("klipy", kind, slug, title, width, height, gif, webp, mp4, still)
}

/** `content.gif` that breaks the rules decodes as null, so the message still shows its text. */
object TolerantGifSerializer : KSerializer<KlipyGif?> {
    private val delegate = KlipyGif.serializer().nullable
    override val descriptor: SerialDescriptor = delegate.descriptor
    override fun serialize(encoder: Encoder, value: KlipyGif?) = delegate.serialize(encoder, value)
    override fun deserialize(decoder: Decoder): KlipyGif? {
        val input = decoder as? JsonDecoder ?: return delegate.deserialize(decoder)
        return klipyGifOf(input.decodeJsonElement())
    }
}
