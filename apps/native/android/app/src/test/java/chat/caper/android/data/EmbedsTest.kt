package chat.caper.android.data

import chat.caper.android.embedFooterDate
import chat.caper.android.model.*
import java.io.File
import java.time.ZoneOffset
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

class EmbedsTest {
    private data class Case(val note: String, val embed: kotlinx.serialization.json.JsonElement, val blocks: List<String>?, val imageCounts: List<Int>?)

    /** Every client runs these same cases; see shared/messages/embed-cases.json. */
    private val cases: List<Case> by lazy {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/embed-cases.json") }.first { it.isFile }
        Json.parseToJsonElement(file.readText()).jsonObject.getValue("cases").jsonArray.map { case ->
            val fields = case.jsonObject
            Case(
                fields.getValue("note").jsonPrimitive.content,
                fields.getValue("embed"),
                fields.getValue("blocks").takeIf { it != JsonNull }?.jsonArray?.map { it.jsonPrimitive.content },
                fields["imageCounts"]?.jsonArray?.map { it.jsonPrimitive.int },
            )
        }
    }

    private fun EmbedBlock.type() = when (this) {
        is EmbedBlock.Author -> "author"
        is EmbedBlock.Title -> "title"
        is EmbedBlock.Text -> "text"
        is EmbedBlock.Images -> "images"
        is EmbedBlock.Video -> "video"
        is EmbedBlock.Player -> "player"
        is EmbedBlock.Footer -> "footer"
    }

    private val card = ChatEmbed(1, "https://x.com/a", EmbedSite("X", "x.com"), listOf(EmbedBlock.Text("a")))
    private val other = ChatEmbed(1, "https://x.com/b", EmbedSite("X", "x.com"), listOf(EmbedBlock.Text("b")))
    private val message = ChatMessage(
        "message00000001", "channel00001", "3", ChatAuthor("author", "A", false),
        ChatContent(1, "text", "https://x.com/a"), "2026-01-01T00:00:00Z", "client",
    )
    private fun update(revision: Int = 1, vararg embeds: ChatEmbed, messageId: String = message.id) =
        EmbedsUpdate("channel00001", messageId, revision, embeds.toList())

    @Test fun `shared embed cases keep the same blocks on every client`() {
        assertTrue("expected the full shared case list", cases.size >= 8)
        cases.forEach { case ->
            val parsed = embedOf(case.embed)
            assertEquals(case.note, case.blocks, parsed?.blocks?.map { it.type() })
            case.imageCounts?.let { counts ->
                assertEquals(case.note, counts, parsed!!.blocks.filterIsInstance<EmbedBlock.Images>().map { it.items.size })
            }
        }
    }

    @Test fun `parsed cards keep their fields and only http(s) links`() {
        val youtube = embedOf(cases.single { it.note.startsWith("YouTube") }.embed)!!
        val player = youtube.blocks.filterIsInstance<EmbedBlock.Player>().single()
        assertEquals("youtube", player.provider)
        assertEquals("https://embed.caper.chat/play/youtube/dQw4w9WgXcQ", player.play)
        assertEquals(1280.0, player.width)
        val generic = embedOf(cases.single { it.note.startsWith("generic") }.embed)!!
        assertTrue(generic.blocks.filterIsInstance<EmbedBlock.Images>().single().thumbnail)
        assertTrue(generic.blocks.filterIsInstance<EmbedBlock.Video>().single().gif)
        assertEquals("What happened.\nIn two lines.", generic.blocks.filterIsInstance<EmbedBlock.Text>().single().text)
        // At most four images; a scheme is matched ASCII case-insensitively, like web's /i.
        val images = (1..6).joinToString(",") { "{\"url\":\"https://i.example/$it\"}" }
        val many = embedOf(Json.parseToJsonElement("""{"version":1,"url":"HTTPS://a.b/c","site":{"name":"A","domain":"a.b"},"blocks":[{"type":"images","items":[$images]}]}"""))!!
        assertEquals(4, (many.blocks.single() as EmbedBlock.Images).items.size)
        assertNull(embedOf(Json.parseToJsonElement("""{"version":1,"url":"httpſ://a.b","site":{"name":"A","domain":"a.b"},"blocks":[{"type":"text","text":"a"}]}""")))
        assertNull("a JSON null is not an absent optional field", embedOf(Json.parseToJsonElement("""{"version":1,"url":"https://a.b","site":{"name":"A","domain":"a.b"},"blocks":[{"type":"author","name":"a","handle":null}]}""")))
        assertNull("a string version is not version 1", embedOf(Json.parseToJsonElement("""{"version":"1","url":"https://a.b","site":{"name":"A","domain":"a.b"},"blocks":[{"type":"text","text":"a"}]}""")))
    }

