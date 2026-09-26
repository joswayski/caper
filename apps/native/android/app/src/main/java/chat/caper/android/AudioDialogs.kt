package chat.caper.android

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import chat.caper.android.model.AppUiState
import chat.caper.android.ui.*
import chat.caper.android.voice.MicComparison
import chat.caper.android.voice.MicComparisonBinding
import chat.caper.android.voice.PrejoinMicTest
import chat.caper.android.voice.SpeakerTest
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceDiagnostics
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.text.AnnotatedString
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import chat.caper.android.voice.VoiceState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

internal enum class AudioPanel { Test, Connection, Diagnostics }

/** Web's User Settings menu. */
@Composable internal fun AudioSettingsMenu(
    state: AppUiState,
    voice: VoiceState,
    close: () -> Unit,
    open: (AudioPanel) -> Unit,
    logout: () -> Unit,
    signIn: () -> Unit,
) = CaperDialog("Audio settings", close) {
    var soundEffects by remember { mutableStateOf(CaperEffects.enabled) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text("Caper sound effects", Modifier.weight(1f), fontSize = 13.sp)
        Switch(soundEffects, { soundEffects = it; CaperEffects.enabled = it; if (it) CaperEffects.play(CaperEffects.Effect.ToggleOn) })
    }
    val testable = voice.phase == VoiceState.Phase.IDLE || voice.phase == VoiceState.Phase.FAILED || voice.phase == VoiceState.Phase.CONNECTED
    MenuButton("Audio test", testable) { open(AudioPanel.Test) }
    if (voice.phase == VoiceState.Phase.CONNECTED && voice.diagnostics != null) MenuButton("Connection details") { open(AudioPanel.Connection) }
    if (state.account?.debugEnabled == true && voice.phase == VoiceState.Phase.CONNECTED && voice.processing.size == 5)
        MenuButton("Audio diagnostics") { open(AudioPanel.Diagnostics) }
    if (state.account != null) MenuButton("Log out") { close(); logout() }
    else MenuButton("Sign in") { close(); signIn() }
}

/** Web's Input Options / Output Options menus beside mute and deafen. */
@Composable internal fun AudioOptionsMenu(input: Boolean, voice: VoiceState) {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("audio", android.content.Context.MODE_PRIVATE) }
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton({ open = true }, Modifier.size(width = 18.dp, height = 30.dp)) {
            Icon(painterResource(R.drawable.lucide_chevron_down), if (input) "Input Options" else "Output Options", Modifier.size(12.dp), tint = TextMuted)
        }
        DropdownMenu(open, { open = false }, Modifier.width(280.dp), containerColor = SurfaceRaised) {
            Column(Modifier.padding(horizontal = 14.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (input) {
                    // Android records from the system's communication microphone.
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text("Microphone", fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("System default", color = TextMuted, fontSize = 12.sp)
                    }
                    var gain by remember { mutableIntStateOf(preferences.getInt("inputGain", 100)) }
                    VolumeRow("Input volume", gain)
                    Slider(gain.toFloat(), { gain = it.toInt(); VoiceCallService.setInputGain(context, gain) },
                        Modifier.semantics { contentDescription = "Input volume" }, valueRange = 0f..200f)
                } else {
                    Text("Audio output", fontWeight = FontWeight.Bold, fontSize = 12.sp)
                    if (Build.VERSION.SDK_INT >= 31 && voice.routes.isNotEmpty()) voice.routes.forEach { route ->
                        Row(Modifier.fillMaxWidth().clickable { VoiceCallService.selectRoute(context, route.id) }, verticalAlignment = Alignment.CenterVertically) {
                            RadioButton(route.id == voice.selectedRouteId, { VoiceCallService.selectRoute(context, route.id) })
                            Spacer(Modifier.width(8.dp)); Text(route.name, fontSize = 13.sp)
                        }
                    } else Text("Choose audio output in system settings.", color = TextMuted, fontSize = 11.sp)
                    var volume by remember { mutableIntStateOf(preferences.getInt("outputVolume", 100)) }
                    VolumeRow("Output volume", volume)
                    Slider(volume.toFloat(), { volume = it.toInt(); VoiceCallService.setOutputVolume(context, volume) },
                        Modifier.semantics { contentDescription = "Output volume" }, valueRange = 0f..200f)
                }
            }
        }
    }
}

