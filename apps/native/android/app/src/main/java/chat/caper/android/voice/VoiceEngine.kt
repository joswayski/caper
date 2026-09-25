package chat.caper.android.voice

import android.content.Context
import chat.caper.android.data.CaperApi
import chat.caper.android.data.ApiException
import chat.caper.android.model.*
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.cancellation.CancellationException
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
    private val resources = VoiceResourceGate()
    private val connected = AtomicBoolean(false)
    private var factory: PeerConnectionFactory? = null
    private var peer: PeerConnection? = null
    private lateinit var rtcConfiguration: PeerConnection.RTCConfiguration
    private var source: AudioSource? = null
    private var microphone: AudioTrack? = null
    private var capture: AudioCapture? = null
    private val audioPreferences = appContext.getSharedPreferences("audio", Context.MODE_PRIVATE)
    private var mediaToken: String? = null
    private var selfId: String? = null
    private val remoteLock = Any()
    private val subscriptions = mutableMapOf<String, String>()
    private val participantForTrack = mutableMapOf<String, String>()
    private val remoteAudio = mutableSetOf<AudioTrack>()
    private val remoteAudioByMid = mutableMapOf<String, MutableSet<AudioTrack>>()
    private val remoteAudioByParticipant = mutableMapOf<String, MutableSet<AudioTrack>>()
    private val remoteParticipantForAudio = mutableMapOf<AudioTrack, String>()
    private var peerDisposing = false // guarded by remoteLock
    private val participantVolumes = mutableMapOf<String, Int>()
    private val locallyMutedParticipants = mutableSetOf<String>()
    private var outputVolume = audioPreferences.getInt("outputVolume", 100).coerceIn(0, 200)
    private val localMute = VoiceLocalMute(lock, { syncStateLocked() }) { muted, _ ->
        resources.use {
            capture?.publication(connected.get() && !muted)
            microphone?.setEnabled(connected.get() && !muted && capture?.isTesting != true)
        }
        applyRemoteAudioPreferences()
    }
    private var previousStats: Triple<Long, Long, Long>? = null
    private val connectionState = MutableStateFlow(PeerConnection.PeerConnectionState.NEW)
    private var turn: TurnGeneration? = null
    private var stateSequence = 0L
    private val restarts = IceRestartLedger()
    val muted get() = localMute.muted
    val deafened get() = localMute.deafened

    internal fun copyAudioIntentFrom(previous: VoiceEngine) { localMute.copyFrom(previous.localMute) }
    fun setInputGain(value: Int) { capture?.gain(value) }
    fun setProcessingStrength(value: Int) { capture?.processingStrength(value) }
    fun processingReport(): LongArray = capture?.report() ?: longArrayOf()
    fun invalidateCapture() { localMute.withCurrent { muted ->
        resources.use { capture?.publication(connected.get() && !muted) }
    } }
    fun beginMicComparison() { resources.use {
        capture?.beginComparison()
        microphone?.setEnabled(false)
    } }
    internal fun finishMicComparison(): MicComparison? = resources.use {
        capture?.endComparison()
    }
    fun resumeAfterMicComparison() { localMute.withCurrent { muted ->
        resources.use {
            capture?.resumePublication()
            capture?.publication(connected.get() && !muted)
            microphone?.setEnabled(connected.get() && !muted)
        }
    } }

    suspend fun connect(onParticipants: (List<Participant>) -> Unit) = withContext(Dispatchers.IO) {
        try {
            // Model copy/warmup can take seconds and must not hold the resource
            // gate needed by synchronous stop on Main.
            val processor = AudioCapture.prepare(appContext)
            try { resources.use {
                PeerConnectionFactory.initialize(
                    PeerConnectionFactory.InitializationOptions.builder(appContext).createInitializationOptions(),
                )
                processor.gain(audioPreferences.getInt("inputGain", 100))
                processor.processingStrength(audioPreferences.getInt("strength", 25))
                capture = processor
                val audioModule = JavaAudioDeviceModule.builder(appContext)
                    .setInputSampleRate(48000)
                    .setAudioBufferCallback { buffer, format, channels, rate, read, time ->
                        processor.onBuffer(buffer, format, channels, rate, read)
                        time
                    }.createAudioDeviceModule()
                try { factory = PeerConnectionFactory.builder().setAudioDeviceModule(audioModule).createPeerConnectionFactory() }
                finally { audioModule.release() }
            } } catch (error: Throwable) {
                if (capture !== processor) processor.close()
                throw error
            }
            // A canceled HTTP join can have committed at the server without
            // delivering its token. Wait for the bounded call to finish, then
            // either publish the token or leave it after a local stop.
            val joined: JoinResponse? = withContext(NonCancellable) {
                val result: JoinResponse = media("join", buildJsonObject { put("name", displayName); put("muted", muted); put("deafened", deafened) })
                if (resources.acceptToken(result.token) { mediaToken = it; selfId = result.id; turn = result.turn }) result
                else {
                    // Stop preceded response: closeLocal never saw this token.
                    leave(result.token)
                    null
                }
            }
            if (joined == null) return@withContext
            val servers = joined.iceServers.map { server ->
                PeerConnection.IceServer.builder(server.urls)
                    .apply { if (server.username != null) setUsername(server.username); if (server.credential != null) setPassword(server.credential) }
                    .createIceServer()
            }
            val (current, transceiver) = resources.use {
                rtcConfiguration = PeerConnection.RTCConfiguration(servers)
                val connection = factory!!.createPeerConnection(rtcConfiguration, observer)
                    ?: error("Could not create voice connection.")
                peer = connection
                source = factory!!.createAudioSource(MediaConstraints())
                // Capture begins silent and stays disabled until connected.
                microphone = factory!!.createAudioTrack("caper-microphone", source).apply { setEnabled(false) }
                connection to connection.addTransceiver(
                    microphone,
                    RtpTransceiver.RtpTransceiverInit(RtpTransceiver.RtpTransceiverDirection.SEND_ONLY, listOf("caper")),
                )
            }
            current.setLocalDescriptionAwait(current.createOfferAwait(resources), resources)
            val mid = resources.use { transceiver.mid ?: error("WebRTC did not assign a microphone MID.") }
            val localSdp = current.currentLocalSdp(RtcSessionDescription.Type.OFFER, resources)
            val published: SignalResponse = media(
                "publish",
                buildJsonObject {
                    put("kind", "microphone"); put("mid", mid)
                    putJsonObject("sessionDescription") { put("type", "offer"); put("sdp", localSdp) }
                }, joined.token,
            )
            val answer = published.sessionDescription ?: error("Media service did not answer publication.")
            current.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.ANSWER, answer.sdp), resources)
            withTimeout(20_000) {
                connectionState.first { it == PeerConnection.PeerConnectionState.CONNECTED || it == PeerConnection.PeerConnectionState.FAILED }
            }
            check(connectionState.value == PeerConnection.PeerConnectionState.CONNECTED) { "Voice transport failed to connect." }
            // The SFU session must be connected before requesting remote tracks.
            reconcile(onParticipants)
            lock.withLock {
                localMute.withCurrent { currentMute ->
                    resources.use {
                        connected.set(true)
                        capture?.publication(!currentMute)
                        microphone?.setEnabled(!currentMute)
                    }
                }
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
        reconcile(snapshot, onParticipants, token)
    }

    suspend fun applySnapshot(snapshot: MediaSnapshot, onParticipants: (List<Participant>) -> Unit) = lock.withLock {
        reconcile(snapshot, onParticipants, mediaToken ?: return@withLock)
    }

    private suspend fun reconcile(snapshot: MediaSnapshot, onParticipants: (List<Participant>) -> Unit, token: String) {
        check(resources.isOpen) { "Voice call ended." }
        onParticipants(snapshot.participants)
        synchronized(remoteLock) {
            check(resources.isOpen) { "Voice call ended." }
            participantForTrack.clear()
            snapshot.participants.forEach { participant -> participant.tracks.forEach { participantForTrack[it.id] = participant.id } }
            subscriptions.forEach { (trackId, mid) -> associateRemoteLocked(trackId, mid) }
        }
        val wanted = snapshot.participants
            .filter { it.id != selfId }
            .flatMap { it.tracks }
            .filter { it.kind == "microphone" }
            .map { it.id }
            .toSet()
        val currentSubscriptions = synchronized(remoteLock) { subscriptions.toMap() }
        for ((track, mid) in currentSubscriptions) {
            if (track !in wanted) {
                closeDepartedSubscription(
                    silence = { synchronized(remoteLock) { if (resources.isOpen) silenceSubscriptionLocked(mid) } },
                    close = { media<Unit>("close", buildJsonObject { put("mid", mid) }, token) },
                    forget = { synchronized(remoteLock) { if (resources.isOpen) removeSubscriptionLocked(track, mid) } },
                )
            }
        }
        val subscribedTracks = synchronized(remoteLock) { subscriptions.keys.toSet() }
        for (track in wanted - subscribedTracks) subscribe(track, token)
    }

    fun eventToken(): String? = resources.use { mediaToken }
    fun selfParticipantId(): String? = selfId

    private suspend fun subscribe(trackId: String, token: String) {
        val response: SignalResponse = try {
            media("subscribe", buildJsonObject { put("trackId", trackId) }, token)
        } catch (error: ApiException) {
            if (departedTrack(error)) return
            throw error
        }
        val mid = subscriptionMid(response)
        synchronized(remoteLock) {
            check(resources.isOpen) { "Voice call ended during subscription." }
            subscriptions[trackId] = mid
            associateRemoteLocked(trackId, mid)
        }
        response.sessionDescription?.let { offer ->
            val current = resources.use { peer ?: error("Voice transport is unavailable.") }
            current.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.OFFER, offer.sdp), resources)
            current.setLocalDescriptionAwait(current.createAnswerAwait(resources), resources)
            val localSdp = current.currentLocalSdp(RtcSessionDescription.Type.ANSWER, resources)
            media<Unit>(
                "negotiate",
                buildJsonObject { putJsonObject("sessionDescription") { put("type", "answer"); put("sdp", localSdp) } }, token,
            )
        }
    }

    suspend fun setMuted(value: Boolean, onLocalApplied: () -> Unit = {}) = localMute.setMuted(value, onLocalApplied)

    suspend fun setDeafened(value: Boolean, onLocalApplied: () -> Unit = {}) = localMute.setDeafened(value, onLocalApplied)

    fun setOutputVolume(value: Int) = synchronized(remoteLock) {
        outputVolume = value.coerceIn(0, 200)
        applyRemoteAudioPreferencesLocked()
    }

    suspend fun setParticipantVolume(id: String, value: Int) = lock.withLock {
        synchronized(remoteLock) {
            participantVolumes[id] = value.coerceIn(0, 200)
            applyRemoteAudioPreferencesLocked(id)
        }
    }

    suspend fun setParticipantLocallyMuted(id: String, value: Boolean) = lock.withLock {
        synchronized(remoteLock) {
            if (value) locallyMutedParticipants += id else locallyMutedParticipants -= id
            applyRemoteAudioPreferencesLocked(id)
        }
    }

    private fun applyRemoteAudioPreferences(onlyParticipant: String? = null) {
        synchronized(remoteLock) { applyRemoteAudioPreferencesLocked(onlyParticipant) }
    }

    private fun applyRemoteAudioPreferencesLocked(onlyParticipant: String? = null) {
        remoteAudioByParticipant.forEach { (participant, tracks) ->
            if (onlyParticipant == null || onlyParticipant == participant) {
                val preference = remoteAudioPreference(
                    mapped = true,
                    deafened = deafened,
                    locallyMuted = participant in locallyMutedParticipants,
                    outputVolume = outputVolume,
                    participantVolume = participantVolumes[participant] ?: 100,
                )
                tracks.forEach { track ->
                    track.setVolume(preference.gain)
                    track.setEnabled(preference.enabled)
                }
            }
        }
    }

    suspend fun diagnostics(): VoiceDiagnostics = kotlinx.coroutines.suspendCancellableCoroutine { continuation ->
        resources.use {
            val current = peer ?: error("Voice transport is unavailable.")
            current.getStats { report ->
            if (!resources.isOpen) return@getStats
            val stats = report.statsMap.values
            fun numbers(type: String, key: String) = stats.filter { it.type == type }.mapNotNull { (it.members[key] as? Number)?.toLong() }
            val received = numbers("inbound-rtp", "bytesReceived").sum()
            val sent = numbers("outbound-rtp", "bytesSent").sum()
            val lost = numbers("inbound-rtp", "packetsLost").sum()
            val jitter = stats.filter { it.type == "inbound-rtp" }.mapNotNull { (it.members["jitter"] as? Number)?.toDouble() }.maxOrNull()?.times(1_000)?.toLong() ?: 0
            val rtt = stats.flatMap { stat -> listOf("roundTripTime", "currentRoundTripTime").mapNotNull { (stat.members[it] as? Number)?.toDouble() } }.maxOrNull()?.times(1_000)?.toLong() ?: 0
            val selected = stats.firstOrNull { it.type == "candidate-pair" && (it.members["selected"] == true || it.members["nominated"] == true) }
            val localId = selected?.members?.get("localCandidateId") as? String
            val route = when (stats.firstOrNull { it.id == localId }?.members?.get("candidateType")) { "relay" -> "relay"; null -> "unknown"; else -> "direct" }
            val now = (report.timestampUs / 1_000).toLong()
            val previous = previousStats
            val elapsed = previous?.let { (now - it.first).coerceAtLeast(1) }
            previousStats = Triple(now, received, sent)
            val receiveRate = if (previous == null || elapsed == null) 0 else ((received - previous.second).coerceAtLeast(0) * 8_000 / elapsed)
            val sendRate = if (previous == null || elapsed == null) 0 else ((sent - previous.third).coerceAtLeast(0) * 8_000 / elapsed)
            if (continuation.isActive) continuation.resume(VoiceDiagnostics(received, receiveRate, sent, sendRate, lost, jitter, rtt, route))
            }
        }
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
        val servers = response.iceServers.map { server ->
            PeerConnection.IceServer.builder(server.urls)
                .apply { if (server.username != null) setUsername(server.username); if (server.credential != null) setPassword(server.credential) }
                .createIceServer()
        }
        resources.use {
            rtcConfiguration.iceServers = servers
            check(peer?.setConfiguration(rtcConfiguration) == true) { "Could not refresh TURN credentials." }
        }
        withTimeout(20_000) {
            restarts.pending?.let { restartIceLocked(it.generation) }
            if (response.turn.generation != old.generation) restartIceLocked(response.turn.generation)
        }
        turn = response.turn
        return response.turn.refreshAfterMs
    }

    fun turnRefreshAfterMs(): Long? = turn?.refreshAfterMs

    suspend fun recoverIce() = lock.withLock {
        check(resources.isOpen) { "Voice call ended." }
        if (connectionState.value == PeerConnection.PeerConnectionState.CONNECTED && restarts.pending == null) return@withLock
        restartIceLocked(restarts.pending?.generation ?: requireNotNull(turn).generation)
    }

    private suspend fun restartIceLocked(generation: String) {
        val current = resources.use { peer ?: error("Voice transport is unavailable.") }
        val token = resources.use { mediaToken ?: error("Voice session is unavailable.") }
        val pending = restarts.pending ?: run {
            resources.use { current.restartIce() }
            current.setLocalDescriptionAwait(current.createOfferAwait(resources), resources)
            restarts.stage(generation, current.currentLocalSdp(RtcSessionDescription.Type.OFFER, resources))
        }
        check(pending.generation == generation) { "Previous ICE restart must be acknowledged first." }
        if (!pending.answerApplied) {
            val answerSdp = pending.answerSdp ?: run {
                val response: SignalResponse = retryIceRequest { media("restart-ice", pending.offer(), token) }
                val answer = response.sessionDescription ?: error("Media service did not answer ICE restart.")
                restarts.recordAnswer(pending, answer.sdp)
                answer.sdp
            }
            // A canceled WebRTC callback may still have applied the answer.
            // Never replace the pending offer or apply the same answer twice.
            if (resources.use { current.remoteDescription?.description } != answerSdp) {
                current.setRemoteDescriptionAwait(RtcSessionDescription(RtcSessionDescription.Type.ANSWER, answerSdp), resources)
            }
            restarts.answerApplied(pending)
        }
        retryIceRequest<Unit> { media("restart-ice-ack", pending.ack(), token) }
        restarts.acknowledged(pending)
    }

    private suspend fun <T> retryIceRequest(block: suspend () -> T): T {
        val waits = longArrayOf(1_000, 2_000, 4_000, 8_000, 15_000, 30_000)
        var attempt = 0
        while (true) {
            try { return block() } catch (error: Throwable) {
                if (error is CancellationException) throw error
                if (!retryableIceRestart(error)) throw error
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
        var token: String? = null
        resources.close {
            connected.set(false)
            capture?.publication(false)
            token = mediaToken
            mediaToken = null
            // An invalidated native wrapper must not strand the microphone or
            // prevent disposal of the remaining peer/factory resources.
            runCatching { microphone?.setEnabled(false) }
            runCatching { microphone?.dispose() }; microphone = null
            runCatching { source?.dispose() }; source = null
            synchronized(remoteLock) {
                remoteAudio.forEach { track ->
                    runCatching { track.setVolume(0.0) }
                    runCatching { track.setEnabled(false) }
                }
                subscriptions.clear(); participantForTrack.clear(); remoteAudio.clear(); remoteAudioByMid.clear()
                remoteAudioByParticipant.clear(); remoteParticipantForAudio.clear(); participantVolumes.clear(); locallyMutedParticipants.clear()
            }
            runCatching { peer?.close() }
            // close() stops transport; only now may callbacks skip receiver
            // wrappers, which dispose() can invalidate on another RTC thread.
            synchronized(remoteLock) { peerDisposing = true }
            runCatching { peer?.dispose() }; peer = null
            runCatching { factory?.dispose() }; factory = null
            runCatching { capture?.close() }; capture = null
        }
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
                if (resources.isOpen) {
                    connectionState.value = newState
                    onTransportState(newState)
                }
            }
        }
        override fun onIceConnectionReceivingChange(receiving: Boolean) = Unit
        override fun onIceGatheringChange(state: PeerConnection.IceGatheringState?) = Unit
        override fun onIceCandidate(candidate: IceCandidate?) = Unit
        override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>?) = Unit
        override fun onAddStream(stream: MediaStream?) = stream?.audioTracks?.forEach { registerRemote(null, it) } ?: Unit
        override fun onRemoveStream(stream: MediaStream?) = Unit
        override fun onDataChannel(channel: DataChannel?) = Unit
        override fun onRenegotiationNeeded() = Unit
        override fun onAddTrack(receiver: RtpReceiver?, streams: Array<out MediaStream>?) {
            (receiver?.track() as? AudioTrack)?.let { registerRemote(null, it) }
        }
        override fun onTrack(transceiver: RtpTransceiver?) {
            (transceiver?.receiver?.track() as? AudioTrack)?.let { registerRemote(transceiver.mid, it) }
        }
    }

    private fun registerRemote(mid: String?, track: AudioTrack) {
        synchronized(remoteLock) {
            // closeLocal sets its closed marker before peer disposal. Callbacks
            // in that interval must be silenced, but callbacks during/after
            // dispose must never dereference a receiver-owned wrapper.
            if (!acceptRemoteCallback(peerDisposing, {
                runCatching { track.setVolume(0.0) }
                runCatching { track.setEnabled(false) }
            }, { resources.isOpen })) return
            remoteAudio += track
            if (mid != null) {
                remoteAudioByMid.getOrPut(mid) { mutableSetOf() } += track
                subscriptions.entries.firstOrNull { it.value == mid }?.let { (trackId, _) -> associateRemoteLocked(trackId, mid) }
            }
        }
    }

    private fun associateRemoteLocked(trackId: String, mid: String) {
        val participant = participantForTrack[trackId] ?: return
        val tracks = remoteAudioByMid[mid] ?: return
        tracks.forEach { track ->
            remoteParticipantForAudio[track]?.let { previous -> remoteAudioByParticipant[previous]?.remove(track) }
            remoteParticipantForAudio[track] = participant
            remoteAudioByParticipant.getOrPut(participant) { mutableSetOf() } += track
        }
        applyRemoteAudioPreferencesLocked(participant)
    }

    private fun removeSubscriptionLocked(trackId: String, mid: String) {
        subscriptions.remove(trackId)
        participantForTrack.remove(trackId)
        remoteAudioByMid.remove(mid)?.forEach { track ->
            remoteParticipantForAudio.remove(track)?.let { participant -> remoteAudioByParticipant[participant]?.remove(track) }
            remoteAudio.remove(track)
        }
    }

    private fun silenceSubscriptionLocked(mid: String) {
        remoteAudioByMid[mid]?.forEach { track ->
            runCatching { track.setVolume(0.0) }
            runCatching { track.setEnabled(false) }
            remoteParticipantForAudio.remove(track)?.let { participant -> remoteAudioByParticipant[participant]?.remove(track) }
        }
    }
}

