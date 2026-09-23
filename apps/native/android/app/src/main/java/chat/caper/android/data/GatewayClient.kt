package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.GatewayStatus
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener

class GatewayClient(
    private val baseUrl: String,
    private val token: String,
    private val channelId: String,
    initialCursor: String,
    private val onMessage: (ChatMessage) -> Unit,
    private val onAccessDenied: () -> Unit,
    private val onResync: () -> Unit,
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
) : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val subscriptionId = UUID.randomUUID().toString()
    private var socket: WebSocket? = null
    private var heartbeat: Job? = null
    private var reconnect: Job? = null
    private var watchdog: Job? = null
    private var closed = false
    private var attempts = 0
    @Volatile private var lastActivityAt = System.currentTimeMillis()
    @Volatile private var cursor = initialCursor
    private val mutableStatus = MutableStateFlow(GatewayStatus.DISCONNECTED)
    val status: StateFlow<GatewayStatus> = mutableStatus

    fun start() { connect() }
    fun reportActivity() { lastActivityAt = System.currentTimeMillis() }

    private fun connect() {
        if (closed || socket != null) return
        mutableStatus.value = GatewayStatus.CONNECTING
        val url = baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events"
        val request = Request.Builder().url(url).header("Authorization", "Bearer $token").build()
        socket = client.newWebSocket(request, listener)
    }

    private val listener = object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > 256 * 1024) return fail(webSocket, terminal = false)
            runCatching { receive(webSocket, json.parseToJsonElement(text).jsonObject) }
                .onFailure { fail(webSocket, terminal = false) }
        }
        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = fail(webSocket, terminal = false)
        override fun onFailure(webSocket: WebSocket, error: Throwable, response: Response?) {
            val denied = response?.code == 401 || response?.code == 403 || response?.code == 404
            fail(webSocket, terminal = denied)
            if (denied) onAccessDenied()
        }
    }

    private fun receive(webSocket: WebSocket, frame: JsonObject) {
        if (socket !== webSocket) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                attempts = 0
                webSocket.send("""{"type":"subscribe","id":"$subscriptionId","kind":"chat","channelId":"$channelId","after":"$cursor"}""")
                heartbeat?.cancel()
                heartbeat = scope.launch {
                    while (true) {
                        delay(10_000)
                        val age = (System.currentTimeMillis() - lastActivityAt).coerceAtLeast(0)
                        webSocket.send("""{"type":"heartbeat","activityAgeMs":$age}""")
                    }
                }
                armWatchdog(webSocket)
            }
            "heartbeat" -> armWatchdog(webSocket)
            "subscribed" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) mutableStatus.value = GatewayStatus.LIVE
            "event" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val event = frame["event"]?.jsonObject ?: return
                when (event["type"]?.jsonPrimitive?.content) {
                    "message.created" -> {
                        val next = event["seq"]?.jsonPrimitive?.content ?: return
                        if (next.toLongOrNull() == cursor.toLongOrNull()?.plus(1)) {
                            val message = json.decodeFromJsonElement(ChatMessage.serializer(), event.getValue("message"))
                                .validated(channelId)
                            require(message.seq == next) { "Gateway event sequence mismatch." }
                            onMessage(message)
                            cursor = next
                        } else if (next.toLongOrNull()?.let { it > (cursor.toLongOrNull() ?: -1) } == true) {
                            fail(webSocket, terminal = false)
                        }
                    }
                    "ready" -> {
                        val checkpoint = event["cursor"]?.jsonPrimitive?.content ?: return
                        if (checkpoint != cursor) fail(webSocket, terminal = false)
                    }
                    "resync_required" -> {
                        fail(webSocket, terminal = true)
                        onResync()
                    }
                }
            }
            "error" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull()
                val denied = status == 401 || status == 403 || status == 404
                fail(webSocket, terminal = denied)
                if (denied) onAccessDenied()
            }
            "migrating" -> fail(webSocket, terminal = false)
        }
    }

    private fun armWatchdog(webSocket: WebSocket) {
        watchdog?.cancel()
        watchdog = scope.launch {
            delay(30_000)
            fail(webSocket, terminal = false)
        }
    }

    @Synchronized private fun fail(webSocket: WebSocket, terminal: Boolean) {
        if (socket !== webSocket) return
        socket = null
        heartbeat?.cancel()
        watchdog?.cancel()
        webSocket.cancel()
        mutableStatus.value = if (terminal) GatewayStatus.ERROR else GatewayStatus.DISCONNECTED
        if (!closed && !terminal && reconnect?.isActive != true) {
            reconnect = scope.launch {
                delay((250L shl attempts.coerceAtMost(4)).coerceAtMost(5_000))
                attempts++
                connect()
            }
        }
    }

    override fun close() {
        closed = true
        heartbeat?.cancel()
        watchdog?.cancel()
        reconnect?.cancel()
        socket?.close(1000, "channel closed")
        socket = null
        scope.coroutineContext[Job]?.cancel()
        mutableStatus.value = GatewayStatus.DISCONNECTED
    }
}
