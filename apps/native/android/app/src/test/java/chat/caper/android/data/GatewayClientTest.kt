package chat.caper.android.data

import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import chat.caper.android.model.GatewayStatus
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

class GatewayClientTest {
    @Test fun `reaction protocol validates shape channel and sequence`() {
        fun event(seq: String = "2", channel: String = "channel") = Json.parseToJsonElement(
            """{"type":"message.reactions","schemaVersion":1,"channelId":"$channel","seq":"$seq","messageId":"message","reactions":[{"emoji":"👍","authorIds":["author"]}]}""",
        ).jsonObject
        assertEquals("2", reactionSequence(event(), "channel"))
        assertEquals("duplicate frames remain valid", "2", reactionSequence(event(), "channel"))
        assertEquals("gap handling uses the parsed sequence", "4", reactionSequence(event("4"), "channel"))
        assertThrows(IllegalArgumentException::class.java) { reactionSequence(event(channel = "other"), "channel") }
        assertThrows(IllegalArgumentException::class.java) { reactionSequence(event("02"), "channel") }
        assertThrows(IllegalArgumentException::class.java) {
            reactionSequence(Json.parseToJsonElement("""{"type":"message.reactions","schemaVersion":1,"channelId":"channel","seq":"2","messageId":"message","reactions":[{"emoji":"👍","authorIds":["author","author"]}]}""").jsonObject, "channel")
        }
        assertThrows(IllegalArgumentException::class.java) {
            reactionSequence(Json.parseToJsonElement("""{"type":"message.reactions","schemaVersion":2,"channelId":"channel","seq":"2","messageId":"message","reactions":[]}""").jsonObject, "channel")
        }
    }

