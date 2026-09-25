package chat.caper.android.data

import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
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
        val rosters = ArrayBlockingQueue<Pair<String, Int>>(8)
        val denied = ArrayBlockingQueue<String>(2)
        val gateway = GatewayClient(
            baseUrl = server.url("/").toString().trimEnd('/'), token = "account-secret",
            channelId = "chat00000001", initialCursor = "0", onMessage = {},
            onMedia = { channel, people -> rosters.add(channel to people.size) },
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
            socket.send("""{"type":"event","id":"$first","event":{"type":"snapshot","revision":2,"participants":[$participant]}}""")
            assertEquals("voice000001" to 1, rosters.poll(2, TimeUnit.SECONDS))
            socket.send("""{"type":"event","id":"$first","event":{"type":"snapshot","revision":1,"participants":[]}}""")
            assertNull(rosters.poll(250, TimeUnit.MILLISECONDS))
            socket.send("""{"type":"error","id":"$second","status":403,"error":"denied"}""")
            assertEquals("voice000002" to 0, rosters.poll(2, TimeUnit.SECONDS))
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
