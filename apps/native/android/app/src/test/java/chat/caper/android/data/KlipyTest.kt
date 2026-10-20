package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.KlipyConfig
import chat.caper.android.model.KlipyGif
import chat.caper.android.model.klipyGifOf
import chat.caper.android.model.klipyMediaUrl
import java.io.File
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.OkHttpClient
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.After
import org.junit.Test

class KlipyTest {
    private val server = MockWebServer()

    @After fun close() { server.close() }

    /** Every client and the API run these same cases; see shared/klipy/cases.json. */
    private val cases: JsonObject by lazy {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/klipy/cases.json") }.first { it.isFile }
        Json.parseToJsonElement(file.readText()).jsonObject
    }

    private fun gifJson(gif: KlipyGif): JsonObject = gif.toJson()

    private fun tileJson(tile: KlipyTile) = buildJsonObject {
        put("url", tile.url); tile.still?.let { put("still", it) }; put("width", tile.width); put("height", tile.height)
    }

    /** The shared vector's shape: absent fields stay absent, except an ad's unknown size (null). */
    private fun itemJson(item: KlipyItem): JsonObject = when (item) {
        is KlipyItem.Ad -> buildJsonObject {
            put("type", "ad"); put("html", item.html); put("width", item.width); put("height", item.height)
        }
        is KlipyItem.Result -> buildJsonObject {
            put("type", "item"); put("slug", item.slug); put("title", item.title)
            put("tile", tileJson(item.tile)); put("gif", gifJson(item.gif))
        }
    }

    private fun checkParse(name: String, kind: KlipyKind) {
        val case = cases.getValue(name).jsonObject
        assertEquals(kind.kind, case.getValue("kind").jsonPrimitive.content)
        val page = parseKlipyPage(case.getValue("response"), kind)
        assertEquals(name, case.getValue("items").jsonArray.toList(), page.items.map(::itemJson))
        assertEquals(name, case.getValue("hasNext").jsonPrimitive.boolean, page.hasNext)
    }

    @Test fun `shared parse cases keep KLIPY's order, ads and fallbacks`() = checkParse("parse", KlipyKind.GIFS)

    @Test fun `shared sticker parse case`() = checkParse("parseSticker", KlipyKind.STICKERS)

    @Test fun `shared message cases render only valid gifs`() {
        val messages = cases.getValue("messages").jsonObject
        val valid = messages.getValue("valid").jsonArray
        val invalid = messages.getValue("invalid").jsonArray
        assertTrue(valid.size >= 4 && invalid.size >= 10)
        valid.forEach { element ->
            val gif = requireNotNull(klipyGifOf(element)) { element.toString() }
            // Rendered exactly as stored, and sent back in the same shape.
            assertEquals(element, gifJson(gif))
        }
        invalid.forEach { element -> assertNull(element.toString(), klipyGifOf(element)) }
    }

    @Test fun `shared url cases follow the API's media host rule`() {
        val urls = cases.getValue("urls").jsonArray
        assertTrue(urls.size >= 10)
        urls.forEach { case ->
            val fields = case.jsonObject
            val url = fields.getValue("url").jsonPrimitive.content
            assertEquals(url.take(80), fields.getValue("valid").jsonPrimitive.boolean, klipyMediaUrl(url))
        }
    }

    private fun message(gif: JsonElement?): String = buildJsonObject {
        put("id", "AAAAAAAAAAAAAAA"); put("channelId", "BBBBBBBBBBBB"); put("seq", "1")
        putJsonObject("author") { put("id", "a"); put("name", "Ada"); put("isGuest", false) }
        putJsonObject("content") {
            put("version", 1); put("type", "text"); put("text", "caption")
            if (gif != null) put("gif", gif)
        }
        put("createdAt", "2026-10-20T00:00:00Z"); put("clientMessageId", "00000000-0000-0000-0000-000000000000")
    }.toString()

    @Test fun `an invalid gif leaves the message with its text alone`() {
        val json = Json { ignoreUnknownKeys = true }
        val messages = cases.getValue("messages").jsonObject
        val valid = json.decodeFromString<ChatMessage>(message(messages.getValue("valid").jsonArray.first()))
        assertEquals("hello-hi-662", valid.content.gif?.slug)
        assertEquals("GIF", valid.content.gif?.copy(title = "")?.label)
        messages.getValue("invalid").jsonArray.forEach { element ->
            val decoded = json.decodeFromString<ChatMessage>(message(element))
            assertNull(decoded.content.gif)
            assertEquals("caption", decoded.content.text)
        }
        assertNull(json.decodeFromString<ChatMessage>(message(JsonNull)).content.gif)
        assertNull(json.decodeFromString<ChatMessage>(message(null)).content.gif)
    }