    @Test fun `reaction followed by message advances stream exactly once`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(4)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { opened.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""") }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
        }))
        val deliveries = ArrayBlockingQueue<String>(4)
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), null, "channel", "0",
            onMessage = { deliveries.add("message:${it.seq}") }, onReaction = { deliveries.add("reaction:${it.seq}") },
            onAccessDenied = {}, onResync = {})
        var socket: WebSocket? = null
        try {
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val subscription = Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject
            val id = subscription.getValue("id").jsonPrimitive.content
            socket.send("""{"type":"event","id":"$id","event":{"type":"message.reactions","schemaVersion":1,"channelId":"channel","seq":"1","messageId":"message","reactions":[{"emoji":"👍","authorIds":["author"]}]}}""")
            socket.send("""{"type":"event","id":"$id","event":{"type":"message.created","seq":"2","message":{"id":"message00000002","channelId":"channel","seq":"2","author":{"id":"author","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"hi"},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}}}""")
            assertEquals("reaction:1", deliveries.poll(2, TimeUnit.SECONDS))
            assertEquals("message:2", deliveries.poll(2, TimeUnit.SECONDS))
            assertNull(deliveries.poll(250, TimeUnit.MILLISECONDS))
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }

    @Test fun `mixed handoffs wait for replay suppress duplicates and retain live state`() {
        val server = MockWebServer()
        val sockets = ArrayBlockingQueue<WebSocket>(4)
        val closed = ArrayBlockingQueue<WebSocket>(4)
        val frames = ArrayBlockingQueue<String>(16)
        repeat(3) {
            server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    sockets.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""")
                }
                override fun onMessage(webSocket: WebSocket, text: String) { frames.add(text) }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    closed.add(webSocket); webSocket.close(code, reason)
                }
            }))
        }
        val messages = ArrayBlockingQueue<String>(4)
        val reactions = ArrayBlockingQueue<String>(4)
        val rosters = ArrayBlockingQueue<Pair<String, Long?>>(4)
        val resets = AtomicInteger()
        val gateway = GatewayClient(
            server.url("/").toString().trimEnd('/'), "account-secret", "chat00000001", "7", { messages.add(it.seq) },
            onReaction = { reactions.add(it.seq) },
            onMedia = { _, people, startedAt -> rosters.add(people.single().name to startedAt) }, onMediaDisconnected = { resets.incrementAndGet() },
            onAccessDenied = { fail("handoff is not revocation") }, onResync = { fail("handoff must not resync") },
        )
        fun subscriptions(after: String): Pair<String, String> {
            val parsed = (1..2).map { Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS)!!).jsonObject }
            val chat = parsed.single { it["kind"]?.jsonPrimitive?.content == "chat" }
            val media = parsed.single { it["kind"]?.jsonPrimitive?.content == "media" }
            assertEquals(after, chat["after"]?.jsonPrimitive?.content)
            assertFalse(media.containsKey("token"))
            return chat.getValue("id").jsonPrimitive.content to media.getValue("id").jsonPrimitive.content
        }
        fun WebSocket.ready(chat: String, media: String, cursor: String) {
            send("""{"type":"subscribed","id":"$chat"}"""); send("""{"type":"subscribed","id":"$media"}""")
            send("""{"type":"event","id":"$chat","event":{"type":"ready","cursor":"$cursor"}}""")
        }
        fun WebSocket.roster(id: String, revision: Int) = send("""{"type":"event","id":"$id","event":{"type":"snapshot","revision":$revision,"sessionStartedAt":${revision * 1000},"participants":[{"id":"speaker","name":"revision-$revision","muted":false,"deafened":false}]}}""")
        fun WebSocket.message(id: String, seq: String) = send("""{"type":"event","id":"$id","event":{"type":"message.created","seq":"$seq","message":{"id":"message-$seq","channelId":"chat00000001","seq":"$seq","clientMessageId":"00000000-0000-4000-8000-00000000000$seq","createdAt":"2026-10-03T00:00:00Z","author":{"id":"author","name":"Author","isGuest":false},"content":{"version":1,"type":"text","text":"message $seq"}}}}""")
        fun WebSocket.reaction(id: String, seq: String) = send("""{"type":"event","id":"$id","event":{"type":"message.reactions","schemaVersion":1,"channelId":"chat00000001","seq":"$seq","messageId":"message-7","reactions":[{"emoji":"👍","authorIds":["author"]}]}}""")
        try {
            gateway.watchMedia(listOf("voice000001"), false); gateway.start()
            val old = sockets.poll(2, TimeUnit.SECONDS)!!; val (chat, media) = subscriptions("7")
            old.ready(chat, media, "7"); old.roster(media, 5)
            assertEquals("revision-5" to 5000L, rosters.poll(2, TimeUnit.SECONDS))
            old.send("""{"type":"migrating"}""")
            val candidate = sockets.poll(2, TimeUnit.SECONDS)!!; assertEquals(chat to media, subscriptions("7"))
            candidate.ready(chat, media, "7"); candidate.roster(media, 4)
            assertNull(closed.poll(250, TimeUnit.MILLISECONDS))
            old.reaction(chat, "8"); old.roster(media, 6)
            assertEquals("8", reactions.poll(2, TimeUnit.SECONDS)); assertEquals("revision-6" to 6000L, rosters.poll(2, TimeUnit.SECONDS))
            candidate.roster(media, 6)
            assertNull("media alone cannot promote a chat stream still behind", closed.poll(250, TimeUnit.MILLISECONDS))
            candidate.reaction(chat, "8")
            assertSame("last replay reaction must re-evaluate promotion", old, closed.poll(2, TimeUnit.SECONDS))
            assertNull("replay must not deliver the same reaction twice", reactions.poll(250, TimeUnit.MILLISECONDS))
            assertNull("equal media revision is deduplicated", rosters.poll(250, TimeUnit.MILLISECONDS))
            assertEquals(GatewayStatus.LIVE, gateway.status.value); assertEquals(0, resets.get())
            candidate.send("""{"type":"migrating"}""")
            val last = sockets.poll(2, TimeUnit.SECONDS)!!; assertEquals(chat to media, subscriptions("8"))
            last.ready(chat, media, "8"); last.roster(media, 6)
            assertSame(candidate, closed.poll(2, TimeUnit.SECONDS))
            last.message(chat, "9"); assertEquals("9", messages.poll(2, TimeUnit.SECONDS))
            assertEquals(GatewayStatus.LIVE, gateway.status.value); assertEquals(0, resets.get())
        } finally {
            gateway.close(); server.close()
        }
    }

    @Test fun `candidate waits for fresh presence while old stream continues delivery`() {
        val server = MockWebServer()
        val sockets = ArrayBlockingQueue<WebSocket>(2)
        val frames = ArrayBlockingQueue<String>(8)
        repeat(2) {
            server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    sockets.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""")
                }
                override fun onMessage(webSocket: WebSocket, text: String) { frames.add(text) }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
            }))
        }
        val presence = ArrayBlockingQueue<String>(4)
        val gateway = GatewayClient(
            server.url("/").toString().trimEnd('/'), "account-secret", "chat00000001", "0", {},
            onPresence = { presence.add(it.members.single().status) }, onAccessDenied = {}, onResync = {},
        )
        fun subscriptions(): Pair<String, String> {
            val parsed = (1..2).map { Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS)!!).jsonObject }
            return parsed.single { it["kind"]?.jsonPrimitive?.content == "chat" }.getValue("id").jsonPrimitive.content to
                parsed.single { it["kind"]?.jsonPrimitive?.content == "presence" }.getValue("id").jsonPrimitive.content
        }
        fun WebSocket.ready(chat: String, presenceId: String) {
            send("""{"type":"subscribed","id":"$chat"}"""); send("""{"type":"subscribed","id":"$presenceId"}""")
            send("""{"type":"event","id":"$chat","event":{"type":"ready","cursor":"0"}}""")
        }
        fun WebSocket.presence(id: String, status: String) = send("""{"type":"event","id":"$id","event":{"type":"snapshot","members":[{"userId":"person","status":"$status"}]}}""")
        try {
            gateway.watchPresence("space000001", listOf("person")); gateway.start()
            val old = sockets.poll(2, TimeUnit.SECONDS)!!; val (chat, presenceId) = subscriptions(); old.ready(chat, presenceId)
            old.presence(presenceId, "online"); assertEquals("online", presence.poll(2, TimeUnit.SECONDS))
            old.send("""{"type":"migrating"}""")
            val candidate = sockets.poll(2, TimeUnit.SECONDS)!!; val candidateIds = subscriptions()
            assertEquals(chat to presenceId, candidateIds); candidate.ready(chat, presenceId)
            old.presence(presenceId, "idle")
            assertEquals("candidate must not replace old before its presence snapshot", "idle", presence.poll(2, TimeUnit.SECONDS))
            candidate.presence(presenceId, "online")
            assertEquals("online", presence.poll(2, TimeUnit.SECONDS))
        } finally {
            gateway.close(); server.close()
        }
    }

    @Test fun `watch registered on open socket waits for hello and subscribes only once`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(8)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { opened.add(webSocket) }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
        }))
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), "account-secret", "chat00000001", "0", {},
            onAccessDenied = {}, onResync = {})
        var socket: WebSocket? = null
        try {
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            gateway.watchMedia(listOf("voice000001"), false)
            assertNull("no media subscription before hello", incoming.poll(300, TimeUnit.MILLISECONDS))
            socket.send("""{"type":"hello","serverTime":1}""")
            val first = Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject
            val second = Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject
            assertEquals(setOf("chat", "media"), listOf(first, second).map { it["kind"]?.jsonPrimitive?.content }.toSet())
            assertNull("hello must not duplicate the media subscription", incoming.poll(300, TimeUnit.MILLISECONDS))
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }

    @Test fun `spectator watch is bounded and demo subscription omits channel`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(4)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.add(webSocket)
                webSocket.send("""{"type":"hello","serverTime":1}""")
            }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
        }))
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), null, "general", "0", {}, onAccessDenied = {}, onResync = {})
        var socket: WebSocket? = null
        try {
            assertThrows(IllegalArgumentException::class.java) { gateway.watchMedia((1..25).map { "voice$it" }, false) }
            gateway.watchMedia(listOf("general"), true)
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val frames = (1..2).map { Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS)!!).jsonObject }
            val media = frames.single { it["kind"]?.jsonPrimitive?.content == "media" }
            assertFalse(media.containsKey("channelId"))
            assertFalse(media.containsKey("token"))
            assertNull(server.takeRequest().headers["Authorization"])
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }

    @Test fun `media rosters multiplex without capability and deny only their channel`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(32)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.add(webSocket)
                webSocket.send("""{"type":"hello","serverTime":1}""")
            }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
        }))
        val rosters = ArrayBlockingQueue<Triple<String, Int, Long?>>(8)
        val denied = ArrayBlockingQueue<String>(2)
        val gateway = GatewayClient(
            baseUrl = server.url("/").toString().trimEnd('/'), token = "account-secret",
            channelId = "chat00000001", initialCursor = "0", onMessage = {},
            onMedia = { channel, people, startedAt -> rosters.add(Triple(channel, people.size, startedAt)) },
            onMediaDenied = { denied.add(it) }, onAccessDenied = { fail("chat must remain accessible") }, onResync = {},
        )
        var socket: WebSocket? = null
        try {
            gateway.watchMedia(listOf("voice000001", "voice000002"), false)
            gateway.start()
            socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val frames = (1..3).map {
                Json.parseToJsonElement(incoming.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("missing subscription")).jsonObject
            }
            val media = frames.filter { it["kind"]?.jsonPrimitive?.content == "media" }
            assertEquals(2, media.size)
            assertTrue(frames.none { it.containsKey("token") })
            val first = media.first { it["channelId"]?.jsonPrimitive?.content == "voice000001" }.getValue("id").jsonPrimitive.content
            val second = media.first { it["channelId"]?.jsonPrimitive?.content == "voice000002" }.getValue("id").jsonPrimitive.content
            val participant = """{"id":"person","name":"One","muted":false,"deafened":false}"""
            socket.send("""{"type":"event","id":"$first","event":{"type":"snapshot","revision":2,"sessionStartedAt":1000,"participants":[$participant]}}""")
            assertEquals(Triple("voice000001", 1, 1000L), rosters.poll(2, TimeUnit.SECONDS))
            socket.send("""{"type":"event","id":"$first","event":{"type":"snapshot","revision":1,"sessionStartedAt":2000,"participants":[]}}""")
            assertNull(rosters.poll(250, TimeUnit.MILLISECONDS))
            socket.send("""{"type":"error","id":"$second","status":403,"error":"denied"}""")
            assertEquals(Triple("voice000002", 0, null), rosters.poll(2, TimeUnit.SECONDS))
            assertEquals("voice000002", denied.poll(2, TimeUnit.SECONDS))
            socket.send("""{"type":"event","id":"$second","event":{"type":"snapshot","revision":3,"participants":[$participant]}}""")
            assertNull(rosters.poll(250, TimeUnit.MILLISECONDS))
            assertEquals("Bearer account-secret", server.takeRequest().headers["Authorization"])
        } finally {
            socket?.close(1000, "done")
            gateway.close()
            server.close()
        }
    }

    @Test fun `server heartbeat is watchdog input not an immediate echo and resync is terminal`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(4)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.add(webSocket)
                webSocket.send("""{"type":"hello","idleTimeoutSeconds":600,"serverTime":1}""")
            }
            override fun onMessage(webSocket: WebSocket, text: String) { incoming.add(text) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
        }))
        val resync = CountDownLatch(1)
        val gateway = GatewayClient(
            baseUrl = server.url("/").toString().trimEnd('/'), token = "account-secret",
            channelId = "channel00001", initialCursor = "7", onMessage = {}, onAccessDenied = {},
            onResync = { resync.countDown() },
        )
        try {
            gateway.start()
            val socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val subscription = Json.parseToJsonElement(
                incoming.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("subscription not sent"),
            ).jsonObject
            assertEquals("subscribe", subscription["type"]?.jsonPrimitive?.content)
            assertEquals("7", subscription["after"]?.jsonPrimitive?.content)
            val id = subscription.getValue("id").jsonPrimitive.content
            socket.send("""{"type":"heartbeat"}""")
            assertNull("client must wait for its own 10s cadence", incoming.poll(400, TimeUnit.MILLISECONDS))
            socket.send("""{"type":"event","id":"$id","event":{"type":"resync_required"}}""")
            assertTrue(resync.await(2, TimeUnit.SECONDS))
            val handshake = server.takeRequest(2, TimeUnit.SECONDS)!!
            assertEquals("Bearer account-secret", handshake.headers["Authorization"])
            assertNull(handshake.requestUrl?.query)
        } finally {
            gateway.close()
            server.close()
        }
    }
}
