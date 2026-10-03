package chat.caper.android.voice

import chat.caper.android.data.ApiException
import chat.caper.android.model.MediaSnapshot
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import okhttp3.*

/** Live roster stream with make-before-break migration. */
internal class MediaEventClient(
    private val baseUrl: String, private val accountToken: String?, private val channelId: String?,
    private val mediaToken: String, private val onSnapshot: (MediaSnapshot) -> Unit,
    private val onTerminal: (Throwable) -> Unit,
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
    private val monotonicMs: () -> Long = { System.nanoTime() / 1_000_000 },
    private val helloTimeoutMs: Long = 10_000, private val receiveTimeoutMs: Long = 30_000,
    private val heartbeatIntervalMs: Long = 10_000, private val catchupTimeoutMs: Long = 15_000,
) : AutoCloseable {
    private data class Stream(val socket: WebSocket, var hello: Boolean = false, var subscribed: Boolean = false,
        var snapshotSeen: Boolean = false, var revision: Long = -1, var heartbeat: Job? = null,
        var watchdog: Job? = null, var catchup: Job? = null)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val subscriptionId = UUID.randomUUID().toString()
    private var active: Stream? = null
    private var candidate: Stream? = null
    private var reconnect: Job? = null
    private var attempts = 0
    private var closed = false
    private var appliedRevision = -1L
    private val startedAt = monotonicMs()

    fun start() = open(false)
    @Synchronized private fun open(replacement: Boolean) {
        if (closed || if (replacement) candidate != null else active != null) return
        val request = Request.Builder().url(baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events")
            .apply { accountToken?.let { header("Authorization", "Bearer $it") } }.build()
        val ws = client.newWebSocket(request, listener)
        val stream = Stream(ws)
        if (replacement) candidate = stream else active = stream
        armWatchdog(stream, helloTimeoutMs)
        if (replacement) stream.catchup = scope.launch { delay(catchupTimeoutMs); fail(stream, null) }
    }
    private fun streams() = listOfNotNull(active, candidate)
    private val listener = object : WebSocketListener() {
        override fun onMessage(ws: WebSocket, text: String) {
            val s = synchronized(this@MediaEventClient) { streams().find { it.socket === ws } } ?: return
            if (text.length > 256 * 1024) return fail(s, null)
            runCatching { receive(s, json.parseToJsonElement(text).jsonObject) }.onFailure { fail(s, null) }
        }
        override fun onClosing(ws: WebSocket, code: Int, reason: String) = findAndFail(ws, null)
        override fun onClosed(ws: WebSocket, code: Int, reason: String) = findAndFail(ws, null)
        override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
            val status = response?.code
            findAndFail(ws, if (status in setOf(401, 403, 404)) ApiException(requireNotNull(status), "Voice access ended.") else null)
        }
    }
    @Synchronized private fun findAndFail(ws: WebSocket, terminal: Throwable?) { streams().find { it.socket === ws }?.let { fail(it, terminal) } }

    @Synchronized private fun receive(s: Stream, frame: JsonObject) {
        if (s !== active && s !== candidate) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                require(!s.hello); s.hello = true; attempts = 0
                s.socket.send(buildJsonObject { put("type", "subscribe"); put("id", subscriptionId); put("kind", "media"); put("token", mediaToken); channelId?.let { put("channelId", it) } }.toString())
                s.heartbeat = scope.launch { while (true) { delay(heartbeatIntervalMs); val age = (monotonicMs() - startedAt).coerceAtLeast(0); s.socket.send("""{"type":"heartbeat","activityAgeMs":$age}""") } }
                armWatchdog(s, receiveTimeoutMs)
            }
            "heartbeat" -> armWatchdog(s, receiveTimeoutMs)
            "migrating" -> if (s === active) open(true) else fail(s, null)
            "subscribed" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) { s.subscribed = true; maybePromote(s) }
            "event" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val event = frame["event"]?.jsonObject ?: return
                if (event["type"]?.jsonPrimitive?.content == "snapshot") {
                    val snapshot = json.decodeFromJsonElement(MediaSnapshot.serializer(), event)
                    val revision = snapshot.revision ?: error("Missing media revision")
                    require(revision >= 0); s.snapshotSeen = true; if (revision > s.revision) s.revision = revision
                    if (revision > appliedRevision) { appliedRevision = revision; onSnapshot(snapshot) }
                    maybePromote(s)
                }
            }
            "error" -> if (frame["id"]?.jsonPrimitive?.content == subscriptionId) {
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull()
                fail(s, if (status in setOf(401, 403, 404)) ApiException(requireNotNull(status), "Voice access ended.") else null)
            }
        }
    }
    private fun maybePromote(s: Stream) {
        if (s !== candidate || !s.subscribed || !s.snapshotSeen || s.revision < appliedRevision) return
        val old = active; candidate = null; active = s; s.catchup?.cancel(); old?.let { dispose(it, "migration complete", graceful = true) }
    }
    private fun armWatchdog(s: Stream, timeout: Long) { s.watchdog?.cancel(); s.watchdog = scope.launch { delay(timeout); fail(s, null) } }
    private fun dispose(s: Stream, reason: String, graceful: Boolean = false) {
        s.heartbeat?.cancel(); s.watchdog?.cancel(); s.catchup?.cancel()
        if (!s.socket.close(1000, reason) || !graceful) s.socket.cancel()
    }
    @Synchronized private fun fail(s: Stream, terminal: Throwable?) {
        if (s !== active && s !== candidate) return
        if (s === candidate) {
            candidate = null
            dispose(s, "candidate failed")
            if (!closed) schedule(active != null)
            return
        }
        active = null; dispose(s, "stream failed")
        if (terminal != null) { candidate?.let { dispose(it, "access ended") }; candidate = null; closed = true; onTerminal(terminal) }
        else if (candidate == null && !closed) schedule(false)
    }
    private fun schedule(replacement: Boolean) { if (reconnect?.isActive == true) return; reconnect = scope.launch { delay((250L shl attempts.coerceAtMost(4)).coerceAtMost(3_000)); attempts++; synchronized(this@MediaEventClient) { reconnect = null }; open(replacement) } }
    @Synchronized override fun close() { closed = true; reconnect?.cancel(); streams().forEach { dispose(it, "voice events closed") }; active = null; candidate = null; scope.cancel() }
}
