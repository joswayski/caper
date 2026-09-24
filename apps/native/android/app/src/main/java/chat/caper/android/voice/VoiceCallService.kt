package chat.caper.android.voice

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import chat.caper.android.MainActivity
import chat.caper.android.BuildConfig
import chat.caper.android.R
import chat.caper.android.data.CaperApi
import chat.caper.android.data.ApiException
import chat.caper.android.data.TokenStore
import chat.caper.android.model.Participant
import chat.caper.android.model.AudioRoute
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import java.io.IOException

data class VoiceState(
    val phase: Phase = Phase.IDLE,
    val channelId: String? = null,
    val spaceId: String? = null,
    val channelName: String? = null,
    val spaceName: String? = null,
    val participants: List<Participant> = emptyList(),
    val selfId: String? = null,
    val muted: Boolean = false,
    val deafened: Boolean = false,
    val outputVolume: Int = 100,
    val participantVolumes: Map<String, Int> = emptyMap(),
    val locallyMutedParticipants: Set<String> = emptySet(),
    val diagnostics: VoiceDiagnostics? = null,
    val routes: List<AudioRoute> = emptyList(),
    val selectedRouteId: Int? = null,
    val error: String? = null,
) { enum class Phase { IDLE, CONNECTING, CONNECTED, RECONNECTING, FAILED } }

