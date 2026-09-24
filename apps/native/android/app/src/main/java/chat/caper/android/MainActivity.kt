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
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import chat.caper.android.model.*
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import chat.caper.android.voice.MicComparison
import chat.caper.android.voice.MicComparisonBinding
import chat.caper.android.voice.PrejoinMicTest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

class MainActivity : ComponentActivity() {
    private val viewModel: CaperViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { CaperTheme { CaperApp(viewModel) } }
    }
}

private sealed interface Overlay {
    data object CreateSpace : Overlay
    data object ManageSpace : Overlay
    data object CreateChannel : Overlay
    data class ManageChannel(val channel: Channel) : Overlay
    data object LeaveSpace : Overlay
    data object Profile : Overlay
    data object Audio : Overlay
}

internal data class VoiceJoinIntent(
    val channelId: String,
    val spaceId: String,
    val channelName: String,
    val spaceName: String,
    val displayName: String,
    val accountId: String?,
    val accountEpoch: Long,
    val demo: Boolean,
) {
    fun isCurrent(state: AppUiState, currentAccountEpoch: Long): Boolean =
        state.screen == SessionScreen.Home && state.selectedChannel?.id == channelId &&
            state.selectedSpace?.space?.id == spaceId && state.account?.id == accountId &&
            currentAccountEpoch == accountEpoch && state.selectedSpace.space.demo == demo
}

@Composable private fun CaperApp(viewModel: CaperViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val voice by VoiceCallService.state.collectAsStateWithLifecycle()
    var overlay by remember { mutableStateOf<Overlay?>(null) }
    var navigationOpen by remember { mutableStateOf(false) }

    val homeVisible = state.screen == SessionScreen.Home || state.screen is SessionScreen.Spaces
    Scaffold(containerColor = Blackout, snackbarHost = {
        if (homeVisible) state.error?.let { Snackbar(containerColor = SurfaceRaised) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(it, Modifier.weight(1f), color = ErrorText)
                TextButton(viewModel::clearError) { Text("Dismiss") }
            }
        } }
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (val screen = state.screen) {
                SessionScreen.Loading -> BrandLoading()
                SessionScreen.SignedOut -> LoginScreen(state.busy, state.error, viewModel::clearError, viewModel::cancelAccountFlow, viewModel::requestCode)
                is SessionScreen.Verify -> VerifyScreen(screen, state.busy, state.error, viewModel::clearError, viewModel::cancelAccountFlow, viewModel::verify)
                is SessionScreen.Profile -> ProfileScreen(screen.account, state.busy, null, viewModel::saveProfile)
                SessionScreen.Home, is SessionScreen.Spaces -> HomeScreen(
                    state, voice, navigationOpen, { navigationOpen = it }, { overlay = it }, viewModel,
                )
            }
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter), color = Terracotta)
        }
    }

    when (val shown = overlay) {
        Overlay.CreateSpace -> CreateSpaceDialog(state.busy, { overlay = null }) { viewModel.createSpace(it) { overlay = null } }
        Overlay.ManageSpace -> state.selectedSpace?.let { detail -> ManageSpaceDialog(state, detail, viewModel, { overlay = null }) }
        Overlay.CreateChannel -> state.selectedSpace?.let { detail -> CreateChannelDialog(detail, state.busy, { overlay = null }) { name, private -> viewModel.createChannel(name, private) { overlay = null } } }
        is Overlay.ManageChannel -> ManageChannelDialog(state, shown.channel, viewModel) { overlay = null }
        Overlay.LeaveSpace -> ConfirmDialog("Leave ${state.selectedSpace?.space?.name}?", "You will lose access to its channels and messages.", "Leave space", state.busy, { overlay = null }) { viewModel.leaveCurrentSpace { overlay = null } }
        Overlay.Profile -> state.account?.let { account -> ProfileScreen(account, state.busy, { overlay = null }) { username, display -> viewModel.updateProfile(username, display); overlay = null } }
        Overlay.Audio -> AudioSettingsDialog(state, voice, { overlay = null }, viewModel::logout)
        null -> Unit
    }
}

@Composable private fun BrandLoading() = Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Wordmark(); CircularProgressIndicator(color = Terracotta)
    }
}

@Composable private fun Wordmark(modifier: Modifier = Modifier) = Row(modifier) {
    Text("caper", color = Text, fontSize = 28.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.25).sp)
    Text(".", color = TerracottaBright, fontSize = 28.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.25).sp)
}

@Composable private fun HomeScreen(
    state: AppUiState,
    voice: VoiceState,
    navigationOpen: Boolean,
    setNavigationOpen: (Boolean) -> Unit,
    show: (Overlay) -> Unit,
    viewModel: CaperViewModel,
) {
    Column(Modifier.fillMaxSize()) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val narrow = maxWidth <= 760.dp
            val medium = maxWidth in 761.dp..1099.dp
            var membersVisible by remember { mutableStateOf(!narrow) }
            LaunchedEffect(narrow) { if (narrow) membersVisible = false }
            Surface(
                Modifier.fillMaxSize(),
                color = Surface,
            ) {
                if (narrow) Box {
                    if (navigationOpen) Row {
                        SpaceRail(state, viewModel, show, Modifier.width(60.dp))
                        ChannelSidebar(state, voice, viewModel, show, Modifier.weight(1f)) { setNavigationOpen(false) }
                    } else Conversation(state, voice, viewModel, show, true, membersVisible, { membersVisible = !membersVisible }) { setNavigationOpen(true) }
                    if (membersVisible && !navigationOpen) MemberPresencePanel(state, viewModel, Modifier.widthIn(max = 280.dp).fillMaxHeight().align(Alignment.CenterEnd))
                } else Row {
                    SpaceRail(state, viewModel, show, Modifier.width(60.dp))
                    ChannelSidebar(state, voice, viewModel, show, Modifier.width(280.dp))
                    if (medium) Column(Modifier.weight(1f)) {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, Modifier.weight(1f)) { setNavigationOpen(true) }
                        if (membersVisible) MemberPresencePanel(state, viewModel, Modifier.fillMaxWidth().heightIn(max = 240.dp), compact = true)
                    } else {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, Modifier.weight(1f)) { setNavigationOpen(true) }
                        if (membersVisible) MemberPresencePanel(state, viewModel, Modifier.width(220.dp).fillMaxHeight())
                    }
                }
            }
        }
    }
}

