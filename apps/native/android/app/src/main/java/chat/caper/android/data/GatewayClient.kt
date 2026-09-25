package chat.caper.android.data

import chat.caper.android.model.ChatMessage
import chat.caper.android.model.ChatAuthor
import chat.caper.android.model.GatewayStatus
import chat.caper.android.model.PresenceSnapshot
import chat.caper.android.model.Participant
import chat.caper.android.model.SpectatorSnapshot
import java.util.UUID
import java.math.BigInteger
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
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener

class GatewayClient(
    private val baseUrl: String,
    private val token: String?,
    private val channelId: String,
    initialCursor: String,
    private val onMessage: (ChatMessage) -> Unit,
    private val onTyping: (ChatAuthor, Boolean, String) -> Unit = { _, _, _ -> },
    private val onPresence: (PresenceSnapshot) -> Unit = {},
    private val onMedia: (String, List<Participant>) -> Unit = { _, _ -> },
    private val onMediaDenied: (String) -> Unit = {},
    private val onMediaDisconnected: () -> Unit = {},
    private val onAccessDenied: () -> Unit,
    private val onResync: () -> Unit,
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
) : AutoCloseable {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val subscriptionId = UUID.randomUUID().toString()
    private var presenceSubscriptionId: String? = null
    private var presenceSpaceId: String? = null
    private var presenceUserIds: List<String> = emptyList()
    private val mediaSubscriptions = mutableMapOf<String, String>() // subscription ID -> channel ID (empty for demo)
    private val mediaRevisions = mutableMapOf<String, Long>()
    private var socket: WebSocket? = null
    private var ready = false
    private var heartbeat: Job? = null
    private var reconnect: Job? = null
    private var watchdog: Job? = null
    private var closed = false
    private var attempts = 0
    @Volatile private var lastActivityAt = System.currentTimeMillis()
    @Volatile private var cursor = initialCursor
    @Volatile private var serverOffsetMs = 0L
    private val mutableStatus = MutableStateFlow(GatewayStatus.DISCONNECTED)
    val status: StateFlow<GatewayStatus> = mutableStatus

    fun start() { connect() }
    fun reportActivity() { lastActivityAt = System.currentTimeMillis() }

    @Synchronized fun watchPresence(spaceId: String, userIds: List<String>) {
        require(userIds.size in 1..100) { "Presence supports 1 to 100 members." }
        if (ready) presenceSubscriptionId?.let { id -> socket?.send("""{"type":"unsubscribe","id":"$id"}""") }
        presenceSpaceId = spaceId
        presenceUserIds = userIds.distinct()
        presenceSubscriptionId = UUID.randomUUID().toString()
        if (ready) socket?.let(::sendPresenceSubscription)
    }

    @Synchronized fun watchMedia(channelIds: List<String>, demo: Boolean) {
        require(channelIds.size <= 24)
        val desired = if (demo) listOf("") else channelIds.distinct().take(24)
        mediaSubscriptions.filterValues { it !in desired }.keys.toList().forEach { id ->
            if (ready) socket?.send("""{"type":"unsubscribe","id":"$id"}""")
            val channel = mediaSubscriptions.remove(id)!!
            mediaRevisions.remove(id)
            onMedia(channel, emptyList())
        }
        desired.filter { it !in mediaSubscriptions.values }.forEach { channel ->
            val id = UUID.randomUUID().toString()
            mediaSubscriptions[id] = channel
            if (ready) socket?.let { sendMediaSubscription(it, id, channel) }
        }
    }

    private fun sendMediaSubscription(webSocket: WebSocket, id: String, channel: String) {
        val frame = buildJsonObject {
            put("type", "subscribe"); put("id", id); put("kind", "media")
            if (channel.isNotEmpty()) put("channelId", channel)
        }
        webSocket.send(frame.toString())
    }

    @Synchronized private fun connect() {
        if (closed || socket != null) return
        ready = false
        mutableStatus.value = GatewayStatus.CONNECTING
        val url = baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events"
        val request = Request.Builder().url(url).apply { token?.let { header("Authorization", "Bearer $it") } }.build()
        socket = client.newWebSocket(request, listener)
    }

    private val listener = object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            if (text.length > 256 * 1024) return fail(webSocket, terminal = false)
            runCatching { receive(webSocket, json.parseToJsonElement(text).jsonObject) }
                .onFailure { fail(webSocket, terminal = false) }
        }
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) = fail(webSocket, terminal = false)
        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = fail(webSocket, terminal = false)
        override fun onFailure(webSocket: WebSocket, error: Throwable, response: Response?) {
            val denied = response?.code == 401 || response?.code == 403 || response?.code == 404
            fail(webSocket, terminal = denied)
            if (denied) onAccessDenied()
        }
    }

    @Synchronized private fun receive(webSocket: WebSocket, frame: JsonObject) {
        if (socket !== webSocket) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                ready = true
                attempts = 0
                serverOffsetMs = frame["serverTime"]?.jsonPrimitive?.content?.toLongOrNull()?.minus(System.currentTimeMillis()) ?: 0L
                webSocket.send("""{"type":"subscribe","id":"$subscriptionId","kind":"chat","channelId":"$channelId","after":"$cursor"}""")
                if (presenceSubscriptionId != null) sendPresenceSubscription(webSocket)
                mediaRevisions.clear()
                mediaSubscriptions.forEach { (id, channel) -> sendMediaSubscription(webSocket, id, channel) }
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
                        val nextSequence = next.toBigIntegerOrNull()
                        val cursorSequence = cursor.toBigIntegerOrNull()
                        if (nextSequence != null && cursorSequence != null && nextSequence == cursorSequence + BigInteger.ONE) {
                            val message = json.decodeFromJsonElement(ChatMessage.serializer(), event.getValue("message"))
                                .validated(channelId)
                            require(message.seq == next) { "Gateway event sequence mismatch." }
                            onMessage(message)
                            cursor = next
                        } else if (nextSequence != null && cursorSequence != null && nextSequence > cursorSequence) {
                            fail(webSocket, terminal = false)
                        }
                    }
                    "typing.updated" -> {
                        val author = json.decodeFromJsonElement(ChatAuthor.serializer(), event.getValue("author"))
                        val typing = event["typing"]?.jsonPrimitive?.content?.toBooleanStrictOrNull() ?: return
                        val revision = event["revision"]?.jsonPrimitive?.content ?: return
                        require(revision.toLongOrNull() != null) { "Invalid typing revision." }
                        onTyping(author, typing, revision)
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
            } else if (frame["id"]?.jsonPrimitive?.content == presenceSubscriptionId) {
                val event = frame["event"]?.jsonObject ?: return
                if (event["type"]?.jsonPrimitive?.content == "snapshot") {
                    onPresence(json.decodeFromJsonElement(PresenceSnapshot.serializer(), event))
                }
            } else {
                val id = frame["id"]?.jsonPrimitive?.content ?: return
                val channel = mediaSubscriptions[id] ?: return
                val event = frame["event"]?.jsonObject ?: return
                if (event["type"]?.jsonPrimitive?.content != "snapshot") return
                require(event["participants"]?.jsonArray?.all { participant -> "tracks" !in participant.jsonObject } == true)
                val snapshot = json.decodeFromJsonElement(SpectatorSnapshot.serializer(), event)
                val revision = snapshot.revision
                require(revision >= 0)
                if (revision > (mediaRevisions[id] ?: -1L)) {
                    mediaRevisions[id] = revision
                    onMedia(channel, snapshot.participants.map { it.asParticipant() })
                }
            }
            "error" -> if (frame["id"]?.jsonPrimitive?.content in mediaSubscriptions) {
                val id = frame["id"]!!.jsonPrimitive.content
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull()
                if (status == 401 || status == 403 || status == 404) {
                    val channel = mediaSubscriptions.remove(id) ?: return
                    mediaRevisions.remove(id)
                    onMedia(channel, emptyList())
                    onMediaDenied(channel)
                } else fail(webSocket, terminal = false)
            } else if (frame["id"]?.jsonPrimitive?.content in setOf(subscriptionId, presenceSubscriptionId)) {
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull()
                val denied = status == 401 || status == 403 || status == 404
                fail(webSocket, terminal = denied)
                if (denied) onAccessDenied()
            }
            "migrating" -> fail(webSocket, terminal = false)
        }
    }

    fun sendTyping(chatToken: String, typing: Boolean) {
        val frame = buildJsonObject {
            put("type", "command")
            put("id", UUID.randomUUID().toString())
            put("issuedAt", System.currentTimeMillis() + serverOffsetMs)
            put("method", "typing")
            put("channelId", channelId)
            put("chatToken", chatToken)
            putJsonObject("body") { put("typing", typing) }
        }
        socket?.send(frame.toString())
    }

    private fun sendPresenceSubscription(webSocket: WebSocket) {
        val id = presenceSubscriptionId ?: return
        val space = presenceSpaceId ?: return
        val users = presenceUserIds.joinToString(",") { "\"$it\"" }
        webSocket.send("""{"type":"subscribe","id":"$id","kind":"presence","spaceId":"$space","userIds":[$users]}""")
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
        ready = false
        mediaRevisions.clear()
        onMediaDisconnected()
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

    @Synchronized override fun close() {
        closed = true
        heartbeat?.cancel()
        watchdog?.cancel()
        reconnect?.cancel()
        socket?.close(1000, "channel closed")
        socket = null
        ready = false
        mediaRevisions.clear()
        onMediaDisconnected()
        scope.coroutineContext[Job]?.cancel()
        mutableStatus.value = GatewayStatus.DISCONNECTED
    }
}
