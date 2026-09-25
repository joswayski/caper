package chat.caper.android.data

import chat.caper.android.model.TurnResponse
import chat.caper.android.model.ChatAuthor
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
