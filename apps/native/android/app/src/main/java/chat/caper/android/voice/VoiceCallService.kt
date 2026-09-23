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
    val channelName: String? = null,
    val spaceName: String? = null,
    val participants: List<Participant> = emptyList(),
    val muted: Boolean = false,
    val deafened: Boolean = false,
    val routes: List<AudioRoute> = emptyList(),
    val selectedRouteId: Int? = null,
    val error: String? = null,
) { enum class Phase { IDLE, CONNECTING, CONNECTED, RECONNECTING, FAILED } }

class VoiceCallService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    private val attempts = CallAttemptGate()
    private var engine: VoiceEngine? = null
    private var connectJob: Job? = null
    private var heartbeat: Job? = null
    private var turnRenewal: Job? = null
    private var recovery: Job? = null
    private var mediaEvents: MediaEventClient? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private lateinit var audio: AudioManager
    private var focus: AudioFocusRequest? = null
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) = updateRoutes()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) = updateRoutes()
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
                requireNotNull(intent.getStringExtra(EXTRA_CHANNEL_NAME)),
                requireNotNull(intent.getStringExtra(EXTRA_SPACE_NAME)),
                requireNotNull(intent.getStringExtra(EXTRA_DISPLAY_NAME)),
                intent.getBooleanExtra(EXTRA_DEMO, false),
            )
            ACTION_MUTE -> scope.launch {
                engine?.let { current ->
                    runCatching { current.setMuted(!state.value.muted) }
                        .onSuccess { update { it.copy(muted = current.muted, error = null) } }
                        .onFailure { update { it.copy(muted = current.muted, error = "Mute is local; voice status will retry.") } }
                    notifyState()
                }
            }
            ACTION_DEAFEN -> scope.launch {
                val current = engine ?: return@launch
                runCatching { current.setDeafened(!state.value.deafened) }
                    .onSuccess { update { it.copy(deafened = current.deafened, muted = current.muted, error = null) } }
                    .onFailure { update { it.copy(deafened = current.deafened, muted = current.muted, error = "Deafen is local; voice status will retry.") } }
                notifyState()
            }
            ACTION_STOP -> stopCall()
            ACTION_ROUTE -> selectRoute(intent.getIntExtra(EXTRA_ROUTE_ID, -1))
        }
        return START_NOT_STICKY
    }

    private fun startCall(channelId: String, channelName: String, spaceName: String, displayName: String, demo: Boolean) {
        if (engine != null) return
        val token = TokenStore(this).read()
        if (!demo && token == null) return stopSelf()
        val attempt = attempts.begin()
        update { VoiceState(VoiceState.Phase.CONNECTING, channelId, channelName, spaceName, muted = true) }
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
            releaseAudio()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            update { it.copy(phase = VoiceState.Phase.FAILED, error = error.message ?: "Voice could not start.") }
            stopSelf()
            return
        }
        connectJob = scope.launch {
            try {
                current.connect { value -> participants(current, attempt, value) }
                if (engine !== current || !attempts.isCurrent(attempt)) { current.disconnect(); return@launch }
                val eventToken = current.eventToken() ?: error("Voice event capability is unavailable.")
                mediaEvents = MediaEventClient(
                    CaperApi().baseUrl, token, channelId.takeUnless { demo }, eventToken,
                    onSnapshot = { snapshot -> scope.launch {
                        if (engine === current && attempts.isCurrent(attempt)) runCatching {
                            current.applySnapshot(snapshot) { value -> participants(current, attempt, value) }
                        }.onFailure { if (it !is CancellationException) failCall(current, it) }
                    } },
                    onTerminal = { error -> scope.launch { failCall(current, error) } },
                ).also { it.start() }
                update { it.copy(phase = VoiceState.Phase.CONNECTED) }
                notifyState()
                heartbeat = launch {
                    try {
                        while (isActive) {
                            delay(15_000)
                            if (state.value.phase != VoiceState.Phase.RECONNECTING) heartbeatWithRecovery(current, attempt)
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
                current.heartbeat { value -> participants(current, attempt, value) }
                update { it.copy(error = null) }
                return
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                if (!transientVoiceControlError(error) || System.currentTimeMillis() - started >= 30_000) throw error
                update { it.copy(error = "Voice control is reconnecting; audio remains protected.") }
                notifyState()
                delay(3_000)
            }
        }
    }

    private fun transportState(current: VoiceEngine, attempt: Long, transport: org.webrtc.PeerConnection.PeerConnectionState) {
        if (engine !== current || !attempts.isCurrent(attempt)) return
        when (transport) {
            org.webrtc.PeerConnection.PeerConnectionState.DISCONNECTED -> recover(current, attempt)
            org.webrtc.PeerConnection.PeerConnectionState.FAILED -> recover(current, attempt)
            org.webrtc.PeerConnection.PeerConnectionState.CONNECTED -> if (state.value.phase == VoiceState.Phase.RECONNECTING) {
                recovery?.cancel(); recovery = null
                update { it.copy(phase = VoiceState.Phase.CONNECTED, error = null) }; notifyState()
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
                if (engine === current && attempts.isCurrent(attempt)) {
                    update { it.copy(phase = VoiceState.Phase.CONNECTED) }; notifyState()
                }
            } catch (error: Throwable) {
                if (error !is CancellationException) failCall(current, error)
            }
        }
    }

    private fun failCall(current: VoiceEngine, error: Throwable) {
        if (engine !== current) return
        engine = null
        heartbeat?.cancel(); turnRenewal?.cancel(); recovery?.cancel(); recovery = null; mediaEvents?.close(); mediaEvents = null
        val token = current.closeLocal()
        update { it.copy(phase = VoiceState.Phase.FAILED, error = error.message ?: "Voice connection failed.") }
        notifyState(); releaseAudio(); stopSelf()
        if (token != null) CoroutineScope(SupervisorJob() + Dispatchers.IO).launch { current.leave(token) }
    }

    private fun participants(current: VoiceEngine, attempt: Long, value: List<Participant>) {
        if (engine !== current || !attempts.isCurrent(attempt)) return
        update { it.copy(participants = value) }
        notifyState()
    }

    private fun stopCall(stopService: Boolean = true) {
        attempts.end()
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
                        engine?.let { current ->
                            runCatching { current.setMuted(true) }
                            update { it.copy(muted = true) }; notifyState()
                        }
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
        val token = current?.closeLocal() // synchronous safety: mic and peer are closed before cancellation
        if (current != null && token != null) CoroutineScope(SupervisorJob() + Dispatchers.IO).launch { current.leave(token) }
        audio.unregisterAudioDeviceCallback(deviceCallback)
        releaseAudio(); scope.cancel(); super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val ACTION_START = "chat.caper.android.voice.START"
        const val ACTION_STOP = "chat.caper.android.voice.STOP"
        const val ACTION_MUTE = "chat.caper.android.voice.MUTE"
        const val ACTION_DEAFEN = "chat.caper.android.voice.DEAFEN"
        const val ACTION_ROUTE = "chat.caper.android.voice.ROUTE"
        const val EXTRA_CHANNEL_ID = "channelId"
        const val EXTRA_CHANNEL_NAME = "channelName"
        const val EXTRA_SPACE_NAME = "spaceName"
        const val EXTRA_DISPLAY_NAME = "displayName"
        const val EXTRA_DEMO = "demo"
        const val EXTRA_ROUTE_ID = "routeId"
        private const val CHANNEL = "caper_voice"
        private const val NOTIFICATION_ID = 7401
        private val mutableState = MutableStateFlow(VoiceState())
        @Volatile private var active: VoiceCallService? = null
        val state: StateFlow<VoiceState> = mutableState
        private fun update(block: (VoiceState) -> VoiceState) { mutableState.update(block) }

        fun start(context: Context, channelId: String, channelName: String, spaceName: String, displayName: String, demo: Boolean = false) {
            if (!BuildConfig.ENABLE_NATIVE_VOICE) return
            val current = active
            if (current != null && state.value.channelId != channelId) {
                current.stopCall(stopService = false)
                current.startCall(channelId, channelName, spaceName, displayName, demo)
                return
            }
            val intent = Intent(context, VoiceCallService::class.java).setAction(ACTION_START)
                .putExtra(EXTRA_CHANNEL_ID, channelId).putExtra(EXTRA_CHANNEL_NAME, channelName).putExtra(EXTRA_SPACE_NAME, spaceName)
                .putExtra(EXTRA_DISPLAY_NAME, displayName).putExtra(EXTRA_DEMO, demo)
            context.startForegroundService(intent)
        }
        fun stop(context: Context) {
            val current = active
            if (current != null) current.stopCall()
            else if (state.value.phase != VoiceState.Phase.IDLE) context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_STOP))
        }
        fun stopIfChannel(context: Context, channelId: String) { if (state.value.channelId == channelId) stop(context) }
        fun toggleMute(context: Context) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_MUTE)) }
        fun toggleDeafen(context: Context) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_DEAFEN)) }
        fun selectRoute(context: Context, id: Int) { context.startService(Intent(context, VoiceCallService::class.java).setAction(ACTION_ROUTE).putExtra(EXTRA_ROUTE_ID, id)) }
    }
}

internal fun transientVoiceControlError(error: Throwable): Boolean = when (error) {
    is ApiException -> error.status == 408 || error.status == 429 || error.status >= 500
    is IOException -> true
    else -> false
}
