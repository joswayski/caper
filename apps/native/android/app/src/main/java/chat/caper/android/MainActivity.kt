package chat.caper.android

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.compose.BackHandler
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.spring
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import chat.caper.android.data.LinkRange
import chat.caper.android.data.MAX_CODE_RESENDS
import chat.caper.android.data.RESEND_COOLDOWN_MILLIS
import chat.caper.android.data.RESEND_LIMIT_HINT
import chat.caper.android.data.RESENT_CODE_STATUS
import chat.caper.android.data.TimelineEntry
import chat.caper.android.data.channelKey
import chat.caper.android.data.channelMuted
import chat.caper.android.data.directKey
import chat.caper.android.data.directMuted
import chat.caper.android.data.inheritedLevel
import chat.caper.android.data.muteActive
import chat.caper.android.data.override
import chat.caper.android.data.spaceKey
import chat.caper.android.data.spaceMuted
import chat.caper.android.data.directUnread
import chat.caper.android.data.groupBlocked
import chat.caper.android.data.groupsWithPrevious
import chat.caper.android.data.linkRanges
import chat.caper.android.data.resendLabel
import chat.caper.android.data.resendSecondsLeft
import chat.caper.android.data.mainDirects
import chat.caper.android.data.messageRequests
import chat.caper.android.model.*
import chat.caper.android.push.CaperNotifications
import chat.caper.android.ui.*
import chat.caper.android.voice.VoiceCallService
import chat.caper.android.voice.VoiceState
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale
import kotlin.math.abs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val viewModel: CaperViewModel by viewModels()
    private val dailyBranding by lazy { DailyBrandingAvatar(applicationContext) }
    private var brandingJob: Job? = null
    private var brandingAvatar by mutableStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        CaperEffects.init(applicationContext)
        // A recreated activity or a relaunch from Recents must not reopen an old tap.
        if (savedInstanceState == null) openFromNotification(intent)
        setContent { CompositionLocalProvider(LocalBrandAvatar provides brandingAvatar) { CaperTheme { CaperApp(viewModel) } } }
    }
    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); setIntent(intent); openFromNotification(intent) }

    private fun openFromNotification(intent: Intent) {
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        viewModel.openFromNotification(
            intent.getStringExtra(CaperNotifications.EXTRA_CONVERSATION_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_SPACE_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_CHANNEL_ID),
        )
    }

    override fun onResume() {
        super.onResume()
        viewModel.setForeground(true)
        if (BuildConfig.FIXTURE_MODE) return
        brandingJob?.cancel()
        brandingJob = lifecycleScope.launch {
            while (isActive) {
                brandingAvatar = dailyBranding.update()
                delay(15 * 60 * 1000L)
            }
        }
    }

    override fun onPause() {
        viewModel.setForeground(false)
        brandingJob?.cancel()
        brandingJob = null
        super.onPause()
    }
}

private sealed interface Overlay {
    data object CreateSpace : Overlay
    data class Invitation(val space: Space) : Overlay
    data object ManageSpace : Overlay
    data object CreateChannel : Overlay
    data object StartDirect : Overlay
    data class ManageChannel(val channel: Channel) : Overlay
    data class LeaveChannel(val channel: Channel) : Overlay
    data object LeaveSpace : Overlay
    data object Profile : Overlay
    data object Audio : Overlay
    data object Privacy : Overlay
    data class AudioPanelOverlay(val panel: AudioPanel) : Overlay
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
    val controlEpoch: Long,
    val joinStartedAt: Long = System.currentTimeMillis(),
) {
    fun isCurrent(state: AppUiState, currentAccountEpoch: Long, freshChannelIds: Set<String>? = null): Boolean =
        state.screen == SessionScreen.Home &&
            state.selectedSpace?.space?.id == spaceId && state.account?.id == accountId &&
            currentAccountEpoch == accountEpoch && state.selectedSpace.space.demo == demo &&
            state.selectedSpace.channels.any { it.id == channelId && it.joined } && channelId !in state.deniedVoiceChannels &&
            (freshChannelIds == null || channelId in freshChannelIds)
}

@Composable private fun CaperApp(viewModel: CaperViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val voice by VoiceCallService.state.collectAsStateWithLifecycle()
    var overlay by remember { mutableStateOf<Overlay?>(null) }
    var navigationOpen by remember { mutableStateOf(false) }
    VoiceChimes(voice)

    val homeVisible = state.screen == SessionScreen.Home || state.screen is SessionScreen.Spaces
    Scaffold(containerColor = Blackout, snackbarHost = {
        if (homeVisible) state.error?.let { Snackbar(containerColor = SurfaceRaised) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(it, Modifier.weight(1f), color = ErrorText)
                TextButton(viewModel::clearError) { Text("Dismiss") }
            }
        } }
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding).imePadding()) {
            when (val screen = state.screen) {
                SessionScreen.Loading -> BrandLoading()
                SessionScreen.SignedOut -> LoginScreen(state.busy, state.error, viewModel::clearError, viewModel::requestCode)
                is SessionScreen.Verify -> VerifyScreen(screen, state.busy, state.error, viewModel::clearError, viewModel::showLogin, viewModel::verify, viewModel::resendCode)
                is SessionScreen.Profile -> ProfileScreen(screen.account, state.busy, state.error, null, viewModel::saveProfile)
                SessionScreen.Home, is SessionScreen.Spaces ->
                    // Web's first-space page: an account with no spaces names one.
                    if (state.account != null && state.limits != null && state.invitations.isEmpty() &&
                        state.spaces.none { !it.demo } && state.selectedDirectId == null && !navigationOpen)
                        FirstSpaceScreen(state, viewModel) { navigationOpen = true }
                    else HomeScreen(
                        state, voice, navigationOpen, { navigationOpen = it }, { overlay = it }, viewModel,
                    )
            }
            // Keep the current conversation stable while a space or channel opens.
            if (state.busy && !homeVisible) LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter), color = Terracotta)
        }
    }

    when (val shown = overlay) {
        Overlay.CreateSpace -> CreateSpaceDialog(state.busy, state.error, viewModel::clearError, { overlay = null }) { viewModel.createSpace(it) { overlay = null } }
        is Overlay.Invitation -> InvitationDialog(shown.space, state.busy, state.error, { overlay = null },
            { viewModel.acceptInvitation(shown.space) { overlay = null } },
            { viewModel.declineInvitation(shown.space) { overlay = null } })
        Overlay.ManageSpace -> state.selectedSpace?.let { detail -> ManageSpaceDialog(state, detail, viewModel, { overlay = null }) }
        Overlay.CreateChannel -> state.selectedSpace?.let { detail -> CreateChannelDialog(detail, state.busy, state.error, viewModel::clearError, { overlay = null }) { name, private ->
            // Web opens a new private channel's Overview so people can be added.
            viewModel.createChannel(name, private) { created -> overlay = if (created.private) Overlay.ManageChannel(created) else null }
        } }
        Overlay.StartDirect -> StartDirectDialog(state.busy, state.error, viewModel::clearError, { overlay = null }) { username -> viewModel.startDirect(username) { overlay = null } }
        is Overlay.ManageChannel -> ManageChannelDialog(state, state.selectedSpace?.channels?.find { it.id == shown.channel.id } ?: shown.channel, viewModel) { overlay = null }
        is Overlay.LeaveChannel -> {
            val owner = state.selectedSpace?.space?.ownerId == state.account?.id
            val privateLoss = shown.channel.private && !owner
            ConfirmDialog("Leave #${shown.channel.name}?", if (privateLoss) "You will lose access to this private channel. Another invitation is required to return." else "You can continue to preview this channel and join it again later.", "Leave channel", state.busy, { overlay = null }, pendingLabel = "Leaving…") {
                viewModel.leaveChannel(shown.channel); overlay = null
            }
        }
        Overlay.LeaveSpace -> ConfirmDialog("Leave ${state.selectedSpace?.space?.name}?", "You will lose access to its channels and conversations. An owner can add you again later.", "Leave space", state.busy, { overlay = null }, pendingLabel = "Leaving…", error = state.error) { viewModel.leaveCurrentSpace { overlay = null } }
        Overlay.Profile -> state.account?.let { account -> ProfileScreen(account, state.busy, state.error, { overlay = null }) { username, display -> viewModel.updateProfile(username, display) { overlay = null } } }
        Overlay.Audio -> AudioSettingsMenu(state, voice, { overlay = null }, { overlay = Overlay.AudioPanelOverlay(it) }, viewModel::logout, viewModel::showLogin,
            notifications = { NotificationSettingsSection(state, viewModel) }) { overlay = Overlay.Privacy }
        Overlay.Privacy -> PrivacySettingsDialog(state, viewModel) { overlay = null }
        is Overlay.AudioPanelOverlay -> when (shown.panel) {
            AudioPanel.Test -> AudioTestDialog(voice) { overlay = null }
            AudioPanel.Connection -> ConnectionDetailsDialog(voice) { overlay = null }
            AudioPanel.Diagnostics -> AudioDiagnosticsDialog(voice) { overlay = null }
        }
        null -> Unit
    }
}

@Composable private fun BrandLoading() = Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Wordmark(); CircularProgressIndicator(color = Terracotta)
    }
}

private val LocalBrandAvatar = compositionLocalOf { 0 }

@Composable private fun Wordmark(modifier: Modifier = Modifier) = Box(
    modifier.size(width = 132.dp, height = 35.dp).clearAndSetSemantics { contentDescription = "Caper" },
) {
    Image(painterResource(R.drawable.caper_wordmark_letters), null, Modifier.matchParentSize())
    // Same dot slot as web/Rust/Apple; keep the two images decorative to accessibility.
    Image(painterResource(caperBrandingResources[LocalBrandAvatar.current]), null,
        Modifier.offset(x = (132f * 904 / 1042).dp, y = (35f * 91 / 276).dp)
            .size((132f * 132 / 1042).dp))
}

@Composable private fun HomeScreen(
    state: AppUiState,
    voice: VoiceState,
    navigationOpen: Boolean,
    setNavigationOpen: (Boolean) -> Unit,
    show: (Overlay) -> Unit,
    viewModel: CaperViewModel,
) {
    var channelsExpanded by rememberSaveable { mutableStateOf(true) }
    val conversationState = rememberSaveableStateHolder()
    val context = LocalContext.current
    val latestState by rememberUpdatedState(state)
    var pendingVoiceJoin by remember(viewModel.accountEpoch, viewModel.spaceAccessEpoch, state.selectedSpace?.space?.id) { mutableStateOf<VoiceJoinIntent?>(null) }
    var voicePermissionError by remember { mutableStateOf<String?>(null) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        val requested = pendingVoiceJoin
        if (requested?.isCurrent(latestState, viewModel.accountEpoch) == true &&
            VoiceCallService.joinAuthorizationCurrent(requested.controlEpoch)) {
            if (grants[Manifest.permission.RECORD_AUDIO] == true) {
                voicePermissionError = null
                viewModel.authorizeVoiceJoin(requested, {
                    VoiceCallService.start(context, requested.channelId, requested.spaceId, requested.channelName,
                        requested.spaceName, requested.displayName, requested.demo, requested.controlEpoch,
                        requested.joinStartedAt, latestState.voiceSessionStartedAt[if (requested.demo) "" else requested.channelId])
                    if (pendingVoiceJoin == requested) pendingVoiceJoin = null
                }, {
                    if (pendingVoiceJoin == requested) pendingVoiceJoin = null
                    voicePermissionError = it
                })
            } else {
                pendingVoiceJoin = null
                voicePermissionError = "Microphone permission is required to join voice. Allow microphone access in Android app settings or try Join again."
            }
        } else pendingVoiceJoin = null
    }
    // Push is on by default: on Android 13+ opening the app asks once per account.
    var pushPromptEpoch by remember { mutableLongStateOf(-1L) }
    val pushPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        viewModel.pushPromptAnswered(granted, pushPromptEpoch)
    }
    LaunchedEffect(state.pushPrompt) {
        if (state.pushPrompt && Build.VERSION.SDK_INT >= 33) {
            pushPromptEpoch = viewModel.accountEpoch
            viewModel.pushPromptShown()
            pushPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    LaunchedEffect(voice, pendingVoiceJoin, state) {
        val requested = pendingVoiceJoin ?: return@LaunchedEffect
        if (!requested.isCurrent(state, viewModel.accountEpoch) || !VoiceCallService.joinAuthorizationCurrent(requested.controlEpoch)) pendingVoiceJoin = null
    }
    val joinVoice: (Channel) -> Unit = { channel ->
        val space = state.selectedSpace?.space
        if (pendingVoiceJoin == null && voice.phase != VoiceState.Phase.CONNECTING && voice.phase != VoiceState.Phase.RECONNECTING &&
            BuildConfig.ENABLE_NATIVE_VOICE && space != null && state.voiceAvailable(channel) == true &&
            channel.joined && state.selectedSpace.channels.any { it.id == channel.id && it.joined } && channel.id !in state.deniedVoiceChannels) {
            voicePermissionError = null
            pendingVoiceJoin = VoiceJoinIntent(channel.id, space.id, channel.name, space.name,
                state.account?.displayName ?: "Guest", state.account?.id, viewModel.accountEpoch, space.demo,
                VoiceCallService.beginJoinAuthorization())
            permission.launch(buildList {
                add(Manifest.permission.RECORD_AUDIO)
                if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            }.toTypedArray())
        }
    }
    Column(Modifier.fillMaxSize()) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val narrow = maxWidth <= 760.dp
            // 60 rail + 280 channels + 320 minimum chat + 220 members.
            val medium = maxWidth < 880.dp
            var membersVisible by remember { mutableStateOf(false) }
            LaunchedEffect(narrow) { if (narrow) membersVisible = false }
            BackHandler(enabled = narrow && (membersVisible || navigationOpen)) {
                if (membersVisible && !navigationOpen) membersVisible = false else setNavigationOpen(false)
            }
            Surface(
                Modifier.fillMaxSize(),
                color = Surface,
            ) {
                if (narrow) {
                    val drawer = rememberBrowseDrawer(navigationOpen)
                    val latestSetNavigationOpen by rememberUpdatedState(setNavigationOpen)
                    LaunchedEffect(navigationOpen) { if (!drawer.dragging) drawer.settle(navigationOpen) }
                    // How much of the conversation stays in view beside Browse.
                    val peek = (maxWidth * 0.2f).coerceIn(56.dp, 96.dp)
                    drawer.peek = with(LocalDensity.current) { peek.toPx() }
                    var barHeight by remember { mutableIntStateOf(0) }
                    // Browse is composed only while it shows or the conversation moves.
                    val browseUncovered by remember { derivedStateOf { drawer.progress.value > 0f } }
                    CompositionLocalProvider(LocalBrowseDrawer provides drawer) {
                        Box(Modifier.fillMaxSize().clipToBounds().background(Blackout).browseDrag(drawer) { latestSetNavigationOpen(it) }) {
                            if (navigationOpen || browseUncovered) Column(Modifier.fillMaxSize()) {
                                Row(Modifier.weight(1f).padding(end = peek)) {
                                    SpaceRail(state, viewModel, show, Modifier.width(60.dp))
                                    ChannelSidebar(state, voice, viewModel, show,
                                        Modifier.weight(1f).padding(top = 8.dp, end = 8.dp).clip(RoundedCornerShape(16.dp))
                                            .browseSwipeRegion(drawer, enabled = state.selectedChannel != null),
                                        channelsExpanded, { channelsExpanded = it }, joinVoice, pendingVoiceJoin, voicePermissionError,
                                        { voicePermissionError = null }, showAccountBar = false) { setNavigationOpen(false) }
                                }
                                // The account bar keeps the full width, below the conversation's edge.
                                Box(Modifier.onSizeChanged { barHeight = it.height }) { AccountBar(state, voice, viewModel, show) }
                            }
                            Box(Modifier.fillMaxSize().conversationLayer(drawer, barHeight).background(Surface)
                                .then(if (navigationOpen) Modifier.clearAndSetSemantics {} else Modifier)) {
                                conversationState.SaveableStateProvider(state.selectedChannel?.id ?: "empty") {
                                    Conversation(state, voice, viewModel, show, true, membersVisible, { membersVisible = !membersVisible }, voicePermissionError) { setNavigationOpen(true) }
                                }
                                if (membersVisible && !navigationOpen && state.selectedChannel?.joined == true && state.selectedDirectId == null) {
                                    Box(Modifier.fillMaxSize().padding(top = 54.dp).clickable(
                                        interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() },
                                        indication = null,
                                    ) { membersVisible = false })
                                    MemberPresencePanel(state, viewModel,
                                        Modifier.padding(top = 62.dp, end = 8.dp, bottom = 8.dp).widthIn(max = 280.dp).fillMaxHeight().align(Alignment.CenterEnd)
                                            .clip(RoundedCornerShape(16.dp)).pointerInput(Unit) { detectTapGestures {} },
                                        close = { membersVisible = false })
                                }
                            }
                            // Beside Browse the conversation's edge is inert: tapping or dragging it returns.
                            if (navigationOpen) Box(Modifier.align(Alignment.TopEnd).width(peek).fillMaxHeight()
                                .padding(bottom = with(LocalDensity.current) { barHeight.toDp() })
                                .browseSwipeRegion(drawer, enabled = true)
                                .clickable(onClickLabel = "Back to conversation") { setNavigationOpen(false) }
                                .semantics { contentDescription = "Back to conversation" })
                        }
                    }
                } else Row {
                    SpaceRail(state, viewModel, show, Modifier.width(60.dp))
                    ChannelSidebar(state, voice, viewModel, show, Modifier.width(280.dp), channelsExpanded, { channelsExpanded = it }, joinVoice, pendingVoiceJoin, voicePermissionError, { voicePermissionError = null })
                    if (medium) Box(Modifier.weight(1f).fillMaxHeight()) {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, voicePermissionError, Modifier.fillMaxSize()) { setNavigationOpen(true) }
                        if (membersVisible && state.selectedChannel?.joined == true && state.selectedDirectId == null) MemberPresencePanel(state, viewModel, Modifier.padding(top = 54.dp).width(220.dp).fillMaxHeight().align(Alignment.CenterEnd))
                    } else {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, voicePermissionError, Modifier.weight(1f)) { setNavigationOpen(true) }
                        if (membersVisible && state.selectedChannel?.joined == true && state.selectedDirectId == null) MemberPresencePanel(state, viewModel, Modifier.width(220.dp).fillMaxHeight())
                    }
                }
            }
        }
    }
}

/**
 * Phones show the conversation, or Browse (spaces and channels) with the
 * conversation's edge still in view. The conversation slides over the still
 * Browse, following a finger from the timeline, channel list or that edge.
 * Buttons, Back and channel choices animate the same slide.
 */