@Composable private fun MenuButton(label: String, enabled: Boolean = true, action: () -> Unit) =
    OutlinedButton(action, Modifier.fillMaxWidth(), enabled = enabled, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Text(label, Modifier.fillMaxWidth(), color = if (enabled) Text else TextMuted)
    }

@Composable private fun VolumeRow(label: String, value: Int) = Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
    Text(label, fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("$value%", color = TextMuted, fontSize = 11.sp)
}

private const val INPUT_METER_SEGMENTS = 40
private const val MAX_RECORDING_SECONDS = 30

/** Web's Audio test (Call.tsx audio dialog with MicPlayback and SpeakerTest). */
@Composable internal fun AudioTestDialog(voice: VoiceState, close: () -> Unit) {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("audio", android.content.Context.MODE_PRIVATE) }
    var inputGain by remember { mutableIntStateOf(preferences.getInt("inputGain", 100)) }
    var strength by remember { mutableIntStateOf(preferences.getInt("strength", 25)) }
    var outputVolume by remember { mutableIntStateOf(preferences.getInt("outputVolume", 100)) }
    val currentOutputVolume by rememberUpdatedState(outputVolume)
    var speakerPlaying by remember { mutableStateOf(false) }
    var speakerError by remember { mutableStateOf(false) }
    val inCall = voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED
    LaunchedEffect(speakerPlaying) {
        if (!speakerPlaying) return@LaunchedEffect
        try { SpeakerTest.loop(context, inCall) { currentOutputVolume } }
        catch (error: CancellationException) { throw error }
        catch (_: Throwable) { speakerPlaying = false; speakerError = true }
    }
    // Preparation owns its scope until it can release the recorder, even when
    // this dialog has already left composition.
    val cleanupScope = remember { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    var generation by remember { mutableLongStateOf(0L) }
    var permissionGeneration by remember { mutableLongStateOf(0L) }
    var testing by remember { mutableStateOf(false) }
    var finishing by remember { mutableStateOf(false) }
    var dialogActive by remember { mutableStateOf(true) }
    var prejoin by remember { mutableStateOf<PrejoinMicTest?>(null) }
    var binding by remember { mutableStateOf<MicComparisonBinding?>(null) }
    var recording by remember { mutableStateOf<MicComparison?>(null) }
    var playback by remember { mutableStateOf<Job?>(null) }
    var playingEnhanced by remember { mutableStateOf<Boolean?>(null) }
    var timeout by remember { mutableStateOf<Job?>(null) }
    var testError by remember { mutableStateOf<String?>(null) }
    var deviceError by remember { mutableStateOf(false) }
    var elapsed by remember { mutableFloatStateOf(0f) }
    val levels = remember { mutableStateListOf<Float>().apply { repeat(INPUT_METER_SEGMENTS) { add(0f) } } }
    fun stopPlayback() {
        playback?.cancel()
        recording?.stopPlayback() // Silence the AudioTrack before publication resumes.
        playback = null
        playingEnhanced = null
    }
    fun playClip(clip: MicComparison, enhanced: Boolean, thenEnhanced: Boolean = false) {
        stopPlayback()
        val ticket = generation
        val next = cleanupScope.launch(start = CoroutineStart.LAZY) {
            if (!dialogActive || ticket != generation || recording !== clip) return@launch
            playingEnhanced = enhanced
            var completed = false
            try { clip.play(enhanced, outputVolume); completed = true }
            catch (error: CancellationException) { throw error }
            catch (_: Throwable) { deviceError = true }
            finally {
                if (playback === coroutineContext[Job]) {
                    playback = null
                    playingEnhanced = null
                }
            }
            // Web: Enhanced plays once Natural has ended.
            if (completed && thenEnhanced && playback == null && dialogActive && ticket == generation && recording === clip) playClip(clip, true)
        }
        playback = next
        next.start()
    }
    fun teardownTest() {
        generation++
        timeout?.cancel(); timeout = null
        testing = false
        finishing = false
        prejoin?.let { local ->
            local.stopNow()
            cleanupScope.launch { local.finish() }
        }
        prejoin = null
        stopPlayback()
        recording = null
        binding?.let { VoiceCallService.resumeMicPublication(it) }
        binding = null
    }
    fun finished(result: MicComparison?) {
        recording = result
        if (result != null && result.frames > 0) playClip(result, false, thenEnhanced = true)
    }
    fun stopTest() {
        if (!testing) return
        testing = false
        finishing = true
        timeout?.cancel(); timeout = null
        val ticket = generation
        val local = prejoin
        prejoin = null
        if (local != null) cleanupScope.launch {
            try {
                val result = local.finish()
                if (ticket == generation && dialogActive) { finishing = false; finished(result) }
            } catch (error: Throwable) {
                if (ticket == generation && dialogActive) testError = error.message ?: "Audio recording failed."
            } finally { if (ticket == generation && dialogActive) finishing = false }
        } else {
            val result = binding?.let { VoiceCallService.finishMicComparison(it) }
            finishing = false
            finished(result)
        }
    }
    fun armTimeout() {
        val ticket = generation
        timeout = cleanupScope.launch { delay(MAX_RECORDING_SECONDS * 1_000L); if (ticket == generation) stopTest() }
    }
    val microphonePermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!dialogActive || permissionGeneration != generation) Unit
        else if (!granted) {
            finishing = false
            testError = "Microphone permission is required to test audio."
        } else {
            testError = null
            deviceError = false
            stopPlayback()
            recording = null
            val ticket = generation
            val existing = binding
            val call = if (existing != null) existing.takeIf { VoiceCallService.restartMicComparison(it) } else VoiceCallService.beginMicComparison()
            if (call != null) {
                binding = call
                finishing = false
                testing = true
                armTimeout()
            } else if (existing != null || !VoiceCallService.canPreparePrejoinMicTest()) {
                finishing = false
                testError = "Wait for the current call to connect or leave it before testing the microphone."
            } else cleanupScope.launch {
                try {
                    val local = PrejoinMicTest.prepare(context)
                    if (!dialogActive || ticket != generation || !VoiceCallService.canPreparePrejoinMicTest()) {
                        local.stopNow(); local.finish()
                    } else {
                        prejoin = local
                        finishing = false
                        testing = true
                        armTimeout()
                    }
                } catch (error: Throwable) {
                    if (dialogActive && ticket == generation) {
                        finishing = false
                        testError = error.message ?: "Audio recording failed."
                    }
                }
            }
        }
    }
    LaunchedEffect(testing) {
        if (!testing) { for (index in levels.indices) levels[index] = 0f; return@LaunchedEffect }
        val started = android.os.SystemClock.elapsedRealtime()
        elapsed = 0f
        var lastLevel = 0L
        while (isActive) {
            val now = android.os.SystemClock.elapsedRealtime()
            elapsed = ((now - started) / 1_000f).coerceAtMost(MAX_RECORDING_SECONDS.toFloat())
            if (now - lastLevel >= 80) {
                val rms = prejoin?.level() ?: binding?.let { VoiceCallService.micComparisonLevel(it) } ?: 0f
                // Web scales the display independently from the recorded signal so speech is legible.
                levels.removeAt(0); levels.add(kotlin.math.min(1f, kotlin.math.sqrt(rms) * 2.5f))
                lastLevel = now
            }
            delay(40)
        }
    }
    DisposableEffect(Unit) { onDispose {
        dialogActive = false
        teardownTest()
    } }
    DisposableEffect(voice.channelId, voice.phase, voice.selectedRouteId) { onDispose {
        if (dialogActive) teardownTest()
    } }
    CaperDialog("Audio test", close) {
        Text("Only you can hear these tests.", color = TextMuted, fontSize = 12.sp)
        VolumeRow("Microphone volume", inputGain)
        Slider(inputGain.toFloat(), { inputGain = it.toInt(); CaperEffects.slider(it / 200f); VoiceCallService.setInputGain(context, inputGain); prejoin?.gain(inputGain) },
            modifier = Modifier.semantics { contentDescription = "Test microphone volume" }, valueRange = 0f..200f)
        Text("Speaker", fontWeight = FontWeight.Bold, fontSize = 12.sp)
        if (Build.VERSION.SDK_INT >= 31 && voice.routes.isNotEmpty()) voice.routes.forEach { route ->
            Row(Modifier.fillMaxWidth().clickable { VoiceCallService.selectRoute(context, route.id) }, verticalAlignment = Alignment.CenterVertically) {
                RadioButton(route.id == voice.selectedRouteId, { VoiceCallService.selectRoute(context, route.id) })
                Spacer(Modifier.width(8.dp)); Text(route.name, fontSize = 13.sp)
            }
        } else Text("Choose speakers in system settings.", color = TextMuted, fontSize = 11.sp)
        VolumeRow("Speaker volume", outputVolume)
        Slider(outputVolume.toFloat(), { outputVolume = it.toInt(); CaperEffects.slider(it / 200f); stopPlayback(); VoiceCallService.setOutputVolume(context, outputVolume) },
            modifier = Modifier.semantics { contentDescription = "Test speaker volume" }, valueRange = 0f..200f)
        OutlinedButton({ speakerError = false; speakerPlaying = !speakerPlaying }, shape = MaterialTheme.shapes.small,
            modifier = Modifier.semantics { stateDescription = if (speakerPlaying) "On" else "Off" }) {
            Text(if (speakerPlaying) "Stop speaker test" else "Test speakers")
        }
        if (speakerError) Text("Couldn’t play audio. Check your output and try again.", color = ErrorText, fontSize = 11.sp)
        HorizontalDivider(color = Border)
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(if (testing) "Recording…" else "Try your microphone", Modifier.weight(1f), fontWeight = FontWeight.Bold, fontSize = 14.sp)
            if (testing) {
                Box(Modifier.size(7.dp).clip(CircleShape).background(TerracottaBright))
                Spacer(Modifier.width(5.dp))
                Text("%.1fs".format(elapsed), color = TextMuted, fontSize = 11.sp)
            }
        }
        Text("Less noise. Clearer voice.", color = TextMuted, fontSize = 12.sp)
        Column {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text("Voice enhancement", fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("$strength%", color = TextMuted, fontSize = 11.sp)
            }
            Slider(strength.toFloat(), { strength = it.toInt(); CaperEffects.slider(it / 100f); VoiceCallService.setProcessingStrength(context, strength); prejoin?.processingStrength(strength) },
                modifier = Modifier.semantics { contentDescription = "Voice processing" }, enabled = !testing, valueRange = 0f..100f)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text("Natural", color = TextMuted, fontSize = 10.sp); Text("Enhanced", color = TextMuted, fontSize = 10.sp)
            }
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button({ if (testing) stopTest() else {
                finishing = true
                permissionGeneration = generation
                microphonePermission.launch(Manifest.permission.RECORD_AUDIO)
            } }, enabled = testing || !finishing, shape = MaterialTheme.shapes.small) {
                Text(if (testing) "Stop recording" else if (finishing) "Preparing…" else "Test microphone")
            }
            Column(Modifier.weight(1f)) {
                Text("Input level", color = TextMuted, fontSize = 10.sp)
                Row(Modifier.fillMaxWidth().height(40.dp).semantics { contentDescription = if (testing) "Received microphone level" else "Microphone level inactive" },
                    horizontalArrangement = Arrangement.spacedBy(1.dp), verticalAlignment = Alignment.CenterVertically) {
                    levels.forEach { level ->
                        Box(Modifier.weight(1f).height((3 + Math.round(level * 8) * 4).dp).clip(MaterialTheme.shapes.extraSmall)
                            .background(if (testing) CaperGreen else Border))
                    }
                }
            }
        }
        testError?.let { Text(it, color = ErrorText, fontSize = 11.sp) }
        if (deviceError) Text("Audio output unavailable; choose another device.", color = ErrorText, fontSize = 11.sp)
        recording?.let { clip ->
            val silent = remember(clip) { !clip.hasSignal() }
            Column(Modifier.semantics { contentDescription = "Recorded samples" }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(false, true).forEach { enhanced ->
                    val label = if (enhanced) "Enhanced" else "Natural"
                    val playing = playingEnhanced == enhanced
                    Surface(color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, if (enhanced) TerracottaBorder else Border)) {
                        Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(label, Modifier.weight(1f), fontWeight = FontWeight.Bold, fontSize = 13.sp)
                                IconButton({ if (playing) stopPlayback() else playClip(clip, enhanced) }, enabled = clip.frames > 0,
                                    modifier = Modifier.semantics { contentDescription = "$label audio sample"; stateDescription = if (playing) "Playing" else "Paused" }) {
                                    Icon(if (playing) painterResource(R.drawable.media_stop) else painterResource(R.drawable.media_play), null, tint = TextMuted)
                                }
                            }
                            if (silent) Text("No audible signal detected. Check your mic and try again.", color = ErrorText, fontSize = 11.sp)
                        }
                    }
                }
            }
        }
    }
}