/** Keep the MID after a transient close failure so the next roster pass retries it. */
internal suspend fun closeDepartedSubscription(
    silence: () -> Unit,
    close: suspend () -> Unit,
    forget: () -> Unit,
) {
    silence()
    try { close() } catch (error: Throwable) {
        if (error is CancellationException || !transientVoiceControlError(error)) throw error
        return
    }
    forget()
}

internal fun acceptRemoteCallback(disposing: Boolean, silence: () -> Unit, isOpen: () -> Boolean): Boolean {
    if (disposing) return false
    silence()
    return isOpen()
}

internal data class RemoteAudioPreference(val gain: Double, val enabled: Boolean)
internal fun remoteAudioPreference(
    mapped: Boolean,
    deafened: Boolean,
    locallyMuted: Boolean,
    outputVolume: Int,
    participantVolume: Int,
): RemoteAudioPreference = if (!mapped) RemoteAudioPreference(0.0, false) else RemoteAudioPreference(
    gain = outputVolume.coerceIn(0, 200) * participantVolume.coerceIn(0, 200) / 10_000.0,
    enabled = !deafened && !locallyMuted,
)

internal class VoiceMuteIntent(initiallyMuted: Boolean = true) {
    var muted = initiallyMuted
        private set
    var deafened = false
        private set
    private var beforeDeafen = initiallyMuted

