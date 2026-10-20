package chat.caper.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import android.view.HapticFeedbackConstants
import android.view.View
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import chat.caper.android.data.VoiceMessages
import chat.caper.android.data.VoiceTranscript
import chat.caper.android.data.seekPosition
import chat.caper.android.data.transcriptState
import chat.caper.android.model.ChatAttachment
import chat.caper.android.ui.*
import java.io.File
import java.util.UUID
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

// --- Playback -----------------------------------------------------------------------------------

/**
 * The one voice message playing anywhere in the app: starting another stops the first, and the
 * end resets to idle. A re-signed URL doesn't restart playback; only a failure reloads the newest
 * URL from the same spot, and a failure with nothing newer reports it (fresh signatures) once.
 */
@OptIn(UnstableApi::class)
internal object VoicePlayback {
    data class Status(val id: String? = null, val playing: Boolean = false, val failed: Boolean = false)

    var status by mutableStateOf(Status())
        private set
    private var player: ExoPlayer? = null
    private var source: String? = null
    private var latestUrl: () -> String? = { null }
    private var failure: (Int?) -> Unit = {}

    private fun player(context: Context): ExoPlayer = player ?: ExoPlayer.Builder(context.applicationContext).build().apply {
        setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_SPEECH).build(), true)
        addListener(object : Player.Listener {
            override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
                if (status.id != null) status = status.copy(playing = playWhenReady)
            }
            override fun onPlaybackStateChanged(playbackState: Int) {
                if (playbackState == Player.STATE_ENDED) this@VoicePlayback.stop()
            }
            override fun onPlayerError(error: PlaybackException) {
                val fresh = latestUrl()
                if (fresh != null && fresh != source) this@VoicePlayback.load(fresh, currentPosition, play = playWhenReady)
                else {
                    source = null
                    status = status.copy(playing = false, failed = true)
                    failure((error.cause as? HttpDataSource.InvalidResponseCodeException)?.responseCode)
                }
            }
        })
    }.also { player = it }

    private fun load(url: String, positionMs: Long, play: Boolean) {
        val current = player ?: return
        source = url
        current.setMediaItem(MediaItem.fromUri(url), positionMs)
        current.prepare()
        current.playWhenReady = play
    }

    /**
     * Plays [id] from [positionMs] (its current spot when null). [latest] gives the newest signed
     * URL; [onFailure] asks for new signatures with the HTTP status, if any.
     */
    fun play(context: Context, id: String, latest: () -> String?, positionMs: Long? = null, onFailure: (Int?) -> Unit) {
        val url = latest() ?: return
        val current = player(context)
        latestUrl = latest
        failure = onFailure
        if (status.id == id && source != null && !status.failed) {
            positionMs?.let { current.seekTo(it) }
            current.play()
        } else {
            val resume = if (status.id == id) current.currentPosition else 0L
            status = Status(id, playing = true)
            load(url, positionMs ?: resume, play = true)
        }
        status = Status(id, playing = true)
    }

    fun toggle(context: Context, id: String, latest: () -> String?, onFailure: (Int?) -> Unit) {
        if (status.id == id && status.playing) pause() else play(context, id, latest, onFailure = onFailure)
    }

    /** Seeks [id] when it is the current message (keeping play or pause), or starts it there. */
    fun seek(context: Context, id: String, latest: () -> String?, positionMs: Long, onFailure: (Int?) -> Unit) {
        val current = player
        if (current != null && status.id == id && source != null && !status.failed) current.seekTo(positionMs)
        else play(context, id, latest, positionMs, onFailure)
    }

    fun position(id: String): Long? = player?.takeIf { status.id == id }?.currentPosition

    fun pause() { player?.pause() }

    /** Back to idle: nothing selected, the next play starts from the beginning. */
    fun stop() {
        player?.run { pause(); stop(); clearMediaItems() }
        source = null
        status = Status()
    }
}

// --- Message: voice player and transcript ----------------------------------------------------------

/** A `voice: true` attachment: the player, then its transcript. [pending] (a local copy while sending) can't be played. */
@Composable internal fun VoiceMessage(attachment: ChatAttachment, pending: Boolean, onLoadFailed: (ChatAttachment, Int?) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier.widthIn(max = 340.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        VoicePlayer(attachment, enabled = !pending && attachment.url != null, onLoadFailed)
        VoiceTranscriptBlock(attachment)
    }
}

