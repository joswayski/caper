package chat.caper.android

import android.Manifest
import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import android.view.HapticFeedbackConstants
import android.view.View
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameMillis
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import chat.caper.android.data.LiveLevels
import chat.caper.android.data.VoiceMessages
import chat.caper.android.data.VoiceTranscript
import chat.caper.android.data.seekPosition
import chat.caper.android.data.transcriptState
import chat.caper.android.data.voiceMicBlockedReason
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.io.File
import java.util.UUID
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

// --- Preference -----------------------------------------------------------------------------------

/**
 * "Send voice messages as soon as I stop recording": a per-device setting stored with the other
 * audio preferences, off by default (stopping stages the recording in the composer).
 */
internal object VoicePreferences {
    private var preferences: SharedPreferences? = null
    private var sendOnStop by mutableStateOf(VoiceMessages.SEND_ON_STOP_DEFAULT)

    private fun load(context: Context) {
        if (preferences != null) return
        val loaded = context.applicationContext.getSharedPreferences("audio", Context.MODE_PRIVATE)
        preferences = loaded
        sendOnStop = loaded.getBoolean(VoiceMessages.SEND_ON_STOP_KEY, VoiceMessages.SEND_ON_STOP_DEFAULT)
    }

    fun sendOnStop(context: Context): Boolean { load(context); return sendOnStop }

    fun setSendOnStop(context: Context, value: Boolean) {
        load(context)
        sendOnStop = value
        preferences?.edit()?.putBoolean(VoiceMessages.SEND_ON_STOP_KEY, value)?.apply()
    }
}

/** The settings row: the switch for [VoicePreferences.sendOnStop]. */
@Composable internal fun VoiceSendOnStopSetting() {
    val context = LocalContext.current
    val on = VoicePreferences.sendOnStop(context)
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(VoiceMessages.SEND_ON_STOP_LABEL, Modifier.weight(1f), fontSize = 13.sp)
        androidx.compose.material3.Switch(on, { VoicePreferences.setSendOnStop(context, it); CaperEffects.toggle(it) },
            Modifier.semantics { contentDescription = VoiceMessages.SEND_ON_STOP_LABEL }, colors = caperSwitchColors())
    }
}

// --- Playback -----------------------------------------------------------------------------------

/**
 * The one voice message playing anywhere in the app (sent ones and staged ones): starting another
 * stops the first, and the end resets to idle. A re-signed URL doesn't restart playback; only a
 * failure reloads the newest URL from the same spot, and a failure with nothing newer reports it
 * (fresh signatures) once.
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

    /** Stops [id] if it is the one playing (a staged recording that is being removed). */
    fun stop(id: String) { if (status.id == id) stop() }
}

/**
 * Where [id] is in its playback (ms), refreshed every display frame while it plays so progress
 * moves smoothly; 0 when it isn't the current voice message.
 */
@Composable private fun playbackPosition(id: String): Long {
    val status = VoicePlayback.status
    val active = status.id == id
    val playing = active && status.playing
    var position by remember(id) { mutableLongStateOf(0L) }
    LaunchedEffect(id, active, playing) {
        if (!active) { position = 0L; return@LaunchedEffect }
        position = VoicePlayback.position(id) ?: 0L
        while (playing && isActive) withFrameMillis { position = VoicePlayback.position(id) ?: position }
    }
    // Leaving the app stops the sound; it isn't background media.
    if (playing) LifecycleEventEffect(Lifecycle.Event.ON_STOP) { VoicePlayback.pause() }
    return position
}

// --- Waveform drawing ------------------------------------------------------------------------------

private val BarWidth = 3.dp
private val BarGap = 2.dp
private val BarMinHeight = 3.dp

/** One bar: fully rounded caps, vertically centred, at least [BarMinHeight] tall so silence is a neat line. */
private fun DrawScope.waveformBar(x: Float, level: Float, color: Color) {
    val width = BarWidth.toPx()
    val height = maxOf(BarMinHeight.toPx(), level.coerceIn(0f, 1f) * size.height)
    drawRoundRect(color, Offset(x, (size.height - height) / 2), Size(width, height), CornerRadius(width / 2))
}