    fun copy() = VoiceMuteIntent(muted).also {
        it.deafened = deafened
        it.beforeDeafen = beforeDeafen
    }

    fun setMuted(value: Boolean) {
        muted = value
        if (!value) {
            deafened = false
            beforeDeafen = false
        }
    }

    fun setDeafened(value: Boolean) {
        if (value == deafened) return
        if (value) { beforeDeafen = muted; muted = true } else muted = beforeDeafen
        deafened = value
    }
}

/** Local tracks change before waiting on serialized, potentially slow signaling. */
internal class VoiceLocalMute(
    private val signaling: Mutex,
    private val sync: suspend () -> Unit,
    private val apply: (Boolean, Boolean) -> Unit,
) {
    private var intent = VoiceMuteIntent()
    @Volatile var muted = true
        private set
    @Volatile var deafened = false
        private set

    // Used before the replacement engine connects. Do not share mutable intent
    // or carry any old peer, media capability, PCM, or comparison state.
    fun copyFrom(previous: VoiceLocalMute) {
        val snapshot = synchronized(previous) { previous.intent.copy() }
        synchronized(this) {
            intent = snapshot
            muted = intent.muted
            deafened = intent.deafened
        }
    }

    suspend fun setMuted(value: Boolean, onLocalApplied: () -> Unit = {}) {
        synchronized(this) {
            intent.setMuted(value)
            muted = intent.muted
            deafened = intent.deafened
            apply(muted, deafened)
        }
        onLocalApplied()
        signaling.withLock { sync() }
    }

    suspend fun setDeafened(value: Boolean, onLocalApplied: () -> Unit = {}) {
        synchronized(this) {
            intent.setDeafened(value)
            deafened = intent.deafened
            muted = intent.muted
            apply(muted, deafened)
        }
        onLocalApplied()
        signaling.withLock { sync() }
    }

    fun withCurrent(block: (Boolean) -> Unit) = synchronized(this) { block(muted) }
}

