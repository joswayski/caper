package chat.caper.android.data

import chat.caper.android.model.TurnResponse
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.Space
import chat.caper.android.model.SpaceList
import chat.caper.android.model.SpaceDetail
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import okhttp3.Call
import okhttp3.EventListener
import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Test

class CaperApiTest {
    private val server = MockWebServer()

    @After fun close() { server.close() }

    @Test fun `logout sends an authenticated POST with an empty object`() = runTest {
        server.enqueue(MockResponse().setResponseCode(204))
        CaperApi(baseUrl = server.url("/").toString()).logout("account-secret")

        val request = server.takeRequest()
        assertEquals("POST", request.method)
        assertEquals("Bearer account-secret", request.headers["Authorization"])
        assertEquals("{}", request.body.readUtf8())
        assertNull(request.requestUrl?.query)
    }

    @Test fun `redirect is rejected without forwarding bearer credential`() = runTest {
        val target = MockWebServer()
        try {
            server.enqueue(MockResponse().setResponseCode(302).addHeader("Location", target.url("/stolen")))
            val error = runCatching { CaperApi(baseUrl = server.url("/").toString()).me("account-secret") }.exceptionOrNull()
            assertTrue(error is ApiException)
            assertEquals(302, (error as ApiException).status)
            assertEquals(0, target.requestCount)
            assertEquals("Bearer account-secret", server.takeRequest().headers["Authorization"])
        } finally { target.close() }
    }

    @Test fun `retry can reuse exact client message id and text while capabilities stay in headers`() = runTest {
        val body = """{"id":"message00001","channelId":"channel00001","seq":"8","author":{"id":"account00001","name":"Jose","isGuest":false},"content":{"version":1,"type":"text","text":"hello"},"createdAt":"2026-09-23T00:00:00Z","clientMessageId":"00000000-0000-0000-0000-000000000123"}"""
        server.enqueue(MockResponse().setBody(body)); server.enqueue(MockResponse().setBody(body))
        val api = CaperApi(baseUrl = server.url("/").toString())
        val id = UUID.fromString("00000000-0000-0000-0000-000000000123")
        repeat(2) { api.sendMessage("account-secret", "chat-secret", "channel00001", ChatAuthor("account00001", "Jose", false), id, "hello") }

        val first = server.takeRequest(); val second = server.takeRequest()
        assertEquals(first.body.readUtf8(), second.body.readUtf8())
        assertEquals("chat-secret", first.headers["x-caper-chat-token"])
        assertEquals("Bearer account-secret", first.headers["Authorization"])
        assertFalse(first.requestUrl.toString().contains("secret"))
    }

    @Test fun `turn urls accept provider string and array shapes`() {
        val response = Json.decodeFromString<TurnResponse>(
            """{"iceServers":[{"urls":"stun:one"},{"urls":["turn:one","turns:two"],"username":"u","credential":"p"}],"turn":{"generation":"g","refreshAfterMs":10,"expiresInMs":20}}""",
        )
        assertEquals(listOf("stun:one"), response.iceServers[0].urls)
        assertEquals(listOf("turn:one", "turns:two"), response.iceServers[1].urls)
    }