/** Debug accounts only: the native processing report of the connected call. */
@Composable internal fun AudioDiagnosticsDialog(voice: VoiceState, close: () -> Unit) = CaperDialog("Audio diagnostics", close) {
    val clipboard = LocalClipboardManager.current
    var copyStatus by remember { mutableStateOf("") }
    Text("Local diagnostics only. No audio, device identifiers, or credentials are included. Nothing is uploaded.", color = TextMuted, fontSize = 11.sp)
    val report = voice.processing
    if (report.size == 5) {
        val rows = listOf(
            "Microphone processing" to when (report[0]) { 1L -> "DPDFNet-8"; 2L -> "RNNoise fallback"; else -> "Unavailable" },
            "Processed frames" to (report[1] * 480).toString(),
            "Mean processing" to if (report[1] == 0L) "Not sampled" else "%.1f ms".format(report[2] / report[1] / 1000.0),
            "Maximum processing" to "%.1f ms".format(report[3] / 1000.0),
            "Queued microphone" to "%.1f ms".format(report[4] / 48.0),
        )
        rows.forEach { (label, value) -> DiagnosticRow(label, value) }
        // Web's Copy diagnostics, with only what Android measures.
        OutlinedButton({
            val json = detailsJson.encodeToString(JsonObject.serializer(), buildJsonObject { rows.forEach { (label, value) -> put(label, value) } })
            copyStatus = try { clipboard.setText(AnnotatedString(json)); "Copied diagnostics" } catch (_: Throwable) { "Copy failed; try again." }
        }, shape = MaterialTheme.shapes.small) { Text("Copy diagnostics") }
        if (copyStatus.isNotEmpty()) Text(copyStatus, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 11.sp)
    } else Text("No microphone capture started. Open Mic Test or join voice first.", color = TextMuted, fontSize = 12.sp)
}