@Composable private fun SpaceRail(state: AppUiState, viewModel: CaperViewModel, show: (Overlay) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxHeight().background(Blackout).padding(vertical = 14.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        state.spaces.forEach { space ->
            val selected = state.selectedSpace?.space?.id == space.id
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.width(3.dp).height(if (selected) 24.dp else 0.dp).background(if (selected) TerracottaBright else Color.Transparent))
                Spacer(Modifier.width(7.dp))
                Surface(
                    Modifier.size(40.dp).clickable { viewModel.selectSpace(space.id) }.semantics { contentDescription = space.name },
                    color = if (selected) TerracottaDark else Surface, shape = MaterialTheme.shapes.medium,
                    border = BorderStroke(1.dp, if (selected) TerracottaBorder else Border),
                ) { Box(contentAlignment = Alignment.Center) { Text(space.name.take(1).uppercase(), fontWeight = FontWeight.Black, color = if (selected) Color.White else TextMuted) } }
            }
        }
        Surface(
            Modifier.size(40.dp).clickable { if (state.account == null) viewModel.showLogin() else show(Overlay.CreateSpace) }.semantics { contentDescription = "Create space" },
            color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, TerracottaBorder),
        ) { Box(contentAlignment = Alignment.Center) { Icon(Icons.Default.Add, null, tint = TerracottaBright) } }
    }
}

@Composable private fun ChannelSidebar(
    state: AppUiState,
    voice: VoiceState,
    viewModel: CaperViewModel,
    show: (Overlay) -> Unit,
    modifier: Modifier,
    closeNavigation: (() -> Unit)? = null,
) {
    val detail = state.selectedSpace
    val owner = state.account?.id == detail?.space?.ownerId
    Column(modifier.fillMaxHeight().background(SurfaceSidebar)) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp)) {
            Row(Modifier.fillMaxWidth().heightIn(min = 42.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(detail?.space?.name ?: "Caper", Modifier.weight(1f), fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (closeNavigation != null) IconButton(closeNavigation) { Icon(Icons.Default.Close, "Close navigation", tint = TextMuted) }
                if (detail != null && !detail.space.demo) IconButton({ show(if (owner) Overlay.ManageSpace else Overlay.LeaveSpace) }) { Icon(if (owner) Icons.Default.Settings else Icons.Default.Logout, "Space actions", tint = TextMuted) }
            }
            HorizontalDivider(color = Border)
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("CHANNELS", Modifier.weight(1f), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                if (owner) IconButton({ show(Overlay.CreateChannel) }, Modifier.size(30.dp)) { Icon(Icons.Default.Add, "Create channel", tint = TextMuted) }
            }
            detail?.channels?.forEach { channel ->
                val selected = channel.id == state.selectedChannel?.id
                Row(
                    Modifier.fillMaxWidth().height(38.dp).clip(MaterialTheme.shapes.small)
                        .background(if (selected) TerracottaWash else Color.Transparent)
                        .clickable { viewModel.selectChannel(channel); closeNavigation?.invoke() }
                        .padding(horizontal = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(if (channel.private) Icons.Default.Lock else Icons.Default.Tag, null, Modifier.size(17.dp), tint = if (selected) TerracottaBright else TextMuted)
                    Spacer(Modifier.width(9.dp)); Text(channel.name, Modifier.weight(1f), color = if (selected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
                    if (owner) IconButton({ show(Overlay.ManageChannel(channel)) }, Modifier.size(28.dp)) { Icon(Icons.Default.Settings, "Manage ${channel.name}", Modifier.size(14.dp), tint = TextMuted) }
                }
            }
            VoiceRoster(voice)
        }
        if (voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED) ConnectedVoiceContext(voice)
        AccountBar(state, voice, viewModel, show)
    }
}

@Composable private fun VoiceRoster(voice: VoiceState) {
    if (voice.participants.isEmpty()) return
    val context = LocalContext.current
    var audioParticipant by remember { mutableStateOf<String?>(null) }
    Text("IN VOICE · ${voice.participants.size}", Modifier.padding(start = 9.dp, top = 24.dp), color = TextMuted, fontSize = 11.sp)
    voice.participants.forEach { participant ->
        Column(Modifier.fillMaxWidth()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 9.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
                Avatar(participant.name, 34.dp)
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    Text(participant.name, fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1)
                    if (participant.deafened || participant.muted) Text(if (participant.deafened) "Deafened" else "Muted", color = TextMuted, fontSize = 10.sp)
                }
                if (participant.id != voice.selfId && voice.phase == VoiceState.Phase.CONNECTED) TextButton({ audioParticipant = participant.id.takeUnless { it == audioParticipant } }) { Text("Audio", fontSize = 10.sp) }
                else if (participant.deafened) Icon(Icons.Default.VolumeOff, null, Modifier.size(15.dp), tint = TextMuted)
                else if (participant.muted) Icon(Icons.Default.MicOff, null, Modifier.size(15.dp), tint = TextMuted)
            }
            if (audioParticipant == participant.id) Surface(Modifier.fillMaxWidth().padding(horizontal = 9.dp, vertical = 4.dp), color = Blackout, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                Column(Modifier.padding(10.dp)) {
                    val volume = voice.participantVolumes[participant.id] ?: 100
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) { Text("User volume", fontSize = 11.sp, fontWeight = FontWeight.Bold); Text("$volume%", color = TextMuted, fontSize = 10.sp) }
                    Slider(volume.toFloat(), { VoiceCallService.setParticipantVolume(context, participant.id, it.toInt()) }, valueRange = 0f..200f)
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Mute locally", Modifier.weight(1f), fontSize = 11.sp, fontWeight = FontWeight.Bold)
                        Switch(participant.id in voice.locallyMutedParticipants, { VoiceCallService.toggleParticipantMute(context, participant.id) })
                    }
                }
            }
        }
    }
}