/**
 * The stored waveform as evenly spaced bars across the width (linear interpolation of the 64
 * levels), the played part in the text colour and the rest muted; the bar at [progress] is
 * partly filled.
 */
private fun Modifier.waveform(levels: FloatArray?, progress: () -> Float, unplayed: Color): Modifier = drawWithCache {
    val bar = BarWidth.toPx()
    val gap = BarGap.toPx()
    val count = maxOf(1, ((size.width + gap) / (bar + gap)).toInt())
    val heights = VoiceMessages.interpolate(levels, count)
    // Centre the bar run in the width.
    val inset = (size.width - (count * bar + (count - 1) * gap)) / 2
    onDrawBehind {
        val played = progress()
        heights.forEachIndexed { index, level ->
            val x = inset + index * (bar + gap)
            val fill = VoiceMessages.playedFraction(x / size.width, (x + bar) / size.width, played)
            if (fill < 1f) waveformBar(x, level, unplayed)
            if (fill > 0f) clipRect(left = x, right = x + bar * fill) { waveformBar(x, level, Text) }
        }
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

/** The eggshell circular play/pause button shared by the player and staged recordings. */
@Composable private fun PlayPauseButton(playing: Boolean, enabled: Boolean, size: Dp, toggle: () -> Unit) {
    val label = if (playing) "Pause voice message" else "Play voice message"
    Box(
        Modifier.size(size).clip(CircleShape).background(if (enabled) Action else SurfaceRaised)
            .then(if (enabled) Modifier.clickable(role = Role.Button, onClickLabel = label, onClick = toggle) else Modifier)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(if (playing) R.drawable.media_pause else R.drawable.media_play), null, Modifier.size(size * 0.55f), tint = if (enabled) OnAction else TextMuted)
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
    val position = playbackPosition(attachment.id)
    var dragging by remember(attachment.id) { mutableStateOf<Float?>(null) }
    var seeked by remember(attachment.id) { mutableStateOf<Long?>(null) }
    val failure: (Int?) -> Unit = { status -> reportFailure(latest, status) }
    val urlOf = { latest.url }
    val shown = seeked?.takeIf { active } ?: position
    LaunchedEffect(position) { seeked = null }
    val progress = dragging ?: if (active && duration > 0) (shown.toFloat() / duration).coerceIn(0f, 1f) else 0f
    val seekTo: (Float) -> Unit = { fraction ->
        val target = seekPosition(fraction, duration)
        seeked = target
        VoicePlayback.seek(context, attachment.id, urlOf, target, failure)
    }
    val clock = VoiceMessages.playerClock(duration, (dragging?.let { seekPosition(it, duration) } ?: shown), active || dragging != null)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).background(Surface, MaterialTheme.shapes.small)
            .border(1.dp, Border, MaterialTheme.shapes.small).padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        PlayPauseButton(playing, enabled, 36.dp) { VoicePlayback.toggle(context, attachment.id, urlOf, failure) }
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
                .waveform(levels, { currentProgress }, TextMuted.copy(alpha = 0.42f)),
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

// --- Composer: staged recording ---------------------------------------------------------------------

/**
 * A recording staged in the composer (sent with the message's text and files): play/pause of
 * the local copy, its waveform and length, its upload status (Retry when it failed and the
 * composer retries from the chip) and Remove, which cancels the upload and deletes it.
 */
@Composable internal fun StagedVoiceChip(
    draft: DraftAttachmentUi, remove: (String) -> Unit, retry: ((String) -> Unit)?, enabled: Boolean, busy: Boolean,
) {
    val context = LocalContext.current
    val id = "draft:${draft.key}"
    val status = VoicePlayback.status
    val playing = status.id == id && status.playing
    val duration = draft.durationMs ?: 0L
    val position = playbackPosition(id)
    val progress = if (status.id == id && duration > 0) (position.toFloat() / duration).coerceIn(0f, 1f) else 0f
    val currentProgress by rememberUpdatedState(progress)
    val levels = remember(draft.waveform) { VoiceMessages.decodeWaveform(draft.waveform) }
    DisposableEffect(id) { onDispose { VoicePlayback.stop(id) } }
    Row(
        Modifier.entrance(MOTION_BASE, scale = 0.94f).widthIn(max = 320.dp).fillMaxWidth().heightIn(min = 52.dp)
            .background(Surface, MaterialTheme.shapes.small)
            .border(1.dp, if (draft.error != null) ErrorText.copy(alpha = 0.6f) else Border, MaterialTheme.shapes.small)
            .padding(start = 8.dp, top = 6.dp, bottom = 6.dp)
            .semantics { contentDescription = "${VoiceMessages.LABEL}, ${VoiceMessages.clock(duration)}" },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PlayPauseButton(playing, draft.localUrl != null, 32.dp) {
            VoicePlayback.toggle(context, id, { draft.localUrl }) {}
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Spacer(Modifier.fillMaxWidth().height(22.dp).waveform(levels, { currentProgress }, TextMuted.copy(alpha = 0.42f)))
            val detail = when {
                draft.error != null -> draft.error
                busy && draft.attachment == null -> "Uploading ${(draft.progress * 100).toInt()}%"
                else -> null
            }
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(VoiceMessages.playerClock(duration, position, status.id == id), color = TextMuted, fontSize = 11.sp, style = TabularNumbers)
                detail?.let { Text(it, Modifier.weight(1f, fill = false), color = if (draft.error != null) ErrorText else TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
        if (draft.error != null && retry != null) TextButton({ retry(draft.key) }, enabled = enabled, shape = MaterialTheme.shapes.small,
            contentPadding = PaddingValues(horizontal = 6.dp), colors = neutralTextButtonColors()) { Text("Retry", fontSize = 12.sp) }
        IconButton({ VoicePlayback.stop(id); remove(draft.key) }, Modifier.size(44.dp), enabled = enabled) {
            Icon(painterResource(R.drawable.lucide_x), "Remove voice message", Modifier.size(16.dp), tint = if (enabled) TextMuted else TextMuted.copy(alpha = 0.4f))
        }
    }
}

// --- Composer: recording ----------------------------------------------------------------------------

/** A finished recording ready to upload: AAC in MP4, its length and its base64 waveform. */
internal data class VoiceRecording(val file: File, val durationMs: Long, val waveform: String)

/**
 * MediaRecorder to a cache file (AAC/MPEG-4, mono, 44.1 kHz, 64 kbps). Each `getMaxAmplitude()`
 * reading (every [VoiceMessages.LIVE_SAMPLE_MS]) feeds the live meter and, as the input envelope,
 * the uploaded waveform's per-slice RMS. Main thread only.
 */
private class VoiceNoteRecorder(private val context: Context) {
    private var recorder: MediaRecorder? = null
    private var file: File? = null
    private var startedAt = 0L
    private var samples = IntArray(512)
    private var count = 0

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

    /** The input amplitude now (0..32767); it also joins the waveform. */
    fun sample(): Int {
        val amplitude = runCatching { recorder?.maxAmplitude ?: 0 }.getOrDefault(0)
        if (count == samples.size) samples = samples.copyOf(samples.size * 2)
        samples[count++] = amplitude
        return amplitude
    }

    /** Stops and keeps the file, or null (deleted) when it was under a second or nothing was written. */
    fun finish(): VoiceRecording? {
        val elapsed = elapsedMs
        val current = recorder ?: return null
        val output = file
        recorder = null; file = null
        // stop() throws when no audio arrived yet (a very short recording).
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

/**
 * The composer's recording: held (release stops, slide left cancels) or hands-free after a tap
 * (Stop or discard in the bar), its timer and live meter, and the hint or error under the field.
 * Stopping hands the recording to [onRecorded], which stages or sends it.
 */
@Stable internal class VoiceRecorderState(context: Context, private val view: View) {
    private val recorder = VoiceNoteRecorder(context)
    private val live = LiveLevels()
    var recording by mutableStateOf(false)
        private set
    /** Started by a tap: the bar offers Stop and discard instead of release and slide. */
    var handsFree by mutableStateOf(false)
        private set
    var elapsedMs by mutableLongStateOf(0L)
        private set
    /** The live bars, oldest first, and when the newest arrived (`SystemClock.elapsedRealtime`). */
    var bars by mutableStateOf(emptyList<Float>())
        private set
    var lastBarAt by mutableLongStateOf(0L)
        private set
    /** How far a hold has slid left (negative px); the bar follows it. */
    var slide by mutableFloatStateOf(0f)
    var notice by mutableStateOf<String?>(null)
        private set
    var noticeIsError by mutableStateOf(false)
        private set
    /** Bumped by every start, so a release after an automatic 10:00 stop does nothing. */
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
            handsFree = false
            elapsedMs = 0L
            slide = 0f
            live.clear()
            bars = emptyList()
            notice = null
            view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
            true
        } catch (_: Throwable) {
            error("Couldn’t start recording. Check that no other app is using the microphone.")
            false
        }
    }

    /** A tap (not a hold) keeps recording hands-free until Stop or discard. */
    fun keepRecordingHandsFree() { if (recording) { handsFree = true; slide = 0f } }

    /** Samples the input and the clock; returns how long to wait before the next sample (ms). */
    fun tick(): Long {
        if (!recording) return VoiceMessages.LIVE_SAMPLE_MS
        val now = SystemClock.elapsedRealtime()
        if (live.sample(VoiceMessages.meterLevel(recorder.sample()), now)) {
            bars = live.values
            lastBarAt = live.lastBarAt ?: now
        }
        elapsedMs = recorder.elapsedMs
        if (elapsedMs >= VoiceMessages.MAX_DURATION_MS) stop()
        return live.nextSampleIn(now)
    }

    /** Release, Stop or 10:00: the recording goes to [onRecorded]; under a second is discarded. */
    fun stop() {
        if (!recording) return
        val finished = recorder.finish()
        reset()
        if (finished == null) hint(VoiceMessages.TOO_SHORT) else onRecorded(finished)
    }

    fun discard(haptic: Boolean) {
        if (!recording) return
        recorder.cancel()
        reset()
        if (haptic) view.performHapticFeedback(if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.REJECT else HapticFeedbackConstants.LONG_PRESS)
    }

    private fun reset() {
        recording = false
        handsFree = false
        slide = 0f
        live.clear()
        bars = emptyList()
        elapsedMs = 0L
    }
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
        while (state.recording && isActive) delay(state.tick())
    }
    LaunchedEffect(state.notice, state.noticeIsError) {
        if (state.notice != null && !state.noticeIsError) { delay(2_500); state.clearHint() }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { state.discard(haptic = false) }
    DisposableEffect(state) { onDispose { state.discard(haptic = false) } }
    return state
}

/** Kept for the reason tests: no staged voice message, so a pending message or an old Android in a call. */
internal fun voiceRecordBlockedReason(pendingMessage: Boolean, inVoiceCall: Boolean, sdk: Int = Build.VERSION.SDK_INT): String? =
    voiceMicBlockedReason(stagedVoice = false, pendingMessage = pendingMessage, inVoiceCall = inVoiceCall, sdk = sdk)

/**
 * Why the mic can't record right now, or null, following the voice call as it starts and ends.
 * [stagedVoice]: the composer already holds one; [pendingMessage]: only blocks when stopping
 * would send at once.
 */
@Composable internal fun voiceRecordBlocked(pendingMessage: Boolean, stagedVoice: Boolean = false): String? {
    val call by VoiceCallService.state.collectAsStateWithLifecycle()
    val sendOnStop = VoicePreferences.sendOnStop(LocalContext.current)
    return voiceMicBlockedReason(stagedVoice, pendingMessage && sendOnStop,
        call.phase != VoiceState.Phase.IDLE && call.phase != VoiceState.Phase.FAILED, Build.VERSION.SDK_INT)
}

/**
 * Stages (the default) or sends a finished recording, by the "send as soon as I stop" setting.
 * [stage] and [send] take the recording over.
 */
@Composable internal fun rememberVoiceStop(stage: (VoiceRecording) -> Unit, send: (VoiceRecording) -> Unit): (VoiceRecording) -> Unit {
    val context = LocalContext.current
    val latestStage by rememberUpdatedState(stage)
    val latestSend by rememberUpdatedState(send)
    return remember(context) {
        { recording ->
            when (VoiceMessages.stopAction(VoicePreferences.sendOnStop(context))) {
                VoiceMessages.StopAction.STAGE -> latestStage(recording)
                VoiceMessages.StopAction.SEND -> latestSend(recording)
            }
        }
    }
}

/**
 * The composer's microphone, a tool button like Emoji: a tap starts hands-free recording (Stop
 * in the bar ends it); a press and hold records until release, and sliding left cancels. The
 * first press asks for the microphone. [blocked] explains why it can't record now.
 */
@Composable internal fun VoiceRecordButton(state: VoiceRecorderState, enabled: Boolean, blocked: String?) {
    val context = LocalContext.current
    val cancelDistance = with(LocalDensity.current) { 96.dp.toPx() }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) { if (state.start()) state.keepRecordingHandsFree() } else state.error(VoiceMessages.PERMISSION_ERROR)
    }
    val currentBlocked by rememberUpdatedState(blocked)
    val interactions = remember { MutableInteractionSource() }
    val allowed = { ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED }
    // Screen readers activate it with a tap: hands-free recording.
    val activate = {
        val reason = currentBlocked
        when {
            reason != null -> state.hint(reason)
            !allowed() -> permission.launch(Manifest.permission.RECORD_AUDIO)
            state.start() -> state.keepRecordingHandsFree()
        }
    }
    val usable = enabled && blocked == null
    val tint = when { state.recording -> Terracotta; usable -> TextMuted; else -> TextMuted.copy(alpha = 0.4f) }
    Box(
        Modifier.size(48.dp)
            .semantics {
                contentDescription = "Record voice message"
                role = Role.Button
                stateDescription = blocked ?: if (state.recording) "Recording" else "Tap to record, or press and hold"
                if (!enabled) disabled()
                if (enabled) onClick("Record voice message") { activate(); true }
            }
            .indication(interactions, ripple(bounded = false, radius = 24.dp))
            .then(if (!enabled) Modifier else Modifier.pointerInput(state) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    val press = PressInteraction.Press(down.position)
                    interactions.tryEmit(press)
                    val reason = currentBlocked
                    val started = when {
                        reason != null -> { state.hint(reason); false }
                        !allowed() -> { permission.launch(Manifest.permission.RECORD_AUDIO); false }
                        else -> state.start()
                    }
                    val session = state.session
                    var lifted = false
                    var heldFor = 0L
                    try {
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            change.consume()
                            heldFor = change.uptimeMillis - down.uptimeMillis
                            if (!change.pressed) { lifted = true; break }
                            if (started && state.recording && state.session == session && !state.handsFree) {
                                val dx = (change.position.x - down.position.x).coerceAtMost(0f)
                                state.slide = dx
                                if (dx < -cancelDistance) state.discard(haptic = true)
                            }
                        }
                    } finally {
                        interactions.tryEmit(PressInteraction.Release(press))
                        if (started && state.recording && state.session == session && !state.handsFree) {
                            when {
                                // A tap records hands-free; a hold stops on release.
                                lifted && heldFor < VoiceMessages.TAP_MS && state.slide == 0f -> state.keepRecordingHandsFree()
                                lifted -> state.stop()
                                else -> state.discard(haptic = false)
                            }
                        }
                    }
                }
            }),
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.lucide_mic), null, Modifier.size(20.dp), tint = tint)
    }
}