@Stable
private class BrowseDrawer(private val scope: CoroutineScope, open: Boolean) {
    /** How much of Browse is uncovered: 0 shows the conversation, 1 shows Browse. */
    val progress = Animatable(if (open) 1f else 0f)
    var dragging = false
    /** Set when a touch lands on the timeline, channel list or the conversation's edge; elsewhere keeps its own drags. */
    var regionTouched = false
    /** The conversation's edge kept in view beside Browse, in pixels. */
    var peek = 0f

    fun drag(value: Float) {
        scope.launch(start = CoroutineStart.UNDISPATCHED) { progress.snapTo(value.coerceIn(0f, 1f)) }
    }

    /** Animates to Browse or the conversation, carrying on at `velocity` (progress per second). */
    fun settle(open: Boolean, velocity: Float = 0f) {
        val target = if (open) 1f else 0f
        if (progress.targetValue == target && (progress.isRunning || progress.value == target)) return
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            progress.animateTo(target, spring(Spring.DampingRatioNoBouncy, Spring.StiffnessMediumLow), velocity)
        }
    }
}

@Composable private fun rememberBrowseDrawer(open: Boolean): BrowseDrawer {
    val scope = rememberCoroutineScope()
    return remember(scope) { BrowseDrawer(scope, open) }
}

private val LocalBrowseDrawer = staticCompositionLocalOf<BrowseDrawer?> { null }

/** Moves the conversation over Browse, lifting its bottom edge off Browse's account bar on the way. */
private fun Modifier.conversationLayer(drawer: BrowseDrawer, barHeight: Int): Modifier = graphicsLayer {
    val uncovered = drawer.progress.value
    translationX = uncovered * (size.width - drawer.peek)
    // A clipped layer also clips touches, so the bar's controls stay reachable.
    val inset = uncovered * barHeight
    clip = uncovered > 0f
    shape = GenericShape { size, _ -> addRect(Rect(0f, 0f, size.width, size.height - inset)) }
    shadowElevation = if (uncovered > 0f) 16.dp.toPx() else 0f
}

// Only the timeline/sidebar starts this gesture; the composer and audio controls
// keep their native drags.
private fun Modifier.browseSwipeRegion(drawer: BrowseDrawer?, enabled: Boolean): Modifier =
    if (drawer == null || !enabled) this else pointerInput(drawer) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            drawer.regionTouched = true
        }
    }

// Tracks the finger on the unmoving container, since the layers it moves would
// otherwise move their own touch coordinates. Consumed moves belong to
// scrolling, selection or a slider.
private fun Modifier.browseDrag(drawer: BrowseDrawer, setOpen: (Boolean) -> Unit): Modifier =
    pointerInput(drawer) {
        val tracker = VelocityTracker()
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val allowed = drawer.regionTouched
            drawer.regionTouched = false
            if (!allowed) return@awaitEachGesture
            val opening = drawer.progress.targetValue < 0.5f
            val width = (size.width - drawer.peek).coerceAtLeast(1f)
            val start = drawer.progress.value
            var dragging = false
            var dx = 0f
            var velocity = 0f
            tracker.resetTracking()
            try {
                while (true) {
                    val event = awaitPointerEvent()
                    val change = event.changes.firstOrNull { it.id == down.id } ?: break
                    dx = change.position.x - down.position.x
                    val dy = change.position.y - down.position.y
                    if (!dragging) {
                        if (!change.pressed || event.changes.size != 1 || change.isConsumed ||
                            (abs(dy) > viewConfiguration.touchSlop && abs(dy) > abs(dx))) break
                        if (abs(dx) <= viewConfiguration.touchSlop) continue
                        if ((dx > 0) != opening || abs(dx) < abs(dy) * 1.5f) break
                        dragging = true
                        drawer.dragging = true
                    }
                    tracker.addPosition(change.uptimeMillis, change.position)
                    // Lifting flings; a second finger settles where the slide is.
                    if (!change.pressed || event.changes.size != 1) {
                        if (!change.pressed) velocity = tracker.calculateVelocity().x
                        break
                    }
                    change.consume() // Cancel a channel row's release click after a drag.
                    drawer.drag(start + dx / width)
                }
            } finally {
                if (dragging) {
                    drawer.dragging = false
                    val toward = if (opening) velocity else -velocity
                    val travelled = abs(drawer.progress.value - start)
                    val flingSpeed = 300.dp.toPx()
                    val commit = toward > -flingSpeed &&
                        (travelled >= 0.5f || (toward >= flingSpeed && abs(dx) >= 40.dp.toPx()))
                    val target = if (commit) opening else !opening
                    drawer.settle(target, velocity / width)
                    setOpen(target)
                }
            }
        }
    }

@Composable private fun SpaceRail(state: AppUiState, viewModel: CaperViewModel, show: (Overlay) -> Unit, modifier: Modifier = Modifier) {
    val now = rememberMuteClock(state.notificationSettings)
    Column(modifier.fillMaxHeight().background(Blackout).verticalScroll(rememberScrollState()).padding(vertical = 14.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        state.spaces.forEach { space ->
            val selected = state.selectedSpace?.space?.id == space.id
            val muted = state.notificationSettings?.spaceMuted(space.id, now) == true
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.width(3.dp).height(if (selected) 24.dp else 0.dp).background(if (selected) TerracottaBright else Color.Transparent))
                Spacer(Modifier.width(3.dp))
                Box {
                    Surface(
                        Modifier.size(48.dp).alpha(if (muted) MUTED_ALPHA else 1f).clickable { viewModel.selectSpace(space.id) }
                            .semantics { contentDescription = space.name; if (muted) stateDescription = "Muted" },
                        color = if (selected) TerracottaDark else Surface, shape = MaterialTheme.shapes.medium,
                        border = BorderStroke(1.dp, if (selected) TerracottaBorder else Border),
                    ) { Box(contentAlignment = Alignment.Center) { Text(space.name.take(1).uppercase(), fontWeight = FontWeight.Black, color = if (selected) Color.White else TextMuted) } }
                    if (muted) Box(Modifier.align(Alignment.BottomEnd).offset(x = 3.dp, y = 3.dp).background(Blackout, CircleShape).padding(2.dp).clearAndSetSemantics {}) { MutedBell() }
                }
            }
        }
        state.invitations.forEach { invitation ->
            Surface(
                Modifier.size(40.dp).clickable { show(Overlay.Invitation(invitation)) }.semantics {
                    contentDescription = "Invitation to ${invitation.name}"
                },
                color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, TerracottaBorder),
            ) { Box(contentAlignment = Alignment.Center) { Text("!", color = TerracottaBright, fontWeight = FontWeight.Black) } }
        }
        val limits = state.limits
        val canCreateSpace = limits != null && state.spaces.count { it.ownerId == state.account?.id } < limits.ownedSpaces &&
            state.spaces.count { !it.demo } < limits.totalSpaces
        val spaceEnabled = state.account == null || canCreateSpace
        Surface(
            Modifier.size(48.dp).clickable(enabled = spaceEnabled) { if (state.account == null) viewModel.showLogin() else show(Overlay.CreateSpace) }.semantics {
                contentDescription = "Create space"
                // Web's tooltip; Android has no hover, so it is the state description.
                if (!spaceEnabled) stateDescription = "Space limit reached (${limits?.ownedSpaces ?: 20} owned, ${limits?.totalSpaces ?: 100} total)"
            },
            color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, TerracottaBorder),
        ) { Box(contentAlignment = Alignment.Center) { Icon(painterResource(R.drawable.lucide_plus), null, tint = if (spaceEnabled) TerracottaBright else TerracottaBright.copy(alpha = 0.4f)) } }
    }
}