    @Test fun `titles lose control characters before the 200-character cut`() {
        assertEquals("ab", klipyCleanTitle("a\u0000b"))
        assertEquals("😀".repeat(200), klipyCleanTitle("😀".repeat(201)))
    }

    @Test fun `requests go straight to KLIPY with the documented parameters`() = runTest {
        run {
            server.enqueue(MockResponse().setBody(cases.getValue("parse").jsonObject.getValue("response").toString()))
            server.enqueue(MockResponse().setBody("{}"))
            val client = KlipyClient(OkHttpClient(), server.url("/api/v1/"))
            val config = KlipyConfig("AppKey123", "0123456789abcdef0123456789abcdef")
            val ads = KlipyAdRequest(360, "16", "Google", "Pixel 9", 1080, 2400, 2.625f, "en")
            val page = client.page(config, KlipyKind.STICKERS, " cats ", 2, "US", ads, "Mozilla/5.0 (Linux; Android 16) Mobile")
            assertTrue(page.hasNext)
            val search = server.takeRequest()
            val query = requireNotNull(search.requestUrl)
            assertEquals("/api/v1/AppKey123/stickers/search", query.encodedPath)
            assertEquals("Mozilla/5.0 (Linux; Android 16) Mobile", search.headers["User-Agent"])
            assertNull(search.headers["Authorization"])
            assertEquals("cats", query.queryParameter("q"))
            assertEquals("2", query.queryParameter("page"))
            assertEquals("24", query.queryParameter("per_page"))
            assertEquals(config.customerId, query.queryParameter("customer_id"))
            assertEquals("off", query.queryParameter("content_filter"))
            assertEquals("us", query.queryParameter("locale"))
            assertEquals("50", query.queryParameter("ad-min-width"))
            assertEquals("360", query.queryParameter("ad-max-width"))
            assertEquals("50", query.queryParameter("ad-min-height"))
            assertEquals("250", query.queryParameter("ad-max-height"))
            assertEquals("android", query.queryParameter("ad-os"))
            assertEquals("Pixel 9", query.queryParameter("ad-model"))
            assertEquals("2.63", query.queryParameter("ad-pxratio"))
            assertEquals("en", query.queryParameter("ad-language"))

            client.share(config, KlipyKind.GIFS, "hello-hi-662", "cats", null)
            val share = server.takeRequest()
            assertEquals("POST", share.method)
            assertEquals("/api/v1/AppKey123/gifs/share/hello-hi-662", share.requestUrl?.encodedPath)
            assertEquals(buildJsonObject { put("customer_id", config.customerId); put("q", "cats") }, Json.parseToJsonElement(share.body.readUtf8()))
        }
    }

    @Test fun `trending omits the search and a failed share stays quiet`() = runTest {
        run {
            server.enqueue(MockResponse().setBody("""{"result":true,"data":{"data":[],"has_next":false}}"""))
            server.enqueue(MockResponse().setResponseCode(500))
            val client = KlipyClient(OkHttpClient(), server.url("/api/v1/"))
            val config = KlipyConfig("AppKey123", "0123456789abcdef0123456789abcdef")
            val page = client.page(config, KlipyKind.GIFS, "  ", 1, "", null, null)
            assertFalse(page.hasNext)
            val trending = requireNotNull(server.takeRequest().requestUrl)
            assertEquals("/api/v1/AppKey123/gifs/trending", trending.encodedPath)
            assertNull(trending.queryParameter("q"))
            assertNull(trending.queryParameter("locale"))
            assertNull(trending.queryParameter("ad-os"))
            client.share(config, KlipyKind.GIFS, "hello-hi-662", "", null)
        }
    }

    @Test fun `config needs a letters-and-digits key and a 32-hex customer id`() {
        assertTrue(KlipyConfig("abc123", "0123456789abcdef0123456789abcdef").valid)
        assertFalse(KlipyConfig("abc/123", "0123456789abcdef0123456789abcdef").valid)
        assertFalse(KlipyConfig("abc123", "xyz").valid)
    }
}
