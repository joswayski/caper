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