    @Test fun `malformed embeds never reject the message`() {
        val json = Json { ignoreUnknownKeys = true }
        fun decode(content: String) = json.decodeFromString(ChatMessage.serializer(),
            """{"id":"message00000001","channelId":"channel00001","seq":"3","author":{"id":"a","name":"A","isGuest":false},"content":$content,"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"c"}""").content
        val mixed = decode("""{"version":1,"type":"text","text":"hi","embeds":["junk",{"version":1,"url":"https://x.com/a","site":{"name":"X","domain":"x.com"},"blocks":[{"type":"text","text":"a"}]},{"version":2}],"suppressEmbeds":true}""")
        assertEquals(listOf(card), mixed.embeds)
        assertTrue(mixed.suppressEmbeds)
        val odd = decode("""{"version":1,"type":"text","text":"hi","embeds":{"not":"a list"},"suppressEmbeds":"yes"}""")
        assertEquals(emptyList<ChatEmbed>(), odd.embeds)
        assertFalse(odd.suppressEmbeds)
        assertEquals(emptyList<ChatEmbed>(), decode("""{"version":1,"type":"text","text":"hi"}""").embeds)
    }

    @Test fun `embeds events are validated and malformed ones dropped`() {
        fun event(body: String) = embedsUpdate(Json.parseToJsonElement(body).jsonObject, "channel00001")
        val cardJson = """{"version":1,"url":"https://x.com/a","site":{"name":"X","domain":"x.com"},"blocks":[{"type":"text","text":"a"}]}"""
        assertEquals(update(2, card), event("""{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"message00000001","revision":2,"embeds":[$cardJson,"bad"]}"""))
        assertEquals("an empty list removes the cards", update(1), event("""{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"message00000001","revision":1,"embeds":[]}"""))
        listOf(
            """{"type":"message.embeds","schemaVersion":2,"channelId":"channel00001","messageId":"m","revision":1,"embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"other0000001","messageId":"m","revision":1,"embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"","revision":1,"embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"m","revision":0,"embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"m","revision":"1","embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"m","revision":1.5,"embeds":[]}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"m","revision":1,"embeds":{}}""",
            """{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"m","revision":1}""",
        ).forEach { assertNull(it, event(it)) }
    }

    @Test fun `cards apply only to their revision of a live message`() {
        val applied = withEmbeds(message, update(1, card))
        assertEquals("revision defaults to 1", listOf(card), applied.content.embeds)
        assertSame("another revision is ignored", message, withEmbeds(message, update(2, card)))
        val edited = message.copy(revision = 2, editedAt = "2026-01-01T00:01:00Z", editSeq = "4")
        assertSame(edited, withEmbeds(edited, update(1, card)))
        assertEquals(listOf(card), withEmbeds(edited, update(2, card)).content.embeds)
        val deleted = message.copy(deletedAt = "2026-01-01T00:02:00Z")
        assertSame("a deleted message gets none", deleted, withEmbeds(deleted, update(1, card)))
        assertSame("another message is untouched", message, withEmbeds(message, update(1, card, messageId = "message00000002")))
        assertEquals("an empty list removes them", emptyList<ChatEmbed>(), withEmbeds(applied, update(1)).content.embeds)
    }

    @Test fun `cards for an unloaded message wait for it, bounded`() {
        val unseen = linkedMapOf<String, EmbedsUpdate>()
        cacheUnseenEmbeds(unseen, update(1, card))
        val merged = mergeMessages(emptyList(), listOf(message), mutableMapOf(), unseenEmbeds = unseen).single()
        assertEquals(listOf(card), merged.content.embeds)
        assertTrue("consumed once applied", unseen.isEmpty())
        // A stale revision is consumed without applying.
        cacheUnseenEmbeds(unseen, update(2, card))
        assertEquals(emptyList<ChatEmbed>(), mergeMessages(emptyList(), listOf(message), mutableMapOf(), unseenEmbeds = unseen).single().content.embeds)
        // The newest update per message wins; the oldest message is dropped when full.
        repeat(3) { cacheUnseenEmbeds(unseen, update(1, card, messageId = "m$it"), limit = 3) }
        cacheUnseenEmbeds(unseen, update(1, other, messageId = "m1"), limit = 3)
        cacheUnseenEmbeds(unseen, update(1, card, messageId = "m3"), limit = 3)
        assertEquals(listOf("m2", "m1", "m3"), unseen.keys.toList())
        assertEquals(listOf(other), unseen.getValue("m1").embeds)
    }