@Composable private fun VoicePlayer(attachment: ChatAttachment, enabled: Boolean, onLoadFailed: (ChatAttachment, Int?) -> Unit) {
    val context = LocalContext.current
    val latest by rememberUpdatedState(attachment)
    val reportFailure by rememberUpdatedState(onLoadFailed)
    val status = VoicePlayback.status
    val active = status.id == attachment.id
    val playing = active && status.playing
    val duration = attachment.durationMs ?: 0L
    var position by remember(attachment.id) { mutableLongStateOf(0L) }
    var dragging by remember(attachment.id) { mutableStateOf<Float?>(null) }
    LaunchedEffect(active, playing) {
        if (!active) { position = 0L; return@LaunchedEffect }
        do {
            position = VoicePlayback.position(attachment.id) ?: 0L
            if (playing) delay(50)
        } while (playing && isActive)
    }
    // Leaving the app stops the sound; it isn't background media.
    if (playing) LifecycleEventEffect(Lifecycle.Event.ON_STOP) { VoicePlayback.pause() }
    val failure: (Int?) -> Unit = { status -> reportFailure(latest, status) }
    val urlOf = { latest.url }
    val progress = dragging ?: if (active && duration > 0) (position.toFloat() / duration).coerceIn(0f, 1f) else 0f
    val seekTo: (Float) -> Unit = { fraction ->
        val target = seekPosition(fraction, duration)
        position = target
        VoicePlayback.seek(context, attachment.id, urlOf, target, failure)
    }
    val clock = VoiceMessages.playerClock(duration, (dragging?.let { seekPosition(it, duration) } ?: position), active || dragging != null)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).background(Surface, MaterialTheme.shapes.small)
            .border(1.dp, Border, MaterialTheme.shapes.small).padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        val label = if (playing) "Pause voice message" else "Play voice message"
        Box(
            Modifier.size(36.dp).clip(CircleShape).background(if (enabled) Action else SurfaceRaised)
                .then(if (enabled) Modifier.clickable(role = Role.Button, onClickLabel = label) {
                    VoicePlayback.toggle(context, attachment.id, urlOf, failure)
                } else Modifier)
                .semantics { contentDescription = label },
            contentAlignment = Alignment.Center,
        ) {
            Icon(painterResource(if (playing) R.drawable.media_pause else R.drawable.media_play), null, Modifier.size(20.dp), tint = if (enabled) OnAction else TextMuted)
        }
        val unplayed = TextMuted.copy(alpha = 0.42f)
        val levels = remember(attachment.waveform) { VoiceMessages.decodeWaveform(attachment.waveform) }
        val currentProgress by rememberUpdatedState(progress)
        Spacer(
            Modifier.weight(1f).height(32.dp)
                .semantics(mergeDescendants = true) {
                    contentDescription = "Voice message, ${VoiceMessages.clock(duration)}"
                    progressBarRangeInfo = ProgressBarRangeInfo(progress, 0f..1f)
                    if (enabled && duration > 0) setProgress("Seek") { seekTo(it.coerceIn(0f, 1f)); true }
                }
                .then(if (enabled && duration > 0) Modifier
                    .pointerInput(attachment.id) { detectTapGestures { seekTo((it.x / size.width).coerceIn(0f, 1f)) } }
                    .pointerInput(attachment.id) {
                        detectHorizontalDragGestures(
                            onDragStart = { dragging = (it.x / size.width).coerceIn(0f, 1f) },
                            onDragEnd = { dragging?.let(seekTo); dragging = null },
                            onDragCancel = { dragging = null },
                        ) { change, _ ->
                            change.consume()
                            dragging = (change.position.x / size.width).coerceIn(0f, 1f)
                        }
                    } else Modifier)
                .drawWithCache {
                    val bar = 3.dp.toPx()
                    val gap = 2.dp.toPx()
                    val count = maxOf(1, ((size.width + gap) / (bar + gap)).toInt())
                    val heights = VoiceMessages.resample(levels, count)
                    onDrawBehind {
                        val played = currentProgress
                        heights.forEachIndexed { index, level ->
                            val x = index * (bar + gap)
                            val height = maxOf(bar, level * size.height)
                            val center = (x + bar / 2) / size.width
                            drawRoundRect(
                                if (played > 0f && center <= played) Text else unplayed,
                                Offset(x, (size.height - height) / 2), Size(bar, height), CornerRadius(bar / 2),
                            )
                        }
                    }
                },
        )
        Text(
            if (active && status.failed) "Can’t play" else clock,
            color = if (active && status.failed) ErrorText else TextMuted, fontSize = 12.sp, style = TabularNumbers, maxLines = 1,
        )
    }
}