/** Serializes short JNI entry and synchronous teardown; never holds its monitor across an await. */
internal class VoiceResourceGate {
    private val monitor = Any()
    @Volatile var isOpen = true
        private set

    fun <T> use(block: () -> T): T = synchronized(monitor) {
        check(isOpen) { "Voice call ended." }
        block()
    }

    fun acceptToken(token: String, accept: (String) -> Unit): Boolean = synchronized(monitor) {
        if (!isOpen) false else { accept(token); true }
    }

    fun close(dispose: () -> Unit) = synchronized(monitor) {
        if (isOpen) { isOpen = false; dispose() }
    }
}

internal data class PendingIceRestart(
    val generation: String,
    val sequence: Long,
    val sdp: String,
    var answerSdp: String? = null,
    var answerApplied: Boolean = false,
) {
    fun offer() = buildJsonObject {
        put("generation", generation); put("sequence", sequence)
        putJsonObject("sessionDescription") { put("type", "offer"); put("sdp", sdp) }
    }
    fun ack() = buildJsonObject { put("generation", generation); put("sequence", sequence) }
}

internal class IceRestartLedger {
    var pending: PendingIceRestart? = null
        private set
    private var nextSequence = 1L

    fun stage(generation: String, sdp: String): PendingIceRestart {
        check(pending == null)
        return PendingIceRestart(generation, nextSequence, sdp).also { pending = it }
    }