    @Test fun `a same-revision refetch fills in missing cards and never drops them`() {
        val live = withEmbeds(message, update(1, card))
        assertEquals("a replay without cards keeps them", listOf(card), mergeMessages(listOf(live), listOf(message), mutableMapOf()).single().content.embeds)
        assertEquals("a refetch carrying cards adds them", listOf(card), mergeMessages(listOf(message), listOf(live), mutableMapOf()).single().content.embeds)
        val refetched = withEmbeds(message, update(1, other))
        assertEquals("the refetch's cards win", listOf(other), mergeMessages(listOf(live), listOf(refetched), mutableMapOf()).single().content.embeds)
        // Newer reactions on the stale copy don't carry its missing cards over the live ones.
        val reacted = message.copy(reactions = listOf(MessageReaction("👍", listOf("a"))), reactionSeq = "9")
        val merged = mergeMessages(listOf(live), listOf(reacted), mutableMapOf()).single()
        assertEquals("9", merged.reactionSeq)
        assertEquals(listOf(card), merged.content.embeds)
        // An edit replaces the content: the older revision's cards don't follow it.
        val edited = message.copy(content = message.content.copy(text = "edited"), revision = 2, editedAt = "2026-01-01T00:01:00Z", editSeq = "4")
        assertEquals(emptyList<ChatEmbed>(), mergeMessages(listOf(live), listOf(edited), mutableMapOf()).single().content.embeds)
    }

    @Test fun `the composer previews the first link that will get a card`() {
        assertEquals("https://a.com", previewLink("look https://a.com and https://b.com"))
        assertEquals("written <like this>, a link gets no card", "https://b.com", previewLink("<https://a.com> then https://b.com"))
        assertNull(previewLink("<https://a.com>"))
        assertEquals("https://www.a.com", previewLink("www.a.com"))
        assertEquals("only one side bracketed still counts", "https://a.com", previewLink("<https://a.com"))
        assertNull(previewLink("no links here"))
        assertNull(previewLink(""))
    }

    @Test fun `player pages load only over https from Caper's embed hosts`() {
        assertEquals("https://embed.caper.chat/play/youtube/x", embedPlayUrl("https://embed.caper.chat/play/youtube/x"))
        assertNotNull(embedPlayUrl("https://embed.staging.caper.chat/play/vimeo/1"))
        assertNotNull(embedPlayUrl("https://EMBED.caper.chat/play/twitch/clip:a"))
        listOf(
            null, "", "http://embed.caper.chat/play/youtube/x", "https://caper.chat/play/youtube/x",
            "https://embed.caper.chat.evil.example/play", "https://evil.example/?embed.caper.chat",
            "https://user@embed.caper.chat/play/youtube/x", "https://embed.caper.chat:8443/play/youtube/x", "javascript:alert(1)",
        ).forEach { assertNull(it, embedPlayUrl(it)) }
        assertFalse(embedPlayerNavigation("https://www.youtube.com/watch?v=x"))
        assertTrue(embedPlayerNavigation("https://embed.caper.chat/play/youtube/y"))
    }

    @Test fun `media frames match web's fit`() {
        assertEquals(EmbedFrame(400, 16f / 9f), embedFrame(null, null))
        assertEquals(EmbedFrame(400, 16f / 9f), embedFrame(100.0, null))
        assertEquals(400, embedFrame(1280.0, 720.0).width)
        assertEquals("tall media keeps to 300 high", 150, embedFrame(500.0, 1000.0).width)
        assertEquals("small media is never enlarged", EmbedFrame(2, 1f), embedFrame(2.0, 2.0))
    }

    @Test fun `footer dates are medium dates and other strings are skipped`() {
        assertNotNull(embedFooterDate("2006-03-21T20:50:14.000Z", ZoneOffset.UTC))
        assertNotNull(embedFooterDate("2006-03-21T20:50:14+02:00", ZoneOffset.UTC))
        assertNull(embedFooterDate("yesterday", ZoneOffset.UTC))
        assertNull(embedFooterDate(null))
    }

