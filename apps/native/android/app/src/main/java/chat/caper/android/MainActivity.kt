package chat.caper.android

import android.Manifest
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.VolumeOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import chat.caper.android.model.*
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState

class MainActivity : ComponentActivity() {
    private val viewModel: CaperViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { CaperTheme { CaperApp(viewModel) } }
    }
}

@Composable private fun CaperApp(viewModel: CaperViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val voice by VoiceCallService.state.collectAsStateWithLifecycle()
    Scaffold(containerColor = Blackout, snackbarHost = {
        state.error?.let { Snackbar { Row(verticalAlignment = Alignment.CenterVertically) { Text(it, Modifier.weight(1f)); TextButton(viewModel::clearError) { Text("Dismiss") } } } }
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (val screen = state.screen) {
                SessionScreen.Loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                SessionScreen.SignedOut -> LoginScreen(state.busy, viewModel::requestCode)
                is SessionScreen.Verify -> VerifyScreen(screen, state.busy, viewModel::verify)
                is SessionScreen.Profile -> ProfileScreen(state.busy, viewModel::saveProfile)
                is SessionScreen.Spaces -> SpacesScreen(state, screen.account, voice, viewModel)
            }
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter))
        }
    }
}

@Composable private fun AuthCard(title: String, subtitle: String, content: @Composable ColumnScope.() -> Unit) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Card(colors = CardDefaults.cardColors(containerColor = Surface), border = BorderStroke(1.dp, Border), modifier = Modifier.widthIn(max = 440.dp)) {
            Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text("CAPER", color = CaperGreen, fontWeight = FontWeight.Black)
                Text(title, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
                Text(subtitle, color = Text.copy(alpha = .7f))
                content()
            }
        }
    }
}

@Composable private fun LoginScreen(busy: Boolean, submit: (String) -> Unit) {
    var email by remember { mutableStateOf("") }
    AuthCard("A quieter place to talk", "Sign in with the code sent to your email.") {
        OutlinedTextField(email, { email = it }, label = { Text("Email") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email), singleLine = true, modifier = Modifier.fillMaxWidth())
        Button({ submit(email) }, enabled = email.contains('@') && !busy, modifier = Modifier.fillMaxWidth()) { Text("Send code") }
    }
}