/** Web's ConnectionDiagnostics, limited to the join phases Android measures. */
@Composable internal fun ConnectionDetailsDialog(voice: VoiceState, close: () -> Unit) = CaperDialog("Connection details", close) {
    // Web samples stats every second; sample while this dialog is open.
    LaunchedEffect(Unit) { while (isActive) { VoiceCallService.refreshDiagnostics(); delay(2_000) } }
    val clipboard = LocalClipboardManager.current
    var copyStatus by remember { mutableStateOf("") }
    val diagnostics = voice.diagnostics.takeIf { voice.phase == VoiceState.Phase.CONNECTED || voice.phase == VoiceState.Phase.RECONNECTING }
    if (diagnostics == null) Text("Join voice to see connection details.", color = TextMuted, fontSize = 12.sp)
    else {
        Column(Modifier.semantics { contentDescription = "Connection statistics" }) {
            connectionDetailRows(diagnostics).forEach { (label, value) -> DiagnosticRow(label, value) }
        }
        OutlinedButton({
            copyStatus = try { clipboard.setText(AnnotatedString(connectionDetailsJson(diagnostics))); "Copied connection details" }
            catch (_: Throwable) { "Copy failed; try again." }
        }, shape = MaterialTheme.shapes.small) { Text("Copy connection details") }
        if (copyStatus.isNotEmpty()) Text(copyStatus, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 11.sp)
    }
}

