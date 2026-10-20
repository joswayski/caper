package chat.caper.android.data

import chat.caper.android.model.KlipyConfig
import chat.caper.android.model.KlipyGif
import chat.caper.android.model.klipyEdge
import chat.caper.android.model.klipyMediaUrl
import chat.caper.android.model.klipySlug
import java.io.IOException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.put
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

/** The picker's two tabs; [path] is KLIPY's collection, [kind] the sent `content.gif` kind. */
enum class KlipyKind(val path: String, val kind: String) { GIFS("gifs", "gif"), STICKERS("stickers", "sticker") }

/** A grid tile: animated [url] (`webp`, else `gif`) and an optional [still] of the same size. */
data class KlipyTile(val url: String, val still: String?, val width: Int, val height: Int)

sealed interface KlipyItem {
    data class Result(val slug: String, val title: String, val tile: KlipyTile, val gif: KlipyGif) : KlipyItem
    /** A full-row sponsored item: KLIPY's [html], sized to the ad when it says so. */
    data class Ad(val html: String, val width: Int?, val height: Int?) : KlipyItem
}

data class KlipyPage(val items: List<KlipyItem>, val hasNext: Boolean)

private fun JsonObject.obj(key: String) = this[key] as? JsonObject
private fun JsonObject.string(key: String) = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

/** Control characters out, then the first 200 characters (code points), as `klipy.rs` counts them. */
internal fun klipyCleanTitle(value: String): String {
    val clean = buildString { value.codePoints().filter { !Character.isISOControl(it) }.forEach { appendCodePoint(it) } }
    val count = clean.codePointCount(0, clean.length)
    return if (count <= 200) clean else clean.substring(0, clean.offsetByCodePoints(0, 200))
}

