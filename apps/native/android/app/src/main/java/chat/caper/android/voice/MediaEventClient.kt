package chat.caper.android.voice

import chat.caper.android.data.ApiException
import chat.caper.android.model.MediaSnapshot
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener

/** Live roster stream. The account bearer authenticates the socket; the media capability stays in the subscription body. */
internal class MediaEventClient(
    private val baseUrl: String,
    private val accountToken: String?,
    private val channelId: String?,
    private val mediaToken: String,
    private val onSnapshot: (MediaSnapshot) -> Unit,
    private val onTerminal: (Throwable) -> Unit,
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
    private val monotonicMs: () -> Long = { System.nanoTime() / 1_000_000 },
    private val helloTimeoutMs: Long = 10_000,
    private val receiveTimeoutMs: Long = 30_000,
    private val heartbeatIntervalMs: Long = 10_000,
) : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val subscriptionId = UUID.randomUUID().toString()
    private var socket: WebSocket? = null
    private var heartbeat: Job? = null
    private var watchdog: Job? = null
    private var reconnect: Job? = null
    private var attempts = 0
    private var closed = false
    private val startedAt = monotonicMs()

    fun start() = connect()

    @Synchronized private fun connect() {
        if (closed || socket != null) return
        val url = baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events"
        val request = Request.Builder().url(url).apply { accountToken?.let { header("Authorization", "Bearer $it") } }.build()
        val created = client.newWebSocket(request, listener)
        socket = created
        armWatchdog(created, helloTimeoutMs)
    }

    private val listener = object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > 256 * 1024) return fail(webSocket, terminal = null)
            runCatching { receive(webSocket, json.parseToJsonElement(text).jsonObject) }
                .onFailure { fail(webSocket, terminal = null) }
        }
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) = fail(webSocket, terminal = null)
        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = fail(webSocket, terminal = null)
        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            val status = response?.code
            fail(webSocket, terminal = if (status in setOf(401, 403, 404)) ApiException(requireNotNull(status), "Voice access ended.") else null)
        }
    }

    @Synchronized private fun receive(webSocket: WebSocket, frame: kotlinx.serialization.json.JsonObject) {
        if (socket !== webSocket) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                attempts = 0
                val subscription = buildJsonObject {
                    put("type", "subscribe"); put("id", subscriptionId); put("kind", "media"); put("token", mediaToken)
                    channelId?.let { put("channelId", it) }
                }
                webSocket.send(subscription.toString())
                heartbeat?.cancel()
                heartbeat = scope.launch {
                    while (true) {
                        delay(heartbeatIntervalMs)
                        val age = (monotonicMs() - startedAt).coerceAtLeast(0)
                        webSocket.send("""{"type":"heartbeat","activityAgeMs":$age}""")
                    }
                }
                armWatchdog(webSocket, receiveTimeoutMs)
            }
            "heartbeat" -> armWatchdog(webSocket, receiveTimeoutMs)
            "event" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val event = frame["event"]?.jsonObject ?: return
                if (event["type"]?.jsonPrimitive?.content == "snapshot") {
                    onSnapshot(json.decodeFromJsonElement(MediaSnapshot.serializer(), event))
                }
            }
            "error" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull()
                fail(webSocket, if (status in setOf(401, 403, 404)) ApiException(requireNotNull(status), "Voice access ended.") else null)
            }
            "migrating" -> fail(webSocket, terminal = null)
        }
    }

    private fun armWatchdog(webSocket: WebSocket, timeoutMs: Long) {
        watchdog?.cancel()
        watchdog = scope.launch { delay(timeoutMs); fail(webSocket, terminal = null) }
    }

    @Synchronized private fun fail(webSocket: WebSocket, terminal: Throwable?) {
        if (socket !== webSocket) return
        socket = null
        heartbeat?.cancel(); watchdog?.cancel(); webSocket.cancel()
        if (terminal != null) {
            closed = true
            onTerminal(terminal)
        } else if (!closed && reconnect?.isActive != true) {
            reconnect = scope.launch {
                delay((250L shl attempts.coerceAtMost(4)).coerceAtMost(3_000))
                attempts++
                connect()
            }
        }
    }

    @Synchronized override fun close() {
        closed = true
        heartbeat?.cancel(); watchdog?.cancel(); reconnect?.cancel()
        socket?.close(1000, "voice events closed"); socket = null
        scope.coroutineContext[Job]?.cancel()
    }
}