/** Slack-style: "Transcribing…" while pending, the text clamped to three lines once ready, nothing otherwise. */
@Composable private fun VoiceTranscriptBlock(attachment: ChatAttachment) {
    val transcript = attachment.transcriptState
    // Only a transcript that arrives while this message is on screen fades in; history is instant.
    val awaited = remember(attachment.id) { transcript is VoiceTranscript.Pending }
    val indent = Modifier.padding(start = 2.dp).drawBehind {
        drawLine(Border, Offset(1.dp.toPx(), 0f), Offset(1.dp.toPx(), size.height), strokeWidth = 2.dp.toPx())
    }.padding(start = 12.dp)
    when (transcript) {
        VoiceTranscript.None -> Unit
        VoiceTranscript.Pending -> Text("Transcribing…", indent.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp)
        is VoiceTranscript.Ready -> {
            var expanded by remember(attachment.id) { mutableStateOf(false) }
            var clamped by remember(attachment.id, transcript.text) { mutableStateOf(false) }
            Column(indent.then(if (awaited) Modifier.entrance(MOTION_BASE) else Modifier)) {
                SelectionContainer {
                    Text(
                        transcript.text, color = TextMuted, fontSize = 13.sp, lineHeight = 19.sp,
                        maxLines = if (expanded) Int.MAX_VALUE else 3, overflow = TextOverflow.Ellipsis,
                        onTextLayout = { if (!expanded) clamped = it.hasVisualOverflow },
                    )
                }
                if (clamped || expanded) TextButton(
                    { expanded = !expanded }, Modifier.heightIn(min = 36.dp), shape = MaterialTheme.shapes.small,
                    contentPadding = PaddingValues(horizontal = 0.dp, vertical = 4.dp), colors = neutralTextButtonColors(),
                ) { Text(if (expanded) "Hide transcript" else "View transcript", fontSize = 12.sp, fontWeight = FontWeight.Bold) }
            }
        }
    }
}

// --- Composer: press-and-hold recording ------------------------------------------------------------

/** A finished recording ready to upload: AAC in MP4, its length and its base64 waveform. */
internal data class VoiceRecording(val file: File, val durationMs: Long, val waveform: String)

/**
 * MediaRecorder to a cache file (AAC/MPEG-4, mono, 44.1 kHz, 64 kbps), sampling
 * `getMaxAmplitude()` for the live meter and the uploaded waveform. Main thread only.
 */
private class VoiceNoteRecorder(private val context: Context) {
    private var recorder: MediaRecorder? = null
    private var file: File? = null
    private var startedAt = 0L
    private var samples = IntArray(256)
    private var count = 0

    val active get() = recorder != null
    val elapsedMs get() = if (recorder == null) 0L else SystemClock.elapsedRealtime() - startedAt

    fun start() {
        // One recording at a time: anything left here is an abandoned one.
        val directory = File(context.cacheDir, "voice-recordings").apply { mkdirs(); listFiles()?.forEach { it.delete() } }
        val output = File(directory, "${UUID.randomUUID()}.m4a")
        val next = if (Build.VERSION.SDK_INT >= 31) MediaRecorder(context) else @Suppress("DEPRECATION") MediaRecorder()
        try {
            next.setAudioSource(MediaRecorder.AudioSource.MIC)
            next.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            next.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            next.setAudioChannels(1)
            next.setAudioSamplingRate(44_100)
            next.setAudioEncodingBitRate(64_000)
            next.setOutputFile(output.absolutePath)
            next.prepare()
            next.start()
        } catch (error: Throwable) {
            next.release()
            output.delete()
            throw error
        }
        recorder = next
        file = output
        startedAt = SystemClock.elapsedRealtime()
        count = 0
    }

    /** The meter level now (0..1); the raw peak joins the waveform. */
    fun sample(): Float {
        val amplitude = runCatching { recorder?.maxAmplitude ?: 0 }.getOrDefault(0)
        if (count == samples.size) samples = samples.copyOf(samples.size * 2)
        samples[count++] = amplitude
        return VoiceMessages.meterLevel(amplitude)
    }

    /** Stops and keeps the file, or null (deleted) when it was a tap or nothing was written. */
    fun finish(): VoiceRecording? {
        val elapsed = elapsedMs
        val current = recorder ?: return null
        val output = file
        recorder = null; file = null
        // stop() throws when no audio arrived yet (a very short hold).
        val stopped = runCatching { current.stop() }.isSuccess
        current.release()
        val duration = VoiceMessages.recordedDuration(elapsed)
        if (!stopped || duration == null || output == null || output.length() == 0L) { output?.delete(); return null }
        return VoiceRecording(output, duration, VoiceMessages.encodeWaveform(VoiceMessages.waveform(samples.copyOf(count))))
    }