@Composable private fun ConnectedVoiceContext(voice: VoiceState) {
    Surface(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp), color = SurfaceRaised, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.small) {
        Column(Modifier.padding(horizontal = 10.dp, vertical = 8.dp)) {
            Text(if (voice.phase == VoiceState.Phase.CONNECTED) "Voice connected" else "Connecting voice…", color = CaperGreen, fontSize = 11.sp, fontWeight = FontWeight.Bold)
            Text(listOfNotNull(voice.spaceName, voice.channelName).joinToString(" / ").ifEmpty { "General" }, color = TextMuted, fontSize = 10.sp)
        }
    }
}

@Composable private fun MemberPresencePanel(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, compact: Boolean = false) {
    val members = state.selectedSpace?.members ?: return
    val detail = state.selectedSpace ?: return
    val pages = ((members.size + 24) / 25).coerceAtLeast(1)
    val shown = members.drop(state.presencePage * 25).take(25)
    Column(modifier.background(SurfaceSidebar).then(if (compact) Modifier else Modifier)) {
        Row(Modifier.fillMaxWidth().height(54.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Members", Modifier.weight(1f), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            if (!detail.space.demo) Text(members.size.toString(), color = TextMuted, fontSize = 10.sp)
        }
        HorizontalDivider(color = Border)
        if (detail.space.demo) Text("General is open to everyone. People in voice appear in the channel sidebar.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp, lineHeight = 16.sp)
        else if (members.isEmpty()) Text("No members to show.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp)
        else LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(8.dp)) {
            items(shown, key = { it.id }) { member ->
                val status = state.presence[member.id] ?: "unknown"
                Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 8.dp, vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box {
                        Avatar(member.displayName, 30.dp)
                        Box(Modifier.size(9.dp).align(Alignment.BottomEnd).clip(CircleShape).background(when (status) { "online" -> CaperGreen; "idle" -> Idle; else -> Border }))
                    }
                    Spacer(Modifier.width(10.dp)); Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        if (pages > 1) Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            TextButton({ viewModel.setPresencePage(state.presencePage - 1) }, enabled = state.presencePage > 0) { Text("Previous", fontSize = 10.sp) }
            Text("${state.presencePage + 1} / $pages", color = TextMuted, fontSize = 10.sp)
            TextButton({ viewModel.setPresencePage(state.presencePage + 1) }, enabled = state.presencePage + 1 < pages) { Text("Next", fontSize = 10.sp) }
        }
    }
}

@Composable private fun AccountAvatar(state: AppUiState) {
    val name = state.account?.displayName ?: "Guest"
    Box {
        Avatar(name, 30.dp)
        state.account?.let { account ->
            val status = state.presence[account.id]
            Box(Modifier.size(9.dp).align(Alignment.BottomEnd).clip(CircleShape).background(when (status) { "online" -> CaperGreen; "idle" -> Idle; else -> Border }))
        }
    }
}

@Composable private fun AccountBar(state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit) {
    val context = LocalContext.current
    Surface(Modifier.fillMaxWidth().padding(12.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Row(Modifier.height(42.dp).padding(5.dp), verticalAlignment = Alignment.CenterVertically) {
            Row(Modifier.weight(1f).fillMaxHeight().clickable { if (state.account == null) viewModel.showLogin() else show(Overlay.Profile) }, verticalAlignment = Alignment.CenterVertically) {
                AccountAvatar(state); Spacer(Modifier.width(7.dp))
                Text(state.account?.displayName ?: "Guest", maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            }
            if (voice.phase == VoiceState.Phase.CONNECTED) {
                IconButton({ VoiceCallService.toggleMute(context) }, Modifier.size(30.dp)) { Icon(if (voice.muted) Icons.Default.MicOff else Icons.Default.Mic, "Toggle mute", Modifier.size(18.dp), tint = if (voice.muted) TerracottaBright else TextMuted) }
                IconButton({ VoiceCallService.toggleDeafen(context) }, Modifier.size(30.dp)) { Icon(if (voice.deafened) Icons.Default.VolumeOff else Icons.Default.Headphones, "Toggle deafen", Modifier.size(18.dp), tint = if (voice.deafened) TerracottaBright else TextMuted) }
            }
            IconButton({ show(Overlay.Audio) }, Modifier.size(30.dp)) { Icon(Icons.Default.Settings, "Audio and account settings", Modifier.size(18.dp), tint = TextMuted) }
        }
    }
}

@Composable private fun Conversation(
    state: AppUiState,
    voice: VoiceState,
    viewModel: CaperViewModel,
    show: (Overlay) -> Unit,
    narrow: Boolean,
    membersVisible: Boolean,
    toggleMembers: () -> Unit,
    modifier: Modifier = Modifier,
    openNavigation: () -> Unit,
) {
    val channel = state.selectedChannel
    if (channel == null) return Box(modifier.fillMaxSize().background(SurfaceConversation), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) { Icon(Icons.Default.Tag, null, tint = TerracottaBright); Text("No accessible channels", fontWeight = FontWeight.Bold); Text("Choose or create a channel.", color = TextMuted) }
    }
    val context = LocalContext.current
    val latestState by rememberUpdatedState(state)
    var pendingVoiceJoin by remember { mutableStateOf<VoiceJoinIntent?>(null) }
    var voicePermissionError by remember(channel.id) { mutableStateOf<String?>(null) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        val requested = pendingVoiceJoin
        pendingVoiceJoin = null
        val current = latestState
        if (requested?.isCurrent(current, viewModel.accountEpoch) == true) {
            if (grants[Manifest.permission.RECORD_AUDIO] == true) {
                voicePermissionError = null
                VoiceCallService.start(
                    context, requested.channelId, requested.spaceId, requested.channelName,
                    requested.spaceName, requested.displayName, requested.demo,
                )
            } else {
                voicePermissionError = "Microphone permission is required to join voice. Allow microphone access in Android app settings or try Join again."
            }
        }
    }
    var draft by remember(channel.id) { mutableStateOf("") }
    val inCall = voice.channelId == channel.id && voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED
    Column(modifier.fillMaxHeight().background(SurfaceConversation)) {
        // Match the 34dp message-avatar column without shrinking the 48dp menu target.
        Row(Modifier.fillMaxWidth().height(53.dp).padding(start = if (narrow) 11.dp else 18.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
            if (narrow) IconButton(openNavigation, Modifier.size(48.dp)) {
                Icon(Icons.Default.Menu, "Open navigation", Modifier.size(24.dp))
            }
            Text("# ${channel.name}", Modifier.weight(1f).padding(start = if (narrow) 3.dp else 0.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (state.gateway != GatewayStatus.LIVE) Text(if (state.gateway == GatewayStatus.ERROR) "Offline" else "Connecting…", color = TextMuted, fontSize = 11.sp)
            Spacer(Modifier.width(10.dp))
            if (BuildConfig.ENABLE_NATIVE_VOICE) Button({
                if (inCall) VoiceCallService.stop(context) else {
                    voicePermissionError = null
                    val space = requireNotNull(state.selectedSpace?.space)
                    pendingVoiceJoin = VoiceJoinIntent(
                        channel.id, space.id, channel.name, space.name,
                        state.account?.displayName ?: "Guest", state.account?.id,
                        viewModel.accountEpoch, space.demo,
                    )
                    permission.launch(buildList {
                        add(Manifest.permission.RECORD_AUDIO); if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
                    }.toTypedArray())
                }
            }, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.buttonColors(containerColor = TerracottaWash, contentColor = TerracottaBright), border = BorderStroke(1.dp, TerracottaBorder), contentPadding = PaddingValues(horizontal = 12.dp)) {
                Icon(if (inCall) Icons.Default.CallEnd else Icons.Default.RecordVoiceOver, null, Modifier.size(16.dp)); Spacer(Modifier.width(7.dp)); Text(if (inCall) "Leave" else "Join")
            }
            IconButton(toggleMembers, Modifier.size(36.dp)) { Icon(Icons.Default.People, if (membersVisible) "Hide member list" else "Show member list", tint = if (membersVisible) Text else TextMuted) }
        }
        HorizontalDivider(color = Border)
        (voicePermissionError ?: voice.error.takeIf { voice.channelId == channel.id })?.let { error ->
            Text(error, Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp), color = ErrorText, fontSize = 12.sp)
        }
        MessageTimeline(state, viewModel, Modifier.weight(1f))
        TypingLine(state.typingAuthors)
        HorizontalDivider(color = Border)
        Column(Modifier.padding(horizontal = 18.dp, vertical = 12.dp)) {
            state.pendingMessage?.error?.let { pending ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(if (state.pendingMessage.rejected) "Not sent. $pending" else "Not confirmed yet. $pending", Modifier.weight(1f), color = ErrorText, fontSize = 11.sp)
                    TextButton({
                        if (state.pendingMessage.rejected) {
                            if (canEditRejectedMessage(draft, state.pendingMessage.text)) viewModel.discardPending()?.let { draft = it }
                        } else viewModel.send(state.pendingMessage.text)
                    }, enabled = !state.pendingMessage.rejected || canEditRejectedMessage(draft, state.pendingMessage.text)) {
                        Text(if (state.pendingMessage.rejected) "Edit" else "Retry send")
                    }
                    if (state.pendingMessage.rejected) TextButton({ viewModel.discardPending() }) { Text("Dismiss") }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                OutlinedTextField(
                    draft, { value -> draft = value.codePointTake(4000); viewModel.reportActivity(); viewModel.setTyping(value.isNotBlank()) },
                    modifier = Modifier.weight(1f), placeholder = { Text("Message #${channel.name}") }, maxLines = 6,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { if (draft.isNotBlank() && state.pendingMessage == null) { val sent = draft; viewModel.setTyping(false); viewModel.send(sent); draft = "" } }),
                    colors = OutlinedTextFieldDefaults.colors(focusedContainerColor = SurfaceComposer, unfocusedContainerColor = SurfaceComposer, focusedBorderColor = Terracotta, unfocusedBorderColor = Border),
                )
                FilledIconButton(
                    { if (draft.isNotBlank() && state.pendingMessage == null) { val sent = draft; viewModel.setTyping(false); viewModel.send(sent); draft = "" } },
                    modifier = Modifier.size(48.dp).semantics { contentDescription = "Send" }, enabled = draft.isNotBlank() && state.pendingMessage == null,
                    shape = MaterialTheme.shapes.small,
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = Terracotta, contentColor = Color.White,
                        disabledContainerColor = Terracotta.copy(alpha = 0.55f), disabledContentColor = Color.White.copy(alpha = 0.6f),
                    ),
                ) { Icon(Icons.Default.ArrowUpward, null) }
            }
            if (draft.codePointCount(0, draft.length) >= 3000) Text("${draft.codePointCount(0, draft.length)} / 4,000", Modifier.align(Alignment.End), color = if (draft.codePointCount(0, draft.length) >= 3900) ErrorText else TextMuted, fontSize = 10.sp)
        }
    }
}

@Composable private fun MessageTimeline(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier) {
    LazyColumn(modifier.fillMaxWidth(), reverseLayout = false, contentPadding = PaddingValues(vertical = 8.dp)) {
        item {
            Box(Modifier.fillMaxWidth().height(44.dp), contentAlignment = Alignment.Center) {
                when {
                    state.olderError != null -> TextButton(viewModel::loadOlder) { Text("Couldn’t load older messages · Retry") }
                    state.hasMoreMessages -> OutlinedButton(viewModel::loadOlder, enabled = !state.loadingOlder, border = BorderStroke(1.dp, Border)) { Text(if (state.loadingOlder) "Loading…" else "Load older messages") }
                    else -> Text("Beginning of conversation", color = TextMuted, fontSize = 11.sp)
                }
            }
        }
        itemsIndexed(state.messages, key = { _, message -> message.id }) { index, message ->
            val previous = state.messages.getOrNull(index - 1)
            MessageRow(message, grouped = previous?.author?.id == message.author.id && minutesBetween(previous.createdAt, message.createdAt) <= 5)
        }
        state.pendingMessage?.let { pending -> item("pending:${pending.clientMessageId}") {
            MessageRow(pending.author?.name ?: "You", pending.author?.isGuest == true, pending.createdAt, pending.text, true)
        } }
        if (state.messages.isEmpty() && state.pendingMessage == null) item { Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) { Text("No messages yet. Start the conversation.", color = TextMuted) } }
    }
}

@Composable private fun MessageRow(message: ChatMessage, grouped: Boolean = false) = MessageRow(message.author.name, message.author.isGuest, message.createdAt, message.content.text, false, grouped)
@Composable private fun MessageRow(author: String, guest: Boolean, createdAt: String, text: String, pending: Boolean, grouped: Boolean = false) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = if (grouped) 2.dp else 10.dp)) {
        if (grouped) Spacer(Modifier.width(34.dp)) else Avatar(author, 34.dp)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            if (!grouped) Row(verticalAlignment = Alignment.CenterVertically) {
                Text(author, fontSize = 13.sp, fontWeight = FontWeight.Bold)
                if (guest) { Spacer(Modifier.width(7.dp)); Surface(color = Color.Transparent, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.extraSmall) { Text("GUEST", Modifier.padding(horizontal = 5.dp, vertical = 2.dp), color = TextMuted, fontSize = 8.sp, fontWeight = FontWeight.Bold) } }
                Spacer(Modifier.width(7.dp)); Text(timeLabel(createdAt), color = TextMuted, fontSize = 10.sp)
            }
            Text(text, color = if (pending) TextMuted else MessageText, fontSize = 14.sp, lineHeight = 21.sp)
        }
    }
}

