package chat.caper.android.data

import chat.caper.android.model.ProfileEvent
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.Json
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

class ProfileFeedTest {
    private class Gateway(count: Int) {
        val server = MockWebServer()
        val opened = ArrayBlockingQueue<WebSocket>(count)
        val frames = ArrayBlockingQueue<String>(16)
        init {
            repeat(count) {
                server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                    override fun onOpen(webSocket: WebSocket, response: Response) {
                        opened.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""")
                    }
                    override fun onMessage(webSocket: WebSocket, text: String) { frames.add(text) }
                    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
                }))
            }
        }
        val url get() = server.url("/").toString().trimEnd('/')
        fun socket() = opened.poll(5, TimeUnit.SECONDS) ?: throw AssertionError("socket did not open")
        fun frame() = Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS) ?: throw AssertionError("nothing sent"))
        /** The profiles subscription; the notifications one follows it on every socket. */
        fun subscription() = frame().also { assertEquals(Json.parseToJsonElement(NOTIFICATIONS_SUBSCRIBE), frame()) }
    }
    private fun person(name: String) =
        """{"type":"event","id":"profiles","event":{"type":"profile.updated","person":{"id":"maya00000001","username":"maya","displayName":"$name","avatarId":3}}}"""

    @Test(timeout = 15000) fun `the feed needs no channel, delivers live events and reconnects`() {
        val gateway = Gateway(2)
        val events = ArrayBlockingQueue<ProfileEvent>(4)
        val feed = ProfileFeedClient(gateway.url, "account-secret", onProfile = { events.add(it) })
        try {
            feed.start()
            val socket = gateway.socket()
            assertEquals("Bearer account-secret", gateway.server.takeRequest(2, TimeUnit.SECONDS)!!.getHeader("Authorization"))
            assertEquals("the profiles and notifications kinds, no chat subscription", Json.parseToJsonElement(PROFILES_SUBSCRIBE), gateway.subscription())
            socket.send("""{"type":"subscribed","id":"profiles"}""")
            socket.send(person("Maya B"))
            assertEquals("Maya B", (events.poll(2, TimeUnit.SECONDS) as ProfileEvent.Person).person.displayName)
            socket.send("""{"type":"event","id":"profiles","event":{"type":"space.updated","space":{"id":"space0000001","name":"Friends"}}}""")
            assertEquals("Friends", (events.poll(2, TimeUnit.SECONDS) as ProfileEvent.SpaceChanged).space.name)
            // Malformed or foreign events are dropped without failing the socket.
            socket.send("""{"type":"event","id":"profiles","event":{"type":"profile.updated","person":7}}""")
            socket.send("""{"type":"event","id":"other","event":{"type":"space.updated","space":{"id":"space0000001","name":"No"}}}""")
            assertNull(events.poll(300, TimeUnit.MILLISECONDS))

            // A dropped socket comes back on its own and subscribes again.
            socket.close(1000, "restart")
            val again = gateway.socket()
            assertEquals(Json.parseToJsonElement(PROFILES_SUBSCRIBE), gateway.subscription())
            again.send("""{"type":"subscribed","id":"profiles"}""")
            again.send(person("Maya C"))
            assertEquals("Maya C", (events.poll(2, TimeUnit.SECONDS) as ProfileEvent.Person).person.displayName)
        } finally {
            feed.close()
            gateway.server.close()
        }
    }

    @Test(timeout = 15000) fun `a migrating gateway hands off to a new socket`() {
        val gateway = Gateway(2)
        val events = ArrayBlockingQueue<ProfileEvent>(4)
        val feed = ProfileFeedClient(gateway.url, "account-secret", onProfile = { events.add(it) })
        try {
            feed.start()
            val old = gateway.socket(); gateway.subscription()
            old.send("""{"type":"subscribed","id":"profiles"}""")
            old.send("""{"type":"migrating"}""")
            val next = gateway.socket()
            assertEquals(Json.parseToJsonElement(PROFILES_SUBSCRIBE), gateway.subscription())
            next.send("""{"type":"subscribed","id":"profiles"}""")
            next.send(person("Maya D"))
            assertEquals("Maya D", (events.poll(2, TimeUnit.SECONDS) as ProfileEvent.Person).person.displayName)
        } finally {
            feed.close()
            gateway.server.close()
        }
    }

    @Test(timeout = 15000) fun `an older gateway's refusal stops the feed for good`() {
        val gateway = Gateway(2)
        val feed = ProfileFeedClient(gateway.url, "account-secret", onProfile = { fail("no events") })
        try {
            feed.start()
            val socket = gateway.socket(); gateway.subscription()
            socket.send("""{"type":"error","id":"profiles","status":400,"error":"unknown subscription kind"}""")
            assertNull("no retry loop", gateway.opened.poll(1500, TimeUnit.MILLISECONDS))
        } finally {
            feed.close()
            gateway.server.close()
        }
    }

    @Test(timeout = 15000) fun `live notifications refresh and report when they stop`() {
        val gateway = Gateway(2)
        val live = ArrayBlockingQueue<Boolean>(8)
        val notified = ArrayBlockingQueue<Unit>(4)
        val feed = ProfileFeedClient(gateway.url, "account-secret", onProfile = {}, onNotification = { notified.add(Unit) }, onNotificationsLive = { live.add(it) })
        try {
            feed.start()
            val socket = gateway.socket(); gateway.subscription()
            socket.send("""{"type":"subscribed","id":"profiles"}""")
            assertNull("profiles alone are not live notifications", live.poll(300, TimeUnit.MILLISECONDS))
            socket.send("""{"type":"subscribed","id":"notifications"}""")
            assertEquals(true, live.poll(2, TimeUnit.SECONDS))
            socket.send("""{"type":"event","id":"notifications","event":{"type":"notification","kind":"direct.message"}}""")
            assertNotNull(notified.poll(2, TimeUnit.SECONDS))

            // A drop reports it at once (timer polls speed up), and the reconnected socket goes live again.
            socket.close(1000, "restart")
            assertEquals(false, live.poll(2, TimeUnit.SECONDS))
            val again = gateway.socket(); gateway.subscription()
            again.send("""{"type":"subscribed","id":"profiles"}""")
            again.send("""{"type":"subscribed","id":"notifications"}""")
            assertEquals(true, live.poll(2, TimeUnit.SECONDS))
            // A gateway refusing the kind leaves notifications off but keeps the socket and its profiles.
            again.send("""{"type":"error","id":"notifications","status":400,"error":"unknown subscription kind"}""")
            assertEquals(false, live.poll(2, TimeUnit.SECONDS))
            assertNull("no reconnect", gateway.opened.poll(500, TimeUnit.MILLISECONDS))
        } finally {
            feed.close()
            gateway.server.close()
        }
    }
}