    @Test fun `sending without cards adds suppressEmbeds and previews post the link`() = runTest {
        val server = MockWebServer()
        try {
            val body = """{"id":"message00001","channelId":"channel00001","seq":"8","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"https://a.com","suppressEmbeds":true},"createdAt":"2026-09-23T00:00:00Z","clientMessageId":"00000000-0000-0000-0000-000000000123"}"""
            server.enqueue(MockResponse().setBody(body)); server.enqueue(MockResponse().setBody(body))
            server.enqueue(MockResponse().setBody("""{"embed":{"version":1,"url":"https://a.com","site":{"name":"A","domain":"a.com"},"blocks":[{"type":"title","text":"A"}]}}"""))
            server.enqueue(MockResponse().setResponseCode(503).setBody("""{"error":"Link previews are unavailable."}"""))
            val api = CaperApi(baseUrl = server.url("/").toString())
            val id = UUID.fromString("00000000-0000-0000-0000-000000000123")
            val author = ChatAuthor("account00001", "Jose", false)
            assertTrue(api.sendMessage("account-secret", "chat-secret", "channel00001", author, id, "https://a.com", suppressEmbeds = true).content.suppressEmbeds)
            assertTrue(Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject.getValue("suppressEmbeds").jsonPrimitive.boolean)
            api.sendMessage("account-secret", "chat-secret", "channel00001", author, id, "https://a.com")
            assertFalse("unset, the body keeps its idempotency hash", "suppressEmbeds" in Json.parseToJsonElement(server.takeRequest().body.readUtf8()).jsonObject)
            assertEquals("A", (api.embedPreview("account-secret", "https://a.com")!!.blocks.single() as EmbedBlock.Title).text)
            val preview = server.takeRequest()
            assertEquals("/api/embeds/preview", preview.path)
            assertEquals("Bearer account-secret", preview.headers["Authorization"])
            assertEquals("https://a.com", Json.parseToJsonElement(preview.body.readUtf8()).jsonObject.getValue("url").jsonPrimitive.content)
            val unavailable = runCatching { api.embedPreview("account-secret", "https://a.com") }.exceptionOrNull()
            assertTrue("503 reaches the caller, which shows nothing", unavailable is ApiException && unavailable.status == 503)
        } finally { server.close() }
    }

    @Test fun `gateway asks for embeds and drops a malformed event without reconnecting`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(4)
        val opened = ArrayBlockingQueue<WebSocket>(2)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { opened.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""") }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
        }))
        val deliveries = ArrayBlockingQueue<String>(4)
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), null, "channel00001", "0",
            onMessage = { deliveries.add("message:${it.seq}") },
            onEmbeds = { deliveries.add("embeds:${it.messageId}:${it.revision}:${it.embeds.size}") },
            onAccessDenied = {}, onResync = { fail("embeds never resync") })
        var socket: WebSocket? = null
        try {
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val subscription = Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject
            assertTrue(subscription.getValue("embeds").jsonPrimitive.boolean)
            val id = subscription.getValue("id").jsonPrimitive.content
            socket.send("""{"type":"subscribed","id":"$id"}""")
            socket.send("""{"type":"event","id":"$id","event":{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"message00000001","revision":"x","embeds":[]}}""")
            socket.send("""{"type":"event","id":"$id","event":{"type":"message.embeds","schemaVersion":1,"channelId":"channel00001","messageId":"message00000001","revision":1,"embeds":[{"version":1,"url":"https://x.com/a","site":{"name":"X","domain":"x.com"},"blocks":[{"type":"text","text":"a"}]}]}}""")
            socket.send("""{"type":"event","id":"$id","event":{"type":"message.created","seq":"1","message":{"id":"message00000002","channelId":"channel00001","seq":"1","author":{"id":"author","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}}}""")
            assertEquals("embeds:message00000001:1:1", deliveries.poll(2, TimeUnit.SECONDS))
            assertEquals("the stream carries on", "message:1", deliveries.poll(2, TimeUnit.SECONDS))
            assertNull(deliveries.poll(250, TimeUnit.MILLISECONDS))
            assertNull("no second connection", opened.poll(250, TimeUnit.MILLISECONDS))
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }
}
