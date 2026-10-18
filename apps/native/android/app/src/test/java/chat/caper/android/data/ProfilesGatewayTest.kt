package chat.caper.android.data

import chat.caper.android.model.ProfileEvent
import chat.caper.android.model.ProfilePerson
import java.util.concurrent.ArrayBlockingQueue
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

class ProfilesGatewayTest {
    private fun event(text: String) = Json.parseToJsonElement(text).jsonObject

    @Test fun `profile and space events parse leniently, and anything else is ignored`() {
        val person = profileEvent(event("""{"type":"profile.updated","person":{"id":"maya00000001","username":"maya","displayName":"Maya","avatarId":312,
            "avatarAsset":"NewPicture0000000001","avatarUrl":"https://cdn.caper.chat/assets/x?sig=s","future":true}}"""))
        assertEquals(ProfileEvent.Person(ProfilePerson("maya00000001", "maya", "Maya", 312, "NewPicture0000000001", "https://cdn.caper.chat/assets/x?sig=s")), person)
        val removed = profileEvent(event("""{"type":"profile.updated","person":{"id":"maya00000001","username":"maya","displayName":"Maya","avatarId":312}}"""))
        assertNull((removed as ProfileEvent.Person).person.avatarAsset)
        val space = profileEvent(event("""{"type":"space.updated","space":{"id":"space0000001","name":"Friends","ownerId":"maya00000001","iconAsset":"Icon0000000000000001","iconUrl":"https://cdn.caper.chat/assets/i"}}"""))
        assertEquals("Friends", (space as ProfileEvent.SpaceChanged).space.name)
        assertEquals("Icon0000000000000001", space.space.iconAsset)
        assertNull(profileEvent(event("""{"type":"space.updated","space":{"id":"space0000001","name":"Friends"}}""")).let { (it as ProfileEvent.SpaceChanged).space.iconUrl })
        // Unknown, malformed or incomplete events are dropped, never thrown.
        assertNull(profileEvent(event("""{"type":"profile.deleted","person":{"id":"maya00000001"}}""")))
        // Android posts notifications only from push, which takes its own lines back, not from this feed.
        assertNull(profileEvent(event("""{"type":"notification.retracted","messageId":"message00000001","conversationId":"direct000001"}""")))
        assertNull(profileEvent(event("""{"type":"profile.updated"}""")))
        assertNull(profileEvent(event("""{"type":"profile.updated","person":{"id":""}}""")))
        assertNull(profileEvent(event("""{"type":"profile.updated","person":"maya"}""")))
        assertNull(profileEvent(event("""{"type":"space.updated","space":{"id":"space0000001"}}""")))
        assertNull(profileEvent(event("""{"type":{"nested":true}}""")))
    }

    @Test(timeout = 10000) fun `signed in, the socket subscribes to profiles, delivers live events, and survives an older gateway's refusal`() {
        val server = MockWebServer()
        val frames = ArrayBlockingQueue<String>(8)
        val opened = ArrayBlockingQueue<WebSocket>(2)
        repeat(2) {
            server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) { opened.add(webSocket); webSocket.send("""{"type":"hello","serverTime":1}""") }
                override fun onMessage(webSocket: WebSocket, text: String) { frames.add(text) }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
            }))
        }
        val profiles = ArrayBlockingQueue<ProfileEvent>(4)
        val messages = ArrayBlockingQueue<String>(4)
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), "account-secret", "channel", "0",
            onMessage = { messages.add(it.seq) }, onAccessDenied = { fail("a refused profiles kind is not revocation") }, onResync = { fail("no resync") },
            profiles = true, onProfile = { profiles.add(it) })
        try {
            gateway.start()
            val socket = opened.poll(2, TimeUnit.SECONDS)!!
            val sent = (1..2).map { Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS)!!).jsonObject }
            val chat = sent.single { it["kind"]?.jsonPrimitive?.content == "chat" }.getValue("id").jsonPrimitive.content
            assertEquals(Json.parseToJsonElement(PROFILES_SUBSCRIBE), sent.single { it["kind"]?.jsonPrimitive?.content == "profiles" })
            assertEquals("""{"type":"subscribe","id":"profiles","kind":"profiles"}""", PROFILES_SUBSCRIBE)
            socket.send("""{"type":"subscribed","id":"$chat"}""")
            socket.send("""{"type":"subscribed","id":"profiles"}""")
            socket.send("""{"type":"event","id":"profiles","event":{"type":"profile.updated","person":{"id":"maya00000001","username":"maya","displayName":"Maya B","avatarId":3}}}""")
            assertEquals("Maya B", (profiles.poll(2, TimeUnit.SECONDS) as ProfileEvent.Person).person.displayName)
            // A malformed one is dropped without failing the socket.
            socket.send("""{"type":"event","id":"profiles","event":{"type":"profile.updated","person":7}}""")
            socket.send("""{"type":"error","id":"profiles","status":400,"error":"unknown subscription kind"}""")
            socket.send("""{"type":"event","id":"$chat","event":{"type":"message.created","seq":"1","message":{"id":"message00000001","channelId":"channel","seq":"1","author":{"id":"author","name":"A","isGuest":false},"content":{"version":1,"type":"text","text":"still live"},"createdAt":"2026-01-01T00:00:00Z","clientMessageId":"00000000-0000-4000-8000-000000000001"}}}""")
            assertEquals("1", messages.poll(2, TimeUnit.SECONDS))
            assertNull(profiles.poll(200, TimeUnit.MILLISECONDS))
            assertNull("the socket stays open: no reconnect", opened.poll(500, TimeUnit.MILLISECONDS))

            // After a reconnect the refused kind is not asked for again.
            socket.close(1000, "restart")
            val again = opened.poll(5, TimeUnit.SECONDS)!!
            val resent = Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS)!!).jsonObject
            assertEquals("chat", resent["kind"]?.jsonPrimitive?.content)
            assertNull(frames.poll(300, TimeUnit.MILLISECONDS)?.takeIf { "profiles" in it })
            again.close(1000, "done")
        } finally {
            gateway.close()
            server.close()
        }
    }

    @Test(timeout = 10000) fun `signed out or not asked, no profiles subscription is sent`() {
        val server = MockWebServer()
        val frames = ArrayBlockingQueue<String>(8)
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) { webSocket.send("""{"type":"hello","serverTime":1}""") }
            override fun onMessage(webSocket: WebSocket, text: String) { frames.add(text) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, reason) }
        }))
        val gateway = GatewayClient(server.url("/").toString().trimEnd('/'), null, "channel", "0", onMessage = {},
            onAccessDenied = {}, onResync = {}, profiles = true)
        try {
            gateway.start()
            assertEquals("chat", Json.parseToJsonElement(frames.poll(2, TimeUnit.SECONDS)!!).jsonObject["kind"]?.jsonPrimitive?.content)
            assertNull(frames.poll(300, TimeUnit.MILLISECONDS)?.takeIf { "profiles" in it })
        } finally {
            gateway.close()
            server.close()
        }
    }
}