    fun recordAnswer(value: PendingIceRestart, sdp: String) { check(pending === value); value.answerSdp = sdp }
    fun answerApplied(value: PendingIceRestart) { check(pending === value); value.answerApplied = true }
    fun acknowledged(value: PendingIceRestart) { check(pending === value); pending = null; nextSequence++ }
}

internal fun retryableIceRestart(error: Throwable): Boolean = when (error) {
    is ApiException -> error.code != "ice_restart_invalid" && (
        error.code == "ice_restart_retry" || error.status in setOf(408, 429, 500, 502, 503, 504) ||
            (error.status == 409 && error.code == "ice_restart_pending")
    )
    is java.io.IOException -> true
    else -> false
}

internal fun subscriptionMid(response: SignalResponse): String {
    if (response.sessionDescription == null && response.requiresImmediateRenegotiation) {
        error("Media service requested negotiation without an offer.")
    }
    return response.tracks.firstOrNull()?.mid ?: error("Media service did not identify the remote track.")
}

internal fun departedTrack(error: ApiException) = error.status == 404 && error.code == "track_gone"

private suspend fun PeerConnection.createOfferAwait(resources: VoiceResourceGate): RtcSessionDescription =
    sdp { resources.use { createOffer(it, MediaConstraints()) } }
private suspend fun PeerConnection.createAnswerAwait(resources: VoiceResourceGate): RtcSessionDescription =
    sdp { resources.use { createAnswer(it, MediaConstraints()) } }