@Composable private fun TypingLine(authors: List<ChatAuthor>) {
    val label = when { authors.size > 2 -> "Several people are typing…"; authors.size == 2 -> "${authors[0].name} and ${authors[1].name} are typing…"; authors.size == 1 -> "${authors[0].name} is typing…"; else -> "" }
    Text(label, Modifier.fillMaxWidth().height(20.dp).padding(horizontal = 18.dp), color = TextMuted, fontSize = 10.sp)
}

@Composable private fun Avatar(name: String, size: Dp) = Box(Modifier.size(size).clip(CircleShape).background(if (size > 32.dp) SurfaceRaised else SurfaceComposer), contentAlignment = Alignment.Center) {
    Text(name.take(1).uppercase(), fontWeight = FontWeight.Black, fontSize = (size.value * .38f).sp)
}

@Composable private fun AuthFrame(content: @Composable ColumnScope.() -> Unit) {
    BoxWithConstraints(Modifier.fillMaxSize().background(Blackout)) {
        val narrow = maxWidth <= 480.dp
        Column(
            Modifier.padding(horizontal = if (narrow) 20.dp else 0.dp).widthIn(max = 440.dp).fillMaxWidth()
                .align(Alignment.TopCenter).padding(vertical = if (narrow) 32.dp else 64.dp),
        ) {
            Wordmark()
            Spacer(Modifier.height(if (narrow) 42.dp else 56.dp))
            content()
        }
    }
}

