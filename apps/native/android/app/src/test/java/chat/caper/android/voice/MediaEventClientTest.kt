package chat.caper.android.voice

import chat.caper.android.data.ApiException
import chat.caper.android.model.MediaSnapshot
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaEventClientTest {
    @Test fun `account bearer stays in header and media capability stays in subscription body`() {
        val server = MockWebServer()
        val incoming = ArrayBlockingQueue<String>(2)
        val opened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.add(webSocket)
                webSocket.send("""{"type":"hello","idleTimeoutSeconds":600,"serverTime":1}""")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                incoming.add(text)
            }
        }))
        val snapshots = ArrayBlockingQueue<MediaSnapshot>(1)
        val terminal = ArrayBlockingQueue<Throwable>(1)
        val now = AtomicLong(100)
        val events = MediaEventClient(
            baseUrl = server.url("/").toString().trimEnd('/'),
            accountToken = "account-secret",
            channelId = "channel00001",
            mediaToken = "media-capability",
            onSnapshot = snapshots::add,
            onTerminal = terminal::add,
            monotonicMs = now::get,
            heartbeatIntervalMs = 20,
        )
        try {
            events.start()
            val socket = opened.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
            val subscription = Json.parseToJsonElement(
                incoming.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("subscription not sent"),
            ).jsonObject
            assertEquals("subscribe", subscription["type"]?.jsonPrimitive?.content)
            assertEquals("media", subscription["kind"]?.jsonPrimitive?.content)
            assertEquals("media-capability", subscription["token"]?.jsonPrimitive?.content)
            assertEquals("channel00001", subscription["channelId"]?.jsonPrimitive?.content)
            now.set(460)
            val heartbeat = Json.parseToJsonElement(
                incoming.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("heartbeat not sent"),
            ).jsonObject
            assertEquals("heartbeat", heartbeat["type"]?.jsonPrimitive?.content)
            assertEquals("360", heartbeat["activityAgeMs"]?.jsonPrimitive?.content)

            val id = subscription.getValue("id").jsonPrimitive.content
            val request = server.takeRequest(2, TimeUnit.SECONDS) ?: throw AssertionError("missing handshake")
            assertEquals("Bearer account-secret", request.headers["Authorization"])
            assertNull(request.requestUrl?.query)

            socket.send(
                """{"type":"event","id":"$id","event":{"type":"snapshot","revision":9,"participants":[{"id":"self","name":"Fixture Owner","muted":true,"deafened":false,"tracks":[]}]}}""",
            )
            assertEquals(9L, snapshots.poll(2, TimeUnit.SECONDS)?.revision)
            socket.send("""{"type":"error","id":"$id","status":403,"code":"forbidden"}""")
            val error = terminal.poll(2, TimeUnit.SECONDS)
            assertTrue(error is ApiException && error.status == 403)
        } finally {
            events.close()
            server.close()
        }
    }

    @Test fun `socket that never receives hello is replaced by watchdog`() {
        val server = MockWebServer()
        val firstOpened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                firstOpened.add(webSocket)
            }
        }))
        val secondOpened = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                secondOpened.add(webSocket)
                webSocket.send("""{"type":"hello","idleTimeoutSeconds":600,"serverTime":1}""")
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }
        }))
        val events = MediaEventClient(
            baseUrl = server.url("/").toString().trimEnd('/'),
            accountToken = null,
            channelId = null,
            mediaToken = "media-capability",
            onSnapshot = {},
            onTerminal = {},
            helloTimeoutMs = 50,
        )
        try {
            events.start()
            assertTrue(firstOpened.poll(2, TimeUnit.SECONDS) != null)
            assertTrue("hello watchdog must reconnect a silent open socket", secondOpened.poll(2, TimeUnit.SECONDS) != null)
        } finally {
            events.close()
            server.close()
        }
    }

    @Test fun `reconnect replaces socket and only current subscription updates roster`() {
        val server = MockWebServer()
        val firstSocket = ArrayBlockingQueue<WebSocket>(1)
        val subscriptions = ArrayBlockingQueue<String>(2)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                firstSocket.add(webSocket)
                webSocket.send("""{"type":"hello","idleTimeoutSeconds":600,"serverTime":1}""")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                subscriptions.add(text)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }
        }))
        val secondSocket = ArrayBlockingQueue<WebSocket>(1)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                secondSocket.add(webSocket)
                webSocket.send("""{"type":"hello","idleTimeoutSeconds":600,"serverTime":1}""")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                subscriptions.add(text)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }
        }))
        val snapshots = ArrayBlockingQueue<MediaSnapshot>(1)
        val events = MediaEventClient(
            baseUrl = server.url("/").toString().trimEnd('/'),
            accountToken = null,
            channelId = null,
            mediaToken = "guest-media-capability",
            onSnapshot = snapshots::add,
            onTerminal = {},
        )
        try {
            events.start()
            val old = firstSocket.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("first socket did not open")
            val oldSubscription = Json.parseToJsonElement(
                subscriptions.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("first subscription not sent"),
            ).jsonObject.getValue("id").jsonPrimitive.content
            old.close(1012, "move")
            val current = secondSocket.poll(3, TimeUnit.SECONDS) ?: throw AssertionError("socket did not reconnect")
            val currentSubscription = Json.parseToJsonElement(
                subscriptions.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("second subscription not sent"),
            ).jsonObject.getValue("id").jsonPrimitive.content
            assertEquals(oldSubscription, currentSubscription)
            assertTrue(!old.send("""{"type":"event","id":"$oldSubscription","event":{"type":"snapshot","revision":1,"participants":[]}}"""))
            assertNull(snapshots.poll(400, TimeUnit.MILLISECONDS))
            current.send("""{"type":"event","id":"$currentSubscription","event":{"type":"snapshot","revision":2,"participants":[]}}""")
            assertEquals(2L, snapshots.poll(2, TimeUnit.SECONDS)?.revision)
        } finally {
            events.close()
            server.close()
        }
    }
}