internal fun connectionDetailRows(diagnostics: VoiceDiagnostics): List<Pair<String, String>> = buildList {
    diagnostics.timing?.let { timing ->
        add("Joined" to "Joined in ${timing.joinedMs} ms")
        add("Session + publish" to "${timing.sessionMs} ms")
        add("Transport + state" to "${timing.transportMs} ms" + (timing.iceMs?.let { " (ICE $it ms)" } ?: ""))
        add("Connectivity checks" to (diagnostics.checks ?: "Not observed yet"))
        add("Roster" to "${timing.rosterMs} ms")
    }
    add("Received" to formatMegabytes(diagnostics.receivedBytes))
    add("Live receive" to formatKbps(diagnostics.receiveBitrate))
    add("Sent" to formatMegabytes(diagnostics.sentBytes))
    add("Live send" to formatKbps(diagnostics.sendBitrate))
    add("Packets lost" to diagnostics.packetsLost.toString())
    add("Max jitter" to "${diagnostics.maxJitterMs} ms")
    add("RTT" to "${diagnostics.roundTripMs} ms")
    add("Route" to when (diagnostics.route) { "relay" -> "TURN relay"; "direct" -> "Direct"; else -> "Not observed yet" })
}

@OptIn(ExperimentalSerializationApi::class)
private val detailsJson = Json { prettyPrint = true; prettyPrintIndent = "  " }