@Composable private fun VerifyScreen(screen: SessionScreen.Verify, busy: Boolean, submit: (String, String) -> Unit) {
    var code by remember { mutableStateOf("") }
    AuthCard("Check your inbox", "Enter the code sent to ${screen.email}.") {
        OutlinedTextField(code, { code = it }, label = { Text("Code") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        Button({ submit(screen.challengeId, code) }, enabled = code.isNotBlank() && !busy, modifier = Modifier.fillMaxWidth()) { Text("Continue") }
    }
}

@Composable private fun ProfileScreen(busy: Boolean, submit: (String, String) -> Unit) {
    var username by remember { mutableStateOf("") }; var name by remember { mutableStateOf("") }
    AuthCard("Create your profile", "Choose how people will find and recognize you.") {
        OutlinedTextField(username, { username = it.lowercase().filter { c -> c in 'a'..'z' || c in '0'..'9' || c == '_' }.take(32) }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(name, { name = it.take(64) }, label = { Text("Display name") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        Button({ submit(username, name) }, enabled = username.length >= 3 && name.isNotBlank() && !busy, modifier = Modifier.fillMaxWidth()) { Text("Enter Caper") }
    }
}

@Composable private fun SpacesScreen(state: AppUiState, account: Account, voice: VoiceState, viewModel: CaperViewModel) {
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().background(Surface).padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Caper", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Black, modifier = Modifier.weight(1f))
            Text(account.displayName.orEmpty(), color = Text.copy(alpha = .7f)); Spacer(Modifier.width(8.dp)); TextButton(viewModel::logout) { Text("Log out") }
        }
        if (state.selectedSpace == null) {
            LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                item { Text("YOUR SPACES", color = Text.copy(alpha = .55f), style = MaterialTheme.typography.labelMedium) }
                items(state.spaces, key = { it.id }) { space -> SurfaceRow(space.name) { viewModel.selectSpace(space.id) } }
                if (state.spaces.isEmpty()) item { Text("You do not belong to a space yet.", color = Text.copy(alpha = .65f), modifier = Modifier.padding(top = 24.dp)) }
            }
        } else if (state.selectedChannel == null) {
            Column(Modifier.padding(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconButton(viewModel::backToSpaces) { Icon(Icons.Default.ArrowBack, "Back to spaces") }
                    Text(state.selectedSpace.space.name, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                }
                Spacer(Modifier.height(16.dp)); Text("CHANNELS", color = Text.copy(alpha = .55f), style = MaterialTheme.typography.labelMedium)
                state.selectedSpace.channels.forEach { channel -> SurfaceRow("# ${channel.name}") { viewModel.selectChannel(channel) } }
            }
        } else ChatScreen(state, account, voice, viewModel)
    }
}

@Composable private fun SurfaceRow(label: String, click: () -> Unit) {
    Surface(color = Surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), modifier = Modifier.fillMaxWidth().clickable(onClick = click)) {
        Text(label, Modifier.padding(16.dp), fontWeight = FontWeight.SemiBold)
    }
}

@Composable private fun ColumnScope.ChatScreen(state: AppUiState, account: Account, voice: VoiceState, viewModel: CaperViewModel) {
    val context = LocalContext.current
    val channel = state.selectedChannel!!
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted[Manifest.permission.RECORD_AUDIO] == true) {
            VoiceCallService.start(context, channel.id, channel.name, account.displayName.orEmpty())
        }
    }
    var draft by remember(channel.id) { mutableStateOf("") }
    val inThisCall = voice.channelId == channel.id && voice.phase != VoiceState.Phase.IDLE
    Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(viewModel::backToChannels) { Icon(Icons.Default.ArrowBack, "Back to channels") }
        Column(Modifier.weight(1f)) { Text("# ${channel.name}", fontWeight = FontWeight.Bold); Text(state.gateway.name.lowercase(), color = Text.copy(alpha = .55f), style = MaterialTheme.typography.labelSmall) }
        if (BuildConfig.ENABLE_NATIVE_VOICE && !inThisCall) IconButton({
            permission.launch(buildList {
                add(Manifest.permission.RECORD_AUDIO)
                if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            }.toTypedArray())
        }) { Icon(Icons.Default.Call, "Join voice") }
        else if (BuildConfig.ENABLE_NATIVE_VOICE) {
            IconButton({ VoiceCallService.toggleMute(context) }) { Icon(if (voice.muted) Icons.Default.MicOff else Icons.Default.Mic, "Toggle mute") }
            IconButton({ context.startService(android.content.Intent(context, VoiceCallService::class.java).setAction(VoiceCallService.ACTION_DEAFEN)) }) {
                Icon(if (voice.deafened) Icons.Default.VolumeOff else Icons.Default.VolumeUp, "Toggle deafen")
            }
            IconButton({ VoiceCallService.stop(context) }) { Icon(Icons.Default.CallEnd, "Leave voice", tint = Terracotta) }
        }
    }
    if (inThisCall) Text("Voice: ${voice.phase.name.lowercase()} · ${voice.participants.size} connected", Modifier.padding(horizontal = 16.dp, vertical = 4.dp), color = CaperGreen)
    HorizontalDivider(color = Border)
    LazyColumn(Modifier.weight(1f).fillMaxWidth(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        items(state.messages, key = { it.id }) { message ->
            Column { Text(message.author.name, fontWeight = FontWeight.Bold); Text(message.content.text, color = Text.copy(alpha = .9f)) }
        }
    }
    Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(draft, { draft = it.take(it.offsetByCodePoints(0, minOf(4000, it.codePointCount(0, it.length)))); viewModel.reportActivity() }, placeholder = { Text("Message #${channel.name}") }, modifier = Modifier.weight(1f), maxLines = 5)
        Spacer(Modifier.width(8.dp)); Button({ val value = draft; viewModel.send(value) { if (draft == value) draft = "" } }, enabled = draft.isNotBlank()) { Text("Send") }
    }
}