@Composable private fun LoginScreen(busy: Boolean, error: String?, clearError: () -> Unit, back: () -> Unit, submit: (String) -> Unit) {
    var email by remember { mutableStateOf("") }
    AuthFrame {
        Text("WELCOME TO CAPER", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.5.sp)
        Text("Come on in.", Modifier.padding(vertical = 20.dp), fontSize = 49.sp, lineHeight = 53.sp, fontWeight = FontWeight.Bold, letterSpacing = (-2.5).sp)
        Text("Use your email to create an account or return to one. No password needed.", color = TextMuted, lineHeight = 26.sp)
        Text("Email address", Modifier.padding(top = 24.dp, bottom = 8.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        OutlinedTextField(
            email, { email = it; if (error != null) clearError() }, placeholder = { Text("you@example.com") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { if (email.contains('@') && !busy) submit(email) }),
            singleLine = true, modifier = Modifier.fillMaxWidth(),
        )
        if (error != null) Surface(Modifier.fillMaxWidth().padding(top = 20.dp), color = Color.Transparent, border = BorderStroke(1.dp, Terracotta), shape = MaterialTheme.shapes.small) {
            Text(error, Modifier.padding(horizontal = 14.dp, vertical = 12.dp), lineHeight = 24.sp)
        }
        Button({ submit(email) }, enabled = email.contains('@') && !busy, modifier = Modifier.fillMaxWidth().padding(top = 28.dp), shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            Text(if (busy) "Sending…" else "Email me a code", Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Start)
            if (!busy) Icon(Icons.Default.ArrowForward, null, Modifier.size(20.dp))
        }
        Text("We only send a code when you ask. Prefer to look around first?", Modifier.padding(top = 10.dp), color = TextMuted, fontSize = 13.sp, lineHeight = 20.sp)
        TextButton(back, contentPadding = PaddingValues(0.dp)) { Text("Join general as a guest.", color = Text) }
    }
}

@Composable private fun VerifyScreen(screen: SessionScreen.Verify, busy: Boolean, error: String?, clearError: () -> Unit, back: () -> Unit, submit: (String, String) -> Unit) {
    var code by remember { mutableStateOf("") }
    AuthFrame {
        Text("WELCOME TO CAPER", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.5.sp)
        Text("Check your email.", Modifier.padding(vertical = 20.dp), fontSize = 49.sp, lineHeight = 53.sp, fontWeight = FontWeight.Bold, letterSpacing = (-2.5).sp)
        Text("Enter the six-character code sent to ${screen.email}. It expires in 10 minutes.", color = TextMuted, lineHeight = 26.sp)
        Text("Sign-in code", Modifier.padding(top = 24.dp, bottom = 8.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        OutlinedTextField(code, { code = it.uppercase().filter { character -> character in "ABCDEFGHJKMNPQRSTWXYZ23456789" }.take(6); if (error != null) clearError() }, singleLine = true, modifier = Modifier.fillMaxWidth())
        if (error != null) Surface(Modifier.fillMaxWidth().padding(top = 20.dp), color = Color.Transparent, border = BorderStroke(1.dp, Terracotta), shape = MaterialTheme.shapes.small) { Text(error, Modifier.padding(14.dp)) }
        Button({ submit(screen.challengeId, code) }, enabled = code.length == 6 && !busy, modifier = Modifier.fillMaxWidth().padding(top = 28.dp), shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            Text(if (busy) "Checking…" else "Continue", Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Start)
            if (!busy) Icon(Icons.Default.ArrowForward, null, Modifier.size(20.dp))
        }
        TextButton(back, contentPadding = PaddingValues(vertical = 16.dp)) { Text("Use a different email", color = TextMuted) }
    }
}

@Composable private fun ProfileScreen(account: Account, busy: Boolean, close: (() -> Unit)?, submit: (String, String) -> Unit) {
    var username by remember(account.id) { mutableStateOf(account.username.orEmpty()) }
    var name by remember(account.id) { mutableStateOf(account.displayName.orEmpty()) }
    val form: @Composable ColumnScope.() -> Unit = {
        Text(if (account.username == null) "Create your profile" else "Edit profile", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
        Text("Your username is unique. Your display name is what people see in conversations.", color = TextMuted, fontSize = 12.sp)
        OutlinedTextField(username, { username = normalizeUsername(it) }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(name, { name = it.codePointTake(64) }, label = { Text("Display name") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        Button({ submit(username, name) }, enabled = username.length >= 3 && name.isNotBlank() && !busy, modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small) { Text("Save profile") }
    }
    if (close == null) AuthFrame { form() } else CaperDialog("Edit profile", close) { form() }
}

@Composable private fun CreateSpaceDialog(busy: Boolean, close: () -> Unit, create: (String) -> Unit) {
    var name by remember { mutableStateOf("") }
    CaperDialog("Create a space", close) {
        Text("A space keeps channels and members together.", color = TextMuted, fontSize = 12.sp)
        OutlinedTextField(name, { name = it.codePointTake(80) }, label = { Text("Space name") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        DialogActions(close, "Create space", busy || name.isBlank()) { create(name.trim()) }
    }
}

@Composable private fun CreateChannelDialog(detail: SpaceDetail, busy: Boolean, close: () -> Unit, create: (String, Boolean) -> Unit) {
    var name by remember { mutableStateOf("") }; var private by remember { mutableStateOf(false) }
    CaperDialog("Create a channel", close) {
        Text("Add a conversation to ${detail.space.name}.", color = TextMuted, fontSize = 12.sp)
        OutlinedTextField(name, { name = normalizeChannel(it) }, label = { Text("Channel name") }, leadingIcon = { Icon(Icons.Default.Tag, null) }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        PrivacyToggle(private) { private = it }
        DialogActions(close, "Create channel", busy || channelInvalid(name)) { create(name.removeSuffix("-"), private) }
    }
}

@Composable private fun ManageSpaceDialog(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(detail.space.id) { mutableStateOf(detail.space.name) }
    var username by remember { mutableStateOf("") }
    var confirmingDelete by remember { mutableStateOf(false) }
    CaperDialog("Manage space", close, wide = true) {
        OutlinedTextField(name, { name = it.codePointTake(80) }, label = { Text("Space name") }, modifier = Modifier.fillMaxWidth())
        Button({ viewModel.renameSpace(name) }, enabled = !state.busy && name.isNotBlank() && name.trim() != detail.space.name, shape = MaterialTheme.shapes.small) { Text("Save name") }
        HorizontalDivider(color = Border)
        Text("Members · ${detail.members.size}", fontWeight = FontWeight.Bold)
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(username, { username = normalizeUsername(it) }, label = { Text("Existing username") }, modifier = Modifier.weight(1f), singleLine = true)
            Spacer(Modifier.width(8.dp)); Button({ viewModel.addSpaceMember(username); username = "" }, enabled = username.length >= 3 && !state.busy, shape = MaterialTheme.shapes.small) { Text("Add") }
        }
        detail.members.forEach { member -> MemberManagerRow(member, member.owner, { viewModel.removeSpaceMember(member) }) }
        HorizontalDivider(color = Border)
        Text("Delete space", fontWeight = FontWeight.Bold); Text("Delete this space and all its channels for every member.", color = TextMuted, fontSize = 11.sp)
        OutlinedButton({ confirmingDelete = true }, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.outlinedButtonColors(contentColor = ErrorText), border = BorderStroke(1.dp, Danger)) { Text("Delete space") }
    }
    if (confirmingDelete) ConfirmDialog("Delete ${detail.space.name}?", "This permanently deletes every channel and message in the space.", "Delete space", state.busy, { confirmingDelete = false }) { viewModel.deleteCurrentSpace { close() } }
}

@Composable private fun ManageChannelDialog(state: AppUiState, channel: Channel, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(channel.id) { mutableStateOf(channel.name) }; var private by remember(channel.id) { mutableStateOf(channel.private) }
    var username by remember { mutableStateOf("") }; var confirmingDelete by remember { mutableStateOf(false) }
    LaunchedEffect(channel.id, channel.private) { viewModel.loadChannelGrants(channel) }
    CaperDialog("Overview", close, wide = true) {
        OutlinedTextField(name, { name = normalizeChannel(it) }, label = { Text("Channel name") }, leadingIcon = { Icon(Icons.Default.Tag, null) }, modifier = Modifier.fillMaxWidth())
        PrivacyToggle(private) { private = it }
        val dirty = name.removeSuffix("-") != channel.name || private != channel.private
        if (channel.private) {
            HorizontalDivider(color = Border); Text("Private channel access · ${state.channelGrants.size}", fontWeight = FontWeight.Bold)
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(username, { username = normalizeUsername(it) }, label = { Text("Existing username") }, modifier = Modifier.weight(1f), singleLine = true)
                Spacer(Modifier.width(8.dp)); Button({ viewModel.addChannelGrant(channel, username); username = "" }, enabled = username.length >= 3 && !state.busy, shape = MaterialTheme.shapes.small) { Text("Grant") }
            }
            state.channelGrants.forEach { member -> MemberManagerRow(member, member.owner) { viewModel.removeChannelGrant(channel, member) } }
        }
        HorizontalDivider(color = Border); Text("Delete channel", fontWeight = FontWeight.Bold)
        Text("Delete this channel for everyone in the space.", color = TextMuted, fontSize = 11.sp)
        OutlinedButton({ confirmingDelete = true }, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.outlinedButtonColors(contentColor = ErrorText), border = BorderStroke(1.dp, Danger)) { Text("Delete channel") }
        if (dirty) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            TextButton({ name = channel.name; private = channel.private }) { Text("Reset") }
            Spacer(Modifier.width(8.dp))
            Button({ viewModel.updateChannel(channel, name.removeSuffix("-"), private) }, enabled = !state.busy && !channelInvalid(name), shape = MaterialTheme.shapes.small) { Text("Save changes") }
        }
    }
    if (confirmingDelete) ConfirmDialog("Delete #${channel.name}?", "This permanently deletes its messages.", "Delete channel", state.busy, { confirmingDelete = false }) { viewModel.deleteChannel(channel) { close() } }
}

@Composable private fun MemberManagerRow(member: Member, protected: Boolean, remove: () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
        Avatar(member.displayName, 30.dp); Spacer(Modifier.width(9.dp)); Column(Modifier.weight(1f)) { Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.Bold); Text("@${member.username}", color = TextMuted, fontSize = 10.sp) }
        if (protected) Text("Owner", color = TextMuted, fontSize = 10.sp) else TextButton(remove) { Text("Remove", color = ErrorText) }
    }
}

@Composable private fun AudioSettingsDialog(state: AppUiState, voice: VoiceState, close: () -> Unit, logout: () -> Unit) {
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("audio", android.content.Context.MODE_PRIVATE) }
    var inputGain by remember { mutableIntStateOf(preferences.getInt("inputGain", 100)) }
    var strength by remember { mutableIntStateOf(preferences.getInt("strength", 25)) }
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
    fun stopPlayback() {
        playback?.cancel()
        recording?.stopPlayback() // Silence the AudioTrack before publication resumes.
        playback = null
        playingEnhanced = null
    }
    fun playClip(clip: MicComparison, enhanced: Boolean) {
        stopPlayback()
        val ticket = generation
        val next = cleanupScope.launch(start = CoroutineStart.LAZY) {
            if (!dialogActive || ticket != generation || recording !== clip) return@launch
            playingEnhanced = enhanced
            try { clip.play(enhanced, VoiceCallService.state.value.outputVolume) }
            catch (error: CancellationException) { throw error }
            catch (error: Throwable) { testError = error.message ?: "Local playback failed." }
            finally {
                if (playback === coroutineContext[Job]) {
                    playback = null
                    playingEnhanced = null
                }
            }
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
                if (ticket == generation && dialogActive) recording = result
            } finally { if (ticket == generation && dialogActive) finishing = false }
        } else {
            recording = binding?.let { VoiceCallService.finishMicComparison(it) }
            finishing = false
        }
    }
    val microphonePermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!dialogActive || permissionGeneration != generation) Unit
        else if (!granted) {
            finishing = false
            testError = "Microphone permission is required to test audio."
        }
        else {
            testError = null
            recording = null
            val ticket = generation
            val call = VoiceCallService.beginMicComparison()
            if (call != null) {
                binding = call
                finishing = false
                testing = true
            } else if (!VoiceCallService.canPreparePrejoinMicTest()) {
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
                        timeout = cleanupScope.launch { delay(30_000); if (ticket == generation) stopTest() }
                    }
                } catch (error: Throwable) {
                    if (dialogActive && ticket == generation) {
                        finishing = false
                        testError = error.message ?: "Microphone test could not start."
                    }
                }
            }
            if (testing) timeout = cleanupScope.launch { delay(30_000); if (ticket == generation) stopTest() }
        }
    }
    DisposableEffect(Unit) { onDispose {
        dialogActive = false
        teardownTest()
    } }
    DisposableEffect(voice.channelId, voice.phase, voice.selectedRouteId) { onDispose {
        if (dialogActive) teardownTest()
    } }
    CaperDialog("Audio settings", close) {
        Text("Input and output routing", fontWeight = FontWeight.Bold)
        if (Build.VERSION.SDK_INT >= 31 && voice.routes.isNotEmpty()) voice.routes.forEach { route ->
            Row(Modifier.fillMaxWidth().clickable { VoiceCallService.selectRoute(context, route.id) }.padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RadioButton(route.id == voice.selectedRouteId, { VoiceCallService.selectRoute(context, route.id) })
                Spacer(Modifier.width(8.dp)); Text(route.name)
            }
        } else Text("Choose audio input and output in Android system settings. Available communication routes appear here during a call on Android 12 and newer.", color = TextMuted, fontSize = 12.sp)
        Text("Android communication routes follow the selected system device; separate microphone and speaker hardware selectors are not available.", color = TextMuted, fontSize = 11.sp)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) { Text("Input gain", fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("$inputGain%", color = TextMuted, fontSize = 11.sp) }
        Slider(inputGain.toFloat(), { inputGain = it.toInt(); VoiceCallService.setInputGain(context, inputGain); prejoin?.gain(inputGain) }, valueRange = 0f..200f)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) { Text("Processing strength", fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("$strength%", color = TextMuted, fontSize = 11.sp) }
        Slider(strength.toFloat(), { strength = it.toInt(); VoiceCallService.setProcessingStrength(context, strength); prejoin?.processingStrength(strength) }, valueRange = 0f..100f)
        Text("DPDFNet-8 with RNNoise fallback, then voice EQ, compression and limiting. Defaults to 25%.", color = TextMuted, fontSize = 11.sp)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton({ if (testing) stopTest() else {
                finishing = true
                permissionGeneration = generation
                microphonePermission.launch(Manifest.permission.RECORD_AUDIO)
            } },
                enabled = !finishing && (testing || recording == null), shape = MaterialTheme.shapes.small) { Text(if (testing) "Stop mic test" else "Test microphone") }
            if (recording != null) TextButton(::teardownTest) { Text("Done") }
        }
        if (testing) Text("Recording locally for up to 30 seconds. Your test audio is not published.", color = TextMuted, fontSize = 11.sp)
        testError?.let { Text(it, color = ErrorText, fontSize = 11.sp) }
        recording?.let { clip ->
            Text("Compare your microphone", fontWeight = FontWeight.Bold, fontSize = 12.sp)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton({ playClip(clip, false) }, enabled = clip.frames > 0, shape = MaterialTheme.shapes.small) { Text("Natural") }
                OutlinedButton({ playClip(clip, true) }, enabled = clip.frames > 0, shape = MaterialTheme.shapes.small) { Text("Enhanced") }
                if (playingEnhanced != null) TextButton(::stopPlayback) { Text("Stop playback") }
            }
        }
        if (voice.phase == VoiceState.Phase.CONNECTED) {
            Text("Connected to #${voice.channelName}", color = CaperGreen, fontSize = 12.sp)
            Text("Mute and deafen controls remain available in the account bar and ongoing notification.", color = TextMuted, fontSize = 12.sp)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) { Text("Output volume", fontWeight = FontWeight.Bold, fontSize = 12.sp); Text("${voice.outputVolume}%", color = TextMuted, fontSize = 11.sp) }
            Slider(voice.outputVolume.toFloat(), { VoiceCallService.setOutputVolume(context, it.toInt()) }, valueRange = 0f..200f)
            if (state.account?.debugEnabled == true && voice.processing.size == 5) {
                val report = voice.processing
                DiagnosticRow("Microphone processing", when (report[0]) { 1L -> "DPDFNet-8"; 2L -> "RNNoise fallback"; else -> "Unavailable" })
                DiagnosticRow("Processed frames", (report[1] * 480).toString())
                DiagnosticRow("Mean processing", if (report[1] == 0L) "Not sampled" else "%.1f ms".format(report[2] / report[1] / 1000.0))
                DiagnosticRow("Maximum processing", "%.1f ms".format(report[3] / 1000.0))
                DiagnosticRow("Queued microphone", "%.1f ms".format(report[4] / 48.0))
            }
            voice.diagnostics?.let { diagnostics ->
                HorizontalDivider(color = Border)
                Text("Connection details", fontWeight = FontWeight.Bold)
                DiagnosticRow("Received", formatBytes(diagnostics.receivedBytes))
                DiagnosticRow("Live receive", formatBitrate(diagnostics.receiveBitrate))
                DiagnosticRow("Sent", formatBytes(diagnostics.sentBytes))
                DiagnosticRow("Live send", formatBitrate(diagnostics.sendBitrate))
                DiagnosticRow("Packets lost", diagnostics.packetsLost.toString())
                DiagnosticRow("Max jitter", "${diagnostics.maxJitterMs} ms")
                DiagnosticRow("RTT", "${diagnostics.roundTripMs} ms")
                DiagnosticRow("Route", when (diagnostics.route) { "relay" -> "TURN relay"; "direct" -> "Direct"; else -> "Not observed yet" })
                Text("Local estimates; counters reset when the call ends.", color = TextMuted, fontSize = 10.sp)
            }
        } else Text("Join voice to inspect an active connection.", color = TextMuted, fontSize = 12.sp)
        if (state.account != null) {
            HorizontalDivider(color = Border)
            Text("Account", fontWeight = FontWeight.Bold)
            Text("Signed in as ${state.account.displayName} (@${state.account.username}).", color = TextMuted, fontSize = 12.sp)
            OutlinedButton({ close(); logout() }, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.outlinedButtonColors(contentColor = ErrorText), border = BorderStroke(1.dp, Danger)) { Text("Sign out") }
        }
    }
}