/** Web's copied shape, with only the fields Android measured. */
internal fun connectionDetailsJson(diagnostics: VoiceDiagnostics): String = detailsJson.encodeToString(JsonObject.serializer(), buildJsonObject {
    diagnostics.timing?.let { timing ->
        put("join", "Joined in ${timing.joinedMs} ms")
        put("sessionMs", timing.sessionMs)
        put("transportMs", timing.transportMs)
        timing.iceMs?.let { put("iceMs", it) }
        put("rosterMs", timing.rosterMs)
        diagnostics.checks?.let { put("checks", it) }
    }
    put("receivedBytes", diagnostics.receivedBytes)
    put("sentBytes", diagnostics.sentBytes)
    put("receiveBitrate", diagnostics.receiveBitrate)
    put("sendBitrate", diagnostics.sendBitrate)
    put("packetsLost", diagnostics.packetsLost)
    put("maxJitterMs", diagnostics.maxJitterMs)
    put("roundTripMs", diagnostics.roundTripMs)
    put("route", diagnostics.route)
})

internal fun formatMegabytes(bytes: Long) = String.format(java.util.Locale.US, "%.2f MB", bytes / 1e6)
internal fun formatKbps(bitsPerSecond: Long) = "${Math.round(bitsPerSecond / 1_000.0)} kbps"

@Composable private fun DiagnosticRow(label: String, value: String) = Row(Modifier.fillMaxWidth().padding(vertical = 3.dp), horizontalArrangement = Arrangement.SpaceBetween) {
    Text(label, color = TextMuted, fontSize = 11.sp); Text(value, fontSize = 11.sp)
}