/** A positive ad dimension, or null when absent or not positive. */
private fun adEdge(element: JsonElement?): Int? =
    (element as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull?.takeIf { it > 0 && it.isFinite() }?.let { Math.round(it).toInt().coerceAtLeast(1) }

private fun JsonObject.mediaUrl(format: String): String? = obj(format)?.string("url")?.takeIf(::klipyMediaUrl)

/**
 * A KLIPY list response for [kind] in KLIPY's order, ads included (`shared/klipy/cases.json`
 * `parse`/`parseSticker`). Items that break the rules are skipped, never the page.
 */
fun parseKlipyPage(response: JsonElement, kind: KlipyKind): KlipyPage {
    val data = (response as? JsonObject)?.obj("data")
    val entries = data?.get("data") as? JsonArray ?: JsonArray(emptyList())
    val items = entries.mapNotNull { entry ->
        val item = entry as? JsonObject ?: return@mapNotNull null
        if (item.string("type") == "ad") {
            val html = item.string("content")?.takeIf { it.isNotEmpty() } ?: return@mapNotNull null
            return@mapNotNull KlipyItem.Ad(html, adEdge(item["width"]), adEdge(item["height"]))
        }
        val slug = item.string("slug")?.takeIf(::klipySlug) ?: return@mapNotNull null
        val file = item.obj("file") ?: return@mapNotNull null
        val sent = listOf("md", "hd", "sm").firstNotNullOfOrNull { file.obj(it) } ?: return@mapNotNull null
        val gif = sent.mediaUrl("gif")
        val webp = sent.mediaUrl("webp")
        val size = listOf("gif", "webp").firstNotNullOfOrNull { format ->
            val entryOf = sent.obj(format)?.takeIf { sent.mediaUrl(format) != null } ?: return@firstNotNullOfOrNull null
            val width = klipyEdge(entryOf["width"]) ?: return@firstNotNullOfOrNull null
            val height = klipyEdge(entryOf["height"]) ?: return@firstNotNullOfOrNull null
            width to height
        } ?: return@mapNotNull null
        // Stickers keep their transparency in a PNG still; GIFs prefer the JPEG.
        val stills = if (kind == KlipyKind.STICKERS || item.string("type") == "sticker") listOf("png", "jpg") else listOf("jpg", "png")
        val title = klipyCleanTitle(item.string("title").orEmpty())
        val tile = listOf("sm", "xs", "md", "hd").firstNotNullOfOrNull { name ->
            val tileSize = file.obj(name) ?: return@firstNotNullOfOrNull null
            val format = listOf("webp", "gif").firstOrNull { tileSize.mediaUrl(it) != null } ?: return@firstNotNullOfOrNull null
            val media = tileSize.obj(format)!!
            KlipyTile(
                tileSize.mediaUrl(format)!!, stills.firstNotNullOfOrNull { tileSize.mediaUrl(it) },
                klipyEdge(media["width"]) ?: size.first, klipyEdge(media["height"]) ?: size.second,
            )
        } ?: return@mapNotNull null
        KlipyItem.Result(slug, title, tile, KlipyGif(
            "klipy", kind.kind, slug, title, size.first, size.second, gif, webp, sent.mediaUrl("mp4"),
            stills.firstNotNullOfOrNull { sent.mediaUrl(it) },
        ))
    }
    return KlipyPage(items, (data?.get("has_next") as? JsonPrimitive)?.booleanOrNull == true)
}

/**
 * Ad parameters for touch layouts: only fields readable without a permission prompt. Never
 * carrier, advertising ID or location. [maxWidth] is the grid's width in dp.
 */
data class KlipyAdRequest(
    val maxWidth: Int, val osVersion: String, val make: String, val model: String,
    val deviceWidth: Int, val deviceHeight: Int, val pixelRatio: Float, val language: String,
) {
    fun appendTo(url: HttpUrl.Builder) {
        url.addQueryParameter("ad-min-width", "50").addQueryParameter("ad-max-width", maxWidth.coerceAtLeast(50).toString())
            .addQueryParameter("ad-min-height", "50").addQueryParameter("ad-max-height", "250")
            .addQueryParameter("ad-os", "android").addQueryParameter("ad-osv", osVersion)
            .addQueryParameter("ad-make", make).addQueryParameter("ad-model", model)
            .addQueryParameter("ad-device-w", deviceWidth.toString()).addQueryParameter("ad-device-h", deviceHeight.toString())
            .addQueryParameter("ad-pxratio", "%.2f".format(java.util.Locale.US, pixelRatio))
            .addQueryParameter("ad-language", language)
    }
}

/**
 * KLIPY's API, called from this device (KLIPY's terms): search, trending and the share trigger.
 * No Caper credential ever goes here. [userAgent] is the system WebView's, which KLIPY uses to
 * serve mobile ads.
 */
class KlipyClient(
    private val client: OkHttpClient,
    private val base: HttpUrl = "https://api.klipy.com/api/v1/".toHttpUrl(),
    private val json: Json = Json { ignoreUnknownKeys = true },
) {
    private fun url(config: KlipyConfig, kind: KlipyKind, vararg path: String): HttpUrl.Builder {
        require(config.valid) { "Invalid KLIPY configuration." }
        return base.newBuilder().addPathSegment(config.appKey).addPathSegment(kind.path).apply { path.forEach { addPathSegment(it) } }
    }

    /** Trending when [query] is blank, otherwise search; [page] starts at 1. [locale] is the region (ISO 3166 alpha-2). */
    suspend fun page(
        config: KlipyConfig, kind: KlipyKind, query: String, page: Int, locale: String?, ads: KlipyAdRequest?, userAgent: String?,
    ): KlipyPage {
        val search = query.trim()
        val url = url(config, kind, if (search.isEmpty()) "trending" else "search").apply {
            if (search.isNotEmpty()) addQueryParameter("q", search)
            addQueryParameter("page", page.coerceAtLeast(1).toString())
            addQueryParameter("per_page", "24")
            addQueryParameter("customer_id", config.customerId)
            addQueryParameter("content_filter", "off")
            locale?.lowercase(java.util.Locale.ROOT)?.takeIf { Regex("^[a-z]{2}$").matches(it) }?.let { addQueryParameter("locale", it) }
            ads?.appendTo(this)
        }.build()
        val request = Request.Builder().url(url).get().apply { userAgent?.let { header("User-Agent", it) } }.build()
        return client.newCall(request).awaitDecoded { response ->
            if (!response.isSuccessful) throw IOException("KLIPY returned ${response.code}.")
            parseKlipyPage(json.parseToJsonElement(response.body.string()), kind)
        }
    }

    /** After a successful send; best effort, so failures are swallowed. [query] is empty for trending. */
    suspend fun share(config: KlipyConfig, kind: KlipyKind, slug: String, query: String, userAgent: String?) {
        if (!klipySlug(slug)) return
        runCatching {
            val body = buildJsonObject { put("customer_id", config.customerId); put("q", query.trim()) }.toString()
            val request = Request.Builder().url(url(config, kind, "share", slug).build())
                .post(body.toRequestBody("application/json".toMediaType()))
                .apply { userAgent?.let { header("User-Agent", it) } }.build()
            client.newCall(request).awaitDecoded { }
        }.onFailure { if (it is kotlinx.coroutines.CancellationException) throw it }
    }
}
