package chat.caper.android.voice

import android.content.Context
import chat.caper.android.data.CaperApi
import chat.caper.android.data.ApiException
import chat.caper.android.model.*
import java.util.Collections
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject
import org.webrtc.*
import org.webrtc.audio.JavaAudioDeviceModule
import org.webrtc.SessionDescription as RtcSessionDescription

class VoiceEngine(
    context: Context,
    private val api: CaperApi,
    private val accountToken: String?,
    private val channelId: String,
    private val displayName: String,
    private val demo: Boolean = false,
    private val onTransportState: (PeerConnection.PeerConnectionState) -> Unit = {},
) {
    private val appContext = context.applicationContext
    private val lock = Mutex()
    private val connected = AtomicBoolean(false)
    private val closed = AtomicBoolean(false)
    private var factory: PeerConnectionFactory? = null
    private var peer: PeerConnection? = null
    private lateinit var rtcConfiguration: PeerConnection.RTCConfiguration
    private var source: AudioSource? = null
    private var microphone: AudioTrack? = null
    private var mediaToken: String? = null
    private var selfId: String? = null
    private val subscriptions = mutableMapOf<String, String>()
    private val remoteAudio = Collections.synchronizedSet(mutableSetOf<AudioTrack>())
    private val connectionState = MutableStateFlow(PeerConnection.PeerConnectionState.NEW)
    private var turn: TurnGeneration? = null
    private var stateSequence = 0L
    private var restartSequence = 1L
    @Volatile var muted = true
        private set
    @Volatile var deafened = false
        private set

    suspend fun connect(onParticipants: (List<Participant>) -> Unit) = withContext(Dispatchers.IO) {
        PeerConnectionFactory.initialize(
            PeerConnectionFactory.InitializationOptions.builder(appContext).createInitializationOptions(),
        )
        val audioModule = JavaAudioDeviceModule.builder(appContext).createAudioDeviceModule()
        factory = PeerConnectionFactory.builder().setAudioDeviceModule(audioModule).createPeerConnectionFactory()
        audioModule.release()
        val joined: JoinResponse = media(
            "join",
            buildJsonObject { put("name", displayName); put("muted", true); put("deafened", false) },
        )
        mediaToken = joined.token
        selfId = joined.id
        turn = joined.turn
        try {
            val servers = joined.iceServers.map { server ->
                PeerConnection.IceServer.builder(server.urls)
                    .apply { if (server.username != null) setUsername(server.username); if (server.credential != null) setPassword(server.credential) }
                    .createIceServer()
            }
            rtcConfiguration = PeerConnection.RTCConfiguration(servers)
            peer = factory!!.createPeerConnection(rtcConfiguration, observer)
                ?: error("Could not create voice connection.")
            source = factory!!.createAudioSource(MediaConstraints())
            // Capture begins silent. Audio opens only after SFU signaling and
            // transport connectivity complete, honoring mute intent changed while joining.
            microphone = factory!!.createAudioTrack("caper-microphone", source).apply { setEnabled(false) }
            val transceiver = peer!!.addTransceiver(
                microphone,
                RtpTransceiver.RtpTransceiverInit(RtpTransceiver.RtpTransceiverDirection.SEND_ONLY, listOf("caper")),
            )
            peer!!.setLocalDescriptionAwait(peer!!.createOfferAwait())
            peer!!.awaitIceGathering()
            val mid = transceiver.mid ?: error("WebRTC did not assign a microphone MID.")
            val local = peer!!.localDescription ?: error("WebRTC did not create an offer.")
            val published: SignalResponse = media(
                "publish",
                buildJsonObject {
                    put("kind", "microphone"); put("mid", mid)
                    putJsonObject("sessionDescription") { put("type", "offer"); put("sdp", local.description) }
                }, joined.token,
            )
            val answer = published.sessionDescription ?: error("Media service did not answer publication.")
            peer!!.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.ANSWER, answer.sdp))
            reconcile(onParticipants)
            withTimeout(20_000) {
                connectionState.first { it == PeerConnection.PeerConnectionState.CONNECTED || it == PeerConnection.PeerConnectionState.FAILED }
            }
            check(connectionState.value == PeerConnection.PeerConnectionState.CONNECTED) { "Voice transport failed to connect." }
            lock.withLock {
                check(!closed.get()) { "Voice call ended during setup." }
                connected.set(true)
                microphone?.setEnabled(!muted)
                syncStateLocked()
            }
        } catch (error: Throwable) {
            withContext(NonCancellable) {
                if (withTimeoutOrNull(3_000) { disconnect() } == null) closeLocal()
            }
            throw error
        }
    }

    suspend fun heartbeat(onParticipants: (List<Participant>) -> Unit) = lock.withLock {
        check(connectionState.value == PeerConnection.PeerConnectionState.CONNECTED) { "Voice transport disconnected." }
        reconcile(onParticipants)
    }

    private suspend fun reconcile(onParticipants: (List<Participant>) -> Unit) {
        val token = mediaToken ?: return
        val snapshot: MediaSnapshot = media("snapshot", mediaToken = token)
        onParticipants(snapshot.participants)
        val wanted = snapshot.participants
            .filter { it.id != selfId }
            .flatMap { it.tracks }
            .filter { it.kind == "microphone" }
            .map { it.id }
            .toSet()
        for ((track, mid) in subscriptions.toMap()) {
            if (track !in wanted) {
                runCatching { media<Unit>("close", buildJsonObject { put("mid", mid) }, token) }
                subscriptions.remove(track)
            }
        }
        for (track in wanted - subscriptions.keys) subscribe(track, token)
    }

    private suspend fun subscribe(trackId: String, token: String) {
        val response: SignalResponse = media(
            "subscribe", buildJsonObject { put("trackId", trackId) }, token,
        )
        val mid = response.tracks.firstOrNull()?.mid ?: error("Media service did not identify the remote track.")
        subscriptions[trackId] = mid
        response.sessionDescription?.let { offer ->
            peer!!.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.OFFER, offer.sdp))
            peer!!.setLocalDescriptionAwait(peer!!.createAnswerAwait())
            peer!!.awaitIceGathering()
            val local = peer!!.localDescription ?: error("WebRTC did not create an answer.")
            media<Unit>(
                "negotiate",
                buildJsonObject { putJsonObject("sessionDescription") { put("type", "answer"); put("sdp", local.description) } }, token,
            )
        }
    }

    suspend fun setMuted(value: Boolean) = lock.withLock {
        muted = value
        microphone?.setEnabled(connected.get() && !value)
        syncStateLocked()
    }

    suspend fun setDeafened(value: Boolean) = lock.withLock {
        deafened = value
        if (value) {
            muted = true
            microphone?.setEnabled(false)
        }
        synchronized(remoteAudio) { remoteAudio.forEach { it.setEnabled(!value) } }
        syncStateLocked()
    }

    private suspend fun syncStateLocked() {
        val token = mediaToken ?: return
        val sequence = ++stateSequence
        media<Unit>(
            "state",
            buildJsonObject { put("muted", muted); put("deafened", deafened); put("sequence", sequence) }, token,
        )
    }

    suspend fun refreshTurn(): Long? = lock.withLock {
        val old = turn ?: return null
        val response: TurnResponse = media(
            "turn", buildJsonObject { put("generation", old.generation) }, mediaToken ?: return null,
        )
        rtcConfiguration.iceServers = response.iceServers.map { server ->
            PeerConnection.IceServer.builder(server.urls)
                .apply { if (server.username != null) setUsername(server.username); if (server.credential != null) setPassword(server.credential) }
                .createIceServer()
        }
        check(peer?.setConfiguration(rtcConfiguration) == true) { "Could not refresh TURN credentials." }
        if (response.turn.generation != old.generation) restartIceLocked(response.turn.generation)
        turn = response.turn
        return response.turn.refreshAfterMs
    }

    suspend fun recoverIce() = lock.withLock {
        check(!closed.get()) { "Voice call ended." }
        restartIceLocked(requireNotNull(turn).generation)
    }

    private suspend fun restartIceLocked(generation: String) {
        val current = peer ?: error("Voice transport is unavailable.")
        val token = mediaToken ?: error("Voice session is unavailable.")
        current.restartIce()
        current.setLocalDescriptionAwait(current.createOfferAwait())
        current.awaitIceGathering()
        val local = current.localDescription ?: error("WebRTC did not create an ICE restart offer.")
        val sequence = restartSequence
        val body = buildJsonObject {
            put("generation", generation); put("sequence", sequence)
            putJsonObject("sessionDescription") { put("type", "offer"); put("sdp", local.description) }
        }
        val response: SignalResponse = retryIceRequest { media("restart-ice", body, token) }
        val answer = response.sessionDescription ?: error("Media service did not answer ICE restart.")
        current.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.ANSWER, answer.sdp))
        retryIceRequest<Unit> { media("restart-ice-ack", buildJsonObject { put("generation", generation); put("sequence", sequence) }, token) }
        restartSequence++
    }

    private suspend fun <T> retryIceRequest(block: suspend () -> T): T {
        val waits = longArrayOf(1_000, 2_000, 4_000, 8_000, 15_000, 30_000)
        var attempt = 0
        while (true) {
            try { return block() } catch (error: Throwable) {
                val retry = error !is ApiException || error.status in setOf(408, 429, 500, 502, 503, 504) ||
                    (error.status == 409 && error.code == "ice_restart_pending")
                if (!retry || error is ApiException && error.code == "ice_restart_invalid") throw error
                delay(waits[attempt.coerceAtMost(waits.lastIndex)])
                attempt++
            }
        }
    }

    suspend fun disconnect() {
        val token = closeLocal()
        leave(token)
    }

    @Synchronized fun closeLocal(): String? {
        closed.set(true)
        connected.set(false)
        val token = mediaToken
        mediaToken = null
        microphone?.setEnabled(false)
        microphone?.dispose(); microphone = null
        source?.dispose(); source = null
        peer?.close(); peer?.dispose(); peer = null
        factory?.dispose(); factory = null
        subscriptions.clear()
        synchronized(remoteAudio) { remoteAudio.clear() }
        return token
    }

    suspend fun leave(token: String?) {
        if (token != null) runCatching { media<Unit>("leave", mediaToken = token) }
    }

    private suspend inline fun <reified T> media(
        operation: String,
        body: kotlinx.serialization.json.JsonObject = buildJsonObject {},
        mediaToken: String? = null,
    ): T = api.media(accountToken, channelId, operation, body, mediaToken, demo)

    private val observer = object : PeerConnection.Observer {
        override fun onSignalingChange(state: PeerConnection.SignalingState?) = Unit
        override fun onIceConnectionChange(state: PeerConnection.IceConnectionState?) = Unit
        override fun onStandardizedIceConnectionChange(newState: PeerConnection.IceConnectionState?) = Unit
        override fun onConnectionChange(newState: PeerConnection.PeerConnectionState?) {
            if (newState != null) {
                connectionState.value = newState
                onTransportState(newState)
            }
        }
        override fun onIceConnectionReceivingChange(receiving: Boolean) = Unit
        override fun onIceGatheringChange(state: PeerConnection.IceGatheringState?) = Unit
        override fun onIceCandidate(candidate: IceCandidate?) = Unit
        override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>?) = Unit
        override fun onAddStream(stream: MediaStream?) = stream?.audioTracks?.forEach {
            remoteAudio += it
            it.setEnabled(!deafened)
        } ?: Unit
        override fun onRemoveStream(stream: MediaStream?) = Unit
        override fun onDataChannel(channel: DataChannel?) = Unit
        override fun onRenegotiationNeeded() = Unit
        override fun onAddTrack(receiver: RtpReceiver?, streams: Array<out MediaStream>?) {
            (receiver?.track() as? AudioTrack)?.let { remoteAudio += it; it.setEnabled(!deafened) }
        }
        override fun onTrack(transceiver: RtpTransceiver?) {
            (transceiver?.receiver?.track() as? AudioTrack)?.let { remoteAudio += it; it.setEnabled(!deafened) }
        }
    }
}