private suspend fun PeerConnection.sdp(start: (SdpObserver) -> Unit): RtcSessionDescription =
    kotlinx.coroutines.suspendCancellableCoroutine { continuation ->
        start(object : SdpObserver {
            override fun onCreateSuccess(value: RtcSessionDescription) = continuation.resume(value)
            override fun onCreateFailure(error: String) = continuation.resumeWithException(IllegalStateException(error))
            override fun onSetSuccess() = Unit
            override fun onSetFailure(error: String) = Unit
        })
    }
private suspend fun PeerConnection.setLocalDescriptionAwait(value: RtcSessionDescription, resources: VoiceResourceGate) = setDescription(true, value, resources)
private suspend fun PeerConnection.setRemoteDescriptionAwait(value: RtcSessionDescription, resources: VoiceResourceGate) = setDescription(false, value, resources)
private suspend fun PeerConnection.setDescription(local: Boolean, value: RtcSessionDescription, resources: VoiceResourceGate) =
    kotlinx.coroutines.suspendCancellableCoroutine { continuation ->
        val callback = object : SdpObserver {
            override fun onSetSuccess() = continuation.resume(Unit)
            override fun onSetFailure(error: String) = continuation.resumeWithException(IllegalStateException(error))
            override fun onCreateSuccess(value: RtcSessionDescription?) = Unit
            override fun onCreateFailure(error: String?) = Unit
        }
        resources.use { if (local) setLocalDescription(callback, value) else setRemoteDescription(callback, value) }
    }
private fun PeerConnection.currentLocalSdp(type: RtcSessionDescription.Type, resources: VoiceResourceGate): String = resources.use {
    // Exchange the pending SDP as soon as setLocalDescription succeeds; optional
    // ICE probes continue in parallel. Connectivity is checked after answer.
    validatedLocalSdp(localDescription, type)
}

internal fun validatedLocalSdp(local: RtcSessionDescription?, type: RtcSessionDescription.Type): String {
    requireNotNull(local) { "WebRTC did not produce a local description." }
    check(local.type == type && local.description.isNotBlank()) { "WebRTC produced an invalid local description." }
    return local.description
}