internal fun canEditRejectedMessage(draft: String, rejectedText: String): Boolean =
    draft.isBlank() || draft == rejectedText

@Composable private fun DiagnosticRow(label: String, value: String) = Row(Modifier.fillMaxWidth().padding(vertical = 3.dp), horizontalArrangement = Arrangement.SpaceBetween) {
    Text(label, color = TextMuted, fontSize = 11.sp); Text(value, fontSize = 11.sp)
}
private fun formatBytes(value: Long) = when { value >= 1_000_000 -> "%.1f MB".format(value / 1_000_000.0); value >= 1_000 -> "%.1f KB".format(value / 1_000.0); else -> "$value B" }
private fun formatBitrate(value: Long) = "${value / 1_000} kbps"

@Composable private fun PrivacyToggle(value: Boolean, change: (Boolean) -> Unit) = Row(Modifier.fillMaxWidth().clickable { change(!value) }, verticalAlignment = Alignment.CenterVertically) {
    Icon(Icons.Default.Lock, null, Modifier.size(17.dp), tint = TextMuted); Spacer(Modifier.width(8.dp)); Column(Modifier.weight(1f)) { Text("Private channel", fontWeight = FontWeight.Bold, fontSize = 13.sp); Text(if (value) "Only you and the people you add can view or join." else "Anyone in this space can view or join this channel.", color = TextMuted, fontSize = 11.sp) }; Switch(value, change)
}