/** Hands-free recording from a hardware keyboard: Enter stops, Escape discards. */
internal fun Modifier.voiceRecordingKeys(state: VoiceRecorderState): Modifier = onPreviewKeyEvent { event ->
    if (!state.recording || !state.handsFree || event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
    when (event.key) {
        Key.Enter, Key.NumPadEnter -> { state.stop(); true }
        Key.Escape -> { state.discard(haptic = false); true }
        else -> false
    }
}

/**
 * Laid over the composer's row while recording. Hands-free: `[trash] ● 0:07 ⋯live⋯ [Stop]`.
 * Held: `● 0:07 ⋯live⋯ ‹ Slide to cancel (mic)`, while the mic underneath keeps the gesture.
 */
@Composable internal fun VoiceRecordingBar(state: VoiceRecorderState, modifier: Modifier = Modifier) {
    val still = reducedMotion()
    val slideDp = with(LocalDensity.current) { state.slide.toDp() }
    Row(
        modifier.background(SurfaceComposer, MaterialTheme.shapes.small).border(1.dp, Border, MaterialTheme.shapes.small)
            .padding(start = if (state.handsFree) 2.dp else 14.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (state.handsFree) IconButton({ state.discard(haptic = true) }, Modifier.size(44.dp)) {
            Icon(painterResource(R.drawable.lucide_trash_2), "Discard voice message", Modifier.size(18.dp), tint = TextMuted)
        }
        RecordingDot(still)
        Text(
            VoiceMessages.clock(state.elapsedMs), Modifier.semantics { contentDescription = "Recording, ${VoiceMessages.clock(state.elapsedMs)}" },
            color = Text, fontSize = 14.sp, fontWeight = FontWeight.Medium, style = TabularNumbers,
        )
        LiveBars(state, still, Modifier.weight(1f).height(28.dp))
        if (state.handsFree) FilledIconButton(
            { state.stop() }, Modifier.size(40.dp).semantics { contentDescription = "Stop recording" }, shape = MaterialTheme.shapes.small,
            colors = IconButtonDefaults.filledIconButtonColors(containerColor = Action, contentColor = OnAction),
        ) { Box(Modifier.size(12.dp).background(OnAction, RoundedCornerShape(2.dp))) }
        else {
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
}

/** The recording dot: pulses gently, or stays still when animations are removed. */
@Composable private fun RecordingDot(still: Boolean) {
    val alpha = if (still) 1f else {
        val pulse = rememberInfiniteTransition(label = "recording")
        pulse.animateFloat(1f, 0.35f, infiniteRepeatable(tween(700), RepeatMode.Reverse), label = "dot").value
    }
    Box(Modifier.size(10.dp).alpha(alpha).background(Danger, CircleShape))
}

/**
 * The live meter ("pinch bars"): a pill per bar, newest at the right, scrolling left at a
 * constant rate (shifted per display frame between new bars; without animations they step).
 * Bars arrive flat and swell in from the right edge, brighten toward eggshell as they get louder,
 * and shrink and fade over the last [LivePinchReach] on the left. Silence shows small dots.
 */
@Composable private fun LiveBars(state: VoiceRecorderState, still: Boolean, modifier: Modifier) {
    var frame by remember { mutableLongStateOf(0L) }
    LaunchedEffect(still) { if (!still) while (isActive) withFrameMillis { frame = it } }
    Spacer(modifier.fillMaxHeight().drawBehind {
        // Read every frame, so the bars glide between new ones.
        if (frame < 0L) return@drawBehind
        val bars = state.bars
        if (bars.isEmpty() || size.width <= 0f) return@drawBehind
        val w = size.width
        val h = size.height
        val bar = LiveBarWidth.toPx()
        val step = LiveBarStep.toPx()
        val minHeight = LiveBarMinHeight.toPx()
        val reach = minOf(LivePinchReach.toPx(), 0.8f * w)
        val entry = LiveEntry.toPx()
        val arrival = LiveArrival.toPx()
        // Keeps sliding past one step while a bar is late instead of pausing; the
        // late bar lands where the slide has got to (the cadence restarts at 3).
        val frac = if (still) 1f
            else ((SystemClock.elapsedRealtime() - state.lastBarAt).toFloat() / VoiceMessages.LIVE_BAR_MS).coerceIn(0f, 3f)
        clipRect {
            for (k in 0 until bars.size) {
                val x = w - frac * step - k * step
                if (x < -step) break
                val cx = x + bar / 2
                val pinch = 1 - LivePinch * (1 - smoothstep(0f, reach, cx))
                val swell = easeOutCubic((w - cx) / entry)
                val alpha = smoothstep(0f, 0.1f * w, cx) * smoothstep(0f, arrival, w - cx)
                if (alpha < 0.02f) continue
                val scale = pinch * swell
                val level = VoiceMessages.liveShape(bars[bars.size - 1 - k])
                val height = maxOf(minHeight * maxOf(0.6f, scale), level * scale * h * 0.94f)
                val width = bar * (scale + 0.4f).coerceIn(0.6f, 1f)
                drawRoundRect(
                    mixRgb(LiveQuiet, Action, minOf(1f, 0.25f + 0.9f * level), alpha),
                    Offset(x + (bar - width) / 2, (h - height) / 2), Size(width, height),
                    CornerRadius(minOf(width, height) / 2),
                )
            }
        }
    })
}

private val LiveBarWidth = 3.dp
/** A 3 dp bar and a 3 dp gap. */
private val LiveBarStep = 6.dp
private val LiveBarMinHeight = 3.dp
/** How small bars get at the left edge (70% smaller), over this distance (at most 80% of the meter). */
private const val LivePinch = 0.7f
private val LivePinchReach = 40.dp
/** New bars enter flat and reach full size this far in, fading in over the last [LiveArrival]. */
private val LiveEntry = 36.dp
private val LiveArrival = 10.dp
/** Quiet bars' colour; loud ones brighten toward eggshell ([Action]). */
private val LiveQuiet = Color(0xFF969C9E)

private fun smoothstep(from: Float, to: Float, t: Float): Float {
    val u = ((t - from) / (to - from)).coerceIn(0f, 1f)
    return u * u * (3 - 2 * u)
}

private fun easeOutCubic(t: Float): Float {
    val u = 1 - t.coerceIn(0f, 1f)
    return 1 - u * u * u
}

/** A straight sRGB mix, as the web meter draws it, at [alpha]. */
private fun mixRgb(from: Color, to: Color, t: Float, alpha: Float) = Color(
    from.red + (to.red - from.red) * t, from.green + (to.green - from.green) * t, from.blue + (to.blue - from.blue) * t, alpha,
)

/**
 * Above the composer: while holding, what release does; otherwise a short hint or the
 * permission error. Hands-free recording explains itself in the bar.
 */
@Composable internal fun VoiceRecordingNotice(state: VoiceRecorderState) {
    val sendOnStop = VoicePreferences.sendOnStop(LocalContext.current)
    val text = when {
        state.recording && !state.handsFree -> if (sendOnStop) "Release to send" else "Release to add it to your message"
        state.recording -> null
        else -> state.notice
    } ?: return
    Text(
        text, Modifier.fillMaxWidth().padding(bottom = 6.dp).semantics { liveRegion = LiveRegionMode.Polite },
        color = if (!state.recording && state.noticeIsError) ErrorText else TextMuted, fontSize = 12.sp,
    )
}