class VoiceCallService : Service() {
    // Android service lifecycle and explicit stop run on Main. Resume all
    // asynchronous results here too, so ownership checks and state commits
    // cannot interleave with a synchronous stop/replacement.
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val attempts = CallAttemptGate()
    private var engine: VoiceEngine? = null
    private var activeAttempt: Long? = null
    private var connectJob: Job? = null
    private var heartbeat: Job? = null
    private var turnRenewal: Job? = null
    private var recovery: Job? = null
    private var mediaEvents: MediaEventClient? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private lateinit var audio: AudioManager
    private var focus: AudioFocusRequest? = null
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) { scope.launch { updateRoutes() } }
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) { scope.launch { updateRoutes() } }
    }

    override fun onCreate() {
        super.onCreate()
        active = this
        createNotificationChannel()
        audio = getSystemService(AudioManager::class.java)
        audio.registerAudioDeviceCallback(deviceCallback, null)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!BuildConfig.ENABLE_NATIVE_VOICE) {
            stopSelf()
            return START_NOT_STICKY
        }
        when (intent?.action) {
            ACTION_START -> startCall(
                requireNotNull(intent.getStringExtra(EXTRA_CHANNEL_ID)),
                requireNotNull(intent.getStringExtra(EXTRA_SPACE_ID)),
                requireNotNull(intent.getStringExtra(EXTRA_CHANNEL_NAME)),
                requireNotNull(intent.getStringExtra(EXTRA_SPACE_NAME)),
                requireNotNull(intent.getStringExtra(EXTRA_DISPLAY_NAME)),
                intent.getBooleanExtra(EXTRA_DEMO, false),
            )
            ACTION_MUTE -> scope.launch {
                val (current, attempt) = currentCall() ?: return@launch
                runCatching { current.setMuted(!current.muted) {
                    commitCallResult(current, attempt) { it.copy(muted = current.muted) }
                } }
                    .onSuccess { commitCallResult(current, attempt) { it.copy(muted = current.muted, error = null) } }
                    .onFailure { localControlFailed(current, attempt, it, "Mute is local; voice status will retry.") }
            }
            ACTION_DEAFEN -> scope.launch {
                val (current, attempt) = currentCall() ?: return@launch
                runCatching { current.setDeafened(!current.deafened) {
                    commitCallResult(current, attempt) { it.copy(deafened = current.deafened, muted = current.muted) }
                } }
                    .onSuccess { commitCallResult(current, attempt) { it.copy(deafened = current.deafened, muted = current.muted, error = null) } }
                    .onFailure { localControlFailed(current, attempt, it, "Deafen is local; voice status will retry.") }
            }
            ACTION_STOP -> stopCall()
            ACTION_ROUTE -> selectRoute(intent.getIntExtra(EXTRA_ROUTE_ID, -1))
            ACTION_OUTPUT_VOLUME -> scope.launch {
                val value = intent.getIntExtra(EXTRA_VOLUME, 100).coerceIn(0, 200)
                val (current, attempt) = currentCall() ?: return@launch
                current.setOutputVolume(value)
                commitCallResult(current, attempt) { it.copy(outputVolume = value) }
            }
            ACTION_PARTICIPANT_VOLUME -> scope.launch {
                val id = intent.getStringExtra(EXTRA_PARTICIPANT_ID) ?: return@launch
                val value = intent.getIntExtra(EXTRA_VOLUME, 100).coerceIn(0, 200)
                val (current, attempt) = currentCall() ?: return@launch
                current.setParticipantVolume(id, value)
                commitCallResult(current, attempt) { it.copy(participantVolumes = it.participantVolumes + (id to value)) }
            }
            ACTION_PARTICIPANT_MUTE -> scope.launch {
                val id = intent.getStringExtra(EXTRA_PARTICIPANT_ID) ?: return@launch
                val muted = id !in state.value.locallyMutedParticipants
                val (current, attempt) = currentCall() ?: return@launch
                current.setParticipantLocallyMuted(id, muted)
                commitCallResult(current, attempt) { value -> value.copy(locallyMutedParticipants = if (muted) value.locallyMutedParticipants + id else value.locallyMutedParticipants - id) }
            }
        }
        return START_NOT_STICKY
    }

    private fun startCall(channelId: String, spaceId: String, channelName: String, spaceName: String, displayName: String, demo: Boolean) {
        if (engine != null) return
        val token = TokenStore(this).read()
        if (!demo && token == null) return stopSelf()
        val attempt = attempts.begin()
        activeAttempt = attempt
        update { VoiceState(VoiceState.Phase.CONNECTING, channelId, spaceId, channelName, spaceName, muted = true) }
        val current = try {
            startForegroundNotification()
            acquireAudio()
            lateinit var created: VoiceEngine
            created = VoiceEngine(this, CaperApi(), token, channelId, displayName, demo) { transport ->
                scope.launch { transportState(created, attempt, transport) }
            }
            created.also { engine = it }
        } catch (error: Throwable) {
            attempts.end()
            activeAttempt = null
            releaseAudio()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            update { it.copy(phase = VoiceState.Phase.FAILED, error = error.message ?: "Voice could not start.") }
            stopSelf()
            return
        }
        connectJob = scope.launch {
            try {
                current.connect { value -> scope.launch { participants(current, attempt, value) } }
                if (engine !== current || !attempts.isCurrent(attempt)) { current.disconnect(); return@launch }
                val eventToken = current.eventToken() ?: error("Voice event capability is unavailable.")
                mediaEvents = MediaEventClient(
                    CaperApi().baseUrl, token, channelId.takeUnless { demo }, eventToken,
                    onSnapshot = { snapshot -> scope.launch {
                        if (engine === current && attempts.isCurrent(attempt)) runCatching {
                            current.applySnapshot(snapshot) { value -> scope.launch { participants(current, attempt, value) } }
                        }.onFailure { if (it !is CancellationException) failCall(current, it) }
                    } },
                    onTerminal = { error -> scope.launch { failCall(current, error) } },
                ).also { it.start() }
                commitCallResult(current, attempt) { it.copy(phase = VoiceState.Phase.CONNECTED, selfId = current.selfParticipantId()) }
                heartbeat = launch {
                    try {
                        while (isActive) {
                            delay(15_000)
                            if (state.value.phase != VoiceState.Phase.RECONNECTING) {
                                heartbeatWithRecovery(current, attempt)
                                runCatching { withTimeout(3_000) { current.diagnostics() } }
                                    .onSuccess { diagnostics -> commitCallResult(current, attempt) { it.copy(diagnostics = diagnostics) } }
                            }
                        }
                    } catch (error: Throwable) {
                        if (error is CancellationException) throw error
                        failCall(current, error)
                    }
                }
                turnRenewal = launch {
                    try {
                        var wait = current.turnRefreshAfterMs() ?: return@launch
                        while (isActive) { delay(wait.coerceAtLeast(1_000)); wait = current.refreshTurn() ?: return@launch }
                    } catch (error: Throwable) {
                        if (error is TimeoutCancellationException) {
                            failCall(current, IOException("TURN renewal timed out.", error))
                            return@launch
                        }
                        if (error is CancellationException) throw error
                        failCall(current, error)
                    }
                }
            } catch (error: Throwable) {
                if (error !is CancellationException) failCall(current, error)
            }
        }
    }

    private suspend fun heartbeatWithRecovery(current: VoiceEngine, attempt: Long) {
        val started = System.currentTimeMillis()
        while (engine === current && attempts.isCurrent(attempt)) {
            try {
                current.heartbeat { value -> scope.launch { participants(current, attempt, value) } }
                commitCallResult(current, attempt) { it.copy(error = null) }
                return
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                if (!transientVoiceControlError(error) || System.currentTimeMillis() - started >= 30_000) throw error
                commitCallResult(current, attempt) { it.copy(error = "Voice control is reconnecting; audio remains protected.") }
                delay(3_000)
            }
        }
    }

    private fun transportState(current: VoiceEngine, attempt: Long, transport: org.webrtc.PeerConnection.PeerConnectionState) {
        if (!callResultIsCurrent(current, engine, attempt, attempts)) return
        when (transport) {
            org.webrtc.PeerConnection.PeerConnectionState.DISCONNECTED -> recover(current, attempt)
            org.webrtc.PeerConnection.PeerConnectionState.FAILED -> recover(current, attempt)
            org.webrtc.PeerConnection.PeerConnectionState.CONNECTED -> if (state.value.phase == VoiceState.Phase.RECONNECTING) {
                // Transport may reconnect before the API acknowledges the ICE
                // transaction. Do not cancel the job or lose its pending ACK.
                if (recovery?.isActive != true) {
                    update { it.copy(phase = VoiceState.Phase.CONNECTED, error = null) }; notifyState()
                }
            }
            else -> Unit
        }
    }

    private fun recover(current: VoiceEngine, attempt: Long) {
        if (recovery?.isActive == true) return
        update { it.copy(phase = VoiceState.Phase.RECONNECTING, error = null) }; notifyState()
        recovery = scope.launch {
            try {
                withTimeout(20_000) { current.recoverIce() }
                commitCallResult(current, attempt) { it.copy(phase = VoiceState.Phase.CONNECTED) }
            } catch (error: TimeoutCancellationException) {
                failCall(current, IOException("Voice recovery timed out.", error))
            } catch (error: Throwable) {
                if (error !is CancellationException) failCall(current, error)
            }
        }
    }

    private fun failCall(current: VoiceEngine, error: Throwable) {
        if (engine !== current) return
        engine = null
        activeAttempt = null
        heartbeat?.cancel(); turnRenewal?.cancel(); recovery?.cancel(); recovery = null; mediaEvents?.close(); mediaEvents = null
        val token = current.closeLocal()
        update { it.copy(phase = VoiceState.Phase.FAILED, error = error.message ?: "Voice connection failed.") }
        notifyState(); releaseAudio(); stopSelf()
        if (token != null) CoroutineScope(SupervisorJob() + Dispatchers.IO).launch { current.leave(token) }
    }

    private fun participants(current: VoiceEngine, attempt: Long, value: List<Participant>) {
        commitCallResult(current, attempt) { it.copy(participants = value) }
    }

    private fun localControlFailed(current: VoiceEngine, attempt: Long, error: Throwable, warning: String) {
        handleVoiceControlError(error,
            terminal = { failCall(current, error) },
            transient = { commitCallResult(current, attempt) { it.copy(muted = current.muted, deafened = current.deafened, error = warning) } },
        )
    }

    private fun stopCall(stopService: Boolean = true) {
        attempts.end()
        activeAttempt = null
        val current = engine
        engine = null
        val joining = connectJob
        connectJob = null
        heartbeat?.cancel(); heartbeat = null; turnRenewal?.cancel(); turnRenewal = null; recovery?.cancel(); recovery = null; mediaEvents?.close(); mediaEvents = null
        joining?.cancel()
        val token = current?.closeLocal()
        update { VoiceState() }
        releaseAudio()
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        if (stopService) stopSelf()
        if (current != null && token != null) CoroutineScope(SupervisorJob() + Dispatchers.IO).launch { current.leave(token) }
    }

    private fun acquireAudio() {
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
        if (Build.VERSION.SDK_INT >= 26) {
            focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE).setAudioAttributes(attributes)
                .setOnAudioFocusChangeListener { change ->
                    if (change < 0) scope.launch {
                        val (current, attempt) = currentCall() ?: return@launch
                        runCatching { current.setMuted(true) {
                            commitCallResult(current, attempt) { it.copy(muted = true) }
                        } }.onFailure { localControlFailed(current, attempt, it, "Audio focus was lost; voice status will retry.") }
                    }
                }.build().also { request ->
                    check(audio.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) { "Audio focus unavailable." }
                }
        }
        wakeLock = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Caper:voice").apply { acquire() }
        updateRoutes()
    }

    private fun updateRoutes() {
        if (Build.VERSION.SDK_INT < 31) return
        val routes = audio.availableCommunicationDevices.map { AudioRoute(it.id, it.productName?.toString() ?: routeName(it.type)) }
        val selected = audio.communicationDevice?.id
        update { it.copy(routes = routes, selectedRouteId = selected) }
    }

    private fun selectRoute(id: Int) {
        if (Build.VERSION.SDK_INT < 31 || id < 0) return
        val device = audio.availableCommunicationDevices.firstOrNull { it.id == id } ?: return
        if (!audio.setCommunicationDevice(device)) {
            update { it.copy(error = "That audio route is unavailable.") }
        }
        updateRoutes(); notifyState()
    }

    private fun routeName(type: Int) = when (type) {
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO, AudioDeviceInfo.TYPE_BLE_HEADSET -> "Bluetooth"
        AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_WIRED_HEADPHONES, AudioDeviceInfo.TYPE_USB_HEADSET -> "Headset"
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "Speaker"
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "Earpiece"
        else -> "System audio"
    }

    private fun releaseAudio() {
        focus?.let(audio::abandonAudioFocusRequest); focus = null
        if (Build.VERSION.SDK_INT >= 31) audio.clearCommunicationDevice()
        audio.mode = AudioManager.MODE_NORMAL
        wakeLock?.takeIf { it.isHeld }?.release(); wakeLock = null
    }

    private fun startForegroundNotification() {
        val notification = notification()
        val type = if (Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, type)
    }
    private fun notifyState() { getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification()) }
    private fun notification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val mute = PendingIntent.getService(this, 1, Intent(this, VoiceCallService::class.java).setAction(ACTION_MUTE), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop = PendingIntent.getService(this, 2, Intent(this, VoiceCallService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val deafen = PendingIntent.getService(this, 3, Intent(this, VoiceCallService::class.java).setAction(ACTION_DEAFEN), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_caper_notification)
            .setContentTitle(state.value.channelName?.let { "Voice · #$it" } ?: "Caper voice")
            .setContentText(if (state.value.muted) "Muted" else "Microphone active")
            .setOngoing(true).setCategory(NotificationCompat.CATEGORY_CALL).setContentIntent(open)
            .addAction(0, if (state.value.muted) "Unmute" else "Mute", mute)
            .addAction(0, if (state.value.deafened) "Undeafen" else "Deafen", deafen)
            .addAction(0, "Disconnect", stop).build()
    }
    private fun createNotificationChannel() {
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, "Ongoing voice calls", NotificationManager.IMPORTANCE_LOW),
        )
    }

    override fun onDestroy() {
        if (active === this) active = null
        attempts.end()
        connectJob?.cancel(); heartbeat?.cancel(); turnRenewal?.cancel(); recovery?.cancel(); mediaEvents?.close(); mediaEvents = null
        val current = engine; engine = null
        activeAttempt = null
        val token = current?.closeLocal() // synchronous safety: mic and peer are closed before cancellation
        if (current != null && token != null) CoroutineScope(SupervisorJob() + Dispatchers.IO).launch { current.leave(token) }
        audio.unregisterAudioDeviceCallback(deviceCallback)
        releaseAudio(); scope.cancel(); super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null

    private fun currentCall(): Pair<VoiceEngine, Long>? {
        val current = engine ?: return null
        val attempt = activeAttempt ?: return null
        return if (callResultIsCurrent(current, engine, attempt, attempts)) current to attempt else null
    }

    private fun commitCallResult(current: VoiceEngine, attempt: Long, block: (VoiceState) -> VoiceState) {
        applyCurrentCallResult(current, engine, attempt, attempts) {
            update(block)
            notifyState()
        }
    }

    companion object {
        const val ACTION_START = "chat.caper.android.voice.START"
        const val ACTION_STOP = "chat.caper.android.voice.STOP"
        const val ACTION_MUTE = "chat.caper.android.voice.MUTE"
        const val ACTION_DEAFEN = "chat.caper.android.voice.DEAFEN"
        const val ACTION_ROUTE = "chat.caper.android.voice.ROUTE"
        const val ACTION_OUTPUT_VOLUME = "chat.caper.android.voice.OUTPUT_VOLUME"
        const val ACTION_PARTICIPANT_VOLUME = "chat.caper.android.voice.PARTICIPANT_VOLUME"
        const val ACTION_PARTICIPANT_MUTE = "chat.caper.android.voice.PARTICIPANT_MUTE"
        const val EXTRA_CHANNEL_ID = "channelId"
        const val EXTRA_SPACE_ID = "spaceId"
        const val EXTRA_CHANNEL_NAME = "channelName"
        const val EXTRA_SPACE_NAME = "spaceName"
        const val EXTRA_DISPLAY_NAME = "displayName"
        const val EXTRA_DEMO = "demo"
        const val EXTRA_ROUTE_ID = "routeId"
        const val EXTRA_PARTICIPANT_ID = "participantId"
        const val EXTRA_VOLUME = "volume"
        private const val CHANNEL = "caper_voice"
        private const val NOTIFICATION_ID = 7401
        private val mutableState = MutableStateFlow(VoiceState())
        @Volatile private var active: VoiceCallService? = null
        val state: StateFlow<VoiceState> = mutableState
        private fun update(block: (VoiceState) -> VoiceState) { mutableState.update(block) }

        fun start(context: Context, channelId: String, spaceId: String, channelName: String, spaceName: String, displayName: String, demo: Boolean = false) {
            if (!BuildConfig.ENABLE_NATIVE_VOICE) return
            val current = active
            if (current != null && state.value.channelId != channelId) {
                current.stopCall(stopService = false)
                current.startCall(channelId, spaceId, channelName, spaceName, displayName, demo)
                return
            }
            val intent = Intent(context, VoiceCallService::class.java).setAction(ACTION_START)
                .putExtra(EXTRA_CHANNEL_ID, channelId).putExtra(EXTRA_SPACE_ID, spaceId)
                .putExtra(EXTRA_CHANNEL_NAME, channelName).putExtra(EXTRA_SPACE_NAME, spaceName)
                .putExtra(EXTRA_DISPLAY_NAME, displayName).putExtra(EXTRA_DEMO, demo)
            context.startForegroundService(intent)
        }
        fun stop(context: Context) {
            val current = active
            if (current != null) current.stopCall()
            else if (state.value.phase != VoiceState.Phase.IDLE) context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_STOP))
        }
        fun stopIfChannel(context: Context, channelId: String) { if (state.value.belongsToChannel(channelId)) stop(context) }
        fun stopIfSpace(context: Context, spaceId: String) { if (state.value.belongsToSpace(spaceId)) stop(context) }
        fun toggleMute(context: Context) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_MUTE)) }
        fun toggleDeafen(context: Context) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_DEAFEN)) }
        fun selectRoute(context: Context, id: Int) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_ROUTE).putExtra(EXTRA_ROUTE_ID, id)) }
        fun setOutputVolume(context: Context, value: Int) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_OUTPUT_VOLUME).putExtra(EXTRA_VOLUME, value)) }
        fun setParticipantVolume(context: Context, id: String, value: Int) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_PARTICIPANT_VOLUME).putExtra(EXTRA_PARTICIPANT_ID, id).putExtra(EXTRA_VOLUME, value)) }
        fun toggleParticipantMute(context: Context, id: String) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_PARTICIPANT_MUTE).putExtra(EXTRA_PARTICIPANT_ID, id)) }
    }
}

