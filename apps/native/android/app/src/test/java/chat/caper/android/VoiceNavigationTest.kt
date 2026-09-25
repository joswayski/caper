package chat.caper.android

import chat.caper.android.data.CaperApi
import chat.caper.android.model.Channel
import chat.caper.android.model.Space
import chat.caper.android.voice.CallAttemptGate
import chat.caper.android.voice.VoiceState
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.Assert.*
import org.junit.Test

class VoiceNavigationTest {
    private val space = Space("space0000001", "Fixture Studio", "owner0000001")
    private val target = Channel("channel00003", space.id, "planning", true)
    private val current = Channel("channel00001", space.id, "general", false)
    private val active = VoiceState(phase = VoiceState.Phase.CONNECTED, spaceId = space.id, channelId = target.id)

    @Test fun `authenticated target not first is selected only after its history is read`() = runTest {
        val server = MockWebServer()
        try {
            server.enqueue(MockResponse().setBody(detail(current, Channel("channel00002", space.id, "design", false), target)))
            server.enqueue(MockResponse().setBody(history(target.id)))
            val destination = readVoiceDestination(CaperApi(baseUrl = server.url("/").toString()), "account-token", listOf(space), space.id, target.id)
            assertEquals(target, destination?.channel)
            assertEquals(target.id, destination?.history?.channel?.id)
            assertEquals("/api/spaces/${space.id}", server.takeRequest().path)
            val historyRequest = server.takeRequest()
            assertEquals("/api/chat/channels/${target.id}/messages", historyRequest.path)
            assertEquals("Bearer account-token", historyRequest.headers["Authorization"])
        } finally { server.close() }
    }

    @Test fun `revoked target does not fetch history or replace current chat`() = runTest {
        val server = MockWebServer()
        try {
            server.enqueue(MockResponse().setBody(detail(current)))
            var selected = current
            val destination = readVoiceDestination(CaperApi(baseUrl = server.url("/").toString()), "account-token", listOf(space), space.id, target.id)
            if (destination != null) selected = destination.channel
            assertNull(destination)
            assertEquals(current, selected)
            assertEquals(1, server.requestCount)
            assertEquals(active.channelId, target.id)
        } finally { server.close() }
    }

    @Test fun `other space target is resolved by id and denied history cannot produce a destination`() = runTest {
        val server = MockWebServer()
        try {
            val browsed = Space("space0000002", "Browsed", "owner0000002")
            server.enqueue(MockResponse().setBody(detail(current, target)))
            server.enqueue(MockResponse().setResponseCode(403))
            val result = runCatching {
                readVoiceDestination(CaperApi(baseUrl = server.url("/").toString()), "account-token", listOf(browsed, space), space.id, target.id)
            }
            assertTrue(result.isFailure)
            assertEquals("/api/spaces/${space.id}", server.takeRequest().path)
            assertEquals("/api/chat/channels/${target.id}/messages", server.takeRequest().path)
        } finally { server.close() }
    }

    @Test fun `demo opens its actual General channel without account token`() = runTest {
        val server = MockWebServer()
        try {
            val demo = Space("demospac0001", "General", demo = true)
            val channel = "demochan0001"
            server.enqueue(MockResponse().setBody("""{"space":{"id":"${demo.id}","name":"General"},"channel":{"id":"$channel","name":"general"},"messages":[],"cursor":"0","hasMore":false}"""))
            val destination = readVoiceDestination(CaperApi(baseUrl = server.url("/").toString()), null, listOf(space, demo), demo.id, channel)
            assertEquals(channel, destination?.channel?.id)
            assertEquals(demo.id, destination?.detail?.space?.id)
            val request = server.takeRequest()
            assertEquals("/api/chat/general", request.path)
            assertNull(request.headers["Authorization"])
        } finally { server.close() }
    }

    @Test fun `late response after chat navigation or account change cannot commit`() = runTest {
        val server = MockWebServer()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                if (request.path == "/api/spaces/${space.id}") {
                    entered.countDown()
                    check(release.await(5, TimeUnit.SECONDS))
                    return MockResponse().setBody(detail(current, target))
                }
                return MockResponse().setBody(history(target.id))
            }
        }
        try {
            val gate = CallAttemptGate()
            val oldCall = gate.begin()
            val context = VoiceNavigationContext(10, 4, oldCall, space.id, target.id)
            val pending = async(start = CoroutineStart.UNDISPATCHED) {
                readVoiceDestination(CaperApi(baseUrl = server.url("/").toString()), "account-token", listOf(space), space.id, target.id)
            }
            assertTrue(entered.await(3, TimeUnit.SECONDS))
            assertFalse(context.isCurrent(11, 4, oldCall, active))
            assertFalse(context.isCurrent(10, 5, oldCall, active))
            assertFalse(context.isCurrent(10, 4, oldCall, active.copy(phase = VoiceState.Phase.IDLE)))
            gate.end()
            val replacement = gate.begin()
            assertFalse("replacement with the same channel IDs must not accept the old response", context.isCurrent(10, 4, replacement, active))
            release.countDown()
            // The target is still valid on the server, but the old request is no longer ours.
            val result = pending.await()
            assertEquals(target, result?.channel)
            assertFalse(context.isCurrent(10, 4, gate.snapshot(), active))
        } finally { release.countDown(); server.close() }
    }

    private fun detail(vararg channels: Channel) = """{"space":{"id":"${space.id}","name":"${space.name}","ownerId":"${space.ownerId}"},"channels":[${channels.joinToString(",") { """{"id":"${it.id}","spaceId":"${it.spaceId}","name":"${it.name}","private":${it.private}}""" }}],"members":[]}"""
    private fun history(channel: String) = """{"channel":{"id":"$channel","name":"planning"},"messages":[],"cursor":"0","hasMore":false}"""
}