    fun cancel() {
        val current = recorder ?: return
        recorder = null
        runCatching { current.stop() }
        current.release()
        file?.delete()
        file = null
    }
}

/** The composer's recording: the hold in progress, its timer and meter, and the hint or error under the field. */
@Stable internal class VoiceRecorderState(context: Context, private val view: View) {
    private val recorder = VoiceNoteRecorder(context)
    var recording by mutableStateOf(false)
        private set
    var elapsedMs by mutableLongStateOf(0L)
        private set
    /** The newest meter levels, oldest first. */
    val levels = mutableStateListOf<Float>()
    /** How far the hold has slid left (negative px); the bar follows it. */
    var slide by mutableFloatStateOf(0f)
    var notice by mutableStateOf<String?>(null)
        private set
    var noticeIsError by mutableStateOf(false)
        private set
    /** Bumped by every start, so a release after an automatic 10:00 send does nothing. */
    var session = 0
        private set
    var onRecorded: (VoiceRecording) -> Unit = {}

    fun hint(text: String) { notice = text; noticeIsError = false }
    fun error(text: String) { notice = text; noticeIsError = true }
    fun clearHint() { if (!noticeIsError) notice = null }

    fun start(): Boolean {
        if (recording) return true
        VoicePlayback.stop()
        return try {
            recorder.start()
            session++
            recording = true
            elapsedMs = 0L
            slide = 0f
            levels.clear()
            notice = null
            view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
            true
        } catch (_: Throwable) {
            error("Couldn’t start recording. Check that no other app is using the microphone.")
            false
        }
    }

    fun tick() {
        if (!recording) return
        val level = recorder.sample()
        levels.add(level)
        while (levels.size > LEVEL_HISTORY) levels.removeAt(0)
        elapsedMs = recorder.elapsedMs
        if (elapsedMs >= VoiceMessages.MAX_DURATION_MS) release()
    }

    /** Lifting the finger (or reaching 10:00) sends; under a second is a tap. */
    fun release() {
        if (!recording) return
        val finished = recorder.finish()
        reset()
        if (finished == null) hint(VoiceMessages.HOLD_HINT) else onRecorded(finished)
    }

    fun cancel(haptic: Boolean) {
        if (!recording) return
        recorder.cancel()
        reset()
        if (haptic) view.performHapticFeedback(if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.REJECT else HapticFeedbackConstants.LONG_PRESS)
    }

    private fun reset() {
        recording = false
        slide = 0f
        levels.clear()
        elapsedMs = 0L
    }

    companion object { const val LEVEL_HISTORY = 48 }
}

/**
 * One recorder per composer ([key]): it samples while recording, and a recording in progress is
 * discarded when the composer goes away or the app leaves the foreground.
 */
@Composable internal fun rememberVoiceRecorder(key: Any, onRecorded: (VoiceRecording) -> Unit): VoiceRecorderState {
    val context = LocalContext.current
    val view = LocalView.current
    val state = remember(key) { VoiceRecorderState(context, view) }
    state.onRecorded = onRecorded
    LaunchedEffect(state, state.recording) {
        while (state.recording && isActive) { state.tick(); delay(50) }
    }
    LaunchedEffect(state.notice, state.noticeIsError) {
        if (state.notice != null && !state.noticeIsError) { delay(2_500); state.clearHint() }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { state.cancel(haptic = false) }
    DisposableEffect(state) { onDispose { state.cancel(haptic = false) } }
    return state
}

/**
 * Why the mic can't record right now, or null. Before Android 10 a second capture can't run
 * beside the call's microphone, so the mic says so instead of breaking the call.
 */
internal fun voiceRecordBlockedReason(pendingMessage: Boolean, inVoiceCall: Boolean, sdk: Int = Build.VERSION.SDK_INT): String? = when {
    pendingMessage -> "Wait for your last message to send, then record."
    inVoiceCall && sdk < 29 -> "Leave the voice channel to record a voice message on this Android version."
    else -> null
}

/**
 * The composer's microphone: press and hold to record, release to send, slide left to cancel.
 * A tap (or a hold under a second) only shows "Hold to record". [blocked] explains why it can't
 * record now; the first press asks for the microphone permission.
 */
@Composable internal fun VoiceRecordButton(state: VoiceRecorderState, enabled: Boolean, blocked: String?) {
    val context = LocalContext.current
    val cancelDistance = with(LocalDensity.current) { 96.dp.toPx() }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) state.hint(VoiceMessages.HOLD_HINT) else state.error(VoiceMessages.PERMISSION_ERROR)
    }
    val currentBlocked by rememberUpdatedState(blocked)
    val tint = when { state.recording -> Terracotta; enabled -> TextMuted; else -> TextMuted.copy(alpha = 0.4f) }
    Box(
        Modifier.size(48.dp)
            .semantics {
                contentDescription = "Record voice message"
                role = Role.Button
                stateDescription = if (state.recording) "Recording" else "Hold to record"
                if (enabled) onClick("Hold to record") { state.hint(VoiceMessages.HOLD_HINT); true }
            }
            .then(if (!enabled) Modifier else Modifier.pointerInput(state) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    val reason = currentBlocked
                    val allowed = ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
                    val started = when {
                        reason != null -> { state.hint(reason); false }
                        !allowed -> { permission.launch(Manifest.permission.RECORD_AUDIO); false }
                        else -> state.start()
                    }
                    val session = state.session
                    var lifted = false
                    try {
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            change.consume()
                            if (!change.pressed) { lifted = true; break }
                            if (started && state.recording && state.session == session) {
                                val dx = (change.position.x - down.position.x).coerceAtMost(0f)
                                state.slide = dx
                                if (dx < -cancelDistance) state.cancel(haptic = true)
                            }
                        }
                    } finally {
                        // A lifted finger sends; an interrupted gesture discards.
                        if (started && state.recording && state.session == session) {
                            if (lifted) state.release() else state.cancel(haptic = false)
                        }
                    }
                }
            }),
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.lucide_mic), null, Modifier.size(20.dp), tint = tint)
    }
}