    @Test fun `reaction PUT validates message id and sends capability header`() = runTest {
        server.enqueue(MockResponse().setBody("""{"type":"message.reactions","schemaVersion":1,"channelId":"channel00001","seq":"9","messageId":"message00000001","reactions":[]}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        api.setReaction("account-secret", "chat-secret", "channel00001", "message00000001", "👍", true)
        val request = server.takeRequest()
        assertEquals("PUT", request.method)
        assertEquals("/api/chat/channels/channel00001/messages/message00000001/reactions", request.path)
        assertEquals("chat-secret", request.headers["x-caper-chat-token"])
        assertEquals("Bearer account-secret", request.headers["Authorization"])
        assertEquals("""{"emoji":"👍","active":true}""", request.body.readUtf8())
        assertThrows(IllegalArgumentException::class.java) { runBlocking { api.setReaction(null, "x", "channel00001", "short", "👍", true) } }
    }

    @Test fun `space list defaults invitations for old APIs`() {
        val response = Json.decodeFromString<SpaceList>(
            """{"spaces":[],"limits":{"ownedSpaces":20,"totalSpaces":100,"channelsPerSpace":100}}""",
        )
        assertTrue(response.invitations.isEmpty())
    }

    @Test fun `channel joining and invitation fields decode with legacy defaults`() {
        val legacy = Json.decodeFromString<SpaceDetail>(
            """{"space":{"id":"space0000001","name":"Studio"},"channels":[{"id":"channel00001","spaceId":"space0000001","name":"general","private":false}],"members":[]}""",
        )
        assertTrue(legacy.channels.single().joined)
        assertTrue(legacy.channelInvitations.isEmpty())
        val current = Json.decodeFromString<SpaceDetail>(
            """{"space":{"id":"space0000001","name":"Studio"},"channels":[{"id":"channel00001","spaceId":"space0000001","name":"general","private":false,"joined":false}],"members":[],"channelInvitations":[{"channel":{"id":"private00001","spaceId":"space0000001","name":"plans","private":true,"joined":false},"inviter":{"username":"host","displayName":"Host"}}]}""",
        )
        assertFalse(current.channels.single().joined)
        assertEquals("host", current.channelInvitations.single().inviter.username)
    }

    @Test fun `channel membership and invitation operations use consent routes`() = runTest {
        val joined = """{"id":"channel00001","spaceId":"space0000001","name":"general","private":false,"joined":true}"""
        server.enqueue(MockResponse().setBody(joined)); server.enqueue(MockResponse().setResponseCode(204))
        server.enqueue(MockResponse().setBody(joined)); server.enqueue(MockResponse().setResponseCode(204))
        val api = CaperApi(baseUrl = server.url("/").toString())
        api.joinChannel("token", "space0000001", "channel00001")
        api.leaveChannel("token", "space0000001", "channel00001")
        api.acceptChannelInvitation("token", "space0000001", "channel00001")
        api.declineChannelInvitation("token", "space0000001", "channel00001")
        val requests = List(4) { server.takeRequest() }
        assertEquals(listOf("POST", "DELETE", "POST", "DELETE"), requests.map { it.method })
        assertEquals("/api/spaces/space0000001/channels/channel00001/membership", requests[0].path)
        assertEquals("/api/spaces/space0000001/channels/channel00001/membership", requests[1].path)
        assertEquals("/api/spaces/space0000001/channels/channel00001/invitation", requests[2].path)
        assertEquals("/api/spaces/space0000001/channels/channel00001/invitation", requests[3].path)
    }

    @Test fun `space invitation decodes inviter while older metadata stays compatible`() {
        val legacy = """{"id":"space0000001","name":"Studio","ownerId":"owner0000001"}"""
        assertNull(Json.decodeFromString<Space>(legacy).inviter)
        val response = Json.decodeFromString<Space>(
            """{"id":"space0000001","name":"Studio","ownerId":"owner0000001","inviter":{"username":"host_user","displayName":"Space Host"}}""",
        )
        assertEquals("host_user", response.inviter?.username)
        assertEquals("Space Host", response.inviter?.displayName)
    }

    @Test fun `invitation operations use consent endpoints`() = runTest {
        server.enqueue(MockResponse().setBody("""{"id":"space0000001","name":"Studio","ownerId":"owner0000001"}"""))
        server.enqueue(MockResponse().setResponseCode(204))
        val api = CaperApi(baseUrl = server.url("/").toString())
        assertEquals("Studio", api.acceptSpaceInvitation("account-secret", "space0000001").name)
        api.declineSpaceInvitation("account-secret", "space0000001")

        val accept = server.takeRequest(); val decline = server.takeRequest()
        assertEquals("POST", accept.method)
        assertEquals("/api/spaces/space0000001/invitation", accept.path)
        assertEquals("DELETE", decline.method)
        assertEquals("/api/spaces/space0000001/invitation", decline.path)
        assertEquals("Bearer account-secret", decline.headers["Authorization"])
    }

    @Test(timeout = 10000) fun `slow response body does not block owner stop and cancellation`() = runBlocking {
        // Send the first byte promptly so responseBodyStart fires, then hold
        // back the rest of the body for four seconds.
        server.enqueue(MockResponse().setBody("""{"ready":true}""").throttleBody(1, 4, TimeUnit.SECONDS))
        val owner = Executors.newSingleThreadExecutor().asCoroutineDispatcher()
        try {
            val bodyStarted = CompletableDeferred<Unit>()
            val client = OkHttpClient.Builder().eventListener(object : EventListener() {
                override fun responseBodyStart(call: Call) { bodyStarted.complete(Unit) }
            }).build()
            val api = CaperApi(client = client, baseUrl = server.url("/").toString())
            val completed = CompletableDeferred<Unit>()
            val request = launch(owner) {
                api.get<JsonObject>("/api/account/me")
                completed.complete(Unit)
            }
            assertNotNull(server.takeRequest(2, TimeUnit.SECONDS))
            // This event fires when OkHttp begins consuming the response body,
            // not merely when the server receives a request or sends headers.
            withTimeout(2000) { bodyStarted.await() }
            withTimeout(1000) {
                val stopped = CompletableDeferred<Unit>()
                launch(owner) {
                    assertFalse("body must still be pending at stop", completed.isCompleted)
                    stopped.complete(Unit)
                    request.cancel()
                }
                stopped.await()
                request.join()
            }
            assertTrue(request.isCancelled)
            assertFalse(completed.isCompleted)
        } finally {
            owner.close()
        }
    }

    @Test fun `voice status reads the web media roots and sends the account token only to channels`() = runTest {
        server.enqueue(MockResponse().setBody("""{"enabled":true}"""))
        server.enqueue(MockResponse().setBody("""{"enabled":false}"""))
        val api = CaperApi(baseUrl = server.url("/").toString())
        assertTrue(api.mediaStatus("account-secret", "chan00000001", demo = true).enabled)
        assertFalse(api.mediaStatus("account-secret", "chan00000002", demo = false).enabled)

        val demo = server.takeRequest(); val channel = server.takeRequest()
        assertEquals("GET", demo.method)
        assertEquals("/api/media/status", demo.path)
        assertNull(demo.headers["Authorization"])
        assertEquals("/api/channels/chan00000002/media/status", channel.path)
        assertEquals("Bearer account-secret", channel.headers["Authorization"])
    }
}
