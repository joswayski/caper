package chat.caper.android.data

import chat.caper.android.model.*
import java.math.BigInteger
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.*
import okhttp3.*

class GatewayClient(
    private val baseUrl: String, private val token: String?, private val channelId: String,
    initialCursor: String, private val onMessage: (ChatMessage) -> Unit,
    private val onTyping: (ChatAuthor, Boolean, String) -> Unit = { _, _, _ -> },
    private val onPresence: (PresenceSnapshot) -> Unit = {},
    private val onMedia: (String, List<Participant>) -> Unit = { _, _ -> },
    private val onMediaDenied: (String) -> Unit = {}, private val onMediaDisconnected: () -> Unit = {},
    private val onAccessDenied: () -> Unit, private val onResync: () -> Unit,
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
    private val helloTimeoutMs: Long = 10_000, private val receiveTimeoutMs: Long = 30_000,
    private val heartbeatIntervalMs: Long = 10_000, private val catchupTimeoutMs: Long = 15_000,
) : AutoCloseable {
    private data class Stream(
        val socket: WebSocket, var hello: Boolean = false, var chatSubscribed: Boolean = false,
        var chatPosition: String = "0", var chatReady: Boolean = false,
        val subscribed: MutableSet<String> = mutableSetOf(),
        val mediaPosition: MutableMap<String, Long> = mutableMapOf(),
        val mediaSeen: MutableSet<String> = mutableSetOf(),
        var pendingPresence: PresenceSnapshot? = null, var heartbeat: Job? = null,
        var watchdog: Job? = null, var catchup: Job? = null,
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val subscriptionId = UUID.randomUUID().toString()
    private var presenceSubscriptionId: String? = null
    private var presenceSpaceId: String? = null
    private var presenceUserIds = emptyList<String>()
    private val mediaSubscriptions = mutableMapOf<String, String>()
    private val mediaRevisions = mutableMapOf<String, Long>()
    private var active: Stream? = null
    private var candidate: Stream? = null
    private var reconnect: Job? = null
    private var closed = false
    private var attempts = 0
    @Volatile private var lastActivityAt = System.currentTimeMillis()
    @Volatile private var cursor = initialCursor
    @Volatile private var serverOffsetMs = 0L
    @Volatile private var idleTimeoutMs = 600_000L
    private val mutableStatus = MutableStateFlow(GatewayStatus.DISCONNECTED)
    val status: StateFlow<GatewayStatus> = mutableStatus

    fun start() = open(false)
    fun reportActivity() { lastActivityAt = System.currentTimeMillis() }
    fun localPresence(now: Long = System.currentTimeMillis()) = when {
        mutableStatus.value != GatewayStatus.LIVE -> "offline"
        now - lastActivityAt >= idleTimeoutMs -> "idle"
        else -> "online"
    }

    @Synchronized fun watchPresence(spaceId: String, userIds: List<String>) {
        require(userIds.size in 1..100) { "Presence supports 1 to 100 members." }
        val old = presenceSubscriptionId
        presenceSpaceId = spaceId; presenceUserIds = userIds.distinct(); presenceSubscriptionId = UUID.randomUUID().toString()
        streams().forEach { stream ->
            stream.pendingPresence = null
            if (stream.hello) old?.let { stream.socket.send("""{"type":"unsubscribe","id":"$it"}""") }
            old?.let { stream.subscribed.remove(it) }
            if (stream.hello) sendPresenceSubscription(stream.socket)
        }
    }

    @Synchronized fun watchMedia(channelIds: List<String>, demo: Boolean) {
        require(channelIds.size <= 24)
        val desired = if (demo) listOf("") else channelIds.distinct().take(24)
        mediaSubscriptions.filterValues { it !in desired }.keys.toList().forEach { id ->
            streams().forEach { s -> if (s.hello) s.socket.send("""{"type":"unsubscribe","id":"$id"}"""); s.subscribed.remove(id); s.mediaPosition.remove(id) }
            val channel = mediaSubscriptions.remove(id)!!; mediaRevisions.remove(id); onMedia(channel, emptyList())
        }
        desired.filter { it !in mediaSubscriptions.values }.forEach { channel ->
            val id = UUID.randomUUID().toString(); mediaSubscriptions[id] = channel
            streams().filter { it.hello }.forEach { sendMediaSubscription(it.socket, id, channel) }
        }
    }

    private fun streams() = listOfNotNull(active, candidate)
    private fun sendMediaSubscription(ws: WebSocket, id: String, channel: String) = ws.send(buildJsonObject {
        put("type", "subscribe"); put("id", id); put("kind", "media"); if (channel.isNotEmpty()) put("channelId", channel)
    }.toString())

    @Synchronized private fun open(replacement: Boolean) {
        if (closed || if (replacement) candidate != null else active != null) return
        if (!replacement) mutableStatus.value = GatewayStatus.CONNECTING
        val request = Request.Builder().url(baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events")
            .apply { token?.let { header("Authorization", "Bearer $it") } }.build()
        val ws = client.newWebSocket(request, listener)
        val stream = Stream(ws, chatPosition = cursor)
        if (replacement) candidate = stream else active = stream
        armWatchdog(stream, helloTimeoutMs)
        if (replacement) stream.catchup = scope.launch { delay(catchupTimeoutMs); fail(stream, false) }
    }

    private val listener = object : WebSocketListener() {
        override fun onMessage(ws: WebSocket, text: String) {
            val stream = synchronized(this@GatewayClient) { streams().find { it.socket === ws } } ?: return
            if (text.length > 256 * 1024) return fail(stream, false)
            runCatching { receive(stream, json.parseToJsonElement(text).jsonObject) }.onFailure { fail(stream, false) }
        }
        override fun onClosing(ws: WebSocket, code: Int, reason: String) = findAndFail(ws, false)
        override fun onClosed(ws: WebSocket, code: Int, reason: String) = findAndFail(ws, false)
        override fun onFailure(ws: WebSocket, error: Throwable, response: Response?) {
            val denied = response?.code in setOf(401, 403, 404); findAndFail(ws, denied)
        }
    }
    @Synchronized private fun findAndFail(ws: WebSocket, terminal: Boolean) { streams().find { it.socket === ws }?.let { fail(it, terminal) } }

    @Synchronized private fun receive(s: Stream, frame: JsonObject) {
        if (s !== active && s !== candidate) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                require(!s.hello); s.hello = true; attempts = 0
                serverOffsetMs = frame["serverTime"]?.jsonPrimitive?.content?.toLongOrNull()?.minus(System.currentTimeMillis()) ?: 0
                frame["idleTimeoutSeconds"]?.jsonPrimitive?.content?.toLongOrNull()?.let { idleTimeoutMs = it * 1_000 }
                s.chatPosition = cursor
                s.socket.send("""{"type":"subscribe","id":"$subscriptionId","kind":"chat","channelId":"$channelId","after":"$cursor"}""")
                presenceSubscriptionId?.let { sendPresenceSubscription(s.socket) }
                mediaSubscriptions.forEach { (id, channel) -> s.mediaPosition[id] = -1; sendMediaSubscription(s.socket, id, channel) }
                s.heartbeat = scope.launch { while (true) { delay(heartbeatIntervalMs); val age = (System.currentTimeMillis() - lastActivityAt).coerceAtLeast(0); s.socket.send("""{"type":"heartbeat","activityAgeMs":$age}""") } }
                armWatchdog(s, receiveTimeoutMs)
            }
            "heartbeat" -> armWatchdog(s, receiveTimeoutMs)
            "migrating" -> if (s === active) open(true) else fail(s, false)
            "subscribed" -> {
                val id = frame["id"]?.jsonPrimitive?.content ?: return
                if (id == subscriptionId) s.chatSubscribed = true else if (id == presenceSubscriptionId || id in mediaSubscriptions) s.subscribed.add(id)
                if (s === active && id == subscriptionId) mutableStatus.value = GatewayStatus.LIVE
                maybePromote(s)
            }
            "event" -> receiveEvent(s, frame["id"]?.jsonPrimitive?.content ?: return, frame["event"]?.jsonObject ?: return)
            "error" -> receiveError(s, frame)
        }
    }

    private fun receiveEvent(s: Stream, id: String, event: JsonObject) {
        if (id == subscriptionId) when (event["type"]?.jsonPrimitive?.content) {
            "message.created" -> {
                val next = event["seq"]?.jsonPrimitive?.content ?: return
                val n = next.toBigIntegerOrNull() ?: error("Invalid sequence")
                val local = s.chatPosition.toBigIntegerOrNull() ?: error("Invalid cursor")
                if (n == local + BigInteger.ONE) s.chatPosition = next else if (n > local) error("Non-contiguous replay")
                val applied = cursor.toBigIntegerOrNull() ?: error("Invalid cursor")
                if (n == applied + BigInteger.ONE) {
                    val message = json.decodeFromJsonElement(ChatMessage.serializer(), event.getValue("message")).validated(channelId)
                    require(message.seq == next); cursor = next; onMessage(message)
                } else if (n > applied) error("Logical delivery gap")
                maybePromote(s)
            }
            "typing.updated" -> if (s === active) {
                val author = json.decodeFromJsonElement(ChatAuthor.serializer(), event.getValue("author"))
                val typing = event["typing"]?.jsonPrimitive?.content?.toBooleanStrictOrNull() ?: return
                val revision = event["revision"]?.jsonPrimitive?.content ?: return; require(revision.toLongOrNull() != null)
                onTyping(author, typing, revision)
            }
            "ready" -> { val checkpoint = event["cursor"]?.jsonPrimitive?.content ?: return; require(checkpoint == s.chatPosition); s.chatReady = true; maybePromote(s) }
            "resync_required" -> if (s === active) { fail(s, true); onResync() } else fail(s, false)
        } else if (id == presenceSubscriptionId && event["type"]?.jsonPrimitive?.content == "snapshot") {
            val snapshot = json.decodeFromJsonElement(PresenceSnapshot.serializer(), event)
            if (s === active) onPresence(snapshot) else { s.pendingPresence = snapshot; maybePromote(s) }
        } else {
            val channel = mediaSubscriptions[id] ?: return
            if (event["type"]?.jsonPrimitive?.content != "snapshot") return
            require(event["participants"]?.jsonArray?.all { "tracks" !in it.jsonObject } == true)
            val snapshot = json.decodeFromJsonElement(SpectatorSnapshot.serializer(), event); require(snapshot.revision >= 0)
            s.mediaSeen.add(id)
            if (snapshot.revision > (s.mediaPosition[id] ?: -1)) s.mediaPosition[id] = snapshot.revision
            if (snapshot.revision > (mediaRevisions[id] ?: -1)) { mediaRevisions[id] = snapshot.revision; onMedia(channel, snapshot.participants.map { it.asParticipant() }) }
            maybePromote(s)
        }
    }

    private fun receiveError(s: Stream, frame: JsonObject) {
        val id = frame["id"]?.jsonPrimitive?.content ?: return
        val denied = frame["status"]?.jsonPrimitive?.content?.toIntOrNull() in setOf(401, 403, 404)
        if (s === candidate) return fail(s, false)
        if (id in mediaSubscriptions && denied) { val channel = mediaSubscriptions.remove(id) ?: return; mediaRevisions.remove(id); onMedia(channel, emptyList()); onMediaDenied(channel) }
        else if (id == subscriptionId || id == presenceSubscriptionId) fail(s, denied)
        else fail(s, false)
    }

    private fun maybePromote(s: Stream) {
        if (s !== candidate || !s.hello || !s.chatSubscribed || !s.chatReady || s.chatPosition != cursor) return
        val required = mediaSubscriptions.keys + listOfNotNull(presenceSubscriptionId)
        if (!s.subscribed.containsAll(required) || !s.mediaSeen.containsAll(mediaSubscriptions.keys) ||
            (presenceSubscriptionId != null && s.pendingPresence == null) ||
            mediaSubscriptions.keys.any { (s.mediaPosition[it] ?: -1) < (mediaRevisions[it] ?: -1) }) return
        val old = active; candidate = null; active = s; s.catchup?.cancel(); mutableStatus.value = GatewayStatus.LIVE
        s.pendingPresence?.let(onPresence); s.pendingPresence = null; old?.let { dispose(it, "migration complete", graceful = true) }
    }

    fun sendTyping(chatToken: String, typing: Boolean) { active?.socket?.send(buildJsonObject {
        put("type", "command"); put("id", UUID.randomUUID().toString()); put("issuedAt", System.currentTimeMillis() + serverOffsetMs)
        put("method", "typing"); put("channelId", channelId); put("chatToken", chatToken); putJsonObject("body") { put("typing", typing) }
    }.toString()) }
    private fun sendPresenceSubscription(ws: WebSocket) { val id = presenceSubscriptionId ?: return; val space = presenceSpaceId ?: return
        ws.send(buildJsonObject { put("type", "subscribe"); put("id", id); put("kind", "presence"); put("spaceId", space); putJsonArray("userIds") { presenceUserIds.forEach { add(it) } } }.toString()) }
    private fun armWatchdog(s: Stream, timeout: Long) { s.watchdog?.cancel(); s.watchdog = scope.launch { delay(timeout); fail(s, false) } }
    private fun dispose(s: Stream, reason: String, graceful: Boolean = false) {
        s.heartbeat?.cancel(); s.watchdog?.cancel(); s.catchup?.cancel()
        if (!s.socket.close(1000, reason) || !graceful) s.socket.cancel()
    }

    @Synchronized private fun fail(s: Stream, terminal: Boolean) {
        if (s !== active && s !== candidate) return
        if (s === candidate) {
            candidate = null
            dispose(s, "candidate failed")
            if (!closed) { if (active != null) scheduleCandidate() else scheduleActive() }
            return
        }
        active = null; dispose(s, "stream failed")
        if (candidate != null && !terminal) { /* candidate remains isolated until it catches up */ return }
        if (terminal) { candidate?.let { dispose(it, "access ended") }; candidate = null }
        mediaRevisions.clear(); onMediaDisconnected(); mutableStatus.value = if (terminal) GatewayStatus.ERROR else GatewayStatus.DISCONNECTED
        if (terminal) onAccessDenied() else if (!closed) scheduleActive()
    }
    private fun scheduleCandidate() { if (reconnect?.isActive == true) return; reconnect = scope.launch { delay(backoff()); synchronized(this@GatewayClient) { reconnect = null }; open(true) } }
    private fun scheduleActive() { if (reconnect?.isActive == true) return; reconnect = scope.launch { delay(backoff()); synchronized(this@GatewayClient) { reconnect = null }; open(false) } }
    private fun backoff() = (250L shl attempts.coerceAtMost(4)).coerceAtMost(5_000).also { attempts++ }

    @Synchronized override fun close() { closed = true; reconnect?.cancel(); val had = active != null || candidate != null; streams().forEach { dispose(it, "channel closed") }; active = null; candidate = null; mediaRevisions.clear(); if (had) onMediaDisconnected(); scope.cancel(); mutableStatus.value = GatewayStatus.DISCONNECTED }
}
