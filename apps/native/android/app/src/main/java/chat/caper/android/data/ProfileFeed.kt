package chat.caper.android.data

import chat.caper.android.model.ProfileEvent
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import okhttp3.*

/**
 * The account-wide [PROFILES_SUBSCRIPTION] feed on its own gateway socket, like Apple's
 * `profileGateway`: profile and space changes arrive with no conversation open (DMs only, the
 * Feedback inbox, a space with no joined channel) and through channel switches. Reconnects with
 * backoff and follows a `migrating` handoff. No replay: refetches repair what a gap missed, and
 * an event applied twice (both sockets during a handoff) changes nothing. Stops for good when
 * the gateway refuses the kind (an older gateway) or the sign-in.
 *
 * The same socket carries the account's [NOTIFICATIONS_SUBSCRIPTION]: each event calls [onNotification] (the app
 * refreshes unread state), and [onNotificationsLive] reports whether it is subscribed, so timer polls can slow
 * down while it is. A gateway that refuses that kind only leaves it off.
 */
class ProfileFeedClient(
    private val baseUrl: String, private val token: String, private val onProfile: (ProfileEvent) -> Unit,
    private val onNotification: () -> Unit = {}, private val onNotificationsLive: (Boolean) -> Unit = {},
    private val json: Json = Json { ignoreUnknownKeys = true },
    private val client: OkHttpClient = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
        .followRedirects(false).followSslRedirects(false).build(),
    private val helloTimeoutMs: Long = 10_000, private val receiveTimeoutMs: Long = 30_000,
    private val heartbeatIntervalMs: Long = 10_000,
) : AutoCloseable {
    private class Stream(val socket: WebSocket) {
        var hello = false; var subscribed = false; var notifications = false
        var heartbeat: Job? = null; var watchdog: Job? = null
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var active: Stream? = null
    /** A `migrating` replacement; takes over once subscribed. */
    private var candidate: Stream? = null
    private var reconnect: Job? = null
    private var closed = false
    private var attempts = 0
    @Volatile private var lastActivityAt = System.currentTimeMillis()
    private var live = false

    fun start() = open(false)
    /** Heartbeats carry activity age like the conversation socket's, so presence reads the same. */
    fun reportActivity() { lastActivityAt = System.currentTimeMillis() }

    @Synchronized private fun open(replacement: Boolean) {
        if (closed || (if (replacement) candidate else active) != null) return
        val request = Request.Builder().url(baseUrl.replaceFirst("https://", "wss://").replaceFirst("http://", "ws://") + "/api/chat/events")
            .header("Authorization", "Bearer $token").build()
        val stream = Stream(client.newWebSocket(request, listener))
        if (replacement) candidate = stream else active = stream
        armWatchdog(stream, helloTimeoutMs)
    }

    private val listener = object : WebSocketListener() {
        override fun onMessage(ws: WebSocket, text: String) {
            val stream = find(ws) ?: return
            if (text.length > 256 * 1024) return fail(stream, false)
            runCatching { receive(stream, json.parseToJsonElement(text).jsonObject) }.onFailure { fail(stream, false) }
        }
        override fun onClosing(ws: WebSocket, code: Int, reason: String) { find(ws)?.let { fail(it, false) } }
        override fun onClosed(ws: WebSocket, code: Int, reason: String) { find(ws)?.let { fail(it, false) } }
        override fun onFailure(ws: WebSocket, error: Throwable, response: Response?) {
            find(ws)?.let { fail(it, response?.code in setOf(401, 403, 404)) }
        }
    }
    @Synchronized private fun find(ws: WebSocket) = listOfNotNull(active, candidate).find { it.socket === ws }

    @Synchronized private fun receive(s: Stream, frame: JsonObject) {
        if (s !== active && s !== candidate) return
        when (frame["type"]?.jsonPrimitive?.content) {
            "hello" -> {
                require(!s.hello); s.hello = true
                s.socket.send(PROFILES_SUBSCRIBE)
                s.socket.send(NOTIFICATIONS_SUBSCRIBE)
                s.heartbeat = scope.launch { while (true) { delay(heartbeatIntervalMs); val age = (System.currentTimeMillis() - lastActivityAt).coerceAtLeast(0); s.socket.send("""{"type":"heartbeat","activityAgeMs":$age}""") } }
                armWatchdog(s, receiveTimeoutMs)
            }
            "heartbeat" -> armWatchdog(s, receiveTimeoutMs)
            "migrating" -> if (s === active) open(true) else fail(s, false)
            "subscribed" -> when (frame["id"]?.jsonPrimitive?.content) {
                PROFILES_SUBSCRIPTION -> {
                    s.subscribed = true; attempts = 0
                    if (s === candidate) { val old = active; active = s; candidate = null; old?.let { dispose(it, "migration complete", graceful = true) } }
                    publishLive()
                }
                NOTIFICATIONS_SUBSCRIPTION -> { s.notifications = true; publishLive() }
            }
            "event" -> when (frame["id"]?.jsonPrimitive?.content) {
                PROFILES_SUBSCRIPTION -> frame["event"]?.jsonObject?.let { profileEvent(it, json) }?.let(onProfile)
                // A handoff's two sockets may both deliver it; the refresh it starts coalesces.
                NOTIFICATIONS_SUBSCRIPTION -> onNotification()
            }
            "error" -> if (frame["id"]?.jsonPrimitive?.content == PROFILES_SUBSCRIPTION) {
                // A client error (an older gateway's unknown kind, a sign-in that ended) is final; an outage retries.
                val status = frame["status"]?.jsonPrimitive?.content?.toIntOrNull() ?: 0
                fail(s, status in 400..499)
            } else if (frame["id"]?.jsonPrimitive?.content == NOTIFICATIONS_SUBSCRIPTION) { s.notifications = false; publishLive() }
        }
    }

    /** Live while the socket in use holds the notifications subscription. */
    private fun publishLive() {
        val now = !closed && active?.notifications == true
        if (now != live) { live = now; onNotificationsLive(now) }
    }

    private fun armWatchdog(s: Stream, timeout: Long) { s.watchdog?.cancel(); s.watchdog = scope.launch { delay(timeout); fail(s, false) } }
    private fun dispose(s: Stream, reason: String, graceful: Boolean = false) {
        s.heartbeat?.cancel(); s.watchdog?.cancel()
        if (!s.socket.close(1000, reason) || !graceful) s.socket.cancel()
    }

    @Synchronized private fun fail(s: Stream, terminal: Boolean) {
        if (terminal) { if (s === active || s === candidate) close(); return }
        if (s === candidate) { candidate = null; dispose(s, "candidate failed"); return }
        if (s !== active) return
        active = null; dispose(s, "stream failed")
        // A handoff in progress carries on as the live socket; it subscribes on its own hello.
        candidate?.let { active = it; candidate = null; publishLive(); return }
        publishLive()
        if (closed || reconnect?.isActive == true) return
        val wait = (250L shl attempts.coerceAtMost(4)).coerceAtMost(5_000); attempts++
        reconnect = scope.launch { delay(wait); synchronized(this@ProfileFeedClient) { reconnect = null }; open(false) }
    }

    @Synchronized override fun close() {
        closed = true; reconnect?.cancel()
        listOfNotNull(active, candidate).forEach { dispose(it, "signed out") }
        active = null; candidate = null; scope.cancel()
        publishLive()
    }
}