/**
 * Laid over the composer's row while holding: `● 0:07  ⋯live levels⋯  ‹ Slide to cancel`. The
 * mic underneath keeps the gesture, so lifting or sliding still reaches it.
 */
@Composable internal fun VoiceRecordingBar(state: VoiceRecorderState, modifier: Modifier = Modifier) {
    val still = reducedMotion()
    val slideDp = with(LocalDensity.current) { state.slide.toDp() }
    Row(
        modifier.background(SurfaceComposer, MaterialTheme.shapes.small).border(1.dp, Border, MaterialTheme.shapes.small)
            .padding(horizontal = 14.dp).semantics(mergeDescendants = true) {
                contentDescription = "Recording voice message, ${VoiceMessages.clock(state.elapsedMs)}. Release to send, slide left to cancel."
            },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.size(10.dp).background(Danger, CircleShape))
        Text(VoiceMessages.clock(state.elapsedMs), color = Text, fontSize = 14.sp, fontWeight = FontWeight.Medium, style = TabularNumbers)
        LevelBars(state.levels, Modifier.weight(1f).height(28.dp))
        Text(
            "‹ Slide to cancel", Modifier.offset { IntOffset(if (still) 0 else (slideDp.toPx() / 2).roundToInt(), 0) }
                .alpha((1f + state.slide / 300f).coerceIn(0.3f, 1f)),
            color = TextMuted, fontSize = 12.sp, maxLines = 1,
        )
        Box(Modifier.size(36.dp).background(Action, CircleShape), contentAlignment = Alignment.Center) {
            Icon(painterResource(R.drawable.lucide_mic), null, Modifier.size(18.dp), tint = OnAction)
        }
    }
}

/** Live input levels, newest at the right; bars scroll in from the right as you speak. */
@Composable private fun LevelBars(levels: List<Float>, modifier: Modifier) {
    Spacer(modifier.fillMaxHeight().drawBehind {
        val bar = 3.dp.toPx()
        val gap = 2.dp.toPx()
        val count = maxOf(1, ((size.width + gap) / (bar + gap)).toInt())
        val shown = levels.takeLast(count)
        val start = size.width - shown.size * (bar + gap) + gap
        shown.forEachIndexed { index, level ->
            val height = maxOf(bar, level * size.height)
            drawRoundRect(Text, Offset(start + index * (bar + gap), (size.height - height) / 2), Size(bar, height), CornerRadius(bar / 2))
        }
    })
}

/** Above the composer: "Release to send" while holding, otherwise a short hint or the permission error. */
@Composable internal fun VoiceRecordingNotice(state: VoiceRecorderState) {
    val text = if (state.recording) "Release to send" else state.notice ?: return
    Text(
        text, Modifier.fillMaxWidth().padding(bottom = 6.dp).semantics { liveRegion = LiveRegionMode.Polite },
        color = if (!state.recording && state.noticeIsError) ErrorText else TextMuted, fontSize = 12.sp,
    )
}