private suspend fun PeerConnection.createOfferAwait(): RtcSessionDescription = sdp { createOffer(it, MediaConstraints()) }
private suspend fun PeerConnection.createAnswerAwait(): RtcSessionDescription = sdp { createAnswer(it, MediaConstraints()) }
private suspend fun PeerConnection.sdp(start: (SdpObserver) -> Unit): RtcSessionDescription =
    kotlinx.coroutines.suspendCancellableCoroutine { continuation ->
        start(object : SdpObserver {
            override fun onCreateSuccess(value: RtcSessionDescription) = continuation.resume(value)
            override fun onCreateFailure(error: String) = continuation.resumeWithException(IllegalStateException(error))
            override fun onSetSuccess() = Unit
            override fun onSetFailure(error: String) = Unit
        })
    }
private suspend fun PeerConnection.setLocalDescriptionAwait(value: RtcSessionDescription) = setDescription(true, value)
private suspend fun PeerConnection.setRemoteDescriptionAwait(value: RtcSessionDescription) = setDescription(false, value)
private suspend fun PeerConnection.setDescription(local: Boolean, value: RtcSessionDescription) =
    kotlinx.coroutines.suspendCancellableCoroutine { continuation ->
        val callback = object : SdpObserver {
            override fun onSetSuccess() = continuation.resume(Unit)
            override fun onSetFailure(error: String) = continuation.resumeWithException(IllegalStateException(error))
            override fun onCreateSuccess(value: RtcSessionDescription?) = Unit
            override fun onCreateFailure(error: String?) = Unit
        }
        if (local) setLocalDescription(callback, value) else setRemoteDescription(callback, value)
    }
private suspend fun PeerConnection.awaitIceGathering() {
    repeat(100) {
        if (iceGatheringState() == PeerConnection.IceGatheringState.COMPLETE) return
        delay(50)
    }
    throw IllegalStateException("Timed out gathering ICE candidates.")
}