internal fun VoiceState.belongsToChannel(id: String) = channelId == id && phase != VoiceState.Phase.IDLE
internal fun VoiceState.belongsToSpace(id: String) = spaceId == id && phase != VoiceState.Phase.IDLE
internal fun callResultIsCurrent(expected: Any, current: Any?, attempt: Long, attempts: CallAttemptGate) =
    expected === current && attempts.isCurrent(attempt)
internal inline fun applyCurrentCallResult(expected: Any, current: Any?, attempt: Long, attempts: CallAttemptGate, block: () -> Unit) {
    if (callResultIsCurrent(expected, current, attempt, attempts)) block()
}

data class VoiceDiagnostics(
    val receivedBytes: Long,
    val receiveBitrate: Long,
    val sentBytes: Long,
    val sendBitrate: Long,
    val packetsLost: Long,
    val maxJitterMs: Long,
    val roundTripMs: Long,
    val route: String,
)

internal fun transientVoiceControlError(error: Throwable): Boolean = when (error) {
    is ApiException -> error.code == "ice_restart_retry" || error.status == 408 || error.status == 429 ||
        (error.status >= 500 && error.code != "ice_restart_invalid")
    is IOException -> true
    else -> false
}

internal fun handleVoiceControlError(error: Throwable, terminal: () -> Unit, transient: () -> Unit) {
    if (error is CancellationException) throw error
    if (transientVoiceControlError(error)) transient() else terminal()
}