@Composable private fun ChannelSidebar(
    state: AppUiState,
    voice: VoiceState,
    viewModel: CaperViewModel,
    show: (Overlay) -> Unit,
    modifier: Modifier,
    channelsExpanded: Boolean,
    setChannelsExpanded: (Boolean) -> Unit,
    joinVoice: (Channel) -> Unit,
    pendingVoiceJoin: VoiceJoinIntent?,
    voicePermissionError: String?,
    dismissVoicePermissionError: () -> Unit,
    showAccountBar: Boolean = true,
    closeNavigation: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    val settings = state.notificationSettings
    val now = rememberMuteClock(settings)
    val detail = state.selectedSpace
    val owner = state.account != null && state.account.id == detail?.space?.ownerId
    val channelCount = detail?.channels?.size ?: 0
    val canCreateChannel = detail != null && state.limits?.let { channelCount < it.channelsPerSpace } == true
    var channelMenuOpen by remember(detail?.space?.id) { mutableStateOf(false) }
    var browsing by remember(detail?.space?.id) { mutableStateOf(false) }
    var channelQuery by remember(detail?.space?.id) { mutableStateOf("") }
    val activeChannel = voice.channelId.takeIf { voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED }
    Column(modifier.fillMaxHeight().background(SurfaceSidebar)) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 8.dp)) {
            Row(Modifier.fillMaxWidth().heightIn(min = 42.dp), verticalAlignment = Alignment.CenterVertically) {
                if (detail != null && !detail.space.demo) Box(Modifier.weight(1f)) {
                    // Web: the space name opens a menu with Space settings (owners) or Leave space…,
                    // plus Notifications and Mute.
                    var spaceMenuOpen by remember(detail.space.id) { mutableStateOf(false) }
                    var spaceMenuPage by remember(detail.space.id) { mutableStateOf(NotificationMenuPage.Main) }
                    val closeSpaceMenu = { spaceMenuOpen = false; spaceMenuPage = NotificationMenuPage.Main }
                    val spaceMuted = settings?.spaceMuted(detail.space.id, now) == true
                    Row(Modifier.fillMaxWidth().heightIn(min = 42.dp).clip(MaterialTheme.shapes.small)
                        .clickable(role = Role.Button) { spaceMenuOpen = true; viewModel.refreshNotificationSettings() }
                        .semantics(mergeDescendants = true) { contentDescription = "${detail.space.name} actions"; if (spaceMuted) stateDescription = "Muted" },
                        verticalAlignment = Alignment.CenterVertically) {
                        Text(detail.space.name, Modifier.weight(1f, fill = false), fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (spaceMuted) MutedBell(Modifier.padding(start = 6.dp))
                        Spacer(Modifier.width(6.dp))
                        Icon(painterResource(R.drawable.lucide_chevron_down), null, Modifier.size(16.dp), tint = TextMuted)
                    }
                    DropdownMenu(spaceMenuOpen, closeSpaceMenu, containerColor = SurfaceRaised) {
                        ScopeMenuContent(
                            spaceMenuPage, { spaceMenuPage = it }, "space", settings?.override(spaceKey(detail.space.id)), settings?.inheritedLevel(),
                            { viewModel.setSpaceNotifications(detail.space.id, it) }, closeSpaceMenu,
                            before = {
                                DropdownMenuItem(
                                    text = { Text(if (browsing) "Joined channels" else "Browse channels") },
                                    onClick = {
                                        browsing = !browsing
                                        channelQuery = ""
                                        closeSpaceMenu()
                                    },
                                    leadingIcon = { Icon(painterResource(R.drawable.lucide_hash), null, Modifier.size(16.dp)) },
                                )
                            },
                            after = {
                                if (owner) DropdownMenuItem({ Text("Space settings") }, { closeSpaceMenu(); show(Overlay.ManageSpace) },
                                    leadingIcon = { Icon(painterResource(R.drawable.lucide_settings), null, Modifier.size(16.dp)) })
                                else DropdownMenuItem({ Text("Leave space…", color = ErrorText) }, { closeSpaceMenu(); show(Overlay.LeaveSpace) },
                                    leadingIcon = { Icon(painterResource(R.drawable.lucide_log_out), null, Modifier.size(16.dp), tint = ErrorText) })
                            },
                        )
                    }
                } else Text("Caper", Modifier.weight(1f), fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (closeNavigation != null) IconButton(closeNavigation) { Icon(painterResource(R.drawable.lucide_x), "Close navigation", tint = TextMuted) }
            }
            if (detail != null) NotificationSaveError(state.notificationErrors[spaceKey(detail.space.id)]) { viewModel.dismissNotificationError(spaceKey(detail.space.id)) }
            HorizontalDivider(color = Border)
            if (browsing) OutlinedTextField(channelQuery, { channelQuery = it }, Modifier.fillMaxWidth().padding(top = 8.dp), singleLine = true, label = { Text("Search channels") })
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Row(
                    Modifier.weight(1f).heightIn(min = 48.dp).clip(MaterialTheme.shapes.small)
                        .clickable(role = Role.Button) { setChannelsExpanded(!channelsExpanded) }
                        .semantics {
                            contentDescription = if (channelsExpanded) "Collapse channels" else "Expand channels"
                            stateDescription = if (channelsExpanded) "Expanded" else "Collapsed"
                        },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(if (channelsExpanded) painterResource(R.drawable.lucide_chevron_down) else painterResource(R.drawable.lucide_chevron_right), null, Modifier.size(18.dp), tint = TextMuted)
                    Text("Channels", color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                    Text(channelCount.toString(), color = TextMuted, fontSize = 10.sp)
                }
                if (owner) {
                    IconButton({ show(Overlay.CreateChannel) }, enabled = canCreateChannel, modifier = Modifier.semantics {
                        if (!canCreateChannel) stateDescription = "Channel limit reached (${state.limits?.channelsPerSpace ?: 100})"
                    }) { Icon(painterResource(R.drawable.lucide_plus), "Create channel", tint = if (canCreateChannel) TextMuted else TextMuted.copy(alpha = 0.4f)) }
                    Box {
                        IconButton({ channelMenuOpen = true }) { Icon(painterResource(R.drawable.lucide_ellipsis), "Channel options", tint = TextMuted) }
                        DropdownMenu(channelMenuOpen, { channelMenuOpen = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                            DropdownMenuItem(
                                text = { Text("Create channel", fontSize = 13.sp) },
                                leadingIcon = { Icon(painterResource(R.drawable.lucide_plus), null) },
                                enabled = canCreateChannel,
                                onClick = { channelMenuOpen = false; show(Overlay.CreateChannel) },
                            )
                            DropdownMenuItem(
                                text = { Text(if (channelsExpanded) "Collapse channels" else "Expand channels", fontSize = 13.sp) },
                                onClick = { channelMenuOpen = false; setChannelsExpanded(!channelsExpanded) },
                            )
                        }
                    }
                }
            }
            if (channelsExpanded) detail?.channels?.filter { channel ->
                (browsing || channel.joined) && channel.name.contains(channelQuery, ignoreCase = true)
            }?.forEach { channel ->
                val selected = channel.id == state.selectedChannel?.id
                val people = if (activeChannel == channel.id) voice.participants else state.voiceRosters[if (detail.space.demo) "" else channel.id].orEmpty()
                val voiceRoot = if (detail.space.demo) "" else channel.id
                val sessionStartedAt = (if (activeChannel == channel.id) {
                    if (voice.phase == VoiceState.Phase.CONNECTING) state.voiceSessionStartedAt[voiceRoot] ?: voice.sessionStartedAt
                    else voice.sessionStartedAt
                } else state.voiceSessionStartedAt[voiceRoot])
                    ?: pendingVoiceJoin?.takeIf { it.channelId == channel.id }?.joinStartedAt
                var rosterOpen by remember(channel.id) { mutableStateOf(false) }
                var channelMenuOpen by remember(channel.id) { mutableStateOf(false) }
                var channelMenuPage by remember(channel.id) { mutableStateOf(NotificationMenuPage.Main) }
                val closeChannelMenu = { channelMenuOpen = false; channelMenuPage = NotificationMenuPage.Main }
                // Notifications and Mute are for account spaces, never the demo.
                val notifiable = state.account != null && !detail.space.demo
                // A muted space dims its channels; the bell marks a channel's own mute.
                val channelMuted = settings?.channelMuted(detail.space.id, channel.id, now) == true
                val ownMute = settings?.override(channelKey(channel.id))?.mutedUntil?.let { muteActive(it, now) } == true
                val available = state.voiceAvailable(channel)
                LaunchedEffect(channel.id, channel.joined, detail.space.demo, state.account?.id, available) {
                    if (channel.joined && BuildConfig.ENABLE_NATIVE_VOICE && available == null) viewModel.checkVoiceAvailability(channel)
                }
                val activeHere = activeChannel == channel.id
                val joiningHere = (activeHere && (voice.phase == VoiceState.Phase.CONNECTING || voice.phase == VoiceState.Phase.RECONNECTING)) || pendingVoiceJoin?.channelId == channel.id
                val switching = activeChannel != null && !activeHere
                val denied = channel.id in state.deniedVoiceChannels
                val actionEnabled = pendingVoiceJoin == null && voice.phase != VoiceState.Phase.CONNECTING && voice.phase != VoiceState.Phase.RECONNECTING &&
                    !denied && available == true
                val actionLabel = if (joiningHere) "Joining…" else if (switching) "Switch here" else "Join voice"
                val actionDescription = if (joiningHere) {
                    "Joining voice in #${channel.name}"
                } else if (switching) "Switch voice to #${channel.name}" else "Join voice in #${channel.name}"
                Column(Modifier.fillMaxWidth()) {
                    Row(
                        Modifier.fillMaxWidth().height(48.dp).clip(MaterialTheme.shapes.small)
                            .background(if (selected) TerracottaWash else Color.Transparent)
                            .padding(start = 9.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Row(Modifier.weight(1f).fillMaxHeight().alpha(if (channelMuted) MUTED_ALPHA else 1f)
                            .clickable { viewModel.selectChannel(channel); closeNavigation?.invoke() }
                            .semantics { if (channelMuted) stateDescription = "Muted" }, verticalAlignment = Alignment.CenterVertically) {
                            Icon(if (channel.private) painterResource(R.drawable.lucide_lock_keyhole) else painterResource(R.drawable.lucide_hash), null, Modifier.size(17.dp), tint = if (selected) TerracottaBright else TextMuted)
                            Spacer(Modifier.width(6.dp)); Text(channel.name, Modifier.weight(1f), color = if (selected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (ownMute) MutedBell(Modifier.padding(horizontal = 6.dp))
                            sessionStartedAt?.let { VoiceSessionTimer(it) }
                        }
                        if (owner || notifiable || (channel.joined && !detail.space.demo)) Box {
                            IconButton({ channelMenuOpen = true; if (notifiable) viewModel.refreshNotificationSettings() }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "${channel.name} channel menu", Modifier.size(18.dp), tint = TextMuted) }
                            DropdownMenu(channelMenuOpen, closeChannelMenu, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                                val settingsItem: @Composable () -> Unit = {
                                    if (owner) DropdownMenuItem(text = { Text("Channel settings", fontSize = 13.sp) }, onClick = { closeChannelMenu(); show(Overlay.ManageChannel(channel)) })
                                }
                                val leaveItem: @Composable () -> Unit = {
                                    if (channel.joined && !detail.space.demo) DropdownMenuItem(text = { Text("Leave channel", fontSize = 13.sp) }, onClick = { closeChannelMenu(); show(Overlay.LeaveChannel(channel)) }, modifier = Modifier.semantics { contentDescription = "Leave ${channel.name}" })
                                }
                                if (notifiable) ScopeMenuContent(
                                    channelMenuPage, { channelMenuPage = it }, "channel", settings?.override(channelKey(channel.id)), settings?.inheritedLevel(detail.space.id),
                                    { viewModel.setChannelNotifications(detail.space.id, channel.id, it) }, closeChannelMenu,
                                    spaceMuted = settings?.spaceMuted(detail.space.id, now) == true, fontSize = 13.sp, before = settingsItem, after = leaveItem,
                                ) else { settingsItem(); leaveItem() }
                            }
                        }
                    }
                    NotificationSaveError(state.notificationErrors[channelKey(channel.id)], Modifier.padding(start = 35.dp)) { viewModel.dismissNotificationError(channelKey(channel.id)) }
                    if (channel.joined && BuildConfig.ENABLE_NATIVE_VOICE) Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 35.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (people.isNotEmpty()) TextButton({ rosterOpen = !rosterOpen }, contentPadding = PaddingValues(horizontal = 4.dp), modifier = Modifier.weight(1f)
                            .semantics { contentDescription = "${people.size} in voice in ${channel.name}. ${if (rosterOpen) "Hide" else "Show"} who is in voice" }) {
                            Text("${people.size} in voice", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 11.sp, color = TextMuted)
                            Icon(if (rosterOpen) painterResource(R.drawable.lucide_chevron_down) else painterResource(R.drawable.lucide_chevron_right), null, Modifier.size(15.dp), tint = TextMuted)
                        } else Spacer(Modifier.weight(1f))
                        if (activeHere && voice.phase == VoiceState.Phase.CONNECTED) {
                            Spacer(Modifier.width(112.dp).height(48.dp))
                        } else TextButton({ joinVoice(channel) },
                            enabled = actionEnabled, shape = MaterialTheme.shapes.small,
                            colors = ButtonDefaults.textButtonColors(contentColor = TextMuted, disabledContentColor = TextMuted.copy(alpha = 0.45f)),
                            contentPadding = PaddingValues(horizontal = 6.dp), modifier = Modifier.width(112.dp).heightIn(min = 48.dp)
                            // Web prepares the join on touch-down, before the tap completes.
                            .pointerInput(channel.id, actionEnabled, activeHere) { awaitEachGesture {
                                awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                                if (actionEnabled && !activeHere) viewModel.prepareVoiceJoin(channel)
                            } }
                            .semantics {
                                contentDescription = if (!activeHere && !denied) voiceJoinUnavailableLabel(available) ?: actionDescription else actionDescription
                            }) {
                            Icon(painterResource(R.drawable.lucide_speech), null, Modifier.size(14.dp))
                            Spacer(Modifier.width(4.dp)); Text(actionLabel, fontSize = 11.sp, maxLines = 1)
                        }
                    }
                    if (people.isNotEmpty() && rosterOpen) {
                        if (activeChannel == channel.id) VoiceRoster(voice)
                        else people.forEach { participant ->
                            Row(Modifier.fillMaxWidth().padding(start = 42.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                                Avatar(participant.name, 28.dp, avatarId = participant.avatarId)
                                Spacer(Modifier.width(8.dp))
                                Text(participant.name, fontSize = 12.sp)
                                // Web: MicOff and/or HeadphoneOff, announced as one status.
                                if (participant.deafened || participant.muted) Row(Modifier.padding(start = 4.dp).semantics { contentDescription = if (participant.deafened) "Deafened" else "Muted" }) {
                                    if (participant.muted) Icon(painterResource(R.drawable.lucide_mic_off), null, Modifier.size(15.dp), tint = TextMuted)
                                    if (participant.deafened) Icon(painterResource(R.drawable.lucide_headphone_off), null, Modifier.size(15.dp), tint = TextMuted)
                                }
                            }
                        }
                    }
                }
            }
            detail?.channelInvitations?.forEach { invitation ->
                Surface(Modifier.fillMaxWidth().padding(top = 8.dp), color = SurfaceRaised, border = BorderStroke(1.dp, TerracottaBorder), shape = MaterialTheme.shapes.small) {
                    Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                        Text("Private invitation · #${invitation.channel.name}", fontSize = 12.sp, fontWeight = FontWeight.Bold)
                        Text("${invitation.inviter.displayName} (@${invitation.inviter.username}) invited you to ${detail.space.name}.", color = TextMuted, fontSize = 11.sp)
                        Text("Expires seven days after it was sent.", color = TextMuted, fontSize = 10.sp)
                        Text("Messages stay hidden until acceptance. Accepting joins the channel, not its voice call.", color = TextMuted, fontSize = 10.sp)
                        Row { TextButton({ viewModel.declineChannelInvitation(invitation) }, enabled = !state.busy) { Text("Decline") }; Button({ viewModel.acceptChannelInvitation(invitation) }, enabled = !state.busy) { Text("Accept") } }
                    }
                }
            }
            if (activeChannel != null && detail?.channels?.none { it.id == activeChannel } == true) VoiceRoster(voice)
            state.openError?.let { error ->
                Row(Modifier.fillMaxWidth().padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                    TextButton(viewModel::retryOpening) { Text("Retry opening", fontSize = 12.sp) }
                }
            }
            }
            if (state.account != null) {
                HorizontalDivider(color = Border)
                val invitePeople = owner && detail?.space?.demo == false
                Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Direct messages", Modifier.weight(1f), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                    IconButton({ show(Overlay.StartDirect) }) { Icon(painterResource(R.drawable.lucide_plus), "Start direct message", tint = TextMuted) }
                }
                // Requests sit at the top of the section and never add to unread dots.
                val requests = messageRequests(state.directConversations)
                // Web: open while one of the requests is shown, unless a tap chose otherwise.
                val requestsShown = state.requestsOpen ?: requests.any { it.id == state.selectedDirectId }
                MessageRequestsSection(requests, state.selectedDirectId, requestsShown,
                    { viewModel.setRequestsOpen(!requestsShown) }) { request -> viewModel.selectDirect(request); closeNavigation?.invoke() }
                val selfDirect = state.directConversations.firstOrNull { it.peer.id == state.account.id }
                val selfSelected = selfDirect != null && selfDirect.id == state.selectedDirectId
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                    .background(if (selfSelected) TerracottaWash else Color.Transparent)
                    .clickable(enabled = !state.busy) { viewModel.openSelfDirect(); closeNavigation?.invoke() }.padding(horizontal = 9.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    Avatar(state.account.displayName ?: state.account.username ?: "You", 20.dp, modifier = Modifier.padding(horizontal = 3.dp), avatarId = state.account.avatarId)
                    Spacer(Modifier.width(9.dp))
                    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                        Text(state.account.displayName ?: state.account.username ?: "You", Modifier.weight(1f, fill = false), color = if (selfSelected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Spacer(Modifier.width(6.dp)); Text("you", color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
                    }
                    if (selfDirect?.let(::directUnread) == true)
                        Box(Modifier.size(8.dp).background(TerracottaBright, CircleShape).semantics { contentDescription = "Unread" })
                }
                mainDirects(state.directConversations).filter { it.peer.id != state.account.id }.forEach { direct ->
                    val selected = state.selectedDirectId == direct.id
                    val muted = settings?.directMuted(direct.id, now) == true
                    // A muted DM shows no unread dot.
                    val unread = directUnread(direct) && !muted
                    var optionsOpen by remember(direct.id) { mutableStateOf(false) }
                    val openOptions = { optionsOpen = true; viewModel.refreshNotificationSettings() }
                    // Options: the ⋯ button, or a long press like message actions.
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                        .background(if (selected) TerracottaWash else Color.Transparent)
                        .combinedClickable(onLongClick = openOptions, onLongClickLabel = "Options for ${direct.peer.displayName}") {
                            viewModel.selectDirect(direct); closeNavigation?.invoke()
                        }.padding(start = 9.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Row(Modifier.weight(1f).alpha(if (muted) MUTED_ALPHA else 1f).semantics { if (muted) stateDescription = "Muted" }, verticalAlignment = Alignment.CenterVertically) {
                            Avatar(direct.peer.displayName, 20.dp, modifier = Modifier.padding(horizontal = 3.dp), avatarId = direct.peer.avatarId)
                            Spacer(Modifier.width(9.dp))
                            Column(Modifier.weight(1f)) {
                                Text(direct.peer.displayName, color = if (selected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text("@${direct.peer.username}", color = TextMuted, fontSize = 10.sp, maxLines = 1)
                            }
                            if (muted) MutedBell(Modifier.padding(start = 6.dp))
                        }
                        if (unread) Box(Modifier.size(8.dp).background(TerracottaBright, CircleShape).semantics { contentDescription = "Unread" })
                        Box {
                            IconButton(openOptions, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "${direct.peer.displayName} options", Modifier.size(18.dp), tint = TextMuted) }
                            DirectOptionsMenu(optionsOpen, { optionsOpen = false }, settings?.override(directKey(direct.id)), settings != null) {
                                viewModel.setDirectNotifications(direct.id, it)
                            }
                        }
                    }
                    NotificationSaveError(state.notificationErrors[directKey(direct.id)], Modifier.padding(horizontal = 25.dp)) { viewModel.dismissNotificationError(directKey(direct.id)) }
                }
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(min = 48.dp).clip(MaterialTheme.shapes.small).clickable {
                    show(if (invitePeople) Overlay.ManageSpace else Overlay.StartDirect)
                }.padding(horizontal = 9.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(painterResource(R.drawable.lucide_plus), null, Modifier.size(17.dp), tint = TextMuted)
                    Spacer(Modifier.width(9.dp))
                    Text(if (invitePeople) "Invite people" else "New message", Modifier.weight(1f), color = TextMuted, fontSize = 13.sp)
                }
            }
        }
        if (voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED) {
            HorizontalDivider(color = Border)
            ConnectedVoiceContext(voice, { viewModel.openVoiceChannel(voice) { closeNavigation?.invoke() } }, {
                if (voice.phase == VoiceState.Phase.CONNECTED) CaperEffects.play(CaperEffects.Effect.Disconnect)
                VoiceCallService.stop(context)
            })
        }
        // Web shows voice errors in the dock with a dismiss button.
        (voicePermissionError ?: voice.error)?.let { error ->
            Surface(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp), color = SurfaceRaised, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.small) {
                Row(Modifier.padding(start = 10.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(error, Modifier.weight(1f).padding(vertical = 8.dp), color = ErrorText, fontSize = 11.sp)
                    IconButton({ dismissVoicePermissionError(); VoiceCallService.clearError() }, Modifier.size(36.dp)) { Icon(painterResource(R.drawable.lucide_x), "Dismiss voice error", Modifier.size(16.dp), tint = TextMuted) }
                }
            }
        }
        if (showAccountBar) AccountBar(state, voice, viewModel, show)
    }
}

/** Muted spaces, channels and DMs are dimmed in the sidebar. */
private const val MUTED_ALPHA = 0.5f

internal fun formatVoiceSessionDuration(startedAt: Long, now: Long): String {
    val seconds = ((now - startedAt).coerceAtLeast(0L) / 1_000L)
    val hours = seconds / 3_600
    val minutes = (seconds % 3_600) / 60
    val remainder = seconds % 60
    return if (hours == 0L) "%02d:%02d".format(Locale.ROOT, minutes, remainder)
    else "%d:%02d:%02d".format(Locale.ROOT, hours, minutes, remainder)
}

@Composable private fun VoiceSessionTimer(startedAt: Long) {
    var now by remember(startedAt) { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(startedAt) {
        while (isActive) {
            now = System.currentTimeMillis()
            delay(1_000L - now.mod(1_000L))
        }
    }
    val duration = formatVoiceSessionDuration(startedAt, now)
    Text(
        duration,
        Modifier.padding(horizontal = 7.dp).clearAndSetSemantics {
            contentDescription = "Voice session duration"
            stateDescription = duration
        },
        color = VoiceSessionGreen,
        fontFamily = FontFamily.Monospace,
        fontSize = 11.sp,
        maxLines = 1,
    )
}

@Composable internal fun VoiceRoster(voice: VoiceState) {
    if (voice.participants.isEmpty()) return
    val context = LocalContext.current
    voice.participants.forEach { participant ->
        key(participant.id) {
            var audioOpen by remember(voice.channelId) { mutableStateOf(false) }
            // Long-press opens the audio menu, like a browser's context menu on touch.
            val menuAvailable = participant.id != voice.selfId && voice.phase == VoiceState.Phase.CONNECTED
            Row(Modifier.fillMaxWidth().heightIn(min = 38.dp)
                .then(if (menuAvailable) Modifier.combinedClickable(onClick = {}, onLongClick = { audioOpen = true }, onLongClickLabel = "Audio controls for ${participant.name}") else Modifier)
                .padding(start = 42.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Avatar(participant.name, 24.dp, avatarId = participant.avatarId, speaking = participant.id in voice.speakingParticipants)
                Spacer(Modifier.width(8.dp))
                Column(Modifier.weight(1f)) {
                    Text(participant.name + if (participant.id == voice.selfId) " (you)" else "", fontSize = 12.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (participant.id != voice.selfId && participant.id in voice.locallyMutedParticipants) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(painterResource(R.drawable.lucide_volume_x), null, Modifier.size(10.dp), tint = TerracottaBright)
                            Spacer(Modifier.width(3.dp))
                            Text("You muted ${participant.name}", color = TerracottaBright, fontSize = 10.sp)
                        }
                    }
                }
                if (if (participant.id == voice.selfId) voice.muted else participant.muted) Icon(painterResource(R.drawable.lucide_mic_off), "Muted", Modifier.size(15.dp), tint = TextMuted)
                if (if (participant.id == voice.selfId) voice.deafened else participant.deafened) Icon(painterResource(R.drawable.lucide_headphone_off), "Deafened", Modifier.size(15.dp), tint = TextMuted)
                if (participant.id != voice.selfId && voice.phase == VoiceState.Phase.CONNECTED) Box {
                    TextButton({ audioOpen = !audioOpen }, modifier = Modifier.semantics { contentDescription = "Audio controls for ${participant.name}" }) { Text("Audio", fontSize = 10.sp) }
                    DropdownMenu(audioOpen, { audioOpen = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                        Column(Modifier.width(220.dp).padding(12.dp).semantics { contentDescription = "${participant.name} local audio settings" }) {
                            val volume = voice.participantVolumes[participant.id] ?: 100
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                                Text("User volume", fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                Text("$volume%", fontSize = 11.sp, color = TextMuted)
                            }
                            Slider(volume.toFloat(), { CaperEffects.slider(it / 200f); VoiceCallService.setParticipantVolume(context, participant.id, it.toInt()) }, Modifier.semantics { contentDescription = "${participant.name} volume" }, valueRange = 0f..200f)
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                Text("Mute", Modifier.weight(1f), fontSize = 12.sp)
                                Switch(participant.id in voice.locallyMutedParticipants, { CaperEffects.toggle(!it); VoiceCallService.toggleParticipantMute(context, participant.id) }, modifier = Modifier.semantics { contentDescription = "Mute ${participant.name} for me" })
                            }
                            Text("Only changes what you hear.", color = TextMuted, fontSize = 10.sp)
                        }
                    }
                }
            }
        }
    }
}

/** Web: channel-join when your call connects, and when someone else joins or leaves it. */
@Composable private fun VoiceChimes(voice: VoiceState) {
    var announced by remember { mutableStateOf(false) }
    var previous by remember { mutableStateOf<Pair<String, Set<String>>?>(null) }
    LaunchedEffect(voice.phase, voice.selfId, voice.participants) {
        if (voice.phase != VoiceState.Phase.CONNECTED || voice.selfId == null) {
            if (voice.phase == VoiceState.Phase.IDLE || voice.phase == VoiceState.Phase.FAILED || voice.phase == VoiceState.Phase.CONNECTING) announced = false
            previous = null
            return@LaunchedEffect
        }
        if (!announced) { announced = true; CaperEffects.play(CaperEffects.Effect.Join) }
        val self = voice.selfId
        val others = voice.participants.map { it.id }.filter { it != self }.toSet()
        previous?.takeIf { it.first == self }?.second?.let { before ->
            if ((before - others).isNotEmpty()) CaperEffects.play(CaperEffects.Effect.Leave)
            else if ((others - before).isNotEmpty()) CaperEffects.play(CaperEffects.Effect.Join)
        }
        previous = self to others
    }
}

@Composable internal fun ConnectedVoiceContext(voice: VoiceState, openChannel: () -> Unit, leave: () -> Unit) {
    Surface(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp), color = SurfaceRaised, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.small) {
        Row(Modifier.padding(start = 10.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            // Web: AudioLines and the status are green when connected, amber while connecting or reconnecting.
            val tone = if (voice.phase == VoiceState.Phase.CONNECTED) Color(0xFF8CB262) else Color(0xFFD9AB5C)
            Row(Modifier.weight(1f).clip(MaterialTheme.shapes.small).clickable(role = Role.Button, onClick = openChannel)
                .semantics { contentDescription = "Open voice channel" }, verticalAlignment = Alignment.CenterVertically) {
                Icon(painterResource(R.drawable.lucide_audio_lines), null, Modifier.size(18.dp), tint = tone)
                Spacer(Modifier.width(8.dp))
                Column {
                    Text(when (voice.phase) { VoiceState.Phase.CONNECTED -> "Voice connected"; VoiceState.Phase.CONNECTING -> "Connecting…"; else -> "Reconnecting…" },
                        color = tone, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                    Text(listOfNotNull(voice.channelName, voice.spaceName).joinToString(" / ").ifEmpty { "General" }, color = TextMuted, fontSize = 10.sp)
                }
            }
            IconButton(leave, Modifier.size(40.dp)) {
                Icon(painterResource(R.drawable.lucide_phone_off), if (voice.phase == VoiceState.Phase.CONNECTED) "Leave voice" else "Cancel joining voice", Modifier.size(18.dp), tint = TextMuted)
            }
        }
    }
}

@Composable private fun MemberPresencePanel(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, compact: Boolean = false, close: (() -> Unit)? = null) {
    val members = state.selectedSpace?.members ?: return
    val detail = state.selectedSpace ?: return
    val pages = ((members.size + 24) / 25).coerceAtLeast(1)
    val shown = members.drop(state.presencePage * 25).take(25)
    Column(modifier.background(SurfaceSidebar).then(if (compact) Modifier else Modifier)) {
        Row(Modifier.fillMaxWidth().height(54.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Members", Modifier.weight(1f), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            if (!detail.space.demo) Text(members.size.toString(), color = TextMuted, fontSize = 10.sp)
            if (close != null) IconButton(close, Modifier.size(48.dp)) {
                Icon(painterResource(R.drawable.lucide_x), "Close member list", tint = TextMuted)
            }
        }
        HorizontalDivider(color = Border)
        if (detail.space.demo) Text("General is open to everyone. People in voice appear in the channel sidebar.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp, lineHeight = 16.sp)
        else if (members.isEmpty()) Text("No members to show.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp)
        else LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(8.dp)) {
            items(shown, key = { it.id }) { member ->
                Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 8.dp, vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box {
                        Avatar(member.displayName, 30.dp, avatarId = member.avatarId)
                        PresenceDot(state.presence[member.id], state.gateway == GatewayStatus.LIVE, SurfaceSidebar, Modifier.align(Alignment.BottomEnd))
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

@Composable private fun AccountAvatar(state: AppUiState, viewModel: CaperViewModel) {
    val name = state.account?.displayName ?: "Guest"
    // Web: account spaces show the server's presence for you; General and guests show local presence.
    val accountPresence = state.account != null && state.selectedSpace?.space?.demo == false
    var local by remember { mutableStateOf("offline") }
    if (!accountPresence) LaunchedEffect(state.gateway) {
        while (true) { local = viewModel.localPresence(); kotlinx.coroutines.delay(1_000) }
    }
    Box {
        Avatar(name, 30.dp, avatarId = state.account?.avatarId)
        PresenceDot(if (accountPresence) state.presence[state.account?.id] else local,
            live = !accountPresence || state.gateway == GatewayStatus.LIVE, SurfaceRaised, Modifier.align(Alignment.BottomEnd))
    }
}

/** Web's PresenceDot: 11px with a surface ring, labeled for assistive technology. */
@Composable private fun PresenceDot(status: String?, live: Boolean, ring: Color, modifier: Modifier = Modifier) {
    val label = presenceLabel(status, live)
    Box(modifier.size(11.dp).clip(CircleShape).background(ring).padding(2.dp).clip(CircleShape)
        .background(when (status) { "online" -> CaperGreen; "idle" -> Idle; "offline" -> Border; else -> ring })
        .semantics { contentDescription = label })
}

internal fun presenceLabel(status: String?, live: Boolean): String =
    if (status == null) "Status unavailable"
    else status.replaceFirstChar { it.uppercase() } + if (live) "" else " (last known; reconnecting)"

@Composable private fun AccountBar(state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit) {
    val context = LocalContext.current
    Surface(Modifier.fillMaxWidth().padding(12.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Row(Modifier.height(52.dp).padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            Row(Modifier.weight(1f).fillMaxHeight().clickable { if (state.account == null) viewModel.showLogin() else show(Overlay.Profile) }
                .semantics { contentDescription = if (state.account == null) "Sign in to edit your profile" else "Edit profile for ${state.account.displayName ?: "Guest"}" }, verticalAlignment = Alignment.CenterVertically) {
                AccountAvatar(state, viewModel); Spacer(Modifier.width(7.dp))
                Text(state.account?.displayName ?: "Guest", maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            }
            // Web shows mute and deafen before joining too; the next join uses them.
            if (BuildConfig.ENABLE_NATIVE_VOICE) {
                // Web: mute and deafen wait while the Audio test holds the microphone.
                val monitoring = Modifier.semantics { if (voice.monitoring) stateDescription = "Stop mic test to change mute" }
                IconButton({ CaperEffects.toggle(voice.muted); VoiceCallService.toggleMute(context) }, Modifier.size(40.dp).then(monitoring), enabled = !voice.monitoring) { Icon(if (voice.muted) painterResource(R.drawable.lucide_mic_off) else painterResource(R.drawable.lucide_mic), if (voice.muted) "Unmute microphone" else "Mute microphone", Modifier.size(18.dp), tint = if (voice.muted) TerracottaBright else TextMuted) }
                AudioOptionsMenu(input = true, voice = voice)
                IconButton({ CaperEffects.toggle(voice.deafened); VoiceCallService.toggleDeafen(context) }, Modifier.size(40.dp).semantics { if (voice.monitoring) stateDescription = "Stop mic test to change deafen" }, enabled = !voice.monitoring) { Icon(if (voice.deafened) painterResource(R.drawable.lucide_volume_x) else painterResource(R.drawable.lucide_headphones), if (voice.deafened) "Undeafen audio" else "Deafen audio", Modifier.size(18.dp), tint = if (voice.deafened) TerracottaBright else TextMuted) }
                AudioOptionsMenu(input = false, voice = voice)
            }
            IconButton({ show(Overlay.Audio) }, Modifier.size(40.dp)) { Icon(painterResource(R.drawable.lucide_settings), "User Settings", Modifier.size(18.dp), tint = TextMuted) }
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
    voicePermissionError: String?,
    modifier: Modifier = Modifier,
    openNavigation: () -> Unit,
) {
    val threadDrafts = rememberSaveableStateHolder()
    Row(modifier) {
        ChannelConversation(state, voice, viewModel, show, narrow, membersVisible, toggleMembers, voicePermissionError, Modifier.weight(1f), openNavigation)
        if (!narrow && state.thread != null) threadDrafts.SaveableStateProvider("${state.selectedChannel?.id}:${state.thread.rootId}") {
            ThreadConversation(state, viewModel, Modifier.width(340.dp).fillMaxHeight())
        }
    }
    if (narrow && state.thread != null) Dialog(onDismissRequest = viewModel::closeThread,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        threadDrafts.SaveableStateProvider("${state.selectedChannel?.id}:${state.thread.rootId}") {
            ThreadConversation(state, viewModel, Modifier.fillMaxSize().systemBarsPadding().imePadding())
        }
    }
}

@Composable private fun ThreadConversation(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier) {
    val thread = state.thread ?: return
    var draft by rememberSaveable(state.selectedChannel?.id, thread.rootId) { mutableStateOf("") }
    var broadcast by rememberSaveable(state.selectedChannel?.id, thread.rootId) { mutableStateOf(false) }
    val pending = state.pendingMessage?.takeIf { it.threadRootId == thread.rootId }
    // DMs are named after the other person, without a channel `#`.
    val direct = state.selectedChannel?.direct == true
    val conversation = "${if (direct) "" else "#"}${state.selectedChannel?.name.orEmpty()}"
    BackHandler { viewModel.closeThread() }
    fun send() {
        if (state.pendingMessage != null && pending == null || pending?.rejected == true || pending != null && pending.error == null) return
        if (pending != null) viewModel.send(pending.text, threadRootId = thread.rootId, broadcast = pending.broadcast)
        else if (draft.isNotBlank()) { viewModel.send(draft, threadRootId = thread.rootId, broadcast = broadcast); draft = "" }
    }
    Column(modifier.background(SurfaceConversation).border(BorderStroke(1.dp, Border))) {
        Row(Modifier.fillMaxWidth().height(53.dp).padding(horizontal = 18.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Thread · $conversation", Modifier.weight(1f), fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            TextButton(viewModel::closeThread) { Text("Back to channel") }
        }
        HorizontalDivider(color = Border)
        thread.error?.let { Text(it, Modifier.padding(12.dp), color = ErrorText); TextButton({ viewModel.loadThread() }) { Text("Retry") } }
        if (thread.hasMore) TextButton({ viewModel.loadThread(older = true) }, enabled = !thread.loading) { Text("Load older replies") }
        val rows = state.messages.filter { it.id == thread.rootId || it.threadRootId == thread.rootId &&
            (thread.windowStart == null || it.seq.toBigInteger() >= thread.windowStart.toBigInteger()) &&
            (thread.windowEnd == null || it.seq.toBigInteger() <= thread.windowEnd.toBigInteger()) }
        MessageTimeline(state.copy(messages = rows, pendingMessage = pending, messagesLoading = false, messagesError = null), viewModel, Modifier.weight(1f), inThread = true) {
            pending?.error?.let { error ->
                Text(error, color = ErrorText)
                if (pending.rejected) Row {
                    TextButton({ viewModel.discardPending()?.let { draft = it } }, enabled = draft.isEmpty()) { Text("Edit") }
                    TextButton({ viewModel.discardPending() }) { Text("Dismiss") }
                } else TextButton(::send) { Text("Retry send") }
            }
        }
        if (thread.hasNewer) TextButton({ viewModel.loadThread(newer = true) }, enabled = !thread.loading) { Text("Load newer replies") }
        if (!thread.loading && rows.none { it.threadRootId == thread.rootId })
            Text(if (state.canParticipate) "No replies yet. Start the thread." else "No replies yet.", Modifier.padding(18.dp), color = TextMuted)
        if (state.canParticipate) Column(Modifier.padding(12.dp)) {
            OutlinedTextField(draft, { draft = it.codePointTake(4000) }, Modifier.fillMaxWidth(), placeholder = { Text("Reply to thread…") }, maxLines = 5,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send), keyboardActions = KeyboardActions(onSend = { send() }), enabled = !thread.loading)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(pending?.broadcast ?: broadcast, { broadcast = it }, enabled = pending == null)
                Text("Also send to $conversation", Modifier.weight(1f), fontSize = 12.sp)
                TextButton(::send, enabled = !thread.loading && state.chatAuthorId != null && state.pendingMessage == null && draft.isNotBlank()) { Text("Send reply") }
            }
            if (state.pendingMessage?.error != null && pending == null) Text("Confirm or dismiss the pending message first.", color = TextMuted)
        } else Text(when {
            state.selectedDirect?.incoming == true -> "Accept the request to reply."
            direct -> "You can’t reply in this conversation."
            else -> "Join the channel to reply."
        }, Modifier.padding(18.dp), color = TextMuted)
    }
}

@Composable private fun ChannelConversation(
    state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit,
    narrow: Boolean, membersVisible: Boolean, toggleMembers: () -> Unit, voicePermissionError: String?,
    modifier: Modifier = Modifier, openNavigation: () -> Unit,
) {
    val channel = state.selectedChannel
    if (channel == null) return EmptyChannel(state, narrow, show, openNavigation, modifier)
    var draft by rememberSaveable(channel.id, stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue("")) }
    var channelMenuOpen by remember(channel.id) { mutableStateOf(false) }
    var showingPins by remember(channel.id) { mutableStateOf(false) }
    // Switching to Pins must not discard the channel's measured scroll position.
    val timelineState = key(channel.id) { rememberLazyListState() }
    val joined = channel.joined
    // A 1:1 DM (not your notes) offers Block / Unblock; a request does so in its own bar.
    val direct = state.selectedDirect?.takeIf { channel.direct && it.peer.id != state.account?.id && !it.incoming }
    var confirmBlock by remember(channel.id) { mutableStateOf(false) }
    if (confirmBlock && direct != null) BlockConfirmDialog(BlockedAccount(direct.peer.id, direct.peer.username, direct.peer.displayName, direct.peer.avatarId), viewModel, { confirmBlock = false })
    fun toggleBlock() { if (direct == null) return; if (direct.blocked) viewModel.unblock(direct.peer.id) else confirmBlock = true }
    Column(modifier.fillMaxHeight().background(SurfaceConversation)) {
        // Web waits a second before announcing a lost connection.
        val live = state.gateway == GatewayStatus.LIVE
        var showConnection by remember(channel.id) { mutableStateOf(false) }
        LaunchedEffect(live, channel.id) { showConnection = false; if (!live) { kotlinx.coroutines.delay(1_000); showConnection = true } }
        Row(Modifier.fillMaxWidth().height(53.dp).padding(horizontal = 18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (narrow) IconButton(openNavigation, Modifier.size(44.dp)) {
                Icon(painterResource(R.drawable.lucide_arrow_right), "Back to Browse", Modifier.size(20.dp).graphicsLayer { rotationZ = 180f }, tint = TextMuted)
            }
            if (narrow) Box(Modifier.weight(1f)) {
                Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                    .clickable(role = Role.Button) { channelMenuOpen = true }
                    .semantics(mergeDescendants = true) { contentDescription = "${if (channel.direct) "" else "# "}${channel.name} channel menu" }, verticalAlignment = Alignment.CenterVertically) {
                    Text(if (channel.direct) channel.name else "# ${channel.name}", Modifier.weight(1f, fill = false), fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Icon(painterResource(R.drawable.lucide_chevron_down), null, Modifier.size(16.dp), tint = TextMuted)
                }
                DropdownMenu(channelMenuOpen, { channelMenuOpen = false }, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                    if (!channel.direct && joined) DropdownMenuItem(text = { Text(if (membersVisible) "Hide member list" else "Members") },
                        leadingIcon = { Icon(painterResource(R.drawable.lucide_users), null) },
                        onClick = { channelMenuOpen = false; toggleMembers() })
                    if (direct != null) DropdownMenuItem(text = { Text(if (direct.blocked) "Unblock" else "Block", color = if (direct.blocked) Text else ErrorText) },
                        onClick = { channelMenuOpen = false; toggleBlock() })
                }
            } else Text(if (channel.direct) channel.name else "# ${channel.name}", Modifier.weight(1f), fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            IconButton({ showingPins = !showingPins }, Modifier.sizeIn(minWidth = 44.dp, minHeight = 44.dp)) {
                Icon(painterResource(R.drawable.lucide_pin), "Pins", Modifier.size(20.dp), tint = TextMuted)
            }
            if (!narrow && direct != null) TextButton(::toggleBlock, Modifier.heightIn(min = 48.dp)) {
                Text(if (direct.blocked) "Unblock" else "Block", color = if (direct.blocked) TextMuted else ErrorText, fontSize = 11.sp, fontWeight = FontWeight.Bold)
            }
            if (!channel.direct && !joined) Button({ viewModel.joinChannel(channel) }, enabled = !state.busy, shape = MaterialTheme.shapes.small) { Text("Join channel") }
            if (!live && showConnection) Text(if (state.gateway == GatewayStatus.ERROR || state.messagesError != null) "Offline" else "Connecting…", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold)
            if (!narrow && !channel.direct && joined) IconButton(toggleMembers, Modifier.size(36.dp)) { Icon(painterResource(R.drawable.lucide_users), if (membersVisible) "Hide member list" else "Show member list", tint = if (membersVisible) Text else TextMuted) }
        }
        HorizontalDivider(color = Border)
        if (!joined) Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            Text("Preview", fontWeight = FontWeight.Bold, fontSize = 12.sp)
            Text(buildAnnotatedString {
                append("Join ")
                withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append("#${channel.name}") }
                append(" to interact with people here")
            }, color = TextMuted, fontSize = 12.sp)
        }
        state.refreshError?.let { error ->
            Surface(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp), color = Surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                Row(Modifier.padding(start = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                    TextButton(viewModel::retryMessages) { Text("Retry", color = Text, fontSize = 12.sp) }
                }
            }
        }
        // The dock (and its voice error row) is hidden behind Browse on phones.
        (voicePermissionError ?: voice.error)?.takeIf { narrow }?.let { error ->
            Text(error, Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp), color = ErrorText, fontSize = 12.sp)
        }
        // Web shows a pending message's status inline, under the message itself.
        val channelPending = state.pendingMessage?.takeIf { it.threadRootId == null }
        if (showingPins) Dialog(onDismissRequest = { showingPins = false }) {
            Surface(shape = MaterialTheme.shapes.medium, color = SurfaceConversation, border = BorderStroke(1.dp, Border)) {
                Column(Modifier.heightIn(max = 560.dp)) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text("Pins", Modifier.weight(1f), fontWeight = FontWeight.Bold)
                        TextButton({ showingPins = false }) { Text("Close") }
                    }
                    PinnedMessages(state, viewModel, Modifier.weight(1f, fill = false)) { showingPins = false }
                }
            }
        }
        MessageTimeline(state, viewModel, Modifier.weight(1f).browseSwipeRegion(LocalBrowseDrawer.current, enabled = narrow && !membersVisible && !channelMenuOpen), listState = timelineState) {
            channelPending?.error?.let { pending ->
                val editable = canEditRejectedMessage(draft.text, channelPending.text)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(if (channelPending.rejected) "Not sent. $pending" else "Not confirmed yet. $pending", Modifier.weight(1f), color = ErrorText, fontSize = 11.sp)
                    TextButton({
                        if (channelPending.rejected) {
                            if (editable) viewModel.discardPending()?.let { draft = TextFieldValue(it, TextRange(it.length)) }
                        } else viewModel.send(channelPending.text)
                    }, Modifier.semantics { if (channelPending.rejected && !editable) stateDescription = "Clear your current draft to edit this message." },
                        enabled = !channelPending.rejected || editable) {
                        Text(if (channelPending.rejected) "Edit" else "Retry send")
                    }
                    if (channelPending.rejected) TextButton({ viewModel.discardPending() }) { Text("Dismiss") }
                }
                if (channelPending.rejected && !editable)
                    Text("Clear your current draft to edit this message.", color = TextMuted, fontSize = 10.sp)
            }
        }
        if (joined) TypingLine(state.typingAuthors)
        if (joined) HorizontalDivider(color = Border)
        val request = state.selectedDirect?.takeIf { channel.direct && it.incoming }
        // An incoming request is read-only until you answer it; a blocked DM offers Unblock instead.
        if (joined && request != null) RequestBar(request, viewModel) { if (narrow) openNavigation() }
        else if (joined && direct?.blocked == true) BlockedDirectNotice(direct, viewModel)
        else if (joined) Column(Modifier.padding(horizontal = 18.dp, vertical = 12.dp)) {
            if (direct?.outgoing == true) OutgoingRequestNotice(direct)
            state.pendingMessage?.takeIf { it.error != null && it.threadRootId != state.thread?.rootId }?.let { pending ->
                pending.threadRootId?.let { root ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(if (pending.rejected) "A thread reply wasn’t sent." else "A thread reply couldn’t be confirmed.", Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                        TextButton({ viewModel.openThread(root) }) { Text("Review reply") }
                    }
                }
            }
            state.sessionError?.let { error ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                    TextButton(viewModel::retrySession) { Text("Retry session", fontSize = 12.sp) }
                }
            }
            val context = LocalContext.current
            val catalog = remember { EmojiArtwork.catalog(context) }
            var dismissedAt by remember(channel.id) { mutableStateOf<TextFieldValue?>(null) }
            var composerFocused by remember(channel.id) { mutableStateOf(false) }
            // `:` emoji and `@` mention tokens never overlap; both share one popup.
            val token = emojiToken(draft)
            val mention = if (token == null) mentionToken(draft) else null
            val open = composerFocused && draft != dismissedAt
            val emojiRows = if (open && token != null) emojiSuggestions(catalog, token.query) else emptyList()
            val mentionRows = if (open && mention != null) mentionSuggestions(mentionSource(state), mention.query) else emptyList()
            val suggestionCount = emojiRows.size + mentionRows.size
            var selectedSuggestion by remember(channel.id) { mutableIntStateOf(0) }
            LaunchedEffect(token, mention) { selectedSuggestion = 0 }
            val suggestionList = rememberLazyListState()
            LaunchedEffect(selectedSuggestion, token, mention) {
                if (suggestionCount > 0) suggestionList.animateScrollToItem(selectedSuggestion.coerceAtMost(suggestionCount - 1))
            }
            fun accept(next: TextFieldValue?) {
                next?.let { draft = it; dismissedAt = null; viewModel.reportActivity(); viewModel.setTyping(it.text.isNotBlank()) }
            }
            fun chooseSuggestion(index: Int): Boolean {
                emojiToken(draft)?.let { current ->
                    val entry = emojiRows.getOrNull(index) ?: return false
                    accept(insertEmoji(draft, current, entry.emoji))
                    return true
                }
                val current = mentionToken(draft) ?: return false
                val candidate = mentionRows.getOrNull(index) ?: return false
                accept(insertMention(draft, current, candidate.username))
                return true
            }
            if (suggestionCount > 0) Surface(
                Modifier.widthIn(max = 260.dp).fillMaxWidth().padding(bottom = 6.dp), color = SurfaceRaised,
                shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
            ) {
                LazyColumn(Modifier.heightIn(max = 192.dp).padding(vertical = 4.dp), state = suggestionList) {
                    itemsIndexed(emojiRows, key = { _, entry -> entry.id }) { index, entry ->
                        SuggestionRow(index == selectedSuggestion, "Insert emoji ${emojiShortcodeLabel(entry.name)}", { selectedSuggestion = index; chooseSuggestion(index) }) {
                            EmojiImage(entry.emoji, null, Modifier.size(28.dp))
                            Text(emojiShortcodeLabel(entry.name), color = if (index == selectedSuggestion) Text else TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    itemsIndexed(mentionRows, key = { _, candidate -> "@${candidate.username}" }) { index, candidate ->
                        MentionSuggestionRow(candidate, index == selectedSuggestion) { selectedSuggestion = index; chooseSuggestion(index) }
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                OutlinedTextField(
                    draft, { value ->
                        val limited = if (value.text.codePointCount(0, value.text.length) <= 4000) value else {
                            val text = value.text.codePointTake(4000)
                            value.copy(text = text, selection = TextRange(value.selection.start.coerceAtMost(text.length), value.selection.end.coerceAtMost(text.length)), composition = null)
                        }
                        draft = limited; dismissedAt = null; viewModel.reportActivity(); viewModel.setTyping(limited.text.isNotBlank())
                    },
                    modifier = Modifier.weight(1f).onFocusChanged { composerFocused = it.isFocused }.onPreviewKeyEvent { event ->
                        if (event.type != KeyEventType.KeyDown || suggestionCount == 0) false else when (event.key) {
                            Key.DirectionDown -> { selectedSuggestion = (selectedSuggestion + 1) % suggestionCount; true }
                            Key.DirectionUp -> { selectedSuggestion = (selectedSuggestion - 1 + suggestionCount) % suggestionCount; true }
                            Key.Enter, Key.Tab -> if (event.isShiftPressed) false else chooseSuggestion(selectedSuggestion.coerceAtMost(suggestionCount - 1))
                            Key.Escape -> { dismissedAt = draft; true }
                            else -> false
                        }
                    }, placeholder = { Text(if (channel.direct) "Message ${channel.name}" else "Message #${channel.name}") }, maxLines = 6,
                    enabled = !state.messagesLoading && state.messagesError == null,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = {
                        if (suggestionCount > 0) { chooseSuggestion(selectedSuggestion.coerceAtMost(suggestionCount - 1)); return@KeyboardActions }
                        val pending = state.pendingMessage
                        // Web: Enter retries an unconfirmed send; a rejected one waits for Edit or Dismiss.
                        if (state.chatAuthorId != null) {
                            if (pending != null) { if (pending.threadRootId == null && !pending.rejected && pending.error != null) viewModel.send(pending.text) }
                            else if (draft.text.isNotBlank()) { val sent = draft.text; viewModel.setTyping(false); viewModel.send(sent); draft = TextFieldValue("") }
                        }
                    }),
                    colors = OutlinedTextFieldDefaults.colors(focusedContainerColor = SurfaceComposer, unfocusedContainerColor = SurfaceComposer, focusedBorderColor = Terracotta, unfocusedBorderColor = Border),
                )
                FilledIconButton(
                    { if (draft.text.isNotBlank() && state.pendingMessage == null) { val sent = draft.text; viewModel.setTyping(false); viewModel.send(sent); draft = TextFieldValue("") } },
                    modifier = Modifier.size(48.dp).semantics { contentDescription = "Send" }, enabled = draft.text.isNotBlank() && state.pendingMessage == null && state.chatAuthorId != null,
                    shape = MaterialTheme.shapes.small,
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = Terracotta, contentColor = Color.White,
                        disabledContainerColor = Terracotta.copy(alpha = 0.55f), disabledContentColor = Color.White.copy(alpha = 0.6f),
                    ),
                ) { Icon(painterResource(R.drawable.lucide_arrow_up), null) }
            }
            val count = draft.text.codePointCount(0, draft.text.length)
            if (count >= 3000) Text("${"%,d".format(java.util.Locale.US, count)} / 4,000", Modifier.align(Alignment.End), color = counterTone(count), fontSize = 10.sp)
        }
    }
}

/** One flat row of the composer's `:` emoji / `@` mention popup. */
@Composable private fun SuggestionRow(selected: Boolean, description: String, choose: () -> Unit, content: @Composable RowScope.() -> Unit) = Row(
    Modifier.fillMaxWidth().heightIn(min = 48.dp)
        .background(if (selected) Terracotta.copy(alpha = 0.18f) else Color.Transparent)
        .clickable(onClick = choose)
        .semantics { contentDescription = description }
        .padding(horizontal = 12.dp),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(12.dp),
    content = content,
)

/** A person (avatar, display name, muted `@username`) or a special (`@everyone`, muted description). */
@Composable private fun MentionSuggestionRow(candidate: MentionCandidate, selected: Boolean, choose: () -> Unit) {
    val primary = if (candidate.special) "@${candidate.username}" else candidate.label
    val secondary = if (candidate.special) candidate.label else "@${candidate.username}"
    SuggestionRow(selected, "Mention $primary, $secondary", choose) {
        if (candidate.special) Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) {
            Icon(painterResource(R.drawable.lucide_users), null, Modifier.size(18.dp), tint = TextMuted)
        } else Avatar(candidate.label, 28.dp, avatarId = candidate.avatarId)
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(primary, Modifier.weight(1f, fill = false), color = Text, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(secondary, Modifier.weight(1f, fill = false), color = TextMuted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Web's empty-channel stage: owners can create the first channel; narrow screens keep Browse spaces. */
@Composable private fun EmptyChannel(state: AppUiState, narrow: Boolean, show: (Overlay) -> Unit, openNavigation: () -> Unit, modifier: Modifier) {
    val detail = state.selectedSpace
    // A space with channels is still opening one; do not flash the empty state.
    if (state.busy && detail?.channels?.isNotEmpty() == true) return Box(modifier.fillMaxSize().background(SurfaceConversation))
    val owner = state.account != null && state.account.id == detail?.space?.ownerId
    val noSpaces = state.spaces.none { !it.demo }
    Box(modifier.fillMaxSize().background(SurfaceConversation)) {
        if (narrow) BrowseButton("Browse spaces", R.drawable.lucide_hash, openNavigation, Modifier.align(Alignment.TopStart).padding(start = 11.dp, top = 8.dp))
        Column(Modifier.align(Alignment.Center).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(painterResource(R.drawable.lucide_hash), null, tint = TerracottaBright)
            Text(if (noSpaces) "Select a direct message" else "No accessible channels", fontWeight = FontWeight.Bold)
            Text(if (noSpaces) "Open a conversation from Direct messages." else if (owner) "Create a channel or browse channels to join one." else "Browse public channels to preview and join one.", color = TextMuted, fontSize = 12.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            if (owner) OutlinedButton({ show(Overlay.CreateChannel) }, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Create channel") }
            OutlinedButton(openNavigation, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Browse channels") }
        }
    }
}

/** Web's `.navigation-toggle`: a bordered icon-and-label button shown on narrow screens. */
@Composable private fun BrowseButton(label: String, icon: Int, open: () -> Unit, modifier: Modifier = Modifier) {
    Surface(open, modifier.heightIn(min = 34.dp), shape = MaterialTheme.shapes.small, color = Color.Transparent, contentColor = TextMuted, border = BorderStroke(1.dp, Border)) {
        Row(Modifier.padding(horizontal = 9.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(painterResource(icon), null, Modifier.size(15.dp))
            Text(label, fontSize = 11.sp, fontWeight = FontWeight.Bold)
        }
    }
}

@Composable private fun MessageTimeline(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, inThread: Boolean = false, listState: LazyListState = rememberLazyListState(), pendingStatus: @Composable () -> Unit = {}) {
    var actionTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var pickerTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var forwardTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var conversationTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var editTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var historyTarget by remember { mutableStateOf<ChatMessage?>(null) }
    // Message ID and the pressed chip's emoji.
    var reactorsTarget by remember { mutableStateOf<Pair<String, String>?>(null) }
    // The tapped person's `user` mention entry; another pill replaces it.
    var mentionTarget by remember { mutableStateOf<MessageMention?>(null) }
    val messages = if (inThread) state.displayedMessages else state.displayedChannelMessages
    // Runs of blocked authors' messages collapse; Show reveals one run, in memory only.
    var revealedRuns by remember { mutableStateOf(emptySet<String>()) }
    var blockTarget by remember { mutableStateOf<BlockedAccount?>(null) }
    val selfId = state.chatAuthorId ?: state.account?.id
    val blocked = state.blockedIds
    val rows = remember(messages, blocked, selfId, revealedRuns) { timelineRows(groupBlocked(messages, blocked, selfId, revealedRuns)) }
    LaunchedEffect(state.focusRevision, state.selectedChannel?.id) {
        val target = state.focusedMessageId ?: return@LaunchedEffect
        val run = rows.filterIsInstance<TimelineRow.Blocked>().firstOrNull { it.run.messages.any { message -> message.id == target } }
        if (run != null && !run.run.revealed) {
            revealedRuns = revealedRuns + run.run.key
            withFrameNanos { }
        }
        val focusedRows = timelineRows(groupBlocked(messages, blocked, selfId, revealedRuns))
        val index = focusedRows.indexOfFirst { row -> row is TimelineRow.Message && row.message.id == target }
        if (index >= 0) listState.scrollToItem(index + if (inThread) 0 else 1, -listState.layoutInfo.viewportSize.height / 3)
    }
    LaunchedEffect(state.account?.id, state.selectedSpace?.space?.id, state.selectedChannel?.id, state.selectedDirectId) {
        actionTarget = null
        pickerTarget = null
        reactorsTarget = null
        mentionTarget = null
        forwardTarget = null
        conversationTarget = null
        editTarget = null
        historyTarget = null
        blockTarget = null
        revealedRuns = emptySet()
    }
    // Web's chat phases: loading, failed first load, then the conversation.
    if (state.messagesLoading) return Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Text("Loading messages…", color = TextMuted, fontSize = 13.sp)
    }
    state.messagesError?.let { error -> return Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(error, color = TextMuted, fontSize = 13.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            OutlinedButton(viewModel::retryMessages, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
                colors = ButtonDefaults.outlinedButtonColors(containerColor = SurfaceRaised, contentColor = Text)) { Text("Try again", fontSize = 12.sp) }
        }
    } }
    LazyColumn(modifier.fillMaxWidth(), state = listState, reverseLayout = false, contentPadding = PaddingValues(vertical = 8.dp)) {
        if (!inThread) item {
            Row(Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 14.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically) {
                val historyButton: @Composable (String, Boolean) -> Unit = { label, enabled ->
                    OutlinedButton(viewModel::loadOlder, enabled = enabled, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp), colors = ButtonDefaults.outlinedButtonColors(contentColor = TextMuted)) {
                        Text(label, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                    }
                }
                when {
                    state.olderError != null -> { Text("Couldn’t load older messages.", color = TextMuted, fontSize = 11.sp); historyButton("Retry", true) }
                    state.hasMoreMessages -> historyButton(if (state.loadingOlder) "Loading…" else "Load older messages", !state.loadingOlder)
                    else -> Text("Beginning of conversation", color = TextMuted, fontSize = 11.sp)
                }
            }
        }
        // Keep the message keys stable while inserting purely presentational day boundaries.
        items(rows, key = { it.key }) { row ->
            val message = row.first
            val previous = row.previous
            if (previous == null || !sameLocalDay(previous.createdAt, message.createdAt)) {
                DateDivider(message.createdAt)
            }
            if (row is TimelineRow.Blocked) BlockedRunRow(row.run.messages.size, row.run.revealed) {
                revealedRuns = if (row.run.revealed) revealedRuns - row.run.key else revealedRuns + row.run.key
            } else Column(Modifier.background(if (state.focusedMessageId == message.id) TerracottaWash else if (!inThread && state.thread?.rootId == message.id) Color(0xFFE4C76A).copy(alpha = 0.1f) else Color.Transparent)) {
              ReactionMessageRow(
                message, state, viewModel::setReaction, viewModel::retryReaction, viewModel::dismissReactionError,
                openReactors = { target, emoji -> reactorsTarget = target.id to emoji },
                openMention = { mentionTarget = it },
                openActions = { actionTarget = it },
                openConversation = { conversationTarget = it },
                openHistory = { historyTarget = it },
                retryPin = viewModel::retryPin,
                dismissPinError = viewModel::dismissPinError,
                grouped = row is TimelineRow.Message && groupsWithPrevious(row.above, message, inThread, state.focusedMessageId),
            )
              if (!inThread && (message.threadRootId != null || (message.thread?.replyCount ?: 0) > 0)) Row(Modifier.padding(start = 62.dp), verticalAlignment = Alignment.CenterVertically) {
                  message.thread?.takeIf { message.threadRootId == null }?.participants?.forEach { Avatar(it.name, 24.dp, avatarId = it.avatarId) }
                  TextButton({ viewModel.openThread(message.threadRootId ?: message.id) }) { Text(message.thread?.takeIf { message.threadRootId == null }?.let { "${it.replyCount} ${if (it.replyCount == 1) "reply" else "replies"} · View thread" } ?: "Replied to a thread · View thread") }
              }
            }
        }
        if (inThread && state.thread?.loading == true && messages.none { it.threadRootId == state.thread.rootId }) item("thread-loading") {
            Column(Modifier.fillMaxWidth().semantics { contentDescription = "Loading thread replies" }) {
                repeat(2) { index ->
                    Row(Modifier.fillMaxWidth().height(70.dp).padding(horizontal = 18.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Box(Modifier.size(34.dp).background(Border, CircleShape))
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                            Box(Modifier.width(96.dp).height(12.dp).background(Border, RoundedCornerShape(4.dp)))
                            Box(Modifier.fillMaxWidth(if (index == 0) 0.76f else 0.54f).height(10.dp).background(Border, RoundedCornerShape(4.dp)))
                        }
                    }
                }
            }
        }
        state.pendingMessage?.takeIf { inThread || it.threadRootId == null }?.let { pending -> item("pending:${pending.clientMessageId}") {
            Column {
                if (messages.lastOrNull()?.createdAt?.let { sameLocalDay(it, pending.createdAt) } != true) {
                    DateDivider(pending.createdAt)
                }
                // Your unsent message joins your run like a sent one.
                val grouped = groupsWithPrevious((rows.lastOrNull() as? TimelineRow.Message)?.message, pending.author?.id ?: selfId, pending.createdAt,
                    pending.threadRootId, jumpTarget = state.focusedMessageId)
                MessageRow(pending.author?.name ?: "You", pending.author?.isGuest == true, pending.createdAt, pending.text, true, pending.author?.avatarId, grouped = grouped)
                Box(Modifier.padding(start = 62.dp, end = 18.dp)) { Column { pendingStatus() } }
            }
        } }
        if (messages.isEmpty() && state.pendingMessage == null && !(inThread && state.thread?.loading == true)) item { Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(7.dp)) {
                if (state.selectedDirect?.let { it.peer.id == state.account?.id } == true) {
                    Text("You can message yourself here to keep notes, reminders, and ideas.", color = TextMuted, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                } else {
                    Text("No messages yet.", color = TextMuted)
                    val name = state.selectedChannel?.name.orEmpty()
                    Text(if (state.selectedChannel?.direct == true) "Only you and $name can read this conversation." else "Start the conversation in #$name.", color = TextMuted, fontSize = 12.sp)
                }
            }
        } }
        if (!inThread && state.hasNewerMessages) item {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
                TextButton(viewModel::loadNewer, enabled = !state.loadingNewer) { Text(if (state.loadingNewer) "Loading…" else "Load newer messages") }
                TextButton(viewModel::retryMessages) { Text("Back to latest") }
            }
        }
    }
    actionTarget?.let { target ->
        val presented = state.displayedMessages.firstOrNull { it.id == target.id } ?: target
        MessageActionsSheet(
            presented, state,
            onDismiss = { actionTarget = null },
            setReaction = viewModel::setReaction,
            openPicker = { actionTarget = null; pickerTarget = presented },
            setPin = { id, active -> viewModel.setPin(id, active); actionTarget = null },
            onReply = if (inThread) null else ({ actionTarget = null; viewModel.openThread(presented.threadRootId ?: presented.id) }),
            forward = { actionTarget = null; forwardTarget = presented },
            onEdit = if (viewModel.canEdit(presented)) ({ actionTarget = null; editTarget = presented }) else null,
            onHistory = { actionTarget = null; historyTarget = presented },
            blockLabel = presented.author.takeIf { state.account != null && !it.isGuest && it.id != selfId }?.let { author ->
                val target = viewModel.blockTarget(author)
                val name = if (target.username.isNotBlank()) "@${target.username}" else target.displayName
                if (author.id in blocked) "Unblock $name" else "Block $name"
            },
            onBlock = {
                actionTarget = null
                if (presented.author.id in blocked) viewModel.unblock(presented.author.id) else blockTarget = viewModel.blockTarget(presented.author)
            },
        )
    }
    blockTarget?.let { target -> BlockConfirmDialog(target, viewModel, { blockTarget = null }) }
    editTarget?.let { target ->
        if (viewModel.canEdit(target)) MessageEditorDialog(target, viewModel) { editTarget = null }
        else LaunchedEffect(target.id) { editTarget = null }
    }
    forwardTarget?.let { target -> key(target.id) { ForwardPickerSheet(target, viewModel) { forwardTarget = null } } }
    conversationTarget?.let { target ->
        val current = state.messages.firstOrNull { it.id == target.id }
        if (current != null) key(target.id) { ForwardConversationSheet(current, viewModel) { conversationTarget = null } }
    }
    historyTarget?.let { target ->
        MessageHistoryDialog(state.messages.find { it.id == target.id } ?: target, viewModel) { historyTarget = null }
    }
    reactorsTarget?.let { (messageId, emoji) ->
        val presented = state.messages.firstOrNull { it.id == messageId }
        if (presented == null) LaunchedEffect(messageId) { reactorsTarget = null }
        else key(messageId) {
            ReactorsSheet(presented, emoji, state.chatAuthorId ?: state.account?.id, viewModel::reactors) { reactorsTarget = null }
        }
    }
    mentionTarget?.let { target ->
        val card = mentionCard(target.id, target.username.orEmpty(), state)
        key(target) {
            MentionCardSheet(card, { done, failed -> card.username?.let { viewModel.messageMentioned(card.id, it, done, failed) } }) { mentionTarget = null }
        }
    }
    pickerTarget?.let { target ->
        val canReact = state.canParticipate && (state.chatAuthorId ?: state.account?.id) != null
        if (canReact) EmojiPicker(onDismiss = { pickerTarget = null }) { emoji ->
            pickerTarget = null
            viewModel.setReaction(target.id, emoji, true)
        } else LaunchedEffect(Unit) { pickerTarget = null }
    }
}

/** A flattened timeline row; [previous] decides the day divider. */
internal sealed interface TimelineRow {
    val key: String
    val first: ChatMessage
    val previous: ChatMessage?
    /** [above] is the message row directly above, for grouping: null under a blocked-messages row. */
    data class Message(val message: ChatMessage, override val previous: ChatMessage?, val above: ChatMessage?) : TimelineRow {
        override val key get() = message.id
        override val first get() = message
    }
    data class Blocked(val run: TimelineEntry.BlockedRun, override val previous: ChatMessage?) : TimelineRow {
        override val key get() = "blocked:${run.key}"
        override val first get() = run.first
    }
}

/** A revealed run lists its messages under its Hide row, without repeating the day divider. */
internal fun timelineRows(entries: List<TimelineEntry>): List<TimelineRow> = buildList {
    var previous: ChatMessage? = null
    entries.forEach { entry ->
        when (entry) {
            is TimelineEntry.Shown -> add(TimelineRow.Message(entry.message, previous, (lastOrNull() as? TimelineRow.Message)?.message))
            is TimelineEntry.BlockedRun -> {
                add(TimelineRow.Blocked(entry, previous))
                if (entry.revealed) entry.messages.forEachIndexed { index, message ->
                    add(TimelineRow.Message(message, if (index == 0) message else entry.messages[index - 1], entry.messages.getOrNull(index - 1)))
                }
            }
        }
        previous = entry.last
    }
}

@Composable private fun DateDivider(createdAt: String) {
    val label = fullDateLabel(createdAt)
    if (label.isEmpty()) return
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        HorizontalDivider(Modifier.weight(1f), color = Border)
        Text(label, color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
        HorizontalDivider(Modifier.weight(1f), color = Border)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun ReactionMessageRow(
    message: ChatMessage, state: AppUiState,
    setReaction: (String, String, Boolean) -> Unit,
    retryReaction: (String, String) -> Unit,
    dismissReactionError: (String, String) -> Unit,
    openReactors: (ChatMessage, String) -> Unit,
    retryPin: (String) -> Unit = {},
    dismissPinError: (String) -> Unit = {},
    openConversation: (ChatMessage) -> Unit = {},
    openHistory: (ChatMessage) -> Unit = {},
    openMention: (MessageMention) -> Unit = {},
    /** Compact under the same person's previous message (`data/Grouping.kt`). */
    grouped: Boolean = false,
    openActions: (ChatMessage) -> Unit,
) {
    val own = state.chatAuthorId ?: state.account?.id
    val canReact = state.canParticipate && own != null
    val saves = state.reactionSaves.filterKeys { it.startsWith("${message.id}:") }.values
    // A message that mentions you: terracotta wash with a 2dp leading edge.
    val mentioned = mentionsMe(message.content.mentions, message.author.id, state.account?.id ?: state.chatAuthorId)
    Column(if (!mentioned) Modifier else Modifier.background(Terracotta.copy(alpha = 0.08f)).drawBehind {
        val edge = 2.dp.toPx()
        drawRect(Terracotta, topLeft = Offset(if (layoutDirection == LayoutDirection.Rtl) size.width - edge else 0f, 0f), size = Size(edge, size.height))
    }) {
        // A grouped row stays a full-width long-press target, but drops the 48dp minimum so a run reads as one block.
        Box(Modifier.heightIn(min = if (grouped) 0.dp else 48.dp).combinedClickable(
            onClick = {},
            onLongClick = { openActions(message) },
            onLongClickLabel = "Message actions for ${message.author.name}",
        )) { MessageRow(message, openMention, { mentionCard(it.id, it.username.orEmpty(), state).title }, grouped) { if (message.forward == null) openHistory(message) } }
        ForwardCard(message) { openConversation(message) }
        FlowRow(Modifier.padding(start = 62.dp, end = 18.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            message.reactions.forEach { reaction ->
                val selected = own != null && own in reaction.authorIds
                ReactionChip(
                    reaction, selected, canReact,
                    toggle = { setReaction(message.id, reaction.emoji, !selected) },
                    showReactors = { openReactors(message, reaction.emoji) },
                )
            }
        }
        saves.filter { it.error != null }.forEach { save ->
            Column(Modifier.padding(start = 62.dp, end = 18.dp, top = 4.dp)) {
                Text(save.error ?: "Reaction could not be saved.", color = Terracotta, fontSize = 11.sp)
                Row {
                    TextButton({ retryReaction(message.id, save.emoji) }, enabled = canReact, modifier = Modifier.heightIn(min = 48.dp)) { Text("Retry") }
                    TextButton({ dismissReactionError(message.id, save.emoji) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Dismiss") }
                }
            }
        }
        state.pinSaves[message.id]?.takeIf { it.error != null }?.let { save ->
            Row(Modifier.padding(start = 62.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(save.error ?: "Pin could not be saved.", Modifier.weight(1f), color = Terracotta, fontSize = 11.sp)
                TextButton({ retryPin(message.id) }) { Text("Retry") }
                TextButton({ dismissPinError(message.id) }) { Text("Dismiss") }
            }
        }
    }
}

private val quickReactions = listOf("👍", "❤️", "😂", "🎉", "👀")

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun MessageActionsSheet(
    message: ChatMessage,
    state: AppUiState,
    onDismiss: () -> Unit,
    setReaction: (String, String, Boolean) -> Unit,
    openPicker: () -> Unit,
    setPin: (String, Boolean) -> Unit = { _, _ -> },
    onReply: (() -> Unit)? = null,
    forward: () -> Unit = {},
    onEdit: (() -> Unit)? = null,
    onHistory: (() -> Unit)? = null,
    blockLabel: String? = null,
    onBlock: () -> Unit = {},
) {
    val context = LocalContext.current
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    val own = state.chatAuthorId ?: state.account?.id
    val canReact = state.canParticipate && own != null
    fun copy(label: String, value: String) {
        clipboard.setPrimaryClip(ClipData.newPlainText(label, value))
        onDismiss()
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = SurfaceRaised, contentColor = Text) {
        Column(Modifier.fillMaxWidth().padding(start = 18.dp, end = 18.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Message actions", fontWeight = FontWeight.Bold, fontSize = 16.sp)
            if (canReact) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    quickReactions.forEach { emoji ->
                        val selected = message.reactions.firstOrNull { it.emoji == emoji }?.authorIds?.contains(own) == true
                        IconButton(
                            onClick = { setReaction(message.id, emoji, !selected); onDismiss() },
                            enabled = canReact,
                            modifier = Modifier.weight(1f).heightIn(min = 48.dp).background(if (selected) TerracottaWash else Color.Transparent, MaterialTheme.shapes.small).semantics {
                                this.selected = selected
                                contentDescription = "$emoji quick reaction"
                            },
                        ) { EmojiImage(emoji, null, Modifier.size(24.dp)) }
                    }
                    IconButton(openPicker, enabled = canReact, modifier = Modifier.weight(1f).heightIn(min = 48.dp)) {
                        Icon(painterResource(R.drawable.lucide_plus), "Add reaction", Modifier.size(18.dp), tint = TextMuted)
                    }
                }
            }
            Surface(shape = MaterialTheme.shapes.small, color = Surface) {
                Column {
                    if (state.account != null && (message.forward == null || message.forward.message != null)) {
                        TextButton(forward, Modifier.fillMaxWidth().heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) { Text("Forward message", Modifier.fillMaxWidth()) }
                        HorizontalDivider(color = Border)
                    }
                    if (canReact) {
                        val saving = state.pinSaves[message.id]?.saving == true
                        TextButton({ setPin(message.id, message.pin == null) }, enabled = !saving, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) {
                            Text(if (message.pin == null) "Pin message" else "Unpin message", Modifier.fillMaxWidth())
                        }
                        HorizontalDivider(color = Border)
                    }
                    onReply?.let { TextButton(it, Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Reply in thread", Modifier.fillMaxWidth()) }; HorizontalDivider(color = Border) }
                    onEdit?.let { TextButton(it, Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Edit message", Modifier.fillMaxWidth()) }; HorizontalDivider(color = Border) }
                    if (message.forward == null && message.revision > 1) onHistory?.let { TextButton(it, Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("View edit history", Modifier.fillMaxWidth()) }; HorizontalDivider(color = Border) }
                    TextButton({ copy("Message text", message.content.text) }, Modifier.fillMaxWidth().heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) { Text("Copy text", Modifier.fillMaxWidth()) }
                    HorizontalDivider(color = Border)
                    TextButton({ copy("Message ID", message.id) }, Modifier.fillMaxWidth().heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = Text)) { Text("Copy message ID", Modifier.fillMaxWidth()) }
                    blockLabel?.let { label ->
                        HorizontalDivider(color = Border)
                        TextButton(onBlock, Modifier.fillMaxWidth().heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = if (label.startsWith("Block")) ErrorText else Text)) { Text(label, Modifier.fillMaxWidth()) }
                    }
                }
            }
        }
    }
}

@Composable private fun PinnedMessages(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, close: () -> Unit) {
    var conversationTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var editTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var historyTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var actionTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var pickerTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var forwardTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var reactorsTarget by remember { mutableStateOf<Pair<String, String>?>(null) }
    var pinnerTarget by remember { mutableStateOf<ChatAuthor?>(null) }
    LaunchedEffect(state.account?.id, state.selectedChannel?.id, state.selectedDirectId) { conversationTarget = null; editTarget = null; historyTarget = null; pinnerTarget = null }
    LazyColumn(modifier.fillMaxWidth(), contentPadding = PaddingValues(vertical = 10.dp)) {
        item {
            state.messageContextError?.let { Text(it, Modifier.padding(18.dp), color = ErrorText) }
        }
        if (state.displayedPins.isEmpty()) item {
            Text("No pinned messages.", Modifier.fillMaxWidth().padding(24.dp), color = TextMuted, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        }
        items(state.displayedPins, key = { "pin:${it.id}" }) { message ->
            Column(Modifier.background(PinGoldWash)) {
                message.pin?.let { pin ->
                    val interaction = remember(message.id) { MutableInteractionSource() }
                    val hovered by interaction.collectIsHoveredAsState()
                    LaunchedEffect(hovered) { if (hovered) pinnerTarget = pin.author }
                    Text("Pinned by ${pin.author.name}", Modifier.padding(start = 62.dp, end = 18.dp)
                        .heightIn(min = 44.dp).wrapContentHeight()
                        .hoverable(interaction)
                        .combinedClickable(role = Role.Button, onClick = { pinnerTarget = pin.author }, onLongClick = { pinnerTarget = pin.author })
                        .semantics { contentDescription = "Open profile for ${pin.author.name}" },
                        color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
                }
                Text(fullDateLabel(message.createdAt), Modifier.padding(start = 62.dp, end = 18.dp), color = TextMuted, fontSize = 10.sp)
                ReactionMessageRow(message, state, viewModel::setReaction, viewModel::retryReaction, viewModel::dismissReactionError,
                    openReactors = { target, emoji -> reactorsTarget = target.id to emoji }, openActions = { actionTarget = it },
                    openConversation = { conversationTarget = it }, openHistory = { historyTarget = it },
                    retryPin = viewModel::retryPin, dismissPinError = viewModel::dismissPinError)
                Row(Modifier.fillMaxWidth().padding(start = 62.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextButton({ pinnerTarget = null; viewModel.goToMessage(message, close) }, enabled = !state.loadingMessageContext,
                        colors = ButtonDefaults.textButtonColors(contentColor = TextMuted), contentPadding = PaddingValues(vertical = 8.dp)) {
                        Text(if (state.loadingMessageContext) "Loading message…" else "Go to message")
                        Spacer(Modifier.width(6.dp))
                        Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(14.dp))
                    }
                    Spacer(Modifier.weight(1f))
                    IconButton({ actionTarget = message }) { Icon(painterResource(R.drawable.lucide_ellipsis), "Message actions for ${message.author.name}", tint = TextMuted) }
                }
            }
        }
    }
    pinnerTarget?.let { author ->
        val card = authorCard(author, state)
        key(author.id) {
            MentionCardSheet(card, { done, failed -> card.username?.let { viewModel.messageMentioned(card.id, it, { pinnerTarget = null; close(); done() }, failed) } }) { pinnerTarget = null }
        }
    }
    actionTarget?.let { target ->
        val current = state.pinnedMessages.find { it.id == target.id } ?: target
        MessageActionsSheet(current, state, onDismiss = { actionTarget = null }, setReaction = viewModel::setReaction,
            openPicker = { actionTarget = null; pickerTarget = current },
            setPin = { id, active -> actionTarget = null; viewModel.setPin(id, active) },
            onReply = { actionTarget = null; close(); viewModel.openThread(current.threadRootId ?: current.id) },
            forward = { actionTarget = null; forwardTarget = current },
            onEdit = if (viewModel.canEdit(current)) ({ actionTarget = null; editTarget = current }) else null,
            onHistory = { actionTarget = null; historyTarget = current })
    }
    pickerTarget?.let { target -> EmojiPicker({ pickerTarget = null }) { emoji -> pickerTarget = null; viewModel.setReaction(target.id, emoji, true) } }
    forwardTarget?.let { target -> key(target.id) { ForwardPickerSheet(target, viewModel) { forwardTarget = null } } }
    reactorsTarget?.let { (id, emoji) ->
        state.pinnedMessages.find { it.id == id }?.let { current ->
            ReactorsSheet(current, emoji, state.chatAuthorId ?: state.account?.id, viewModel::reactors) { reactorsTarget = null }
        }
    }
    conversationTarget?.let { target ->
        state.displayedPins.firstOrNull { it.id == target.id }?.let { current ->
            key(target.id) { ForwardConversationSheet(current, viewModel) { conversationTarget = null } }
        }
    }
    editTarget?.let { target ->
        if (viewModel.canEdit(target)) MessageEditorDialog(target, viewModel) { editTarget = null }
        else LaunchedEffect(target.id) { editTarget = null }
    }
    historyTarget?.let { target -> MessageHistoryDialog(state.pinnedMessages.find { it.id == target.id } ?: target, viewModel) { historyTarget = null } }
}

@Composable internal fun EmojiPicker(onDismiss: () -> Unit, select: (String) -> Unit) {
    val context = LocalContext.current
    val catalog = remember { EmojiArtwork.catalog(context).filter { it.selectable } }
    var query by rememberSaveable { mutableStateOf("") }
    val shown = remember(query, catalog) {
        val needle = query.trim().lowercase(Locale.ROOT)
        if (needle.isEmpty()) catalog else catalog.filter { it.name.lowercase(Locale.ROOT).contains(needle) || it.keywords.lowercase(Locale.ROOT).contains(needle) }
    }
    Dialog(onDismissRequest = onDismiss) {
        Surface(shape = MaterialTheme.shapes.medium, color = SurfaceRaised, border = BorderStroke(1.dp, Border), modifier = Modifier.fillMaxWidth().heightIn(max = 520.dp)) {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) { Text("Add reaction", Modifier.weight(1f), fontWeight = FontWeight.Bold); TextButton(onDismiss) { Text("Close") } }
                OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth(), placeholder = { Text("Search emoji") }, singleLine = true)
                if (shown.isEmpty()) Box(Modifier.fillMaxWidth().height(120.dp), contentAlignment = Alignment.Center) { Text("No emoji found.", color = TextMuted) }
                else LazyVerticalGrid(GridCells.Adaptive(44.dp), modifier = Modifier.heightIn(max = 390.dp)) {
                    gridItems(shown, key = { it.id }) { entry ->
                        IconButton({ select(entry.emoji) }, Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp).semantics { contentDescription = entry.name }) { EmojiImage(entry.emoji, null, Modifier.size(30.dp)) }
                    }
                }
            }
        }
    }
}
@Composable private fun MessageRow(
    message: ChatMessage, openMention: ((MessageMention) -> Unit)? = null,
    mentionLabel: (MessageMention) -> String = { "@${it.username}" }, grouped: Boolean = false, openHistory: () -> Unit = {},
) = MessageRow(
    message.author.name, message.author.isGuest, message.createdAt, message.content.text, false, message.author.avatarId,
    message.forward == null && message.revision > 1, openHistory, highlightedMentions(message.content.text, message.content.mentions), message.content.mentions, openMention, mentionLabel,
    grouped,
)
@Composable private fun MessageRow(
    author: String, guest: Boolean, createdAt: String, text: String, pending: Boolean, avatarId: Int? = null,
    edited: Boolean = false, openHistory: () -> Unit = {}, mentions: List<MentionSpan> = emptyList(), entries: List<MessageMention> = emptyList(),
    openMention: ((MessageMention) -> Unit)? = null, mentionLabel: (MessageMention) -> String = { "@${it.username}" },
    /** No avatar or header under the same person's previous message; the time shows on pointer hover. */
    grouped: Boolean = false,
) {
    val hover = remember { MutableInteractionSource() }
    val hovered by hover.collectIsHoveredAsState()
    Row(Modifier.fillMaxWidth().hoverable(hover).padding(start = 18.dp, end = 18.dp, top = if (grouped) 2.dp else 10.dp, bottom = 10.dp)) {
        if (grouped) Box(
            // The empty avatar column still names the author and time for screen readers.
            Modifier.width(34.dp).clearAndSetSemantics {
                contentDescription = listOfNotNull(author, "Guest".takeIf { guest }, timeLabel(createdAt).ifEmpty { null }).joinToString(", ")
            },
            contentAlignment = Alignment.TopCenter,
        ) {
            if (hovered) Text(shortTimeLabel(createdAt), color = TextMuted, fontSize = 10.sp, lineHeight = 21.sp, maxLines = 1, softWrap = false)
        } else Avatar(author, 34.dp, avatarId = avatarId)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            if (!grouped) Row(verticalAlignment = Alignment.CenterVertically) {
                Text(author, fontSize = 13.sp, fontWeight = FontWeight.Bold)
                if (guest) { Spacer(Modifier.width(7.dp)); Surface(color = Color.Transparent, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.extraSmall) { Text("GUEST", Modifier.padding(horizontal = 5.dp, vertical = 2.dp), color = TextMuted, fontSize = 8.sp, fontWeight = FontWeight.Bold) } }
                Spacer(Modifier.width(7.dp)); Text(timeLabel(createdAt), color = TextMuted, fontSize = 10.sp)
                if (edited) { Spacer(Modifier.width(7.dp)); Text("(edited)", Modifier.clickable(onClickLabel = "View edit history", onClick = openHistory), color = TextMuted, fontSize = 10.sp) }
            }
            // A grouped row's "(edited)" follows its text instead.
            val inlineEdited = grouped && edited
            // Unsent text stays plain; sent text gets mention pills and links.
            val links = remember(text, mentions, pending) { if (pending) emptyList() else linkRanges(text, mentions.map { it.start until it.end }) }
            if (mentions.isEmpty() && links.isEmpty() && !inlineEdited) Text(text, color = if (pending) TextMuted else MessageText, fontSize = 14.sp, lineHeight = 21.sp)
            else {
                // Pills and links are link annotations: their tap is consumed before the row's long-press handler sees it.
                val open by rememberUpdatedState(openMention)
                val history by rememberUpdatedState(openHistory)
                val uriHandler = LocalUriHandler.current
                val linked = if (openMention == null) null else entries
                val annotated = remember(text, mentions, linked, links, inlineEdited, uriHandler) {
                    messageText(text, mentions, linked, links, { user -> open?.invoke(user) }, { openExternalLink(uriHandler, it) }, if (inlineEdited) ({ history() }) else null)
                }
                val people = linked?.let { mentions.mapNotNull { span -> mentionedUser(span, it) }.distinct() }.orEmpty()
                // TalkBack also reaches each pill, link and the inline "(edited)" through labelled custom actions.
                Text(
                    annotated,
                    if (people.isEmpty() && links.isEmpty() && !inlineEdited) Modifier else Modifier.semantics {
                        customActions = people.map { user -> CustomAccessibilityAction("Open profile for ${mentionLabel(user)}") { open?.invoke(user); true } } +
                            links.map { link -> CustomAccessibilityAction("Open link ${text.substring(link.start, link.end)}") { openExternalLink(uriHandler, link.href); true } } +
                            listOfNotNull(if (inlineEdited) CustomAccessibilityAction("View edit history") { history(); true } else null)
                    },
                    color = if (pending) TextMuted else MessageText, fontSize = 14.sp, lineHeight = 21.sp,
                )
            }
        }
    }
}

/**
 * Message text: resolved mentions as pills (text #F3F4F5, one weight step bolder, terracotta at 24%),
 * then [links] (found outside the mentions, `data/Links.kt`) in terracotta-bright with an underline.
 * `SpanStyle` backgrounds cannot round corners or pad, so Android pills are square-edged.
 * [openHistory] appends a grouped row's "(edited)" marker right after the text.
 */
internal fun messageText(
    text: String, mentions: List<MentionSpan>, entries: List<MessageMention>? = null, links: List<LinkRange> = emptyList(),
    open: (MessageMention) -> Unit = {}, openLink: (String) -> Unit = {}, openHistory: (() -> Unit)? = null,
) = buildAnnotatedString {
    append(text)
    mentions.forEach { span ->
        // With [entries], a person's pill is a link (focusable, Enter-activatable; 32% while hovered, focused or pressed).
        val user = entries?.let { mentionedUser(span, it) }
        if (user == null) addStyle(MentionPill, span.start, span.end)
        else addLink(LinkAnnotation.Clickable("mention:${span.name}", MentionPillLink) { open(user) }, span.start, span.end)
    }
    // Only http(s) URLs reach here, and they open outside the app.
    links.forEach { link -> addLink(LinkAnnotation.Url(link.href, MessageLink) { openLink(link.href) }, link.start, link.end) }
    openHistory?.let { history ->
        append(" ")
        withLink(LinkAnnotation.Clickable("edited", EditedMarker) { history() }) { append("(edited)") }
    }
}

private val MentionPill = SpanStyle(color = Text, fontWeight = FontWeight.Medium, background = Terracotta.copy(alpha = 0.24f))
private val MentionPillActive = SpanStyle(background = Terracotta.copy(alpha = 0.32f))
private val MentionPillLink = TextLinkStyles(MentionPill, focusedStyle = MentionPillActive, hoveredStyle = MentionPillActive, pressedStyle = MentionPillActive)
private val MessageLinkActive = SpanStyle(background = TerracottaWash)
internal val MessageLink = TextLinkStyles(
    SpanStyle(color = TerracottaBright, textDecoration = TextDecoration.Underline),
    focusedStyle = MessageLinkActive, hoveredStyle = MessageLinkActive, pressedStyle = MessageLinkActive,
)
private val EditedMarker = TextLinkStyles(SpanStyle(color = TextMuted, fontSize = 10.sp))

/** Opens an http(s) link in the system browser (or the app that handles it); with neither, the tap does nothing. */
internal fun openExternalLink(uriHandler: UriHandler, href: String) {
    runCatching { uriHandler.openUri(href) }
}

@Composable private fun TypingLine(authors: List<ChatAuthor>) {
    val label = when { authors.size > 2 -> "Several people are typing…"; authors.size == 2 -> "${authors[0].name} and ${authors[1].name} are typing…"; authors.size == 1 -> "${authors[0].name} is typing…"; else -> "" }
    // Web keeps the last label for its 180 ms fade-out.
    var shown by remember { mutableStateOf("") }
    LaunchedEffect(label) { if (label.isNotEmpty()) shown = label else { kotlinx.coroutines.delay(180); shown = "" } }
    val alpha by androidx.compose.animation.core.animateFloatAsState(if (label.isNotEmpty()) 1f else 0f, androidx.compose.animation.core.tween(180), label = "typing")
    Row(Modifier.fillMaxWidth().height(20.dp).padding(horizontal = 18.dp).graphicsLayer { this.alpha = alpha }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
        if (shown.isNotEmpty()) {
            TypingDots()
            Text(shown, color = TextMuted, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Web's three 4px dots: 1.2 s bounce staggered by .15 s, still when animations are off. */
@Composable private fun TypingDots() {
    val context = LocalContext.current
    val still = remember { android.provider.Settings.Global.getFloat(context.contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
    val transition = androidx.compose.animation.core.rememberInfiniteTransition(label = "typing-dots")
    Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.CenterVertically) {
        repeat(3) { index ->
            val bounce = if (still) 0f else transition.animateFloat(0f, 0f, androidx.compose.animation.core.infiniteRepeatable(
                androidx.compose.animation.core.keyframes {
                    durationMillis = 1_200
                    0f at 0 using androidx.compose.animation.core.FastOutSlowInEasing
                    1f at 360 using androidx.compose.animation.core.FastOutSlowInEasing
                    0f at 720
                },
                initialStartOffset = androidx.compose.animation.core.StartOffset(150 * index),
            ), label = "dot$index").value
            Box(Modifier.size(4.dp).graphicsLayer { translationY = -3.dp.toPx() * bounce; alpha = .45f + .55f * bounce }.clip(CircleShape).background(TextMuted))
        }
    }
}

/** Web's `.chat-counter` tones at 3,500, 3,750 and 3,900 characters. */
internal fun counterTone(count: Int): Color = when {
    count >= 3900 -> Color(0xFFFF827C)
    count >= 3750 -> Color(0xFFEDA361)
    count >= 3500 -> Color(0xFFE4C76A)
    else -> TextMuted
}

@Composable internal fun Avatar(name: String, size: Dp, modifier: Modifier = Modifier, avatarId: Int? = null, speaking: Boolean = false) {
    val index = caperAvatarIndex(avatarId)
    Box(
        modifier
            // Web: caper-green border with a soft outer ring while speaking.
            .size(size)
            .then(if (speaking) Modifier.drawBehind {
                drawCircle(CaperGreen.copy(alpha = .2f), radius = this.size.minDimension / 2 + 1.5.dp.toPx(), style = androidx.compose.ui.graphics.drawscope.Stroke(3.dp.toPx()))
            } else Modifier).clip(CircleShape)
            // Bundled image avatars retain their transparent backing. Initials
            // still need contrast against every surface.
            .then(if (index == null) Modifier.background(if (size > 32.dp) SurfaceRaised else SurfaceComposer) else Modifier)
            .then(if (speaking) Modifier.border(2.dp, CaperGreen, CircleShape) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        if (index == null) Text(name.take(1).uppercase(), fontWeight = FontWeight.Black, fontSize = (size.value * .38f).sp)
        else {
            Image(
                painter = painterResource(caperAvatarResources[index]),
                contentDescription = null,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
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

@Composable private fun LoginScreen(busy: Boolean, error: String?, clearError: () -> Unit, submit: (String) -> Unit) {
    var email by remember { mutableStateOf("") }
    val emailFocus = remember { FocusRequester() }
    var requested by remember { mutableStateOf(false) }
    AuthFrame {
        // Requests keep the field focused (read-only, not disabled) so a retry needs no extra tap.
        // Like the Apple client, the keyboard isn't raised when sign-in first opens; only once a
        // request has finished. Inside AuthFrame's subcomposition, so the field is attached here.
        LaunchedEffect(busy) {
            if (busy) requested = true else if (requested) runCatching { emailFocus.requestFocus() }
        }
        Text("Welcome to Caper", Modifier.padding(bottom = 4.dp), fontSize = 49.sp, lineHeight = 53.sp, fontWeight = FontWeight.Bold, letterSpacing = (-2.5).sp)
        Text("Use your email to create an account or return to one. We’ll send a code to your email.", color = TextMuted, lineHeight = 26.sp)
        Text("Email address", Modifier.padding(top = 20.dp, bottom = 8.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        OutlinedTextField(
            email, { email = it; if (error != null) clearError() }, placeholder = { Text("you@example.com") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { if (email.contains('@') && !busy) submit(email) }),
            singleLine = true, readOnly = busy, modifier = Modifier.fillMaxWidth().focusRequester(emailFocus),
        )
        if (error != null) Surface(Modifier.fillMaxWidth().padding(top = 20.dp), color = Color.Transparent, border = BorderStroke(1.dp, Terracotta), shape = MaterialTheme.shapes.small) {
            Text(error, Modifier.padding(horizontal = 14.dp, vertical = 12.dp), lineHeight = 24.sp)
        }
        Button({ submit(email) }, enabled = email.contains('@') && !busy, modifier = Modifier.align(Alignment.End).padding(top = 12.dp), shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            Text(if (busy) "Sending…" else "Email me a code")
            if (!busy) {
                Spacer(Modifier.width(12.dp))
                Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(20.dp))
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable private fun VerifyScreen(screen: SessionScreen.Verify, busy: Boolean, error: String?, clearError: () -> Unit, back: () -> Unit, submit: (String, String) -> Unit, resend: () -> Unit) {
    var code by remember(screen.challengeId) { mutableStateOf("") }
    val exhausted = screen.attemptsRemaining == 0
    val codeFocus = remember { FocusRequester() }
    // Which request is running, so only that control says so.
    var resending by remember { mutableStateOf(false) }
    LaunchedEffect(busy) { if (!busy) resending = false }
    val sendNewCode = { resending = true; resend() }
    AuthFrame {
        LaunchedEffect(busy, screen.challengeId) { if (!busy && !exhausted) runCatching { codeFocus.requestFocus() } }
        Text("Check your email.", Modifier.padding(bottom = 4.dp), fontSize = 49.sp, lineHeight = 53.sp, fontWeight = FontWeight.Bold, letterSpacing = (-2.5).sp)
        Text("Enter the six-character code sent to ${screen.email}. It expires in 10 minutes.", color = TextMuted, lineHeight = 26.sp)
        Text("Sign-in code", Modifier.padding(top = 20.dp, bottom = 8.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        OutlinedTextField(code, { code = it.uppercase().filter { character -> character in "ABCDEFGHJKMNPQRSTWXYZ23456789" }.take(6); if (error != null) clearError() }, singleLine = true,
            readOnly = busy, enabled = !exhausted, modifier = Modifier.fillMaxWidth().focusRequester(codeFocus))
        if (error != null) Surface(Modifier.fillMaxWidth().padding(top = 20.dp), color = Color.Transparent, border = BorderStroke(1.dp, Terracotta), shape = MaterialTheme.shapes.small) { Text(error, Modifier.padding(14.dp)) }
        if (screen.attemptsRemaining == 1) Text("One attempt left. Check the code carefully.", Modifier.padding(top = 12.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        // Shown after a resend, announced politely; the new challenge also cleared the field and the attempts.
        if (screen.resends > 0) Text(RESENT_CODE_STATUS, Modifier.padding(top = 12.dp).semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 14.sp)
        // "Email me a new code" counts as a resend too, and restarts the wait.
        // At the resend limit the API would send nothing, so the button is hidden (as on web).
        if (exhausted) { if (screen.resends < MAX_CODE_RESENDS) Button(sendNewCode, enabled = !busy, modifier = Modifier.fillMaxWidth().padding(top = 12.dp), shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            Text(if (busy) "Sending…" else "Email me a new code", Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Start)
            if (!busy) Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(20.dp))
        } } else Button({ submit(screen.challengeId, code) }, enabled = code.length == 6 && !busy, modifier = Modifier.fillMaxWidth().padding(top = 12.dp), shape = MaterialTheme.shapes.small, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            Text(if (busy && !resending) "Checking…" else "Continue", Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Start)
            if (!busy || resending) Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(20.dp))
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(20.dp), itemVerticalAlignment = Alignment.CenterVertically) {
            TextButton(back, enabled = !busy, contentPadding = PaddingValues(vertical = 16.dp)) { Text("Use a different email", color = TextMuted) }
            // Hidden while "Email me a new code" is shown; after the last resend, a hint replaces it.
            if (screen.resends >= MAX_CODE_RESENDS) Text(RESEND_LIMIT_HINT, Modifier.padding(vertical = 16.dp), color = TextMuted, fontSize = 14.sp)
            else if (!exhausted) {
                var now by remember { mutableLongStateOf(android.os.SystemClock.elapsedRealtime()) }
                LaunchedEffect(screen.sentAt) {
                    while (true) {
                        now = android.os.SystemClock.elapsedRealtime()
                        val left = screen.sentAt + RESEND_COOLDOWN_MILLIS - now
                        if (left <= 0) break
                        // Wake when the whole-second countdown next changes.
                        delay((left - 1) % 1_000 + 1)
                    }
                }
                val secondsLeft = resendSecondsLeft(screen.sentAt, now)
                val ready = !busy && secondsLeft == 0
                TextButton(sendNewCode, enabled = ready, contentPadding = PaddingValues(vertical = 16.dp)) {
                    Text(if (resending) "Sending…" else resendLabel(secondsLeft), color = if (ready) TextMuted else TextMuted.copy(alpha = 0.6f))
                }
            }
        }
    }
}

@Composable internal fun ProfileScreen(account: Account, busy: Boolean, error: String?, close: (() -> Unit)?, submit: (String, String) -> Unit) {
    var username by remember(account.id) { mutableStateOf(account.username.orEmpty()) }
    var name by remember(account.id) { mutableStateOf(account.displayName.orEmpty()) }
    val form: @Composable ColumnScope.() -> Unit = {
        if (close == null) Text("ONE LAST THING", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.5.sp)
        if (close == null) Text("Choose how you show up.", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
        Text("Your username is unique. Your display name is what people see in conversations.", color = TextMuted, fontSize = 12.sp)
        // Web's field hints (account/ProfileForm.tsx).
        OutlinedTextField(username, { username = normalizeUsername(it) }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ImeAction.Next),
            supportingText = { Text("3-32 lowercase letters, numbers, or underscores.", color = TextMuted) })
        OutlinedTextField(name, { name = it.codePointTake(64) }, label = { Text("Display name") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            supportingText = { Text("Shown to other people. It does not need to be unique.", color = TextMuted) })
        Box(Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        }
        Button({ submit(username, name) }, enabled = profileValid(username, name) && !busy, modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small) { Text(if (busy) "Saving…" else if (account.username.isNullOrEmpty()) "Finish account" else "Save profile") }
    }
    if (close == null) AuthFrame { form() } else CaperDialog("Edit profile", close) { form() }
}

@Composable private fun FirstSpaceScreen(state: AppUiState, viewModel: CaperViewModel, browse: () -> Unit) {
    var name by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    val limits = state.limits
    val allowed = limits != null && state.spaces.count { it.ownerId == state.account?.id } < limits.ownedSpaces &&
        state.spaces.count { !it.demo } < limits.totalSpaces
    AuthFrame {
        Text("Name your space", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(12.dp))
        Text("Choose something you will recognize easily. You can always change it later!", color = TextMuted, fontSize = 12.sp)
        Spacer(Modifier.height(20.dp))
        OutlinedTextField(name, { name = it.codePointTake(80); error = null }, label = { Text("Space name") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        if (!allowed) Text("You have reached your space limit.", color = TextMuted, fontSize = 12.sp)
        (error ?: state.error)?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        Spacer(Modifier.height(16.dp))
        Button({ spaceNameError(name)?.let { error = it } ?: viewModel.createSpace(name.trim()) },
            enabled = allowed && !state.busy && name.isNotBlank(), modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small,
        ) { Text(if (state.busy) "Creating…" else "Create space") }
        OutlinedButton(browse, modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small) { Text("Direct messages") }
        TextButton(viewModel::logout, Modifier.align(Alignment.End)) { Text("Log out", color = TextMuted) }
    }
}

@Composable private fun CreateSpaceDialog(busy: Boolean, serverError: String?, clearError: () -> Unit, close: () -> Unit, create: (String) -> Unit) {
    var name by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    // The app-wide error is this dialog's once it has submitted; editing clears it.
    var submitted by remember { mutableStateOf(false) }
    CaperDialog("Create a space", close, busy = busy) {
        val field = remember { FocusRequester() }
        LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
        OutlinedTextField(name, { name = it.codePointTake(80); error = null; if (submitted && serverError != null) clearError() }, label = { Text("Space name") }, placeholder = { Text("Studio") },
            modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true)
        (error ?: serverError?.takeIf { submitted })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        // Web validates on submit and says why (spaces/client.ts spaceNameError).
        DialogActions(close, "Create space", busy, pendingLabel = "Creating…") {
            spaceNameError(name)?.let { error = it } ?: run { submitted = true; create(name.trim()) }
        }
    }
}

@Composable private fun StartDirectDialog(busy: Boolean, serverError: String?, clearError: () -> Unit, close: () -> Unit, start: (String) -> Unit) {
    var username by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var submitted by remember { mutableStateOf(false) }
    CaperDialog("New direct message", close, busy = busy) {
        val field = remember { FocusRequester() }
        LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
        Text("Enter an account’s exact username.", color = TextMuted, fontSize = 12.sp)
        OutlinedTextField(username, { username = normalizeUsername(it); error = null; if (submitted && serverError != null) clearError() }, label = { Text("Username") }, placeholder = { Text("username") },
            modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true)
        (error ?: serverError?.takeIf { submitted })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        DialogActions(close, "Start conversation", busy, pendingLabel = "Opening…", canSubmit = username.isNotBlank()) {
            if (!Regex("^[a-z0-9_]{3,32}$").matches(username)) error = "Enter an exact valid username." else { submitted = true; start(username) }
        }
    }
}

@Composable private fun CreateChannelDialog(detail: SpaceDetail, busy: Boolean, serverError: String?, clearError: () -> Unit, close: () -> Unit, create: (String, Boolean) -> Unit) {
    var name by remember { mutableStateOf("") }; var private by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var submitted by remember { mutableStateOf(false) }
    CaperDialog("Create a channel", close, busy = busy) {
        val field = remember { FocusRequester() }
        LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
        OutlinedTextField(name, { name = normalizeChannel(it); error = null; if (submitted && serverError != null) clearError() }, label = { Text("Channel name") }, placeholder = { Text("project-updates") },
            leadingIcon = { Icon(painterResource(if (private) R.drawable.lucide_lock_keyhole else R.drawable.lucide_hash), null, Modifier.size(18.dp)) },
            modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true)
        Text("Channels are where conversations happen around a topic. Use a name that is easy to find and understand.", color = TextMuted, fontSize = 12.sp, lineHeight = 18.sp)
        PrivacyToggle(private, detail.space.name) { private = it }
        (error ?: serverError?.takeIf { submitted })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        DialogActions(close, "Create channel", busy, pendingLabel = "Creating…", canSubmit = name.removeSuffix("-").isNotEmpty()) {
            channelNameError(name.removeSuffix("-"))?.let { error = it } ?: run { submitted = true; create(name.removeSuffix("-"), private) }
        }
    }
}

@Composable private fun ManageSpaceDialog(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(detail.space.id) { mutableStateOf(detail.space.name) }
    var confirmingDelete by remember { mutableStateOf(false) }
    var removing by remember(detail.space.id) { mutableStateOf<Member?>(null) }
    LaunchedEffect(detail.space.id) { viewModel.loadSpaceInvitations() }
    CaperDialog("Manage space", close, wide = true, description = "Only the owner can change this space and its membership.") {
        state.error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        OutlinedTextField(name, { name = it.codePointTake(80); if (state.error != null) viewModel.clearError() }, label = { Text("Space name") }, placeholder = { Text("Studio") }, modifier = Modifier.fillMaxWidth())
        Button({ viewModel.renameSpace(name) }, enabled = !state.busy && name.isNotBlank() && name.trim() != detail.space.name, shape = MaterialTheme.shapes.small) { Text("Save name") }
        HorizontalDivider(color = Border)
        InviteManager(detail.members, state.pendingSpaceInvitations, state.busy, viewModel::addSpaceMember,
            { removing = it }, viewModel::cancelSpaceInvitation)
        HorizontalDivider(color = Border)
        DangerZone("Delete space", "Delete this space and all its channels for every member.", state.busy) { confirmingDelete = true }
    }
    if (confirmingDelete) ConfirmDialog("Delete space", "Delete ${detail.space.name} for everyone? All its channels and their messages will disappear from the space. This cannot be undone.", "Delete space", state.busy, { confirmingDelete = false }, warn = true, error = state.error) { viewModel.deleteCurrentSpace { CaperEffects.play(CaperEffects.Effect.Delete); close() } }
    // Manage space stays open behind the confirmation, with the member gone once it succeeds.
    removing?.let { member ->
        key(member.id) {
            ConfirmDialog("Remove ${member.displayName}?", "They’ll lose access to ${detail.space.name} and its channels. You can invite them again later.", "Remove", state.busy,
                { removing = null }, pendingLabel = "Removing…", error = state.error, focusCancel = true) { viewModel.removeSpaceMember(member) { removing = null } }
        }
    }
}

@Composable private fun InvitationDialog(space: Space, busy: Boolean, error: String?, close: () -> Unit, accept: () -> Unit, decline: () -> Unit) {
    // Which answer is being sent: only that button says so.
    var accepting by remember(space.id) { mutableStateOf<Boolean?>(null) }
    CaperDialog("You’re invited!", close, titleIcon = R.drawable.incoming_envelope) {
        Text("Join ${space.name}?", fontSize = 20.sp, fontWeight = FontWeight.Bold)
        space.inviter?.let { Text("${it.displayName} (@${it.username}) invited you.", color = TextMuted) }
        error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            OutlinedButton({ accepting = false; decline() }, enabled = !busy, shape = MaterialTheme.shapes.small) {
                Text(if (busy && accepting == false) "Declining…" else "Decline")
            }
            Spacer(Modifier.width(8.dp))
            Button({ accepting = true; accept() }, enabled = !busy, shape = MaterialTheme.shapes.small) {
                Text(if (busy && accepting == true) "Accepting…" else "Accept")
            }
        }
    }
}

@Composable private fun InviteManager(
    members: List<Member>, pending: List<Member>, busy: Boolean, add: (String, () -> Unit) -> Unit,
    remove: (Member) -> Unit, cancel: (Member) -> Unit,
) {
    Text("Invite", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    MemberManager(members, busy, add, remove, heading = "Members", action = "Invite")
    Text("Pending invitations", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    if (pending.isEmpty()) Text("No pending invitations.", color = TextMuted, fontSize = 11.sp)
    pending.forEach { member ->
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(member.displayName, 30.dp, avatarId = member.avatarId); Spacer(Modifier.width(9.dp))
            Column(Modifier.weight(1f)) {
                Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("@${member.username}", color = TextMuted, fontSize = 10.sp)
            }
            TextButton({ cancel(member) }, enabled = !busy) { Text("Cancel", color = ErrorText) }
        }
    }
}

@Composable private fun ManageChannelDialog(state: AppUiState, channel: Channel, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(channel.id) { mutableStateOf(channel.name) }; var private by remember(channel.id) { mutableStateOf(channel.private) }
    var confirmingDelete by remember { mutableStateOf(false) }
    var removing by remember(channel.id) { mutableStateOf<Member?>(null) }
    LaunchedEffect(channel.id, channel.private) { viewModel.loadChannelGrants(channel) }
    val dirty = name.removeSuffix("-") != channel.name || private != channel.private
    // Keep the sticky save bar's footprint stable while hiding clean controls.
    CaperDialog("Overview", close, wide = true, footer = {
        val hiddenSemantics = if (dirty) Modifier else Modifier.clearAndSetSemantics { }
        Row(Modifier.fillMaxWidth().then(hiddenSemantics).graphicsLayer { alpha = if (dirty) 1f else 0f }.background(Blackout).padding(horizontal = 22.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("You have unsaved changes.", Modifier.weight(1f), fontSize = 12.sp)
            OutlinedButton({ name = channel.name; private = channel.private }, enabled = dirty && !state.busy, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) { Text("Reset") }
            Spacer(Modifier.width(8.dp))
            Button({ viewModel.updateChannel(channel, name.removeSuffix("-"), private) }, enabled = dirty && !state.busy && !channelInvalid(name), shape = MaterialTheme.shapes.small) { Text(if (state.busy) "Saving…" else "Save changes") }
        }
    }) {
        state.error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        OutlinedTextField(name, { name = normalizeChannel(it); if (state.error != null) viewModel.clearError() }, label = { Text("Channel name") }, placeholder = { Text("project-updates") }, modifier = Modifier.fillMaxWidth())
        PrivacyToggle(private, state.selectedSpace?.space?.name ?: "this space", stableSwitch = true) { private = it }
        if (channel.private) {
            HorizontalDivider(color = Border)
            InviteManager(state.channelGrants, state.pendingChannelInvitations, state.busy, { username, added -> viewModel.addChannelGrant(channel, username, added) },
                { removing = it }, { viewModel.cancelChannelInvitation(channel, it) })
        }
        HorizontalDivider(color = Border)
        DangerZone("Delete channel", "Delete this channel for everyone in the space.", state.busy) { confirmingDelete = true }
    }
    if (confirmingDelete) ConfirmDialog("Delete channel", "Delete #${channel.name} for everyone? This channel and its messages will disappear from the space. This cannot be undone.", "Delete channel", state.busy, { confirmingDelete = false }, warn = true, error = state.error) { viewModel.deleteChannel(channel) { CaperEffects.play(CaperEffects.Effect.Delete); close() } }
    removing?.let { member ->
        key(member.id) {
            ConfirmDialog("Remove ${member.displayName} from #${channel.name}?", "They’ll lose access to this private channel. You can add them again later.", "Remove", state.busy,
                { removing = null }, pendingLabel = "Removing…", error = state.error, focusCancel = true) { viewModel.removeChannelGrant(channel, member) { removing = null } }
        }
    }
}

/** Web's MemberManager: exact-username add, then "@username · Owner" rows. */
@Composable private fun MemberManager(members: List<Member>, busy: Boolean, add: (String, () -> Unit) -> Unit, remove: (Member) -> Unit, heading: String = "Members", action: String = "Add") {
    var username by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(heading, fontSize = 13.sp, fontWeight = FontWeight.Bold)
        Surface(color = SurfaceRaised, shape = CircleShape) { Text(members.size.toString(), Modifier.padding(horizontal = 6.dp, vertical = 2.dp), color = TextMuted, fontSize = 10.sp) }
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        // The field empties only after the add succeeds, so a failed add keeps the username to fix.
        val submitMember = { if (!usernameValid(username)) error = "Use 3–32 lowercase letters, numbers, or underscores." else {
            error = null
            val submitted = username
            add(submitted) { if (username == submitted) username = "" }
        } }
        OutlinedTextField(username, { username = normalizeUsername(it); error = null }, label = { Text("Exact username") }, modifier = Modifier.weight(1f), singleLine = true,
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (!busy) submitMember() }))
        Spacer(Modifier.width(8.dp))
        Button(submitMember, enabled = !busy, shape = MaterialTheme.shapes.small) { Text(action) }
    }
    error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
    members.forEach { member ->
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(member.displayName, 30.dp, avatarId = member.avatarId); Spacer(Modifier.width(9.dp))
            Column(Modifier.weight(1f)) {
                Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("@${member.username}${if (member.owner) " · Owner" else ""}", color = TextMuted, fontSize = 10.sp)
            }
            if (!member.owner) TextButton({ remove(member) }, enabled = !busy) { Text("Remove", color = ErrorText) }
        }
    }
}

@Composable private fun DangerZone(title: String, body: String, busy: Boolean, delete: () -> Unit) {
    Text(title, fontSize = 13.sp, fontWeight = FontWeight.Bold)
    Text(body, color = TextMuted, fontSize = 11.sp)
    OutlinedButton(delete, enabled = !busy, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.outlinedButtonColors(contentColor = ErrorText), border = BorderStroke(1.dp, Danger)) { Text(title) }
}

internal fun canEditRejectedMessage(draft: String, rejectedText: String): Boolean =
    draft.isBlank() || draft == rejectedText

@Composable private fun PrivacyToggle(value: Boolean, spaceName: String, stableSwitch: Boolean = false, changed: (Boolean) -> Unit) = Row(Modifier.fillMaxWidth().clickable { CaperEffects.toggle(!value); changed(!value) }, verticalAlignment = if (stableSwitch) Alignment.Top else Alignment.CenterVertically) {
    Icon(painterResource(R.drawable.lucide_lock_keyhole), null, Modifier.size(17.dp), tint = TextMuted); Spacer(Modifier.width(8.dp)); Column(Modifier.weight(1f)) { Text("Private channel", fontWeight = FontWeight.Bold, fontSize = 13.sp); Text(if (value) "Only you and the people you add can view or join." else "Anyone in $spaceName can view or join this channel.", color = TextMuted, fontSize = 11.sp) }; Switch(value, { CaperEffects.toggle(it); changed(it) })
}

@Composable private fun ConfirmDialog(
    title: String, body: String, action: String, busy: Boolean, close: () -> Unit, warn: Boolean = false,
    pendingLabel: String = if (action.startsWith("Delete")) "Deleting…" else "Saving…",
    /** The app-wide error, shown once this action has run. */
    error: String? = null,
    /** Cancel takes the initial focus, so Enter or a stray key press never confirms. */
    focusCancel: Boolean = false,
    confirm: () -> Unit,
) = CaperDialog(title, close, busy = busy) {
    // Web plays its warning once when a delete confirmation opens.
    if (warn) LaunchedEffect(Unit) { CaperEffects.play(CaperEffects.Effect.Warning) }
    var confirmed by remember { mutableStateOf(false) }
    val cancelFocus = remember { FocusRequester() }
    if (focusCancel) LaunchedEffect(Unit) { runCatching { cancelFocus.requestFocus() } }
    Text(body, color = TextMuted)
    error?.takeIf { confirmed }?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
        // Web: Cancel waits for the action; the action shows its progress.
        TextButton(close, Modifier.focusRequester(cancelFocus), enabled = !busy) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
        Button({ confirmed = true; confirm() }, enabled = !busy, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.buttonColors(containerColor = Danger)) {
            Text(if (busy) pendingLabel else action)
        }
    }
}

@Composable internal fun CaperDialog(
    title: String,
    close: () -> Unit,
    wide: Boolean = false,
    description: String? = null,
    footer: (@Composable () -> Unit)? = null,
    titleIcon: Int? = null,
    /** A request is in flight: X, back and outside taps wait, so it can't finish behind a closed dialog. */
    busy: Boolean = false,
    content: @Composable ColumnScope.() -> Unit,
) {
    Dialog({ if (!busy) close() }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        BoxWithConstraints(Modifier.padding(16.dp).widthIn(max = if (wide) 600.dp else 460.dp).fillMaxWidth(), contentAlignment = Alignment.Center) {
            Surface(Modifier.fillMaxWidth().height(minOf(maxHeight, 760.dp)), color = Surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                Column {
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(22.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
                            Column(Modifier.weight(1f)) {
                                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                    titleIcon?.let { Image(painterResource(it), contentDescription = null, modifier = Modifier.size(32.dp)) }
                                    Text(title, fontSize = 19.sp, fontWeight = FontWeight.Bold)
                                }
                                description?.let { Text(it, color = TextMuted, fontSize = 12.sp, lineHeight = 17.sp) }
                            }
                            IconButton(close, enabled = !busy) { Icon(painterResource(R.drawable.lucide_x), "Close", tint = if (busy) TextMuted.copy(alpha = 0.4f) else TextMuted) }
                        }
                        HorizontalDivider(color = Border); content()
                    }
                    if (footer != null) { HorizontalDivider(color = Border); footer() }
                }
            }
        }
    }
}

/** [pendingLabel] says what the request is doing; [canSubmit] is false while the required field is empty. */
@Composable private fun DialogActions(
    close: () -> Unit, label: String, busy: Boolean, pendingLabel: String = "Saving…", canSubmit: Boolean = true, action: () -> Unit,
) = Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
    TextButton(close, enabled = !busy) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
    Button(action, enabled = !busy && canSubmit, shape = MaterialTheme.shapes.small) { Text(if (busy) pendingLabel else label) }
}

/** Web's spaces/client.ts validation copy. */
internal fun spaceNameError(name: String): String? {
    val trimmed = name.trim()
    return when {
        trimmed.isEmpty() -> "Enter a space name."
        trimmed.codePointCount(0, trimmed.length) > 80 -> "Space names can be at most 80 characters."
        trimmed.any { it.isISOControl() } -> "Space names cannot contain control characters."
        else -> null
    }
}

internal fun channelNameError(name: String): String? = when {
    name.isEmpty() -> "Enter a channel name."
    name.codePointCount(0, name.length) > 80 -> "Channel names can be at most 80 characters."
    channelInvalid(name) -> "Use lowercase letters separated by single dashes."
    else -> null
}

private fun normalizeUsername(value: String) = value.lowercase().filter { it in 'a'..'z' || it in '0'..'9' || it == '_' }.take(32)
internal fun usernameValid(username: String) = Regex("^[a-z0-9_]{3,32}$").matches(username)
internal fun profileValid(username: String, displayName: String) =
    usernameValid(username) && displayName.isNotBlank() &&
        displayName.codePointCount(0, displayName.length) <= 64 && displayName.none { it.isISOControl() }
private fun normalizeChannel(value: String) = value.lowercase().replace(Regex("\\s+"), "-").filter { it in 'a'..'z' || it == '-' }.replace(Regex("-+"), "-").removePrefix("-").take(80)
private fun channelInvalid(value: String) = !Regex("^[a-z]+(?:-[a-z]+)*$").matches(value.removeSuffix("-"))
private fun String.codePointTake(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max))
private fun localDate(value: String, zoneId: ZoneId): LocalDate? = runCatching { Instant.parse(value).atZone(zoneId).toLocalDate() }.getOrNull()
internal fun sameLocalDay(first: String, second: String, zoneId: ZoneId = ZoneId.systemDefault()): Boolean =
    localDate(first, zoneId)?.let { it == localDate(second, zoneId) } == true
internal fun fullDateLabel(value: String, zoneId: ZoneId = ZoneId.systemDefault(), locale: Locale = Locale.getDefault()): String = runCatching {
    DateTimeFormatter.ofLocalizedDate(FormatStyle.FULL).withLocale(locale).format(Instant.parse(value).atZone(zoneId))
}.getOrDefault("")
private fun timeLabel(value: String): String = runCatching { DateTimeFormatter.ofPattern("h:mm a").format(Instant.parse(value).atZone(ZoneId.systemDefault())) }.getOrDefault("")
/** A grouped row's hover time, without AM/PM ("8:28"). */
private fun shortTimeLabel(value: String): String = runCatching { DateTimeFormatter.ofPattern("h:mm").format(Instant.parse(value).atZone(ZoneId.systemDefault())) }.getOrDefault("")