@Composable private fun ConfirmDialog(title: String, body: String, action: String, busy: Boolean, close: () -> Unit, confirm: () -> Unit) = CaperDialog(title, close) {
    Text(body, color = TextMuted); Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { TextButton(close) { Text("Cancel") }; Spacer(Modifier.width(8.dp)); Button(confirm, enabled = !busy, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.buttonColors(containerColor = Danger)) { Text(action) } }
}

@Composable private fun CaperDialog(title: String, close: () -> Unit, wide: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    Dialog(close) { Surface(Modifier.widthIn(max = if (wide) 600.dp else 460.dp).fillMaxWidth().heightIn(max = 760.dp), color = Surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(22.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { Text(title, Modifier.weight(1f), fontSize = 19.sp, fontWeight = FontWeight.Bold); IconButton(close) { Icon(Icons.Default.Close, "Close", tint = TextMuted) } }
            HorizontalDivider(color = Border); content()
        }
    } }
}

@Composable private fun DialogActions(close: () -> Unit, label: String, disabled: Boolean, action: () -> Unit) = Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
    TextButton(close) { Text("Cancel") }; Spacer(Modifier.width(8.dp)); Button(action, enabled = !disabled, shape = MaterialTheme.shapes.small) { Text(label) }
}

private fun normalizeUsername(value: String) = value.lowercase().filter { it in 'a'..'z' || it in '0'..'9' || it == '_' }.take(32)
private fun normalizeChannel(value: String) = value.lowercase().replace(Regex("\\s+"), "-").filter { it in 'a'..'z' || it == '-' }.replace(Regex("-+"), "-").removePrefix("-").take(80)
private fun channelInvalid(value: String) = !Regex("^[a-z]+(?:-[a-z]+)*$").matches(value.removeSuffix("-"))
private fun String.codePointTake(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max))
private fun timeLabel(value: String): String = runCatching { DateTimeFormatter.ofPattern("h:mm a").format(Instant.parse(value).atZone(ZoneId.systemDefault())) }.getOrDefault("")
private fun minutesBetween(first: String, second: String): Long = runCatching {
    java.time.Duration.between(Instant.parse(first), Instant.parse(second)).abs().toMinutes()
}.getOrDefault(Long.MAX_VALUE)
