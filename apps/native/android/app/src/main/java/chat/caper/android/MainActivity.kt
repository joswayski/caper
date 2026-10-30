package chat.caper.android

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.focusable
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.DragInteraction
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.mimeTypes
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
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
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.setProgress
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
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
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
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.ui.platform.LocalView
import chat.caper.android.data.WhatsNewSnapshot
import chat.caper.android.data.AttachCapacity
import chat.caper.android.data.AttachmentPolicy
import chat.caper.android.data.VoiceMessages
import chat.caper.android.data.FileDrop
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
import chat.caper.android.data.collapsesFor
import chat.caper.android.data.groupBlocked
import chat.caper.android.data.groupsWithPrevious
import chat.caper.android.data.linkRanges
import chat.caper.android.data.resendLabel
import chat.caper.android.data.resendSecondsLeft
import chat.caper.android.data.mainDirects
import chat.caper.android.data.messageRequests
import chat.caper.android.data.withFreshUrls
import chat.caper.android.data.ADMIN_BADGE
import chat.caper.android.data.CAPER_TEAM_NOTIFIED
import chat.caper.android.data.CAPER_DM_EMPTY_NOTE
import chat.caper.android.data.notifiesCaperTeam
import chat.caper.android.data.visitTitle
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
import kotlin.math.cos
import kotlin.math.sin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** First character of a name, whole even when it is an emoji (a surrogate pair). */
internal fun initialOf(name: String): String {
    val trimmed = name.trim()
    return if (trimmed.isEmpty()) "" else trimmed.substring(0, trimmed.offsetByCodePoints(0, 1)).uppercase()
}

class MainActivity : ComponentActivity() {
    private val viewModel: CaperViewModel by viewModels()
    private val dailyBranding by lazy { DailyBrandingAvatar(applicationContext) }
    private var brandingJob: Job? = null
    private var brandingAvatar by mutableStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Edge to edge on every API level, as Android 15+ already enforces: the root pads for the
        // system bars and the keyboard itself, with light bar icons over the dark app.
        val bars = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
        enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
        CaperEffects.init(applicationContext)
        // A recreated activity or a relaunch from Recents must not reopen an old tap or link.
        if (savedInstanceState == null) openFromIntent(intent)
        setContent { CompositionLocalProvider(LocalBrandAvatar provides brandingAvatar) { CaperTheme { CaperApp(viewModel) } } }
    }
    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); setIntent(intent); openFromIntent(intent) }

    /** A notification tap, or an invite link from the manifest's VIEW filters. */
    private fun openFromIntent(intent: Intent) {
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        openFromNotification(intent)
        if (intent.action == Intent.ACTION_VIEW) intent.dataString?.let(viewModel::openInviteLink)
    }

    private fun openFromNotification(intent: Intent) {
        viewModel.openFromNotification(
            intent.getStringExtra(CaperNotifications.EXTRA_CONVERSATION_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_SPACE_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_CHANNEL_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_KIND),
            intent.getStringExtra(CaperNotifications.EXTRA_MESSAGE_ID),
            intent.getStringExtra(CaperNotifications.EXTRA_THREAD_ROOT_ID),
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
    data object JoinInvite : Overlay
    /** "Join with invite link" for a link opened from another app, already loading its preview. */
    data class OpenedInviteLink(val request: InviteLinkRequest) : Overlay
    data class Invitation(val space: Space) : Overlay
    data object ManageSpace : Overlay
    data class InvitePeople(val initialChannelId: String? = null) : Overlay
    data object CreateChannel : Overlay
    data object StartDirect : Overlay
    data class ManageChannel(val channel: Channel) : Overlay
    data class LeaveChannel(val channel: Channel) : Overlay
    data object LeaveSpace : Overlay
    data object Profile : Overlay
    data object Audio : Overlay
    data object Privacy : Overlay
    data class WhatsNew(val snapshot: WhatsNewSnapshot? = null, val automatic: Boolean = false) : Overlay
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
        state.screen == SessionScreen.Home && state.account?.id == accountId && currentAccountEpoch == accountEpoch &&
            // A joined channel of the open space, or the visited space channel (feedback team).
            (state.selectedSpace?.let { detail ->
                detail.space.id == spaceId && detail.space.demo == demo && detail.channels.any { it.id == channelId && it.joined }
            } == true || !demo && state.visit?.let { !it.direct && it.spaceId == spaceId && it.channelId == channelId } == true) &&
            channelId !in state.deniedVoiceChannels &&
            (freshChannelIds == null || channelId in freshChannelIds)
}

@Composable private fun CaperApp(viewModel: CaperViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val voice by VoiceCallService.state.collectAsStateWithLifecycle()
    val whatsNew by viewModel.whatsNew.state.collectAsStateWithLifecycle()
    var overlay by remember { mutableStateOf<Overlay?>(null) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val view = LocalView.current
    val workspaceReady = state.account != null && state.limits != null && !state.restoring &&
        (state.screen == SessionScreen.Home || state.screen is SessionScreen.Spaces)
    LaunchedEffect(workspaceReady, lifecycle) {
        if (!workspaceReady) return@LaunchedEffect
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            // A missing immutable snapshot may be generated after this client ships.
            do {
                viewModel.whatsNew.refresh()
                if (viewModel.whatsNew.state.value.snapshot != null) break
                delay(60_000)
            } while (isActive)
        }
    }
    LaunchedEffect(workspaceReady, lifecycle) {
        if (!workspaceReady) return@LaunchedEffect
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (isActive) {
                delay(500)
                val notes = viewModel.whatsNew.state.value
                // Native dialogs, sheets, permission prompts and focusable menus take
                // window focus, including ones owned by nested message views.
                val idle = VoiceCallService.state.value.phase.let { it == VoiceState.Phase.IDLE || it == VoiceState.Phase.FAILED }
                if (overlay == null && idle && view.hasWindowFocus() && notes.unread.isNotEmpty()) {
                    overlay = Overlay.WhatsNew(notes.snapshot?.copy(releases = notes.unread.asReversed()), automatic = true)
                }
            }
        }
    }
    LaunchedEffect(whatsNew.snapshot, overlay) {
        val shown = overlay as? Overlay.WhatsNew ?: return@LaunchedEffect
        if (shown.snapshot == null && whatsNew.snapshot != null) overlay = shown.copy(snapshot = whatsNew.snapshot)
    }
    // Web: a space removed elsewhere closes its dialogs; an invitation answered elsewhere closes its prompt.
    LaunchedEffect(state.selectedSpace?.space?.id) {
        if (overlay.let { it == Overlay.ManageSpace || it is Overlay.InvitePeople || it == Overlay.CreateChannel || it == Overlay.LeaveSpace || it is Overlay.ManageChannel || it is Overlay.LeaveChannel }) overlay = null
    }
    LaunchedEffect(state.invitations) { (overlay as? Overlay.Invitation)?.let { shown -> if (state.invitations.none { it.id == shown.space.id }) overlay = null } }
    var navigationOpen by remember { mutableStateOf(false) }
    LaunchedEffect(state.notificationInvitation, state.notificationBrowseSpace, state.selectedSpace?.space?.id) {
        state.notificationInvitation?.let { overlay = Overlay.Invitation(it); viewModel.consumeNotificationInvitation() }
        if (state.notificationBrowseSpace != null && state.notificationBrowseSpace == state.selectedSpace?.space?.id) {
            overlay = null
            navigationOpen = true
            viewModel.consumeNotificationInvitation()
        }
    }
    VoiceChimes(voice)

    // Web: closing User Settings, or a panel opened from it, returns focus to its button.
    val settingsFocus = remember { FocusRequester() }
    var settingsShown by remember { mutableStateOf(false) }
    LaunchedEffect(overlay) {
        if (settingsShown && overlay == null) runCatching { settingsFocus.requestFocus() }
        settingsShown = overlay.let { it == Overlay.Audio || it == Overlay.Privacy || it is Overlay.AudioPanelOverlay || it is Overlay.WhatsNew }
    }

    val homeVisible = state.screen == SessionScreen.Home || state.screen is SessionScreen.Spaces
    // An opened invite link (signed in, or once sign-in finishes) takes over whatever dialog is open.
    val openedInviteLink by viewModel.inviteLinkToOpen.collectAsStateWithLifecycle()
    LaunchedEffect(openedInviteLink) {
        openedInviteLink?.let { overlay = Overlay.OpenedInviteLink(it); viewModel.inviteLinkShown(it) }
    }
    Scaffold(containerColor = Blackout) { padding ->
        // The bottom inset Scaffold pads for is part of the keyboard's, so only the rest is added.
        // A finger that slides off a button, row or image never presses it, on every screen.
        Box(Modifier.fillMaxSize().swipeCancelsPresses().padding(padding).consumeWindowInsets(padding).imePadding()) {
            CompositionLocalProvider(LocalSettingsFocus provides settingsFocus) { when (val screen = state.screen) {
                SessionScreen.Loading -> BrandLoading()
                SessionScreen.SignedOut -> LoginScreen(state.busy, state.error, viewModel::clearError, viewModel::requestCode)
                is SessionScreen.Verify -> VerifyScreen(screen, state.busy, state.error, viewModel::clearError, viewModel::showLogin, viewModel::verify, viewModel::resendCode)
                is SessionScreen.Profile -> ProfileScreen(screen.account, state.busy, state.error, null, submit = viewModel::saveProfile)
                SessionScreen.Home, is SessionScreen.Spaces ->
                    // Launch stays on the brand screen until the first conversation is ready.
                    if (state.restoring) BrandLoading()
                    // Web's first-space page: an account with no spaces names one.
                    else if (state.account != null && state.limits != null && state.invitations.isEmpty() &&
                        state.spaces.none { !it.demo } && state.selectedDirectId == null && state.visit == null && !state.feedbackShown && !navigationOpen)
                        FirstSpaceScreen(state, viewModel)
                    else HomeScreen(
                        state, voice, navigationOpen, { navigationOpen = it }, { overlay = it }, viewModel,
                    )
            } }
            // Keep the current conversation stable while a space or channel opens.
            if (state.busy && !homeVisible) LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter), color = Terracotta)
            // Web's notice sits at the top centre: at the bottom it covered the composer and the
            // account bar's audio controls.
            if (homeVisible) state.error?.let { error ->
                Surface(
                    Modifier.align(Alignment.TopCenter).padding(16.dp).widthIn(max = 560.dp).semantics { liveRegion = LiveRegionMode.Polite },
                    color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), shadowElevation = 6.dp,
                ) {
                    Row(Modifier.padding(start = 14.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(error, Modifier.weight(1f, fill = false).padding(vertical = 12.dp), color = ErrorText, fontSize = 13.sp)
                        Spacer(Modifier.width(8.dp))
                        TextButton(viewModel::clearError, colors = ButtonDefaults.textButtonColors(contentColor = Text), shape = MaterialTheme.shapes.small) { Text("Dismiss") }
                    }
                }
            }
        }
    }

    when (val shown = overlay) {
        Overlay.CreateSpace -> CreateSpaceDialog(state.busy, state.error, viewModel::clearError, { overlay = null }) { viewModel.createSpace(it) { overlay = null } }
        Overlay.JoinInvite -> if (state.account != null) JoinInviteDialog(state.busy, viewModel) { overlay = null }
        // Keyed by the opening, so another link starts over even while this one is shown.
        is Overlay.OpenedInviteLink -> if (state.account != null) key(shown.request.id) {
            JoinInviteDialog(state.busy, viewModel, shown.request) { overlay = null }
        }
        is Overlay.Invitation -> InvitationDialog(shown.space, state.busy, state.error, { overlay = null },
            { viewModel.acceptInvitation(shown.space) { overlay = null } },
            { viewModel.declineInvitation(shown.space) { overlay = null } })
        Overlay.ManageSpace -> state.selectedSpace?.let { detail -> ManageSpaceDialog(state, detail, viewModel, { overlay = null }) }
        is Overlay.InvitePeople -> state.selectedSpace?.takeIf { it.space.ownerId == state.account?.id }?.let { detail ->
            key(detail.space.id, shown.initialChannelId) {
                InvitePeopleDialog(state, detail, viewModel, shown.initialChannelId, { overlay = null })
            }
        }
        Overlay.CreateChannel -> state.selectedSpace?.let { detail -> CreateChannelDialog(detail, state.busy, state.error, viewModel::clearError, { overlay = null }) { name, private ->
            // Web opens a new private channel's settings so people can be added.
            viewModel.createChannel(name, private) { created -> overlay = if (created.private) Overlay.ManageChannel(created) else null }
        } }
        Overlay.StartDirect -> StartDirectDialog(state.busy, directCandidates(state.people, state.account?.id ?: state.chatAuthorId), viewModel::refreshPeople, { overlay = null }, viewModel::lookupDirectPerson) { username, failed ->
            viewModel.startDirect(username, done = { overlay = null }, failed = failed)
        }
        is Overlay.ManageChannel -> ManageChannelDialog(state, state.selectedSpace?.channels?.find { it.id == shown.channel.id } ?: shown.channel, viewModel) { overlay = null }
        is Overlay.LeaveChannel -> {
            val owner = state.selectedSpace?.space?.ownerId == state.account?.id
            val privateLoss = shown.channel.private && !owner
            ConfirmDialog("Leave #${shown.channel.name}?", if (privateLoss) "You will lose access to this private channel. Another invitation is required to return." else "You can continue to preview this channel and join it again later.", "Leave channel", state.busy, { overlay = null }, pendingLabel = "Leaving…") {
                viewModel.leaveChannel(shown.channel); overlay = null
            }
        }
        Overlay.LeaveSpace -> ConfirmDialog("Leave ${state.selectedSpace?.space?.name}?", "You will lose access to its channels and conversations. An owner can add you again later.", "Leave space", state.busy, { overlay = null }, pendingLabel = "Leaving…", error = state.error) { viewModel.leaveCurrentSpace { overlay = null } }
        Overlay.Profile -> state.account?.let { account ->
            ProfileScreen(account, state.busy, state.error, { overlay = null }, picture = { ProfilePictureSection(state, viewModel) }) { username, display ->
                viewModel.updateProfile(username, display) { overlay = null }
            }
        }
        Overlay.Audio -> AudioSettingsMenu(state, voice, { overlay = null }, { overlay = Overlay.AudioPanelOverlay(it) }, viewModel::logout, viewModel::showLogin,
            notifications = { NotificationSettingsSection(state, viewModel) },
            openWhatsNew = { overlay = Overlay.WhatsNew(); viewModel.refreshWhatsNew() }) { overlay = Overlay.Privacy }
        is Overlay.WhatsNew -> {
            DisposableEffect(shown.snapshot) { onDispose { viewModel.whatsNew.dismiss(shown.snapshot?.sequence) } }
            WhatsNewDialog(whatsNew, shown.snapshot, shown.automatic, viewModel::refreshWhatsNew) {
                viewModel.whatsNew.dismiss(shown.snapshot?.sequence)
                overlay = null
            }
        }
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
        Text("Loading your spaces…", color = TextMuted, fontSize = 12.sp,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
    }
}

private val LocalBrandAvatar = compositionLocalOf { 0 }
/** The account bar's User Settings button, which takes focus back when settings close. */
private val LocalSettingsFocus = staticCompositionLocalOf<FocusRequester?> { null }

@Composable private fun Wordmark(modifier: Modifier = Modifier) = Image(
    painterResource(R.drawable.caper_wordmark), null,
    modifier.size(width = 132.dp, height = 35.dp).clearAndSetSemantics { contentDescription = "Caper" },
)

@Composable private fun HomeScreen(
    state: AppUiState,
    voice: VoiceState,
    navigationOpen: Boolean,
    setNavigationOpen: (Boolean) -> Unit,
    show: (Overlay) -> Unit,
    viewModel: CaperViewModel,
) {
    var channelsExpanded by rememberSaveable { mutableStateOf(true) }
    // Browse channels (every channel, with search) or only joined ones; the empty state can open it too.
    var browsing by remember(state.selectedSpace?.space?.id) { mutableStateOf(false) }
    val conversationState = rememberSaveableStateHolder()
    // Outside per-conversation saved state and transient thread panels: picker results
    // return to their original channel/thread, including after navigation or resizing.
    var pickingChannel by rememberSaveable { mutableStateOf<String?>(null) }
    var pickingThread by rememberSaveable { mutableStateOf<String?>(null) }
    var pickingEpoch by rememberSaveable { mutableLongStateOf(0L) }
    fun picked(uris: List<android.net.Uri>) {
        pickingChannel?.let { viewModel.addAttachments(uris, AttachmentDraftTarget(it, pickingThread), pickingEpoch) }
        pickingChannel = null; pickingThread = null
    }
    val media = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(AttachmentPolicy.MAX_ATTACHMENTS), ::picked)
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments(), ::picked)
    fun pickFiles(target: AttachmentDraftTarget, photos: Boolean) {
        pickingChannel = target.channelId; pickingThread = target.threadRootId; pickingEpoch = viewModel.accountEpoch
        runCatching {
            if (photos) media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
            else documents.launch(arrayOf("*/*"))
        }.onFailure { pickingChannel = null; pickingThread = null }
    }
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
        // The open space's joined channel, or a visited space channel (feedback team).
        val visited = state.visit?.takeIf { !it.direct && it.channelId == channel.id }
        val space = state.selectedSpace?.space
        val spaceId = if (visited != null) visited.spaceId else space?.id
        val spaceName = if (visited != null) visited.spaceName.orEmpty() else space?.name
        val demo = visited == null && space?.demo == true
        val allowed = visited != null || channel.joined && state.selectedSpace?.channels?.any { it.id == channel.id && it.joined } == true
        if (pendingVoiceJoin == null && voice.phase != VoiceState.Phase.CONNECTING && voice.phase != VoiceState.Phase.RECONNECTING &&
            BuildConfig.ENABLE_NATIVE_VOICE && spaceId != null && spaceName != null && state.voiceAvailable(channel) == true &&
            allowed && channel.id !in state.deniedVoiceChannels) {
            voicePermissionError = null
            pendingVoiceJoin = VoiceJoinIntent(channel.id, spaceId, channel.name, spaceName,
                state.account?.displayName ?: "Guest", state.account?.id, viewModel.accountEpoch, demo,
                VoiceCallService.beginJoinAuthorization())
            permission.launch(buildList {
                add(Manifest.permission.RECORD_AUDIO)
                if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            }.toTypedArray())
        }
    }
    // Your call card's Leave, or Cancel while joining.
    val leaveVoice: () -> Unit = {
        if (voice.phase == VoiceState.Phase.CONNECTED) CaperEffects.play(CaperEffects.Effect.Disconnect)
        VoiceCallService.stop(context)
    }
    Column(Modifier.fillMaxSize()) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val narrow = maxWidth <= 760.dp
            LaunchedEffect(narrow, navigationOpen) { viewModel.setConversationVisible(!narrow || !navigationOpen) }
            // 60 rail + 280 channels + 320 minimum chat + 220 members.
            val medium = maxWidth < 880.dp
            var membersVisible by remember { mutableStateOf(!narrow) }
            LaunchedEffect(narrow) { if (narrow) membersVisible = false }
            // The member list only shows in a joined channel; elsewhere Back must reach the system.
            val membersShown = membersVisible && state.selectedChannel?.joined == true && state.selectedDirectId == null && state.visit == null
            BackHandler(enabled = narrow && (membersShown || navigationOpen)) {
                if (membersShown && !navigationOpen) membersVisible = false else setNavigationOpen(false)
            }
            // Web's empty state "Browse channels": the sidebar lists every channel, in the drawer on phones.
            val browseChannels: () -> Unit = { browsing = true; channelsExpanded = true; if (narrow) setNavigationOpen(true) }
            Surface(
                Modifier.fillMaxSize(),
                color = Surface,
            ) {
                if (narrow) {
                    val drawer = rememberSlideDrawer(navigationOpen)
                    val latestSetNavigationOpen by rememberUpdatedState(setNavigationOpen)
                    LaunchedEffect(navigationOpen) { if (!drawer.dragging) drawer.settle(navigationOpen) }
                    // Browse is composed only while it shows or the conversation moves.
                    val browseUncovered by remember { derivedStateOf { drawer.value > 0f } }
                    // The member list slides over the conversation the same way, from the trailing edge.
                    val members = rememberSlideDrawer(membersShown)
                    // Browse and the member list cover the composer: its focus and keyboard go with it.
                    val focusManager = LocalFocusManager.current
                    LaunchedEffect(navigationOpen, membersShown) { if (navigationOpen || membersShown) focusManager.clearFocus() }
                    LaunchedEffect(membersShown) { if (!members.dragging) members.settle(membersShown) }
                    val membersInView by remember { derivedStateOf { members.value > 0f } }
                    CompositionLocalProvider(LocalBrowseDrawer provides drawer, LocalMembersDrawer provides members) {
                        Box(Modifier.fillMaxSize().clipToBounds().background(Blackout).browseDrag(drawer) { latestSetNavigationOpen(it) }) {
                            // Browse, account bar included, stays still beneath the conversation, which slides off to reveal it.
                            if (navigationOpen || browseUncovered) Column(Modifier.fillMaxSize().then(if (navigationOpen) Modifier else Modifier.clearAndSetSemantics {})) {
                                Row(Modifier.weight(1f).browseSwipeRegion(drawer, enabled = state.selectedChannel != null || state.selectedDirectId != null || state.feedbackShown)) {
                                    SpaceRail(state, voice, viewModel, show, Modifier.width(60.dp))
                                    ChannelSidebar(state, voice, viewModel, show,
                                        Modifier.weight(1f).padding(top = 8.dp, end = 8.dp).clip(RoundedCornerShape(16.dp)),
                                        channelsExpanded, { channelsExpanded = it }, browsing, { browsing = it }, pendingVoiceJoin, voicePermissionError,
                                        { voicePermissionError = null }, joinVoice, leaveVoice, showAccountBar = false) { setNavigationOpen(false) }
                                }
                                // The full width, below the rail too.
                                AccountBar(state, voice, viewModel, show, narrow = true)
                            }
                            Box(Modifier.fillMaxSize().conversationLayer(drawer).background(Surface)
                                .browseDrag(members, direction = -1f,
                                    // Known before the panel is first composed, so the first swipe follows the finger too.
                                    travel = { width -> membersTravel(width, MEMBERS_MAX_WIDTH.toPx(), MEMBERS_END.toPx(), MEMBERS_MARGIN.toPx()) }) { membersVisible = it }
                                .then(if (navigationOpen) Modifier.clearAndSetSemantics {} else Modifier)) {
                                conversationState.SaveableStateProvider(state.selectedChannel?.id ?: if (state.feedbackShown) "feedback" else "empty") {
                                    Conversation(state, voice, viewModel, show, true, membersVisible, { membersVisible = !membersVisible }, voicePermissionError,
                                        { voicePermissionError = null }, joinVoice, pendingVoiceJoin, ::pickFiles, browseChannels) { setNavigationOpen(true) }
                                }
                                // Composed while shown or sliding; it moves with the conversation over Browse.
                                if (membersShown || membersInView) {
                                    Box(Modifier.fillMaxSize().padding(top = 54.dp).graphicsLayer { alpha = members.value }
                                        .background(Color.Black.copy(alpha = 0.25f))
                                        .browseSwipeRegion(members, enabled = membersShown)
                                        .then(if (membersShown) Modifier.clickable(
                                            interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() },
                                            indication = null,
                                        ) { membersVisible = false } else Modifier))
                                    MemberPresencePanel(state, viewModel,
                                        Modifier.padding(top = 62.dp, end = MEMBERS_END, bottom = 8.dp).widthIn(max = MEMBERS_MAX_WIDTH).fillMaxHeight().align(Alignment.CenterEnd)
                                            .membersLayer(members)
                                            .border(1.dp, Border, RoundedCornerShape(16.dp))
                                            .browseSwipeRegion(members, enabled = membersShown)
                                            .pointerInput(Unit) { detectTapGestures {} }
                                            .then(if (membersShown) Modifier else Modifier.clearAndSetSemantics {}),
                                        close = { membersVisible = false })
                                }
                            }
                        }
                    }
                } else Row {
                    SpaceRail(state, voice, viewModel, show, Modifier.width(60.dp))
                    ChannelSidebar(state, voice, viewModel, show, Modifier.width(280.dp), channelsExpanded, { channelsExpanded = it }, browsing, { browsing = it }, pendingVoiceJoin, voicePermissionError, { voicePermissionError = null },
                        joinVoice, leaveVoice)
                    if (medium) Box(Modifier.weight(1f).fillMaxHeight()) {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, voicePermissionError,
                            { voicePermissionError = null }, joinVoice, pendingVoiceJoin, ::pickFiles, browseChannels, Modifier.fillMaxSize()) { setNavigationOpen(true) }
                        if (membersVisible && state.selectedChannel?.joined == true && state.selectedDirectId == null && state.visit == null) MemberPresencePanel(state, viewModel, Modifier.padding(top = 54.dp).width(220.dp).fillMaxHeight().align(Alignment.CenterEnd), close = { membersVisible = false })
                    } else {
                        Conversation(state, voice, viewModel, show, false, membersVisible, { membersVisible = !membersVisible }, voicePermissionError,
                            { voicePermissionError = null }, joinVoice, pendingVoiceJoin, ::pickFiles, browseChannels, Modifier.weight(1f)) { setNavigationOpen(true) }
                        if (membersVisible && state.selectedChannel?.joined == true && state.selectedDirectId == null && state.visit == null) MemberPresencePanel(state, viewModel, Modifier.width(220.dp).fillMaxHeight(), close = { membersVisible = false })
                    }
                }
            }
        }
    }
}

@Composable private fun SpaceRail(state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit, modifier: Modifier = Modifier) {
    val now = rememberMuteClock(state.notificationSettings)
    Column(modifier.fillMaxHeight().background(Blackout).verticalScroll(rememberScrollState()).padding(vertical = 14.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        state.spaces.forEach { space ->
            val selected = state.selectedSpace?.space?.id == space.id
            val muted = state.notificationSettings?.spaceMuted(space.id, now) == true
            val unread = state.unreadSpaces.any { it.id == space.id }
            val connected = state.account != null && voice.phase == VoiceState.Phase.CONNECTED && voice.spaceId == space.id
            val voiceActive = state.account != null && (connected || space.id in state.voiceActivitySpaceIds)
            val interaction = remember(space.id) { MutableInteractionSource() }
            val hovered by interaction.collectIsHoveredAsState()
            // Compose's animation clock honors the system animator duration scale.
            val indicatorHeight by animateFloatAsState(if (selected) 32f else if (hovered) 20f else 8f, tween(140), label = "Space indicator")
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.width(4.dp).height(indicatorHeight.dp).background(Text, RoundedCornerShape(topEnd = 4.dp, bottomEnd = 4.dp)))
                Spacer(Modifier.width(2.dp))
                Box {
                    Surface(
                        Modifier.size(48.dp).alpha(if (muted) MUTED_ALPHA else 1f).clip(MaterialTheme.shapes.medium).hoverable(interaction).clickable(interactionSource = interaction, indication = null) { viewModel.selectSpace(space.id) }
                            .semantics { contentDescription = space.name; stateDescription = listOfNotNull(if (unread) "Unread activity" else null, if (muted) "Muted" else null, if (connected) "Connected to voice" else if (voiceActive) "Voice active" else null).joinToString(", ") },
                        color = Surface, shape = MaterialTheme.shapes.medium,
                    ) { SpaceTileFace(space.name, space.iconAsset, space.iconUrl, 48.dp, TextMuted) }
                    if (muted) Box(Modifier.align(Alignment.BottomEnd).offset(x = 3.dp, y = 3.dp).background(Blackout, CircleShape).padding(2.dp).clearAndSetSemantics {}) { MutedBell() }
                    if (voiceActive) Box(Modifier.align(Alignment.TopEnd).offset(x = 3.dp, y = (-3).dp).size(18.dp).background(if (connected) VoiceSessionGreen else Blackout, CircleShape).border(2.dp, Blackout, CircleShape).clearAndSetSemantics {}, contentAlignment = Alignment.Center) {
                        Icon(painterResource(R.drawable.lucide_volume_2), null, Modifier.size(11.dp), tint = Text)
                    }
                    if (unread) Box(Modifier.align(Alignment.TopStart).offset(x = (-2).dp, y = (-2).dp).size(10.dp).background(Blackout, CircleShape).padding(2.dp).background(Text, CircleShape).clearAndSetSemantics {})
                }
            }
        }
        state.invitations.forEach { invitation ->
            Surface(
                // The same 48dp tile (and touch target) as the spaces above it.
                Modifier.size(48.dp).clip(MaterialTheme.shapes.medium).clickable { show(Overlay.Invitation(invitation)) }.semantics {
                    contentDescription = "Invitation to ${invitation.name}"
                },
                color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, TerracottaBorder),
            ) { SpaceTileFace(invitation.name, invitation.iconAsset, invitation.iconUrl, 48.dp, TerracottaBright) }
        }
        val limits = state.limits
        val canCreateSpace = limits != null && state.spaces.count { it.ownerId == state.account?.id } < limits.ownedSpaces &&
            state.spaces.count { !it.demo } < limits.totalSpaces
        val spaceEnabled = state.account == null || canCreateSpace
        Surface(
            Modifier.size(48.dp).clip(MaterialTheme.shapes.medium).clickable(enabled = spaceEnabled) { if (state.account == null) viewModel.showLogin() else show(Overlay.CreateSpace) }.semantics {
                contentDescription = "Create space"
                // Web's tooltip; Android has no hover, so it is the state description.
                if (!spaceEnabled) stateDescription = "Space limit reached (${limits?.ownedSpaces ?: 20} owned, ${limits?.totalSpaces ?: 100} total)"
            },
            color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, TerracottaBorder),
        ) { Box(contentAlignment = Alignment.Center) { Icon(painterResource(R.drawable.lucide_plus), null, tint = if (spaceEnabled) TerracottaBright else TerracottaBright.copy(alpha = 0.4f)) } }
        // Joining by link stays open at the owned-space limit; the server enforces the membership limit.
        if (state.account != null) Surface(
            Modifier.size(48.dp).clip(MaterialTheme.shapes.medium).clickable { show(Overlay.JoinInvite) }.semantics { contentDescription = "Join with invite link" },
            color = Surface, shape = MaterialTheme.shapes.medium, border = BorderStroke(1.dp, Border),
        ) { Box(contentAlignment = Alignment.Center) { Icon(painterResource(R.drawable.lucide_link), null, Modifier.size(20.dp), tint = TextMuted) } }
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
    browsing: Boolean,
    setBrowsing: (Boolean) -> Unit,
    pendingVoiceJoin: VoiceJoinIntent?,
    voicePermissionError: String?,
    dismissVoicePermissionError: () -> Unit,
    joinVoice: (Channel) -> Unit,
    leaveVoice: () -> Unit,
    showAccountBar: Boolean = true,
    /** Phones only: Browse closes once a conversation opens. */
    closeNavigation: (() -> Unit)? = null,
) {
    val settings = state.notificationSettings
    val now = rememberMuteClock(settings)
    val detail = state.selectedSpace
    val owner = state.account != null && state.account.id == detail?.space?.ownerId
    val channelCount = detail?.channels?.size ?: 0
    val canCreateChannel = detail != null && state.limits?.let { channelCount < it.channelsPerSpace } == true
    var channelMenuOpen by remember(detail?.space?.id) { mutableStateOf(false) }
    var channelQuery by remember(detail?.space?.id) { mutableStateOf("") }
    // Keep the two long lists independently reachable.  Persist a proportion rather
    // than pixels so a rotation or freeform-window resize retains the intent.
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("layout", android.content.Context.MODE_PRIVATE) }
    var channelsShare by rememberSaveable(detail?.space?.id) {
        mutableFloatStateOf(preferences.getFloat("channelSidebarChannelsShare", 0.5f).takeIf { it.isFinite() && it > 0f && it < 1f } ?: 0.5f)
    }
    // Your call's row becomes its call card when its channel is listed here.
    val callRowId = callRowChannelId(detail, voice)
    Column(modifier.fillMaxHeight().background(SurfaceSidebar)) {
        // Web: the space name and its menu stay put while a long channel list scrolls.
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp)) {
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
                    DropdownMenu(spaceMenuOpen, closeSpaceMenu, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                        ScopeMenuContent(
                            spaceMenuPage, { spaceMenuPage = it }, "space", settings?.override(spaceKey(detail.space.id)), settings?.inheritedLevel(),
                            { viewModel.setSpaceNotifications(detail.space.id, it) }, closeSpaceMenu,
                            saving = spaceKey(detail.space.id) in state.notificationSaving,
                            before = {
                                DropdownMenuItem(
                                    text = { Text(if (browsing) "Joined channels" else "Browse channels") },
                                    onClick = {
                                        setBrowsing(!browsing)
                                        channelQuery = ""
                                        closeSpaceMenu()
                                    },
                                    leadingIcon = { Icon(painterResource(R.drawable.lucide_hash), null, Modifier.size(16.dp)) },
                                )
                            },
                            after = {
                                if (owner) DropdownMenuItem({ Text("Invite people…") }, { closeSpaceMenu(); show(Overlay.InvitePeople()) })
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
        }
        BoxWithConstraints(Modifier.weight(1f)) {
            // On a short window, reserve no more than a third for either list;
            // otherwise each starts with a useful 96dp of visible rows.
            val available = (maxHeight - if (state.account == null) 0.dp else 24.dp).coerceAtLeast(0.dp)
            val minimum = minOf(96.dp, available / 3)
            val minimumShare = if (available > 0.dp) (minimum / available).coerceIn(0f, 0.5f) else 1f / 3
            val clampedShare = channelsShare.coerceIn(minimumShare, 1f - minimumShare)
            val density = LocalDensity.current
            val resizeFocus = remember { FocusRequester() }
            fun saveShare() { preferences.edit().putFloat("channelSidebarChannelsShare", channelsShare).apply() }
            fun resizeShare(value: Float) { channelsShare = value.coerceIn(minimumShare, 1f - minimumShare); saveShare() }
            val dragState = rememberDraggableState { delta ->
                val heightPx = with(density) { available.toPx() }
                if (heightPx > 0f) {
                    channelsShare = (clampedShare + delta / heightPx).coerceIn(minimumShare, 1f - minimumShare)
                }
            }
            Column(Modifier.fillMaxSize()) {
            Column(Modifier.weight(if (state.account == null) 1f else clampedShare).verticalScroll(rememberScrollState())) {
                Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp)) {
            // Voice with no row in this list (your call elsewhere, a visited channel's room) sits above
            // Channels; listed channels show theirs in place. Joining keeps the chat being read.
            LiveNowSection(
                state, voice, pendingVoiceJoin, narrow = closeNavigation != null, joinVoice = joinVoice, prepareVoice = viewModel::prepareVoiceJoin,
                // A visited channel's room is the conversation already open.
                openChannel = { if (state.visit?.channelId != it.id) viewModel.selectChannel(it); closeNavigation?.invoke() },
                openCall = { viewModel.openVoiceChannel(voice) { closeNavigation?.invoke() } },
                leaveVoice = leaveVoice,
            )
            if (browsing) OutlinedTextField(channelQuery, { channelQuery = it }, Modifier.fillMaxWidth().padding(top = 8.dp).browseSwipeExcluded(LocalBrowseDrawer.current), singleLine = true, label = { Text("Search channels") })
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
                    DisclosureChevron(channelsExpanded, Modifier.size(18.dp))
                    Text("Channels", color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                    Text((detail?.channels?.count { it.joined } ?: 0).toString(), color = TextMuted, fontSize = 10.sp, style = TabularNumbers)
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
                            // Every item has an icon, so the labels line up (web).
                            DropdownMenuItem(
                                text = { Text(if (channelsExpanded) "Collapse channels" else "Expand channels", fontSize = 13.sp) },
                                leadingIcon = { Icon(painterResource(if (channelsExpanded) R.drawable.lucide_chevrons_down_up else R.drawable.lucide_chevrons_up_down), null) },
                                onClick = { channelMenuOpen = false; setChannelsExpanded(!channelsExpanded) },
                            )
                        }
                    }
                }
            }
            // People often type the name as it is shown, with its `#` (web).
            val channelSearch = channelQuery.trim().trimStart('#').trim()
            val voiceOf = { channel: Channel ->
                if (BuildConfig.ENABLE_NATIVE_VOICE && detail != null) rowVoice(channel, detail.space.demo, state.voiceRosters, callRowId) else RowVoice.QUIET
            }
            // Rows with voice stay listed, in list order, while collapsing or searching hides quiet rows.
            detail?.channels?.let { channels ->
                listedChannels(channels, { channelsExpanded && (browsing || it.joined) && it.name.contains(channelSearch, ignoreCase = true) }, voiceOf)
            }?.forEach { channel ->
                key(channel.id) {
                    val selected = channel.id == state.selectedChannel?.id
                    val kind = voiceOf(channel)
                    val people = if (kind == RowVoice.CALL) voice.participants else state.voiceRosters[voiceRootKey(detail.space.demo, channel.id)].orEmpty()
                    val channelVoice = channel.joined && BuildConfig.ENABLE_NATIVE_VOICE
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
                    val openChannel: () -> Unit = { viewModel.selectChannel(channel); closeNavigation?.invoke() }
                    // The channel's row. In a voice card the whole card opens the chat, so the row itself doesn't.
                    val row: @Composable () -> Unit = {
                        Row(
                            Modifier.fillMaxWidth().height(48.dp).clip(MaterialTheme.shapes.small)
                                .background(if (selected) TerracottaWash else Color.Transparent),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            // The press starts at the row's edge; only the content is inset.
                            Row(Modifier.weight(1f).fillMaxHeight().alpha(if (channelMuted) MUTED_ALPHA else 1f)
                                .then(if (kind == RowVoice.QUIET) Modifier.clickable(onClick = openChannel) else Modifier).padding(start = 9.dp)
                                .semantics { if (channelMuted) stateDescription = "Muted" }, verticalAlignment = Alignment.CenterVertically) {
                                Icon(if (channel.private) painterResource(R.drawable.lucide_lock_keyhole) else painterResource(R.drawable.lucide_hash), null, Modifier.size(17.dp), tint = if (selected) TerracottaBright else TextMuted)
                                Spacer(Modifier.width(6.dp)); Text(channel.name, Modifier.weight(1f), color = if (selected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                if (state.unreadSpaces.any { it.channels.any { it.id == channel.id } }) Box(Modifier.padding(horizontal = 6.dp).size(6.dp).background(Text, CircleShape).semantics { contentDescription = "Unread activity" })
                                if (ownMute) MutedBell(Modifier.padding(horizontal = 6.dp))
                                // A live room's row ends with a waveform and its count, your call's with its status and timer.
                                when (kind) {
                                    RowVoice.ROOM -> LiveVoiceCount(people.size)
                                    // The ⋯ button's own inset spaces it from the menu; the name gives way first.
                                    RowVoice.CALL -> CallStatus(voice, callSessionStartedAt(state, voice), Modifier.padding(start = 6.dp))
                                    RowVoice.QUIET -> Unit
                                }
                            }
                            // Touch keeps a quiet row's one-tap join in view; a live room's card has its Join.
                            if (channelVoice && kind == RowVoice.QUIET) RowVoiceJoin(channel.name, voiceJoinAction(state, voice, pendingVoiceJoin, channel),
                                { joinVoice(channel) }) { viewModel.prepareVoiceJoin(channel) }
                            if (owner || notifiable || (channel.joined && !detail.space.demo)) Box {
                                IconButton({ channelMenuOpen = true; if (notifiable) viewModel.refreshNotificationSettings() }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "${channel.name} channel menu", Modifier.size(18.dp), tint = TextMuted) }
                                DropdownMenu(channelMenuOpen, closeChannelMenu, containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                                    val settingsItem: @Composable () -> Unit = {
                                        if (owner && !channel.private && !channel.direct) DropdownMenuItem(text = { Text("Invite people…", fontSize = 13.sp) }, onClick = { closeChannelMenu(); show(Overlay.InvitePeople(channel.id)) })
                                        if (owner) DropdownMenuItem(text = { Text("Channel settings", fontSize = 13.sp) }, onClick = { closeChannelMenu(); show(Overlay.ManageChannel(channel)) })
                                    }
                                    val leaveItem: @Composable () -> Unit = {
                                        if (channel.joined && !detail.space.demo) DropdownMenuItem(text = { Text("Leave channel", fontSize = 13.sp) }, onClick = { closeChannelMenu(); show(Overlay.LeaveChannel(channel)) }, modifier = Modifier.semantics { contentDescription = "Leave ${channel.name}" })
                                    }
                                    if (notifiable) ScopeMenuContent(
                                        channelMenuPage, { channelMenuPage = it }, "channel", settings?.override(channelKey(channel.id)), settings?.inheritedLevel(detail.space.id),
                                        { viewModel.setChannelNotifications(detail.space.id, channel.id, it) }, closeChannelMenu,
                                        saving = channelKey(channel.id) in state.notificationSaving,
                                        spaceMuted = settings?.spaceMuted(detail.space.id, now) == true, fontSize = 13.sp, before = settingsItem, after = leaveItem,
                                    ) else { settingsItem(); leaveItem() }
                                }
                            }
                        }
                    }
                    Column(Modifier.fillMaxWidth()) {
                        // In its place in the list: your call's row grows into your call card, a live room's gains who is there and Join.
                        when (kind) {
                            RowVoice.CALL -> RowCallCard(voice, state.account, narrow = closeNavigation != null, open = openChannel, leave = leaveVoice,
                                Modifier.padding(vertical = 4.dp), row = row)
                            RowVoice.ROOM -> RowRoomCard(
                                LiveRoom(channel, people), voiceJoinAction(state, voice, pendingVoiceJoin, channel),
                                state.voiceSessionStartedAt[voiceRootKey(detail.space.demo, channel.id)], open = openChannel, join = { joinVoice(channel) },
                                prepare = { viewModel.prepareVoiceJoin(channel) }, Modifier.padding(vertical = 4.dp), row = row,
                            )
                            RowVoice.QUIET -> row()
                        }
                        NotificationSaveError(state.notificationErrors[channelKey(channel.id)], Modifier.padding(start = 35.dp)) { viewModel.dismissNotificationError(channelKey(channel.id)) }
                    }
                }
            }
            detail?.channelInvitations?.forEach { invitation ->
                Surface(Modifier.fillMaxWidth().padding(top = 8.dp), color = SurfaceRaised, border = BorderStroke(1.dp, TerracottaBorder), shape = MaterialTheme.shapes.small) {
                    Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                        Text("Private invitation · #${invitation.channel.name}", fontSize = 12.sp, fontWeight = FontWeight.Bold)
                        Text("${invitation.inviter.displayName} (@${invitation.inviter.username}) invited you to private channel #${invitation.channel.name}.", color = TextMuted, fontSize = 11.sp)
                        Row { TextButton({ viewModel.declineChannelInvitation(invitation) }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Decline") }; Button({ viewModel.acceptChannelInvitation(invitation) }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Accept") } }
                    }
                }
            }
            state.openError?.let { error ->
                Row(Modifier.fillMaxWidth().padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                    TextButton(viewModel::retryOpening, shape = MaterialTheme.shapes.small) { Text("Retry opening", fontSize = 12.sp) }
                }
            }
                }
            }
            if (state.account != null) {
                // The divider is a slider for TalkBack and keyboard/accessibility
                // services, and a vertical-resize pointer target on mouse devices.
                Box(
                    Modifier.fillMaxWidth().height(24.dp)
                        .browseSwipeExcluded(LocalBrowseDrawer.current)
                        .pointerHoverIcon(PointerIcon(android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_VERTICAL_DOUBLE_ARROW)))
                        .focusRequester(resizeFocus)
                        .onPreviewKeyEvent { event ->
                            if (event.type != KeyEventType.KeyDown) false else {
                                val step = 10f / available.value.coerceAtLeast(1f)
                                val next = when (event.key) {
                                    Key.DirectionUp -> clampedShare - step
                                    Key.DirectionDown -> clampedShare + step
                                    Key.MoveHome -> minimumShare
                                    Key.MoveEnd -> 1f - minimumShare
                                    else -> null
                                }
                                if (next == null) false else { resizeShare(next); true }
                            }
                        }
                        .focusable()
                        .draggable(dragState, Orientation.Vertical, onDragStarted = { resizeFocus.requestFocus() }, onDragStopped = { saveShare() })
                        .pointerInput(minimumShare) {
                            detectTapGestures(onTap = { resizeFocus.requestFocus() }, onDoubleTap = { resizeShare(0.5f); resizeFocus.requestFocus() })
                        }
                        .semantics {
                            contentDescription = "Channels and direct messages divider"
                            stateDescription = "${(clampedShare * 100).toInt()} percent channels"
                            progressBarRangeInfo = ProgressBarRangeInfo(clampedShare, minimumShare..1f - minimumShare)
                            setProgress { resizeShare(it); true }
                            customActions = listOf(CustomAccessibilityAction("Reset channel and direct message heights") { resizeShare(0.5f); true })
                        },
                    contentAlignment = Alignment.Center,
                ) { HorizontalDivider(Modifier.fillMaxWidth().padding(horizontal = 16.dp), color = Border) }
                Column(Modifier.weight(1f - clampedShare).verticalScroll(rememberScrollState())) {
                Spacer(Modifier.height(6.dp))
                // Feedback team only: the `@feedback` inbox, beside the DMs.
                if (state.account.feedbackTeam) {
                    val feedbackSelected = state.feedbackShown || state.visit != null
                    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp).heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                        .background(if (feedbackSelected) TerracottaWash else Color.Transparent)
                        .clickable(role = Role.Button) { viewModel.openFeedbackInbox(); closeNavigation?.invoke() }
                        .semantics { if (feedbackSelected) selected = true }
                        .padding(horizontal = 9.dp),
                        verticalAlignment = Alignment.CenterVertically) {
                        Icon(painterResource(R.drawable.lucide_inbox), null, Modifier.size(17.dp), tint = if (feedbackSelected) TerracottaBright else TextMuted)
                        Spacer(Modifier.width(9.dp))
                        Text("Feedback", Modifier.weight(1f), color = if (feedbackSelected) Text else TextMuted, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
                        if (state.feedbackWaiting > 0) {
                            Text(state.feedbackWaiting.toString(),
                                Modifier.padding(start = 6.dp).background(Terracotta, CircleShape)
                                    .padding(horizontal = 6.dp, vertical = 2.dp)
                                    .clearAndSetSemantics { contentDescription = "${state.feedbackWaiting} feedback conversations waiting for a reply" },
                                color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold)
                        }
                    }
                }
                Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("Direct messages", Modifier.weight(1f), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                    IconButton({ show(Overlay.StartDirect) }) { Icon(painterResource(R.drawable.lucide_plus), "New direct message", tint = TextMuted) }
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
                    Avatar(state.account.displayName ?: state.account.username ?: "You", 20.dp, modifier = Modifier.padding(horizontal = 3.dp), avatarId = state.account.avatarId,
                        avatarAsset = state.account.avatarAsset, avatarUrl = state.account.avatarUrl)
                    Spacer(Modifier.width(9.dp))
                    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                        Text(state.account.displayName ?: state.account.username ?: "You", Modifier.weight(1f, fill = false), color = if (selfSelected || selfDirect?.let(::directUnread) == true) Text else TextMuted, fontSize = 13.sp, fontWeight = if (selfDirect?.let(::directUnread) == true) FontWeight.Bold else FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Spacer(Modifier.width(6.dp)); Text("you", color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
                    }
                    if (selfDirect?.let(::directUnread) == true)
                        DirectUnreadBadge(selfDirect.unreadCount)
                }
                val blockedIds = state.blockedIds
                mainDirects(state.directConversations).filter { it.peer.id != state.account.id }.forEach { direct ->
                    key(direct.id) {
                        val selected = state.selectedDirectId == direct.id
                        val muted = settings?.directMuted(direct.id, now) == true
                        // A muted DM shows no unread dot.
                        val unread = directUnread(direct) && !muted && direct.peer.id !in blockedIds
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
                                Avatar(direct.peer.displayName, 20.dp, modifier = Modifier.padding(horizontal = 3.dp), avatarId = direct.peer.avatarId,
                                    avatarAsset = direct.peer.avatarAsset, avatarUrl = direct.peer.avatarUrl)
                                Spacer(Modifier.width(9.dp))
                                Column(Modifier.weight(1f)) {
                                    Text(direct.peer.displayName, color = if (selected || unread) Text else TextMuted, fontSize = 13.sp, fontWeight = if (unread) FontWeight.Bold else FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    // Web: a blocked person or an unanswered request says so in place of the username.
                                    Text(when {
                                        direct.blocked || direct.peer.id in blockedIds -> "Blocked"
                                        direct.outgoing -> "Request sent"
                                        else -> "@${direct.peer.username}"
                                    }, color = TextMuted, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                                if (muted) MutedBell(Modifier.padding(start = 6.dp))
                            }
                            if (unread) DirectUnreadBadge(direct.unreadCount)
                            Box {
                                IconButton(openOptions, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_ellipsis), "${direct.peer.displayName} options", Modifier.size(18.dp), tint = TextMuted) }
                                DirectOptionsMenu(optionsOpen, { optionsOpen = false }, settings?.override(directKey(direct.id)), settings != null && directKey(direct.id) !in state.notificationSaving) {
                                    viewModel.setDirectNotifications(direct.id, it)
                                }
                            }
                        }
                        NotificationSaveError(state.notificationErrors[directKey(direct.id)], Modifier.padding(horizontal = 25.dp)) { viewModel.dismissNotificationError(directKey(direct.id)) }
                    }
                }
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).heightIn(min = 48.dp).clip(MaterialTheme.shapes.small).clickable {
                    show(Overlay.StartDirect)
                }.padding(horizontal = 9.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(painterResource(R.drawable.lucide_plus), null, Modifier.size(17.dp), tint = TextMuted)
                    Spacer(Modifier.width(9.dp))
                    Text("New message", Modifier.weight(1f), color = TextMuted, fontSize = 13.sp)
                }
                }
            }
            }
        }
        // Voice errors keep their panel above the account bar, with a dismiss button.
        (voicePermissionError ?: voice.error)?.let { error -> VoiceErrorBanner(error, dismissVoicePermissionError) }
        if (showAccountBar) AccountBar(state, voice, viewModel, show)
    }
}

/** Web's voice error row, in the sidebar above the account bar or (on phones) above the composer; dismissing clears the call's error too. */
@Composable private fun VoiceErrorBanner(error: String, dismiss: () -> Unit) {
    Surface(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp), color = SurfaceRaised, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.small) {
        Row(Modifier.padding(start = 10.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(error, Modifier.weight(1f).padding(vertical = 8.dp), color = ErrorText, fontSize = 11.sp)
            IconButton({ dismiss(); VoiceCallService.clearError() }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_x), "Dismiss voice error", Modifier.size(16.dp), tint = TextMuted) }
        }
    }
}

/** Muted spaces, channels and DMs are dimmed in the sidebar. */
private const val MUTED_ALPHA = 0.5f

/** Web's disclosure chevron: it turns between collapsed (toward the reading direction) and expanded rather than swapping icons. */
@Composable internal fun DisclosureChevron(expanded: Boolean, modifier: Modifier = Modifier) {
    val collapsed = if (LocalLayoutDirection.current == LayoutDirection.Rtl) 90f else -90f
    val turn by animateFloatAsState(if (expanded) 0f else collapsed, tween(150), label = "chevron")
    Icon(painterResource(R.drawable.lucide_chevron_down), null, modifier.graphicsLayer { rotationZ = turn }, tint = TextMuted)
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

/** Phones' line under the conversation name: "N members · M in voice", leaving out parts that don't apply. */
internal fun headerSubtitle(members: Int?, inVoice: Int): String = listOfNotNull(
    members?.let { "$it ${if (it == 1) "member" else "members"}" },
    inVoice.takeIf { it > 0 }?.let { "$it in voice" },
).joinToString(" · ")

/** Web's voice-stack faces: up to [limit] small avatars, each overlapping the one before. */
@Composable internal fun VoiceFaces(people: List<Participant>, limit: Int, size: Dp, ring: Color, speaking: Set<String> = emptySet()) {
    val shown = people.take(limit)
    if (shown.isEmpty()) return
    val step = size - 6.dp
    Box(Modifier.size(width = step * (shown.size - 1) + size, height = size).clearAndSetSemantics {}) {
        shown.forEachIndexed { index, participant ->
            key(participant.id) {
                // The ring in the surrounding surface colour separates overlapping faces.
                Avatar(participant.name, size, Modifier.offset(x = step * index),
                    avatarId = participant.avatarId, speaking = participant.id in speaking,
                    avatarAsset = participant.avatarAsset, avatarUrl = participant.avatarUrl, outline = BorderStroke(1.5.dp, ring))
            }
        }
    }
}

@Composable private fun MemberPresencePanel(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, compact: Boolean = false, close: (() -> Unit)? = null) {
    val members = state.selectedSpace?.members ?: return
    val detail = state.selectedSpace ?: return
    var profile by remember(detail.space.id) { mutableStateOf<MentionCard?>(null) }
    var query by remember(detail.space.id) { mutableStateOf("") }
    var searching by remember(detail.space.id) { mutableStateOf(false) }
    val searchFocus = remember { FocusRequester() }
    LaunchedEffect(searching) { if (searching) searchFocus.requestFocus() }
    // A private channel's readers lead the list in their own group.
    val channel = state.selectedChannel
    val groups = detail.memberGroups(channel?.id)
    val shown = detail.memberOrder(channel?.id, query)
    Column(modifier.background(SurfaceSidebar).then(if (compact) Modifier else Modifier)) {
        // The heading starts where the rows' avatars do (list and row insets, 8dp each).
        Row(Modifier.fillMaxWidth().height(54.dp).padding(start = 16.dp, end = if (close != null) 4.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Members", Modifier.padding(end = 6.dp), color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold)
            if (!detail.space.demo) Text(members.size.toString(), color = TextMuted, fontSize = 10.sp, style = TabularNumbers)
            Spacer(Modifier.weight(1f))
            if (!detail.space.demo && !searching) IconButton({ searching = true }, Modifier.size(48.dp)) {
                Icon(painterResource(R.drawable.lucide_search), "Search members", tint = TextMuted)
            }
            if (close != null) IconButton(close, Modifier.size(48.dp)) {
                Icon(painterResource(R.drawable.lucide_x), "Close member list", tint = TextMuted)
            }
        }
        HorizontalDivider(color = Border)
        if (searching) Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                BasicTextField(query, { query = it }, Modifier.weight(1f).focusRequester(searchFocus).onPreviewKeyEvent {
                    if (it.key == Key.Escape) { query = ""; searching = false; true } else false
                }, singleLine = true, textStyle = TextStyle(color = Text, fontSize = 12.sp), decorationBox = { inner ->
                    if (query.isEmpty()) Text("Name or username", color = TextMuted, fontSize = 12.sp)
                    inner()
                })
                IconButton({ query = ""; searching = false }, Modifier.size(48.dp)) { Icon(painterResource(R.drawable.lucide_x), "Clear and close member search", tint = TextMuted) }
        }
        if (detail.space.demo) Text("General is open to everyone. People in voice appear in the channel sidebar.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp, lineHeight = 16.sp)
        else if (members.isEmpty()) Text("No members to show.", Modifier.padding(16.dp), color = TextMuted, fontSize = 11.sp)
        else LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)) {
            @Composable fun MemberRow(member: Member, quiet: Boolean, ring: Color) {
                Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).clip(MaterialTheme.shapes.small)
                    .clickable(role = Role.Button, onClickLabel = "Open profile for ${member.displayName}") { profile = mentionCard(member.id, member.username, state) }
                    .padding(horizontal = 8.dp, vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box {
                        Avatar(member.displayName, 30.dp, avatarId = member.avatarId, avatarAsset = member.avatarAsset, avatarUrl = member.avatarUrl)
                        PresenceDot(state.presence[member.id], state.gateway == GatewayStatus.LIVE, ring, Modifier.align(Alignment.BottomEnd))
                    }
                    Spacer(Modifier.width(10.dp))
                    Text(member.displayName, color = if (quiet) TextMuted else Text, fontSize = 12.sp,
                        fontWeight = if (quiet) FontWeight.Medium else FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            @Composable fun GroupLabel(title: String, count: Int, primary: Boolean) {
                Row(Modifier.padding(start = 8.dp, end = 8.dp, top = 6.dp, bottom = 2.dp).semantics { heading() },
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(title, color = if (primary) Text else TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(count.toString(), color = TextMuted, fontSize = 10.sp, style = TabularNumbers)
                }
            }
            fun androidx.compose.foundation.lazy.LazyListScope.presenceSections(visible: List<Member>, quiet: Boolean, ring: Color) {
                val sections = listOf(
                    "Online" to visible.filter { state.presence[it.id] in listOf("online", "idle") },
                    "Offline" to visible.filter { state.presence[it.id] == "offline" },
                    "Status unavailable" to visible.filter { state.presence[it.id] !in listOf("online", "idle", "offline") },
                )
                sections.forEach { (title, rows) ->
                    if (rows.isNotEmpty()) {
                        item(key = "${quiet}-$title") { GroupLabel(title, rows.size, primary = false) }
                        items(rows, key = { it.id }) { member ->
                            MemberRow(member, quiet = quiet || state.presence[member.id] == "offline", ring = ring)
                        }
                    }
                }
            }
            if (groups != null) {
                val readers = groups.first.map { it.id }.toSet()
                val top = shown.filter { it.id in readers }
                val rest = shown.filter { it.id !in readers }
                // Private readers lead; the rest of the space follows, quieter.
                if (top.isNotEmpty()) {
                    item(key = "in-channel") { GroupLabel("In #${channel?.name.orEmpty()}", top.size, primary = true) }
                    presenceSections(top, quiet = false, ring = SurfaceSidebar)
                }
                if (rest.isNotEmpty()) {
                    item(key = "others") { GroupLabel("Others in this space", rest.size, primary = false) }
                    presenceSections(rest, quiet = true, ring = SurfaceSidebar)
                }
            } else {
                presenceSections(shown, quiet = false, ring = SurfaceSidebar)
            }
            if (shown.isEmpty()) item { Text("No matching members.", color = TextMuted, fontSize = 11.sp) }
        }
    }
    profile?.let { card ->
        key(card.id) {
            MentionCardSheet(card, { done, failed -> card.username?.let { username ->
                viewModel.messageMentioned(card.id, username, { profile = null; close?.invoke(); done() }, failed)
            } }, state = state, viewModel = viewModel, onMessageSent = { profile = null; close?.invoke() }) { profile = null }
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
        Avatar(name, 30.dp, avatarId = state.account?.avatarId, avatarAsset = state.account?.avatarAsset, avatarUrl = state.account?.avatarUrl)
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

@Composable private fun AccountBar(state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit, narrow: Boolean = false) {
    val context = LocalContext.current
    // Match the one-line Material composer and its bottom inset, retaining touch targets.
    Surface(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 12.dp, bottom = 16.dp), color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
        Row(Modifier.height(56.dp).padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            BoxWithConstraints(Modifier.weight(1f).fillMaxHeight().clip(MaterialTheme.shapes.small)
                .clickable { if (state.account == null) viewModel.showLogin() else show(Overlay.Profile) }
                .semantics { contentDescription = if (state.account == null) "Sign in to edit your profile" else "Edit profile for ${state.account.displayName ?: "Guest"}" }
                .padding(horizontal = 4.dp), contentAlignment = Alignment.CenterStart) {
                // Web: where the controls would leave the name a letter or two ("F…"), the
                // avatar shows alone; the button above still names the account.
                val nameFits = maxWidth >= 30.dp + 7.dp + 44.dp
                Row(verticalAlignment = Alignment.CenterVertically) {
                    AccountAvatar(state, viewModel)
                    if (nameFits) {
                        Spacer(Modifier.width(7.dp))
                        Text(state.account?.displayName ?: "Guest", maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                    }
                }
            }
            // Phones use the call card's controls and Settings → Audio test for devices/volume.
            if (!narrow && BuildConfig.ENABLE_NATIVE_VOICE) {
                // Web: mute and deafen wait while the Audio test holds the microphone.
                val monitoring = Modifier.semantics { if (voice.monitoring) stateDescription = "Stop mic test to change mute" }
                IconButton({ CaperEffects.toggle(voice.muted); VoiceCallService.toggleMute(context) }, Modifier.size(40.dp).then(monitoring), enabled = !voice.monitoring) { Icon(if (voice.muted) painterResource(R.drawable.lucide_mic_off) else painterResource(R.drawable.lucide_mic), if (voice.muted) "Unmute microphone" else "Mute microphone", Modifier.size(18.dp), tint = if (voice.muted) TerracottaBright else TextMuted) }
                AudioOptionsMenu(input = true, voice = voice)
                IconButton({ CaperEffects.toggle(voice.deafened); VoiceCallService.toggleDeafen(context) }, Modifier.size(40.dp).semantics { if (voice.monitoring) stateDescription = "Stop mic test to change deafen" }, enabled = !voice.monitoring) { Icon(if (voice.deafened) painterResource(R.drawable.lucide_volume_x) else painterResource(R.drawable.lucide_headphones), if (voice.deafened) "Undeafen audio" else "Deafen audio", Modifier.size(18.dp), tint = if (voice.deafened) TerracottaBright else TextMuted) }
                AudioOptionsMenu(input = false, voice = voice)
            }
            IconButton({ show(Overlay.Audio) }, Modifier.size(40.dp).then(LocalSettingsFocus.current?.let { Modifier.focusRequester(it) } ?: Modifier)) {
                Icon(painterResource(R.drawable.lucide_settings), "User Settings", Modifier.size(18.dp), tint = TextMuted)
            }
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
    dismissVoiceError: () -> Unit,
    joinVoice: (Channel) -> Unit,
    pendingVoiceJoin: VoiceJoinIntent?,
    pickFiles: (AttachmentDraftTarget, Boolean) -> Unit,
    /** The empty state's Browse channels. */
    browseChannels: () -> Unit,
    modifier: Modifier = Modifier,
    openNavigation: () -> Unit,
) {
    val threadDrafts = rememberSaveableStateHolder()
    val context = LocalContext.current
    val preferences = remember(context) { context.getSharedPreferences("layout", android.content.Context.MODE_PRIVATE) }
    var preferredWidth by rememberSaveable { mutableFloatStateOf(preferences.getFloat("threadWidth", 340f).coerceIn(300f, 600f)) }
    val density = LocalDensity.current.density
    val resizeFocus = remember { FocusRequester() }
    var resizeFocused by remember { mutableStateOf(false) }
    BoxWithConstraints(modifier) {
        val maximum = minOf(600f, maxWidth.value - minOf(300f, maxWidth.value / 2))
        val minimum = minOf(300f, maximum)
        val width = preferredWidth.coerceIn(minimum, maximum)
        fun saveWidth() { preferences.edit().putFloat("threadWidth", preferredWidth.coerceIn(300f, 600f)).apply() }
        fun resize(value: Float) { preferredWidth = value.coerceIn(minimum, maximum).coerceAtLeast(300f); saveWidth() }
        Row(Modifier.fillMaxSize()) {
            ChannelConversation(state, voice, viewModel, show, narrow, membersVisible, toggleMembers, voicePermissionError, dismissVoiceError,
                joinVoice, pendingVoiceJoin, pickFiles, browseChannels, Modifier.weight(1f), openNavigation)
            // The thread opens beside the conversation (web's `.chat-thread-panel`).
            if (!narrow && state.thread != null) Box(Modifier.entrance(MOTION_BASE, dx = 12.dp).width(width.dp).fillMaxHeight()) {
                threadDrafts.SaveableStateProvider("${state.selectedChannel?.id}:${state.thread.rootId}") {
                    ThreadConversation(state, viewModel, pickFiles, Modifier.fillMaxSize())
                }
                Box(Modifier.align(Alignment.CenterStart).width(12.dp).fillMaxHeight()
                    .pointerHoverIcon(PointerIcon(android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW)))
                    .drawBehind { if (resizeFocused) drawRect(Terracotta, size = Size(2.dp.toPx(), size.height)) }
                    .focusRequester(resizeFocus).onFocusChanged { resizeFocused = it.isFocused }
                    .onPreviewKeyEvent { event ->
                        if (event.type != KeyEventType.KeyDown) false else {
                            val next = when (event.key) {
                                Key.DirectionLeft -> width + 10f
                                Key.DirectionRight -> width - 10f
                                Key.MoveHome -> minimum
                                Key.MoveEnd -> maximum
                                else -> null
                            }
                            if (next == null) false else { resize(next); true }
                        }
                    }
                    .focusable()
                    .semantics {
                        contentDescription = "Thread width"
                        stateDescription = "${width.toInt()} dp"
                        progressBarRangeInfo = ProgressBarRangeInfo(width, minimum..maximum)
                        setProgress { resize(it); true }
                        customActions = listOf(CustomAccessibilityAction("Reset thread width") { resize(340f); true })
                    }
                    .draggable(rememberDraggableState { delta ->
                        preferredWidth = (preferredWidth.coerceIn(minimum, maximum) - delta / density).coerceIn(minimum, maximum).coerceAtLeast(300f)
                    }, Orientation.Horizontal, onDragStarted = { resizeFocus.requestFocus() }, onDragStopped = { saveWidth() })
                    .pointerInput(maximum) {
                        detectTapGestures(onTap = { resizeFocus.requestFocus() }, onDoubleTap = { resize(340f); resizeFocus.requestFocus() })
                    })
            }
        }
    }
    if (narrow && state.thread != null) Dialog(onDismissRequest = viewModel::closeThread,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        val dialogView = LocalView.current
        SideEffect {
            (dialogView.parent as? androidx.compose.ui.window.DialogWindowProvider)?.window?.apply {
                clearFlags(android.view.WindowManager.LayoutParams.FLAG_DIM_BEHIND)
                setBackgroundDrawableResource(android.R.color.transparent)
                setWindowAnimations(0)
            }
        }
        threadDrafts.SaveableStateProvider("${state.selectedChannel?.id}:${state.thread.rootId}") {
            // Fill before insets; the bars and keyboard area move with the thread surface.
            ThreadConversation(state, viewModel, pickFiles, Modifier.swipeCancelsPresses().entrance(MOTION_BASE, dx = 12.dp).fillMaxSize()
                .background(SurfaceConversation).systemBarsPadding().imePadding(), swipeToClose = true)
        }
    }
}

/** Drop targets retain their composer identity while compression/upload continues elsewhere. */
@Composable private fun Modifier.attachmentDrop(
    enabled: Boolean, target: AttachmentDraftTarget, viewModel: CaperViewModel, dropping: MutableState<Boolean>,
): Modifier {
    val attachable by rememberUpdatedState(enabled)
    val activity = LocalActivity.current
    val dropTarget = remember(viewModel, activity, target, dropping) {
        object : DragAndDropTarget {
            override fun onEntered(event: DragAndDropEvent) { dropping.value = true }
            override fun onExited(event: DragAndDropEvent) { dropping.value = false }
            override fun onEnded(event: DragAndDropEvent) { dropping.value = false }
            override fun onDrop(event: DragAndDropEvent): Boolean {
                dropping.value = false
                val drag = event.toAndroidDragEvent()
                val clip = drag.clipData ?: return false
                val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
                if (uris.isEmpty() || !attachable) return false
                activity?.requestDragAndDropPermissions(drag)
                viewModel.addAttachments(uris, target)
                return true
            }
        }
    }
    return dragAndDropTarget({ attachable && FileDrop.carriesFiles(it.mimeTypes()) }, dropTarget)
}

/** The row owns checkbox semantics and input; this indicator has no delayed check/color tween. */
@Composable private fun ThreadBroadcastIndicator(checked: Boolean, enabled: Boolean) {
    val colors = MaterialTheme.colorScheme
    val box = if (!enabled) colors.onSurface.copy(alpha = 0.38f) else if (checked) colors.primary else colors.onSurfaceVariant
    val check = if (enabled) colors.onPrimary else colors.surface
    androidx.compose.foundation.Canvas(Modifier.padding(12.dp).size(24.dp).padding(2.dp)) {
        val corner = androidx.compose.ui.geometry.CornerRadius(2.dp.toPx())
        if (checked) {
            drawRoundRect(box, cornerRadius = corner)
            drawPath(Path().apply {
                moveTo(size.width * 0.2f, size.height * 0.5f)
                lineTo(size.width * 0.4f, size.height * 0.7f)
                lineTo(size.width * 0.8f, size.height * 0.3f)
            }, check, style = Stroke(2.dp.toPx(), cap = StrokeCap.Square))
        } else {
            val inset = 1.dp.toPx()
            drawRoundRect(box, topLeft = Offset(inset, inset), size = Size(size.width - 2 * inset, size.height - 2 * inset),
                cornerRadius = corner, style = Stroke(2.dp.toPx()))
        }
    }
}

@Composable private fun ThreadConversation(
    state: AppUiState, viewModel: CaperViewModel, pickFiles: (AttachmentDraftTarget, Boolean) -> Unit, modifier: Modifier,
    swipeToClose: Boolean = false,
) {
    val thread = state.thread ?: return
    val channel = state.selectedChannel ?: return
    val still = reducedMotion()
    val swipeScope = rememberCoroutineScope()
    var swipeOffset by remember(channel.id, thread.rootId) { mutableFloatStateOf(0f) }
    var swipeWidth by remember { mutableIntStateOf(0) }
    var swipeAnimation by remember { mutableStateOf<Job?>(null) }
    val dragThread: (Float) -> Unit = { offset ->
        swipeAnimation?.cancel()
        swipeOffset = offset
    }
    val settleThread: (Boolean) -> Unit = { dismiss ->
        swipeAnimation?.cancel()
        swipeAnimation = swipeScope.launch {
            val destination = if (dismiss) maxOf(swipeWidth.toFloat(), swipeOffset) else 0f
            if (still) swipeOffset = destination
            else androidx.compose.animation.core.animate(swipeOffset, destination, animationSpec = tween(MOTION_FAST, easing = EaseOut)) { value, _ -> swipeOffset = value }
            if (dismiss) viewModel.closeThread()
        }
    }
    val target = AttachmentDraftTarget(channel.id, thread.rootId)
    val files = state.attachmentDrafts[target].orEmpty()
    val canAttach = state.canParticipate && state.uploadsEnabled && state.chatAuthorId != null && !thread.loading && thread.error == null &&
        !state.messagesLoading && state.messagesError == null && state.sessionError == null
    val dropping = remember(target) { mutableStateOf(false) }
    var draft by rememberSaveable(state.selectedChannel?.id, thread.rootId, stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue("")) }
    var broadcast by rememberSaveable(state.selectedChannel?.id, thread.rootId) { mutableStateOf(false) }
    val pending = state.pendingMessage?.takeIf { it.threadRootId == thread.rootId }
    // DMs are named after the other person, without a channel `#`.
    val direct = state.selectedChannel?.direct == true
    val conversation = "${if (direct) "" else "#"}${state.selectedChannel?.name.orEmpty()}"
    BackHandler { viewModel.closeThread() }
    fun send() {
        if (state.pendingMessage != null && pending == null || pending?.rejected == true || pending != null && pending.error == null) return
        if (pending != null) viewModel.send(pending.text, threadRootId = thread.rootId, broadcast = pending.broadcast)
        // "Also send to channel" applies to one reply, so it resets with the draft.
        else if (draft.text.isNotBlank() || files.isNotEmpty()) {
            val sent = draft.text
            val sentBroadcast = broadcast
            val clear = { draft = TextFieldValue(""); broadcast = false }
            if (viewModel.send(sent, threadRootId = thread.rootId, broadcast = sentBroadcast,
                unchanged = { draft.text == sent && broadcast == sentBroadcast }, accepted = clear)) clear()
        }
    }
    Box(Modifier.onSizeChanged { swipeWidth = it.width }
        .graphicsLayer { translationX = if (swipeToClose) swipeOffset else 0f }
        .then(modifier).attachmentDrop(canAttach, target, viewModel, dropping)) {
        Column(Modifier.fillMaxSize().background(SurfaceConversation).border(BorderStroke(1.dp, Border))) {
            Column(Modifier.weight(1f).fillMaxWidth().threadBackSwipe(swipeToClose, dragThread, settleThread)) {
                Row(Modifier.fillMaxWidth().height(53.dp).padding(start = 4.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(viewModel::closeThread, Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.lucide_chevron_left), if (direct) "Back to conversation" else "Back to channel", Modifier.size(20.dp), tint = TextMuted)
                    }
                    Text("Thread · $conversation", Modifier.weight(1f), fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                HorizontalDivider(color = Border)
                thread.error?.let { Text(it, Modifier.padding(12.dp), color = ErrorText); TextButton({ viewModel.loadThread(older = thread.failedOlder, newer = thread.failedNewer) }, shape = MaterialTheme.shapes.small) { Text("Retry") } }
                if (thread.hasMore) HistoryRow { HistoryButton("Load older replies", !thread.loading) { viewModel.loadThread(older = true) } }
                // Remembered: every keystroke in the reply draft recomposes this panel.
                val rows = remember(state.messages, thread.rootId, thread.windowStart, thread.windowEnd) {
                    state.messages.filter { it.id == thread.rootId || it.threadRootId == thread.rootId &&
                        (thread.windowStart == null || it.seq.toBigInteger() >= thread.windowStart.toBigInteger()) &&
                        (thread.windowEnd == null || it.seq.toBigInteger() <= thread.windowEnd.toBigInteger()) }
                }
                MessageTimeline(state.copy(messages = rows, pendingMessage = pending, messagesLoading = false, messagesError = null), viewModel, Modifier.weight(1f), inThread = true) {
                    pending?.error?.let { error ->
                        Text(error, color = ErrorText)
                        if (pending.rejected) Row {
                            TextButton({
                                viewModel.discardPending(restoreFiles = true)?.let {
                                    draft = TextFieldValue(it, TextRange(it.length)); broadcast = pending.broadcast
                                }
                            }, enabled = canEditRejectedMessage(draft.text, pending.text), shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Edit") }
                            TextButton({ viewModel.discardPending() }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Dismiss") }
                        } else TextButton(::send, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Retry send") }
                    }
                }
                if (thread.hasNewer) HistoryRow { HistoryButton("Load newer replies", !thread.loading) { viewModel.loadThread(newer = true) } }
                // A failed load shows its error and Retry instead (web).
                if (!thread.loading && thread.error == null && rows.none { it.threadRootId == thread.rootId })
                    Text(if (state.canParticipate) "No replies yet. Start the thread." else "No replies yet.", Modifier.padding(18.dp), color = TextMuted)
            }
            if (state.canParticipate) Column(Modifier.padding(start = 12.dp, end = 12.dp, top = 12.dp, bottom = 16.dp)) {
                state.replyTargets[thread.rootId]?.let { ReplyDraft(it, state) { viewModel.cancelReply(thread.rootId) } }
                state.attachmentErrors[target]?.let { Text(it, Modifier.padding(bottom = 6.dp), color = ErrorText, fontSize = 12.sp) }
                DraftAttachmentStrip(files, viewModel::removeDraft, viewModel::retryDraft, edit = viewModel::editDraft, toggleSpoiler = viewModel::toggleDraftSpoiler)
                // A voice reply goes on its own; the draft text stays, "Also send to" applies to it and resets.
                // Stopping stages the recording with the reply's files; with "send as soon as I stop" it goes on its own.
                val recorder = rememberVoiceRecorder(target, rememberVoiceStop(
                    stage = { viewModel.stageVoice(it.file, it.durationMs, it.waveform, target) },
                    send = {
                        viewModel.sendVoice(it.file, it.durationMs, it.waveform, threadRootId = thread.rootId, broadcast = broadcast)
                        broadcast = false
                    },
                ))
                val voiceBlocked = voiceRecordBlocked(state.pendingMessage != null, stagedVoice = files.any { it.voice })
                val broadcastChecked = pending?.broadcast ?: broadcast
                // The label toggles too, as one checkbox control.
                Row(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small)
                    .toggleable(value = broadcastChecked, enabled = pending == null, role = Role.Checkbox, onValueChange = { broadcast = it }),
                    verticalAlignment = Alignment.CenterVertically) {
                    // Padded to the 48dp a checkbox with its own click handler would have.
                    // Material Checkbox animates its stroke and colors; this reply option responds immediately.
                    if (swipeToClose) ThreadBroadcastIndicator(broadcastChecked, enabled = pending == null)
                    else Checkbox(broadcastChecked, onCheckedChange = null, modifier = Modifier.padding(12.dp), enabled = pending == null)
                    Text(buildAnnotatedString {
                        append("Also send to ")
                        withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(conversation) }
                    }, Modifier.weight(1f), fontSize = 12.sp)
                }
                VoiceRecordingNotice(recorder)
                SuggestingComposer(draft, { draft = it }, mentionSource(state), thread.rootId, ::send,
                    focusRequest = state.composerFocusRequest?.takeIf { it.channelId == state.selectedChannel?.id && it.rootId == thread.rootId },
                    focusEnabled = !thread.loading, consumeFocus = viewModel::consumeComposerFocus) { field, change, actions ->
                    Box(Modifier.voiceRecordingKeys(recorder)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                            EmojiTextField(draft, change, Modifier.weight(1f).attachmentPaste(canAttach) { viewModel.addAttachments(it, target) }.then(field), placeholder = { Text("Reply to thread…", maxLines = 1, overflow = TextOverflow.Ellipsis) }, maxLines = 5,
                                keyboardAttachment = if (canAttach) { uri -> viewModel.addAttachments(listOf(uri), target).singleOrNull() } else null,
                                visualTransformation = MentionVisualTransformation(mentionSource(state)),
                                leadingIcon = {
                                    AttachButton(attachDisabledReason(state, target), { viewModel.addAttachments(it, target) }, pick = { pickFiles(target, it) },
                                        drawer = AttachDrawerHost(files, viewModel::removeDraft,
                                            canSend = !thread.loading && state.chatAuthorId != null && state.pendingMessage == null, send = ::send))
                                },
                                trailingIcon = {
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        // Sent at once as its own reply; the draft stays, and "Also send to" applies like a reply's.
                                        EmojiInsertButton(!thread.loading, load = viewModel::klipyPage, mediaAvailable = state.klipy != null,
                                            sendMedia = { gif, kind, query ->
                                                !thread.loading && state.chatAuthorId != null && state.pendingMessage == null &&
                                                    viewModel.sendGif(gif, kind, query, thread.rootId, broadcast).also { if (it) broadcast = false }
                                            }) { emoji -> insertEmojiAtSelection(draft, emoji)?.let { draft = it } }
                                        if (state.uploadsEnabled) VoiceRecordButton(recorder, canAttach, voiceBlocked)
                                    }
                                },
                                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Send), keyboardActions = actions, enabled = !thread.loading, colors = composerFieldColors())
                            // Web's eggshell Send icon button, like the channel composer's; disabled, it steps back to neutral chrome.
                            FilledIconButton(
                                ::send, modifier = Modifier.padding(bottom = 4.dp).size(48.dp).semantics { contentDescription = "Send reply" },
                                enabled = !thread.loading && state.chatAuthorId != null && state.pendingMessage == null && (draft.text.isNotBlank() || files.isNotEmpty()),
                                shape = MaterialTheme.shapes.small,
                                colors = IconButtonDefaults.filledIconButtonColors(
                                    containerColor = Action, contentColor = OnAction,
                                    disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
                                ),
                            ) { Icon(painterResource(R.drawable.lucide_send), null, Modifier.size(20.dp)) }
                        }
                        if (recorder.recording) VoiceRecordingBar(recorder, Modifier.matchParentSize())
                    }
                }
                DraftCounter(draft.text)
                if (state.pendingMessage?.error != null && pending == null) Text("Confirm or dismiss the pending message first.", color = TextMuted)
            } else Text(when {
                state.selectedDirect?.incoming == true -> "Accept the request to reply."
                direct -> "You can’t reply in this conversation."
                else -> "Join the channel to reply."
            }, Modifier.padding(18.dp), color = TextMuted)
        }
        if (dropping.value && canAttach) {
            val (_, detail) = FileDrop.text(channel.name, channel.direct, files.size)
            FileDropOverlay("Attach to thread", detail, caperBrandingResources[LocalBrandAvatar.current])
        }
    }
}

@Composable private fun ChannelConversation(
    state: AppUiState, voice: VoiceState, viewModel: CaperViewModel, show: (Overlay) -> Unit,
    narrow: Boolean, membersVisible: Boolean, toggleMembers: () -> Unit, voicePermissionError: String?,
    dismissVoiceError: () -> Unit, joinVoice: (Channel) -> Unit, pendingVoiceJoin: VoiceJoinIntent?,
    pickFiles: (AttachmentDraftTarget, Boolean) -> Unit,
    browseChannels: () -> Unit, modifier: Modifier = Modifier, openNavigation: () -> Unit,
) {
    val channel = state.selectedChannel
    if (channel == null) {
        // Feedback team: the inbox takes the conversation's place.
        // Like a conversation, the inbox swipes back to Browse on phones.
        if (state.feedbackShown) FeedbackInbox(state, viewModel, narrow, openNavigation,
            modifier.fillMaxHeight().browseSwipeRegion(LocalBrowseDrawer.current, enabled = narrow))
        else EmptyChannel(state, narrow, show, openNavigation, browseChannels, modifier)
        return
    }
    // A feedback visit: the normal conversation (with the phone voice strip for a space channel), without a member list,
    // joining or settings; Back returns to the inbox.
    val visit = state.visit
    // Back leaves a visit for the inbox, unless phone Browse is showing (Back closes that first).
    val browse = LocalBrowseDrawer.current
    // Derived, so a sliding drawer doesn't recompose the conversation every frame.
    val browseClosed by remember(browse) { derivedStateOf { browse == null || browse.target == 0f } }
    BackHandler(enabled = visit != null && browseClosed) { viewModel.closeVisit() }
    var draft by rememberSaveable(channel.id, stateSaver = TextFieldValue.Saver) {
        viewModel.drafts[channel.id].let { mutableStateOf(TextFieldValue(it, TextRange(it.length))) }
    }
    // Kept per conversation, so switching channels or DMs and coming back restores it.
    fun updateDraft(value: TextFieldValue) { draft = value; viewModel.drafts[channel.id] = value.text }
    var channelMenuOpen by remember(channel.id) { mutableStateOf(false) }
    var showingPins by remember(channel.id) { mutableStateOf(false) }
    // Switching to Pins must not discard the channel's measured scroll position.
    val timelineState = key(channel.id) { rememberLazyListState() }
    val timelineFollow = rememberSaveable(channel.id, saver = TimelineFollow.Saver) { TimelineFollow() }
    val joined = channel.joined
    // A 1:1 DM (not your notes) offers Block / Unblock; a request does so in its own bar.
    val direct = state.selectedDirect?.takeIf { channel.direct && it.peer.id != state.account?.id && !it.incoming }
    var confirmBlock by remember(channel.id) { mutableStateOf(false) }
    if (confirmBlock && direct != null) BlockConfirmDialog(BlockedAccount(direct.peer.id, direct.peer.username, direct.peer.displayName, direct.peer.avatarId, direct.peer.avatarAsset, direct.peer.avatarUrl, direct.peer.admin, direct.peer.team), viewModel, { confirmBlock = false })
    fun toggleBlock() { if (direct == null) return; if (direct.blocked) viewModel.unblock(direct.peer.id) else confirmBlock = true }
    // Files dragged in from another app (split screen, freeform windows, desktop mode, ChromeOS).
    val canAttach = joined && state.uploadsEnabled && state.chatAuthorId != null && !state.messagesLoading && state.messagesError == null && state.sessionError == null
    val target = AttachmentDraftTarget(channel.id)
    val dropping = remember(target) { mutableStateOf(false) }
    Box(modifier.fillMaxHeight().attachmentDrop(canAttach, target, viewModel, dropping)) {
        Column(Modifier.fillMaxSize().background(SurfaceConversation)) {
            // Web waits a second before announcing a lost connection.
            val live = state.gateway == GatewayStatus.LIVE
            var showConnection by remember(channel.id) { mutableStateOf(false) }
            LaunchedEffect(live, channel.id) { showConnection = false; if (!live) { kotlinx.coroutines.delay(1_000); showConnection = true } }
            // The header carries no voice (the channel list has it); phones' strip under it offers this channel's Join.
            val channelVoice = !channel.direct && joined && BuildConfig.ENABLE_NATIVE_VOICE
            // A visited channel is never in the (demo) space you have open.
            val demo = visit == null && state.selectedSpace?.space?.demo == true
            val available = if (channelVoice) state.voiceAvailable(channel) else null
            // Phones compose Browse only while it shows, so the conversation asks for availability too.
            if (channelVoice) LaunchedEffect(channel.id, demo, state.account?.id, available) {
                if (available == null) viewModel.checkVoiceAvailability(channel)
            }
            val activeChannel = voice.channelId.takeIf { voice.phase != VoiceState.Phase.IDLE && voice.phase != VoiceState.Phase.FAILED }
            val activeHere = activeChannel == channel.id
            val voicePeople = when {
                !channelVoice -> emptyList()
                activeHere -> voice.participants
                else -> state.voiceRosters[voiceRootKey(demo, channel.id)].orEmpty()
            }
            // Phones: who is here, under the name (web's "N members · M in voice").
            val memberCount = state.selectedSpace?.members?.size?.takeIf { !channel.direct && joined && !demo && visit == null }
            val subtitle = headerSubtitle(memberCount, voicePeople.size)
            val title = when {
                visit != null -> visitTitle(visit).ifEmpty { "Feedback" }
                channel.direct -> channel.name
                else -> "# ${channel.name}"
            }
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                val headerWidth = maxWidth
                // On phones Back's arrow, not its 48dp touch target, lines up with the messages' 18dp inset.
                Row(Modifier.fillMaxWidth().height(53.dp).padding(start = if (narrow) 4.dp else 18.dp, end = if (narrow) 8.dp else 18.dp)
                    .browseSwipeRegion(LocalBrowseDrawer.current, enabled = narrow && !membersVisible && !channelMenuOpen)
                    .browseSwipeRegion(LocalMembersDrawer.current, enabled = narrow && !channelMenuOpen && !channel.direct && joined && visit == null),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (visit != null) IconButton(viewModel::closeVisit, Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.lucide_arrow_right), "Back to Feedback", Modifier.size(20.dp).graphicsLayer { rotationZ = 180f }, tint = TextMuted)
                    } else if (narrow) IconButton(openNavigation, Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.lucide_arrow_right), "Back to Browse", Modifier.size(20.dp).graphicsLayer { rotationZ = 180f }, tint = TextMuted)
                    }
                    if (visit == null && channel.direct) state.selectedDirect?.peer?.let { peer ->
                        Avatar(peer.displayName, 28.dp, avatarId = peer.avatarId, avatarAsset = peer.avatarAsset, avatarUrl = peer.avatarUrl)
                    }
                    // Phones: the name opens the conversation's menu, as in Slack; Pins, Members and Block move there.
                    if (narrow) Column(Modifier.weight(1f).heightIn(min = 48.dp).clip(MaterialTheme.shapes.small)
                        .background(if (channelMenuOpen) Surface else Color.Transparent)
                        .clickable(role = Role.Button) { channelMenuOpen = true }
                        .semantics(mergeDescendants = true) { contentDescription = "${if (channel.direct) "" else "#"}${channel.name} options" }
                        .padding(horizontal = 8.dp, vertical = 2.dp), verticalArrangement = Arrangement.Center) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(title, Modifier.weight(1f, fill = false), color = Text, fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Icon(painterResource(R.drawable.lucide_chevron_down), null, Modifier.size(16.dp), tint = TextMuted)
                        }
                        if (subtitle.isNotEmpty()) Text(subtitle, color = TextMuted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    } else Text(title, Modifier.weight(1f), fontSize = 14.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (!narrow) IconButton({ showingPins = !showingPins }, Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.lucide_pin), "Pins", Modifier.size(20.dp), tint = TextMuted)
                    }
                    // Right of Pins and the same size, as on web.
                    if (!narrow && !channel.direct && joined && visit == null) IconButton(toggleMembers, Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.lucide_users), if (membersVisible) "Hide member list" else "Show member list",
                            Modifier.size(20.dp), tint = if (membersVisible) Text else TextMuted)
                    }
                    if (!channel.direct && !joined) Button({ viewModel.joinChannel(channel) }, enabled = !state.busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Join channel") }
                    if (!live && showConnection) Text(if (state.gateway == GatewayStatus.ERROR || state.messagesError != null) "Offline" else "Connecting…", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                }
                // Web's title menu spans the header, inset 8dp, under it.
                if (narrow) DropdownMenu(channelMenuOpen, { channelMenuOpen = false }, Modifier.width(headerWidth - 16.dp), offset = DpOffset(8.dp, 0.dp),
                    containerColor = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                    DropdownMenuItem(
                        text = { Text("Pins", fontSize = 14.sp) },
                        leadingIcon = { Icon(painterResource(R.drawable.lucide_pin), null, Modifier.size(18.dp), tint = TextMuted) },
                        onClick = { channelMenuOpen = false; showingPins = true },
                    )
                    if (!channel.direct && joined && visit == null) DropdownMenuItem(
                        text = { Text(if (membersVisible) "Hide members" else "Members", fontSize = 14.sp) },
                        leadingIcon = { Icon(painterResource(R.drawable.lucide_users), null, Modifier.size(18.dp), tint = TextMuted) },
                        trailingIcon = if (memberCount != null) { { Text("$memberCount", color = TextMuted, fontSize = 13.sp, style = TabularNumbers) } } else null,
                        onClick = { channelMenuOpen = false; toggleMembers() },
                    )
                    // A 1:1 DM keeps Block / Unblock here on phones.
                    if (direct != null && (direct.blocked || direct.peer.canBlock)) DropdownMenuItem(
                        text = { Text(if (direct.blocked) "Unblock" else "Block", fontSize = 14.sp, color = if (direct.blocked) Text else ErrorText) },
                        leadingIcon = { Spacer(Modifier.size(18.dp)) },
                        onClick = { channelMenuOpen = false; toggleBlock() },
                    )
                }
            }
            HorizontalDivider(color = Border)
            // Phones: a slim strip under the header shows your call (tapping it opens Browse, where the call card is),
            // or, out of a call, who is talking here with Join.
            if (narrow) {
                val stripSwipe = Modifier.browseSwipeRegion(LocalBrowseDrawer.current, enabled = !membersVisible && !channelMenuOpen)
                if (inVoice(voice)) VoiceCallStrip(voice, state.account, callSessionStartedAt(state, voice), openNavigation, stripSwipe)
                else if (voicePeople.isNotEmpty()) VoiceJoinStrip(channel.name, voicePeople, voiceJoinAction(state, voice, pendingVoiceJoin, channel),
                    { joinVoice(channel) }, { viewModel.prepareVoiceJoin(channel) }, stripSwipe)
            }
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
                        TextButton(viewModel::retryMessages, shape = MaterialTheme.shapes.small) { Text("Retry", color = Text, fontSize = 12.sp) }
                    }
                }
            }
            // Web shows a pending message's status inline, under the message itself.
            val channelPending = state.pendingMessage?.takeIf { it.threadRootId == null }
            if (showingPins) Dialog(onDismissRequest = { showingPins = false }) {
                Surface(Modifier.swipeCancelsPresses(), shape = MaterialTheme.shapes.medium, color = SurfaceConversation, border = BorderStroke(1.dp, Border)) {
                    Column(Modifier.heightIn(max = 560.dp)) {
                        Row(Modifier.fillMaxWidth().padding(start = 18.dp, end = 8.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text("Pins", Modifier.weight(1f).semantics { heading() }, fontWeight = FontWeight.Bold)
                            TextButton({ showingPins = false }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Close") }
                        }
                        HorizontalDivider(color = Border)
                        PinnedMessages(state, viewModel, Modifier.weight(1f, fill = false)) { showingPins = false }
                    }
                }
            }
            MessageTimeline(state, viewModel, Modifier.weight(1f).browseSwipeRegion(LocalBrowseDrawer.current, enabled = narrow && !membersVisible && !channelMenuOpen)
                // A left swipe opens the member list, as a right one opens Browse.
                .browseSwipeRegion(LocalMembersDrawer.current, enabled = narrow && !membersVisible && !channelMenuOpen && !channel.direct && joined && visit == null),
                listState = timelineState, follow = timelineFollow) {
                channelPending?.error?.let { pending ->
                    val editable = canEditRejectedMessage(draft.text, channelPending.text)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(if (channelPending.uploadFailed) pending else if (channelPending.rejected) "Not sent. $pending" else "Not confirmed yet. $pending", Modifier.weight(1f), color = ErrorText, fontSize = 11.sp)
                        TextButton({
                            if (channelPending.rejected) {
                                if (editable) viewModel.discardPending(restoreFiles = true)?.let { updateDraft(TextFieldValue(it, TextRange(it.length))) }
                            } else viewModel.send(channelPending.text)
                        }, Modifier.semantics { if (channelPending.rejected && !editable) stateDescription = "Clear your current draft to edit this message." },
                            enabled = !channelPending.rejected || editable, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) {
                            Text(if (channelPending.uploadFailed) "Retry" else if (channelPending.rejected) "Edit" else "Retry send")
                        }
                        if (channelPending.rejected) TextButton({ viewModel.discardPending() }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Dismiss") }
                    }
                    if (channelPending.rejected && !editable)
                        Text("Clear your current draft to edit this message.", color = TextMuted, fontSize = 10.sp)
                }
            }
            // Phones show the conversation alone, so voice errors from the dock (behind Browse) come with it, above the composer.
            (voicePermissionError ?: voice.error)?.takeIf { narrow }?.let { error -> VoiceErrorBanner(error, dismissVoiceError) }
            val request = state.selectedDirect?.takeIf { channel.direct && it.incoming }
            // An incoming request is read-only until you answer it; a blocked DM offers Unblock instead.
            if (joined && request != null) RequestBar(request, viewModel) { if (narrow) openNavigation() }
            else if (joined && direct?.blocked == true) BlockedDirectNotice(direct, viewModel)
            // Keep the composer inset from the timeline and the panel edge.
            else if (joined) Column(Modifier.padding(start = 18.dp, end = 18.dp, top = 12.dp, bottom = 16.dp)) {
                TypingLine(state.typingAuthors)
                if (direct?.outgoing == true) OutgoingRequestNotice(direct)
                state.replyTargets["channel"]?.let { ReplyDraft(it, state) { viewModel.cancelReply() } }
                state.pendingMessage?.takeIf { it.error != null && it.threadRootId != state.thread?.rootId }?.let { pending ->
                    pending.threadRootId?.let { root ->
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(if (pending.rejected) "A thread reply wasn’t sent." else "A thread reply couldn’t be confirmed.", Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                            TextButton({ viewModel.openThread(root) }, shape = MaterialTheme.shapes.small) { Text("Review reply") }
                        }
                    }
                }
                state.sessionError?.let { error ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(error, Modifier.weight(1f), color = ErrorText, fontSize = 12.sp)
                        TextButton(viewModel::retrySession, shape = MaterialTheme.shapes.small) { Text("Retry session", fontSize = 12.sp) }
                    }
                }
                state.attachmentError?.let { error -> Text(error, Modifier.padding(bottom = 6.dp), color = ErrorText, fontSize = 12.sp) }
                // Signed in, the draft's first link shows the card it will get, with × to send it without one.
                val preview = rememberLinkPreview(if (state.account != null) draft.text else "", channel.id, viewModel::linkPreview)
                preview.embed?.let { embed -> Box(Modifier.padding(bottom = 8.dp)) { MessageEmbeds(listOf(embed), onRemove = preview.remove, arriving = true) } }
                DraftAttachmentStrip(state.drafts, viewModel::removeDraft, viewModel::retryDraft, edit = viewModel::editDraft, toggleSpoiler = viewModel::toggleDraftSpoiler)
                // A voice message goes on its own: the draft text and files stay in the composer.
                // Stopping stages the recording with the draft's files; with "send as soon as I stop" it goes on its own.
                val recorder = rememberVoiceRecorder(target, rememberVoiceStop(
                    stage = { viewModel.stageVoice(it.file, it.durationMs, it.waveform, target) },
                    send = { viewModel.sendVoice(it.file, it.durationMs, it.waveform) },
                ))
                val voiceBlocked = voiceRecordBlocked(state.pendingMessage != null, stagedVoice = state.drafts.any { it.voice })
                VoiceRecordingNotice(recorder)
                val hasContent = draft.text.isNotBlank() || state.drafts.isNotEmpty()
                fun sendDraft() {
                    if (hasContent && state.pendingMessage == null) {
                        val sent = draft.text
                        viewModel.setTyping(false)
                        val clear = { updateDraft(TextFieldValue("")) }
                        if (viewModel.send(sent, unchanged = { draft.text == sent }, accepted = clear, suppressEmbeds = preview.removed(sent))) clear()
                    }
                }
                fun submit() {
                    val pending = state.pendingMessage
                    // Web: Enter retries an unconfirmed send; a rejected one waits for Edit or Dismiss.
                    if (state.chatAuthorId != null) {
                        if (pending != null) { if (pending.threadRootId == null && !pending.rejected && pending.error != null) viewModel.send(pending.text) }
                        else sendDraft()
                    }
                }
                SuggestingComposer(draft, {
                    // Typing follows the text, not caret moves or selections.
                    val edited = it.text != draft.text
                    updateDraft(it); viewModel.reportActivity()
                    if (edited) viewModel.setTyping(it.text.isNotBlank())
                }, mentionSource(state), channel.id, ::submit,
                    focusRequest = state.composerFocusRequest?.takeIf { it.channelId == channel.id && it.rootId == null && state.thread == null },
                    consumeFocus = viewModel::consumeComposerFocus) { field, change, actions ->
                    Box(Modifier.voiceRecordingKeys(recorder)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                            EmojiTextField(
                                draft, change, modifier = Modifier.weight(1f).attachmentPaste(canAttach) { viewModel.addAttachments(it, target) }.then(field),
                                keyboardAttachment = if (canAttach) { uri -> viewModel.addAttachments(listOf(uri), target).singleOrNull() } else null,
                                // One line: a long channel or DM name would otherwise wrap and grow the empty field.
                                placeholder = { Text(if (channel.direct) "Message ${channel.name}" else "Message #${channel.name}", maxLines = 1, overflow = TextOverflow.Ellipsis) }, maxLines = 6,
                                visualTransformation = MentionVisualTransformation(mentionSource(state)),
                                // Reserve the icon inside the field while uploads/session become ready.
                                // Read-only previews and locked DMs have no composer.
                                leadingIcon = {
                                    AttachButton(attachDisabledReason(state), { viewModel.addAttachments(it, target) }, pick = { pickFiles(target, it) },
                                        drawer = AttachDrawerHost(state.drafts, viewModel::removeDraft,
                                            canSend = hasContent && state.pendingMessage == null && state.chatAuthorId != null, send = ::sendDraft))
                                },
                                // The `:name:` shortcut still works; this is the visible way in.
                                trailingIcon = {
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        // Sent at once as its own message; the draft stays.
                                        EmojiInsertButton(!state.messagesLoading && state.messagesError == null,
                                            load = viewModel::klipyPage, mediaAvailable = state.klipy != null,
                                            sendMedia = { gif, kind, query ->
                                                !state.messagesLoading && state.messagesError == null && state.pendingMessage == null && state.chatAuthorId != null && viewModel.sendGif(gif, kind, query)
                                            }) { emoji ->
                                            insertEmojiAtSelection(draft, emoji)?.let { updateDraft(it); viewModel.setTyping(true) }
                                        }
                                        // Wherever files can be sent: hidden when uploads are unavailable.
                                        if (state.uploadsEnabled) VoiceRecordButton(recorder, canAttach, voiceBlocked)
                                    }
                                },
                                enabled = !state.messagesLoading && state.messagesError == null,
                                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Send), keyboardActions = actions,
                                colors = composerFieldColors(),
                            )
                            FilledIconButton(
                                { sendDraft() },
                                // Centred on a one-line field (56dp), and near the bottom as the field grows.
                                modifier = Modifier.padding(bottom = 4.dp).size(48.dp).semantics { contentDescription = "Send" }, enabled = hasContent && state.pendingMessage == null && state.chatAuthorId != null,
                                shape = MaterialTheme.shapes.small,
                                colors = IconButtonDefaults.filledIconButtonColors(
                                    containerColor = Action, contentColor = OnAction,
                                    disabledContainerColor = SurfaceRaised, disabledContentColor = TextMuted,
                                ),
                            ) { Icon(painterResource(R.drawable.lucide_send), null, Modifier.size(20.dp)) }
                        }
                        if (recorder.recording) VoiceRecordingBar(recorder, Modifier.matchParentSize())
                    }
                }
                DraftCounter(draft.text)
            }
        }
        if (dropping.value && canAttach) {
            val (title, detail) = FileDrop.text(channel.name, channel.direct, state.drafts.size)
            FileDropOverlay(title, detail, caperBrandingResources[LocalBrandAvatar.current])
        }
    }
}

/**
 * A composer's `:` emoji / `@` mention popup above [content]'s field; the tokens never overlap, so both share it.
 * While it is open Up/Down move, Enter or Tab inserts and Escape dismisses; otherwise a hardware Enter without
 * Shift sends (Shift+Enter keeps the newline), like the keyboard's Send action.
 */
@Composable internal fun SuggestingComposer(
    draft: TextFieldValue, onDraft: (TextFieldValue) -> Unit, mentions: MentionSource, scope: Any?, send: () -> Unit,
    focusRequest: chat.caper.android.model.ComposerFocusRequest? = null,
    focusEnabled: Boolean = true, consumeFocus: (String) -> Unit = {},
    content: @Composable (field: Modifier, change: (TextFieldValue) -> Unit, actions: KeyboardActions) -> Unit,
) {
    val context = LocalContext.current
    val view = androidx.compose.ui.platform.LocalView.current
    val keyboard = LocalSoftwareKeyboardController.current
    val requester = remember(scope) { androidx.compose.ui.focus.FocusRequester() }
    LaunchedEffect(focusRequest, focusEnabled) {
        val request = focusRequest ?: return@LaunchedEffect
        if (!focusEnabled) return@LaunchedEffect
        // Modal sheets own the window until disposed. Wait for the destination's
        // window and mounted/enabled field, not an arbitrary animation delay.
        do { withFrameNanos { } } while (!view.hasWindowFocus())
        if (requester.requestFocus()) {
            withFrameNanos { }
            keyboard?.show()
            consumeFocus(request.id)
        }
    }
    val catalog = remember { EmojiArtwork.catalog(context) }
    var dismissedAt by remember(scope) { mutableStateOf<TextFieldValue?>(null) }
    var focused by remember(scope) { mutableStateOf(false) }
    val token = emojiToken(draft)
    val mention = if (token == null) mentionToken(draft) else null
    val open = focused && draft != dismissedAt
    val emojiRows = if (open && token != null) emojiSuggestions(catalog, token.query) else emptyList()
    val mentionRows = if (open && mention != null) mentionSuggestions(mentions, mention.query) else emptyList()
    val count = emojiRows.size + mentionRows.size
    var selected by remember(scope) { mutableIntStateOf(0) }
    LaunchedEffect(token, mention) { selected = 0 }
    val list = rememberLazyListState()
    LaunchedEffect(selected, token, mention) {
        if (count > 0) list.animateScrollToItem(selected.coerceAtMost(count - 1))
    }
    fun accept(next: TextFieldValue?) { next?.let { dismissedAt = null; onDraft(it) } }
    fun choose(index: Int): Boolean {
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
    if (count > 0) Surface(
        Modifier.entrance(MOTION_FAST, dy = 4.dp)
            .then(if (mentionRows.isEmpty()) Modifier.widthIn(max = 260.dp) else Modifier)
            .fillMaxWidth().padding(bottom = 6.dp), color = SurfaceRaised,
        shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
    ) {
        LazyColumn(Modifier.heightIn(max = 192.dp).padding(vertical = 4.dp), state = list) {
            itemsIndexed(emojiRows, key = { _, entry -> entry.id }) { index, entry ->
                SuggestionRow(index == selected, "Insert emoji ${emojiShortcodeLabel(entry.name)}", { selected = index; choose(index) }) {
                    EmojiImage(entry.emoji, null, Modifier.size(28.dp))
                    Text(emojiShortcodeLabel(entry.name), color = if (index == selected) Text else TextMuted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            itemsIndexed(mentionRows, key = { _, candidate -> "@${candidate.username}" }) { index, candidate ->
                MentionSuggestionRow(candidate, index == selected) { selected = index; choose(index) }
            }
        }
    }
    content(
        Modifier.focusRequester(requester).onFocusChanged { focused = it.isFocused }.onPreviewKeyEvent { event ->
            val enter = event.key == Key.Enter || event.key == Key.NumPadEnter
            if (event.type != KeyEventType.KeyDown) false
            // A hardware Enter outside IME composition sends; the text field never sees it.
            else if (count == 0) (enter && !event.isShiftPressed && draft.composition == null).also { if (it) send() }
            else when (event.key) {
                Key.DirectionDown -> { selected = (selected + 1) % count; true }
                Key.DirectionUp -> { selected = (selected - 1 + count) % count; true }
                Key.Enter, Key.NumPadEnter, Key.Tab -> if (event.isShiftPressed) false else choose(selected.coerceAtMost(count - 1))
                Key.Escape -> { dismissedAt = draft; true }
                else -> false
            }
        },
        { value ->
            val limited = if (value.text.codePointCount(0, value.text.length) <= 4000) value else {
                val text = value.text.codePointTake(4000)
                value.copy(text = text, selection = TextRange(value.selection.start.coerceAtMost(text.length), value.selection.end.coerceAtMost(text.length)), composition = null)
            }
            accept(limited)
        },
        KeyboardActions(onSend = { if (count > 0) choose(selected.coerceAtMost(count - 1)) else send() }),
    )
}

/** The channel and thread composers' field: the composer surface, terracotta while focused. */
@Composable internal fun composerFieldColors() = OutlinedTextFieldDefaults.colors(
    focusedContainerColor = SurfaceComposer, unfocusedContainerColor = SurfaceComposer, focusedBorderColor = Terracotta, unfocusedBorderColor = Border,
)

/** Web's `.chat-counter`: shown from 3,000 characters, toned toward the 4,000 limit. */
@Composable private fun ColumnScope.DraftCounter(text: String) {
    val count = text.codePointCount(0, text.length)
    if (count >= 3000) Text("${"%,d".format(java.util.Locale.US, count)} / 4,000", Modifier.align(Alignment.End), color = counterTone(count), fontSize = 10.sp, style = TabularNumbers)
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

/** A person (avatar, display name over a muted `@username`) or a special (`@everyone` over its muted description). */
@Composable private fun MentionSuggestionRow(candidate: MentionCandidate, selected: Boolean, action: String = "Mention", choose: () -> Unit) {
    val primary = if (candidate.special) "@${candidate.username}" else candidate.label
    val secondary = if (candidate.special) candidate.label else "@${candidate.username}" + if (candidate.invited) " · Already invited" else ""
    SuggestionRow(selected, "$action $primary, $secondary", choose) {
        if (candidate.special) Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) {
            Icon(painterResource(R.drawable.lucide_users), null, Modifier.size(18.dp), tint = TextMuted)
        } else Avatar(candidate.label, 28.dp, avatarId = candidate.avatarId, avatarAsset = candidate.avatarAsset, avatarUrl = candidate.avatarUrl)
        Column(Modifier.weight(1f)) {
            Text(primary, color = Text, fontSize = 13.sp, lineHeight = 18.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(secondary, color = TextMuted, fontSize = 11.sp, lineHeight = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/**
 * An exact-username field ([content] applies `field` to it) with [people] suggested inline below it when its
 * text matches someone (`usernameSuggestions`). Focus changes must not collapse the list and move nearby controls.
 * While the list is open Up/Down move, Enter or the
 * IME action (`imeAction`, which runs `closed` otherwise) fills in the highlighted username, and Escape hides the
 * list without closing the dialog. Choosing never submits; the list stays hidden until the text changes.
 */
@Composable private fun UsernameSuggestions(
    username: String, onUsername: (String) -> Unit, people: List<MentionCandidate>,
    content: @Composable (field: Modifier, imeAction: (closed: () -> Unit) -> Unit) -> Unit,
) {
    var dismissedAt by remember { mutableStateOf<String?>(null) }
    // Hidden only until the text changes: coming back to the same text later lists again.
    LaunchedEffect(username) { if (username != dismissedAt) dismissedAt = null }
    var escaping by remember { mutableStateOf(false) }
    val rows = if (username != dismissedAt) usernameSuggestions(people, username) else emptyList()
    var selected by remember(rows) { mutableIntStateOf(0) }
    fun choose(index: Int) = rows.getOrNull(index)?.let { dismissedAt = it.username; onUsername(it.username) }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        content(
            Modifier.onPreviewKeyEvent { event ->
                when {
                    // Dialogs close on Escape's key-up: swallow the one whose key-down hid the list.
                    event.key == Key.Escape && event.type == KeyEventType.KeyUp -> escaping.also { escaping = false }
                    rows.isEmpty() || event.type != KeyEventType.KeyDown -> false
                    event.key == Key.DirectionDown -> { selected = (selected + 1) % rows.size; true }
                    event.key == Key.DirectionUp -> { selected = (selected - 1 + rows.size) % rows.size; true }
                    event.key == Key.Enter || event.key == Key.NumPadEnter -> { choose(selected); true }
                    event.key == Key.Escape -> { dismissedAt = username; escaping = true; true }
                    else -> false
                }
            },
            { closed -> if (rows.isEmpty()) closed() else choose(selected) },
        )
        if (rows.isNotEmpty()) Surface(Modifier.fillMaxWidth(), color = SurfaceRaised, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
            Column(Modifier.padding(vertical = 4.dp)) {
                rows.forEachIndexed { index, person ->
                    key(person.username) { MentionSuggestionRow(person, index == selected, "Select") { selected = index; choose(index) } }
                }
            }
        }
    }
}

/** Web's empty-channel stage: owners can create the first channel; narrow screens keep Browse spaces. */
@Composable private fun EmptyChannel(state: AppUiState, narrow: Boolean, show: (Overlay) -> Unit, openNavigation: () -> Unit, browseChannels: () -> Unit, modifier: Modifier) {
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
            if (owner) OutlinedButton({ show(Overlay.CreateChannel) }, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = neutralOutlinedButtonColors()) { Text("Create channel") }
            // Lists every channel in the sidebar (with search), in the Browse drawer on phones.
            OutlinedButton(browseChannels, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = neutralOutlinedButtonColors()) { Text("Browse channels") }
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

@Composable private fun MessageTimeline(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, inThread: Boolean = false, listState: LazyListState = rememberLazyListState(), follow: TimelineFollow = remember { TimelineFollow() }, pendingStatus: @Composable () -> Unit = {}) {
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val displacement = remember(listState, state.selectedChannel?.id, if (inThread) state.thread?.rootId else null) { TimelineDisplacement() }
    // Drag interactions exclude automatic following, paging and viewport changes.
    LaunchedEffect(listState, focus, keyboard, displacement) {
        listState.interactionSource.interactions.collect { interaction ->
            if (interaction is DragInteraction.Start) {
                displacement.clear()
                focus.clearFocus()
                keyboard?.hide()
            }
        }
    }
    var actionTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var pickerTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var forwardTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var conversationTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var editTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var deleteTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var historyTarget by remember { mutableStateOf<ChatMessage?>(null) }
    // Message ID and the pressed chip's emoji.
    var reactorsTarget by remember { mutableStateOf<Pair<String, String>?>(null) }
    // Mentions, author avatars and author names share the same profile sheet.
    var mentionTarget by remember { mutableStateOf<MentionCard?>(null) }
    // Above the list, so new messages scrolling a row away don't close its viewer.
    val viewerHost = remember { MediaViewerHost() }
    val messages = if (inThread) state.displayedMessages else state.displayedChannelMessages
    // Runs of blocked authors' messages collapse; Show reveals one run, in memory only.
    var revealedRuns by remember { mutableStateOf(emptySet<String>()) }
    var blockTarget by remember { mutableStateOf<BlockedAccount?>(null) }
    val selfId = state.chatAuthorId ?: state.account?.id
    val blocked = state.blockedIds
    val rows = remember(messages, blocked, selfId, revealedRuns) { timelineRows(groupBlocked(messages, blocked, selfId, revealedRuns)) }
    val newMessagesId = if (!inThread && state.visit == null && !state.feedbackShown && state.selectedDirectId == null && state.selectedChannel?.direct == false)
        chat.caper.android.data.channelNewMessagesId(messages, state.channelVisitUnread, state.hasMoreMessages, blocked, selfId) else null
    // Recheck while stationary too: foreground return and failed acknowledgements need no scroll.
    // The bottom of a real reply must be visible (also works for replies taller than the viewport).
    LaunchedEffect(inThread, state.selectedChannel?.id, state.thread, rows) {
        if (!inThread) return@LaunchedEffect
        val channel = state.selectedChannel?.id ?: return@LaunchedEffect
        val root = state.thread?.rootId ?: return@LaunchedEffect
        while (true) {
            val layout = listState.layoutInfo
            val visible = layout.visibleItemsInfo.filter {
                it.offset + it.size > layout.viewportStartOffset && it.offset + it.size <= layout.viewportEndOffset
            }.map { it.key }.toSet()
            rows.filterIsInstance<TimelineRow.Message>().map { it.message }
                .filter { it.id in visible && it.threadRootId == root }
                .maxByOrNull { it.seq.toBigInteger() }?.let { viewModel.threadReplyVisible(channel, root, it.seq) }
            delay(1_000)
        }
    }
    val listedPending = state.pendingMessage?.takeIf { inThread || it.threadRootId == null }
    // Rows sent or received while the reader is at the live end settle in; see Arrivals.
    val arrivals = remember { Arrivals() }
    arrivals.observe(
        if (inThread) "thread" else "channel",
        (if (inThread) state.thread?.rootId else state.selectedChannel?.id).orEmpty(),
        messages.map { it.clientMessageId } + listOfNotNull(listedPending?.clientMessageId),
        !state.messagesLoading && state.messagesError == null &&
            if (inThread) state.thread?.let { !it.loading && !it.hasNewer } == true else !state.hasNewerMessages && !state.loadingNewer,
    )
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
        deleteTarget = null
        historyTarget = null
        blockTarget = null
        revealedRuns = emptySet()
        viewerHost.open = null
    }
    HostedMediaViewer(viewerHost, state, viewModel::reportAttachmentFailure)
    // Web's chat phases: loading, failed first load, then the conversation.
    if (state.messagesLoading) {
        SideEffect { follow.settled = false; displacement.clear() }
        return MessageSkeleton(modifier.clipToBounds())
    }
    state.messagesError?.let { error -> return Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(error, color = TextMuted, fontSize = 13.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            OutlinedButton(viewModel::retryMessages, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
                colors = ButtonDefaults.outlinedButtonColors(containerColor = SurfaceRaised, contentColor = Text)) { Text("Try again", fontSize = 12.sp) }
        }
    } }
    val skeleton = inThread && state.thread?.loading == true && messages.none { it.threadRootId == state.thread.rootId }
    val empty = messages.isEmpty() && state.pendingMessage == null && !(inThread && state.thread?.loading == true)
    val firstRow = if (inThread) 0 else 1
    val newer = !inThread && state.hasNewerMessages
    val keys = remember(rows) { rows.map { it.key } }
    val threadPaging = state.thread?.loading == true && !skeleton
    FollowLatest(listState, follow, FollowFrame(keys, listedPending?.clientMessageId,
        clientIds = messages.map { it.clientMessageId } + listOfNotNull(listedPending?.clientMessageId),
        loadingOlder = if (inThread) threadPaging else state.loadingOlder,
        historical = if (inThread) state.thread?.hasNewer == true else state.hasNewerMessages,
        loadingNewer = if (inThread) threadPaging && state.thread?.hasNewer == true else state.loadingNewer,
        reactions = messages.takeLast(FOLLOW_REACTION_ROWS).map { it.reactions },
        jump = state.focusRevision.takeIf { state.focusedMessageId?.let { id -> messages.any { it.id == id } } == true }),
        firstRow, firstRow + rows.size + listOf(skeleton, listedPending != null, empty, newer).count { it } - 1, ready = !skeleton, displacement = displacement)
    CompositionLocalProvider(LocalMediaViewerHost provides viewerHost) { LazyColumn(modifier.fillMaxWidth().displacementViewport(displacement), state = listState, reverseLayout = false, contentPadding = PaddingValues(vertical = 8.dp)) {
        if (!inThread) item {
            HistoryRow {
                when {
                    state.olderError != null -> { Text("Couldn’t load older messages.", color = TextMuted, fontSize = 11.sp); HistoryButton("Retry", onClick = viewModel::loadOlder) }
                    state.hasMoreMessages -> HistoryButton(if (state.loadingOlder) "Loading…" else "Load older messages", !state.loadingOlder, viewModel::loadOlder)
                    else -> Text("Beginning of conversation", color = TextMuted, fontSize = 11.sp)
                }
            }
        }
        // Keep the message keys stable while inserting purely presentational day boundaries.
        items(rows, key = { it.key }) { row ->
            val message = row.first
            val previous = row.previous
            val divider = previous == null || !sameLocalDay(previous.createdAt, message.createdAt)
            val revealed = if (row is TimelineRow.Blocked) row.run.revealed else collapsesFor(message, blocked, selfId)
            // The initial date stays outside the group; later dates stay within it.
            if (divider && row is TimelineRow.Blocked) DateDivider(message.createdAt)
            Column(Modifier.fillMaxWidth().displacement(displacement, listState, row.key).drawBehind {
                if (revealed) {
                    drawRect(chat.caper.android.ui.Surface)
                    drawRect(Border, size = Size(2.dp.toPx(), size.height))
                }
            }) {
                if (divider && row !is TimelineRow.Blocked) DateDivider(message.createdAt)
                if (row is TimelineRow.Message && message.id == newMessagesId) Row(
                    Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    HorizontalDivider(Modifier.weight(1f), color = Terracotta, thickness = 1.dp)
                    Text("New messages", color = Terracotta, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                    HorizontalDivider(Modifier.weight(1f), color = Terracotta, thickness = 1.dp)
                }
                if (row is TimelineRow.Blocked) {
                    BlockedRunRow(row.run.messages.size, row.run.revealed) {
                        revealedRuns = if (row.run.revealed) revealedRuns - row.run.key else revealedRuns + row.run.key
                    }
                } else Column(Modifier.arrival(arrivals, message.clientMessageId).then(
                    // Selections and your pins are terracotta; other people's pins are faded gold.
                    if (!inThread && state.thread?.rootId == message.id) Modifier.background(if (state.focusedMessageId == message.id) TerracottaWash else Terracotta.copy(alpha = 0.12f)).drawBehind {
                        val edge = 3.dp.toPx()
                        drawRect(Terracotta, topLeft = Offset(if (layoutDirection == LayoutDirection.Rtl) size.width - edge else 0f, 0f), size = Size(edge, size.height))
                    } else Modifier.background(if (state.focusedMessageId == message.id) TerracottaWash else if (message.pin != null) {
                        if (message.pin.author.id == (state.account?.id ?: state.chatAuthorId)) Terracotta.copy(alpha = 0.12f) else PinGoldWash
                    } else Color.Transparent))) {
                    ReactionMessageRow(
                        message, state, viewModel::setReaction, viewModel::retryReaction, viewModel::dismissReactionError,
                        openReactors = { target, emoji -> reactorsTarget = target.id to emoji },
                        openPicker = { pickerTarget = it },
                        openMention = { mentionTarget = mentionCard(it.id, it.username, state) },
                        openAuthor = { mentionTarget = authorCard(it, state) },
                        openActions = { actionTarget = it },
                        // A reply to a deleted or unavailable message says so instead of doing nothing.
                        openReference = { reference -> viewModel.goToMessage(message.copy(id = reference.id, threadRootId = reference.threadRootId), notify = true) {} },
                        openConversation = { message -> viewModel.openForwardedConversation(message, { conversationTarget = message }) },
                        openHistory = { historyTarget = it },
                        retryPin = viewModel::retryPin,
                        dismissPinError = viewModel::dismissPinError,
                        onAttachmentFailed = viewModel::reportAttachmentFailure,
                        grouped = message.id != newMessagesId && row is TimelineRow.Message && groupsWithPrevious(row.above, message, inThread, state.focusedMessageId),
                    )
                    if (!inThread && (message.threadRootId != null || (message.thread?.replyCount ?: 0) > 0)) Row(Modifier.padding(start = 62.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                        TextButton({ viewModel.openThread(message.threadRootId ?: message.id) }, modifier = Modifier.fillMaxWidth(), contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp), shape = MaterialTheme.shapes.small) {
                            message.thread?.takeIf { message.threadRootId == null }?.participants?.forEach {
                                Avatar(it.name, 24.dp, avatarId = it.avatarId, avatarAsset = it.avatarAsset, avatarUrl = it.avatarUrl)
                                Spacer(Modifier.width(6.dp))
                            }
                            val summary = message.thread?.takeIf { message.threadRootId == null }
                            // As on web: the reply count in bold terracotta, then a quiet "View thread".
                            if (summary != null) Text(buildAnnotatedString {
                                withStyle(SpanStyle(color = TerracottaBright, fontWeight = FontWeight.Bold)) {
                                    append("${summary.replyCount} ${if (summary.replyCount == 1) "reply" else "replies"}")
                                }
                                if (state.unreadThreads.containsKey(message.id)) {
                                    withStyle(SpanStyle(color = PinGold)) { append("  ● Unread") }
                                }
                                withStyle(SpanStyle(color = TextMuted)) { append("  View thread") }
                            }) else Text("Replied to a thread · View thread")
                            Spacer(Modifier.weight(1f))
                        }
                    }
                }
            }
        }
        if (skeleton) item("thread-loading") {
            MessageSkeleton(rows = 2, label = "Loading thread replies")
        }
        listedPending?.let { pending -> item("pending:${pending.clientMessageId}") {
            Column(Modifier.displacement(displacement, listState, "pending:${pending.clientMessageId}")) {
                if (messages.lastOrNull()?.createdAt?.let { sameLocalDay(it, pending.createdAt) } != true) {
                    DateDivider(pending.createdAt)
                }
                // Your unsent message joins your run like a sent one.
                val grouped = groupsWithPrevious((rows.lastOrNull() as? TimelineRow.Message)?.message, pending.author?.id ?: selfId, pending.createdAt,
                    pending.threadRootId, threadContext = pending.replyTo != null, jumpTarget = state.focusedMessageId)
                Column(Modifier.arrival(arrivals, pending.clientMessageId)) {
                    pending.replyTo?.let { ReplyContext(it, state, pending.clientMessageId) }
                    MessageRow(pending.author?.name ?: "You", pending.author?.isGuest == true, pending.createdAt, pending.text, true, pending.author?.avatarId, pending.attachments, grouped = grouped,
                        avatarAsset = pending.author?.avatarAsset, avatarUrl = pending.author?.avatarUrl, gif = pending.gif)
                    Box(Modifier.padding(start = 62.dp, end = 18.dp)) { Column { pendingStatus() } }
                }
            }
        } }
        if (empty) item { Box(Modifier.fillParentMaxSize(), contentAlignment = Alignment.Center) {
            // Inset like messages, so longer notes wrap clear of the edges on phones.
            Column(Modifier.padding(horizontal = 18.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(7.dp)) {
                if (state.selectedDirect?.let { it.peer.id == state.account?.id } == true) {
                    Text("You can message yourself here to keep notes, reminders, and ideas.", color = TextMuted, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                } else {
                    Text("No messages yet.", color = TextMuted)
                    val name = state.selectedChannel?.name.orEmpty()
                    if (state.visit == null) Text(when {
                        state.selectedChannel?.direct != true -> "Start the conversation in #$name."
                        // The Caper DM: each top-level message becomes a report the team answers in its thread.
                        state.selectedDirect?.peer?.team == true -> CAPER_DM_EMPTY_NOTE
                        else -> "Only you and $name can read this conversation."
                    }, color = TextMuted, fontSize = 12.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                }
            }
        } }
        if (newer) item {
            HistoryRow {
                HistoryButton(if (state.loadingNewer) "Loading…" else "Load newer messages", !state.loadingNewer, viewModel::loadNewer)
                HistoryButton("Back to latest", onClick = viewModel::retryMessages)
            }
        }
    } }
    actionTarget?.let { target ->
        val presented = state.displayedMessages.firstOrNull { it.id == target.id } ?: target
        MessageActionsSheet(
            presented, state,
            onDismiss = { actionTarget = null },
            setReaction = viewModel::setReaction,
            openPicker = { actionTarget = null; pickerTarget = presented },
            setPin = { id, active -> viewModel.setPin(id, active); actionTarget = null },
            onReply = if (inThread) null else ({ actionTarget = null; viewModel.replyInThread(presented) }),
            onReplyMessage = if (state.canParticipate && state.pendingMessage == null) ({ actionTarget = null; viewModel.replyTo(presented, inThread) }) else null,
            forward = { actionTarget = null; forwardTarget = presented },
            onEdit = if (viewModel.canEdit(presented)) ({ actionTarget = null; editTarget = presented }) else null,
            onDelete = if (viewModel.canDelete(presented)) ({ actionTarget = null; deleteTarget = presented }) else null,
            onHistory = { actionTarget = null; historyTarget = presented },
            onViewReactions = { presented.reactions.firstOrNull()?.let { reactorsTarget = presented.id to it.emoji }; actionTarget = null },
            blockLabel = presented.author.takeIf { state.account != null && !it.isGuest && it.id != selfId && (it.id in blocked || viewModel.blockTarget(it).canBlock) }?.let { author ->
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
        // Closes if the message is deleted meanwhile.
        if (viewModel.canEdit(state.messages.find { it.id == target.id } ?: target)) MessageEditorDialog(target, viewModel) { editTarget = null }
        else LaunchedEffect(target.id) { editTarget = null }
    }
    deleteTarget?.let { target -> key(target.id) { DeleteMessageDialog(target, viewModel) { deleteTarget = null } } }
    forwardTarget?.let { target -> key(target.id) { ForwardPickerSheet(target, viewModel) { forwardTarget = null } } }
    conversationTarget?.let { target ->
        val current = state.messages.firstOrNull { it.id == target.id }
        if (current != null && current.deletedAt == null && current.forward?.message != null) key(target.id) { ForwardConversationSheet(current, viewModel) { conversationTarget = null } }
        else LaunchedEffect(target.id) { conversationTarget = null }
    }
    historyTarget?.let { target ->
        val current = state.messages.find { it.id == target.id } ?: target
        if (current.deletedAt == null) MessageHistoryDialog(current, viewModel) { historyTarget = null }
        else LaunchedEffect(target.id) { historyTarget = null }
    }
    reactorsTarget?.let { (messageId, emoji) ->
        val presented = state.messages.firstOrNull { it.id == messageId }
        if (presented == null) LaunchedEffect(messageId) { reactorsTarget = null }
        else key(messageId) {
            ReactorsSheet(presented, emoji, state.chatAuthorId ?: state.account?.id, state.account, viewModel::reactors, state.profileRevision) { reactorsTarget = null }
        }
    }
    mentionTarget?.let { card ->
        key(card.id) {
            MentionCardSheet(card, { done, failed -> card.username?.let { viewModel.messageMentioned(card.id, it, done, failed) } }, state = state, viewModel = viewModel) { mentionTarget = null }
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

/** What a message list last showed. Only [settled] (it was positioned since opening) survives recreation. */
internal class TimelineFollow(var settled: Boolean = false) {
    var last: FollowFrame? = null
    /** You sent from a historical context; the return to the newest page reveals it. */
    var sent = false
    var jump: Long? = null
    companion object { val Saver = Saver<TimelineFollow, Boolean>({ it.settled }, ::TimelineFollow) }
}

/**
 * One composition of a message list: its row keys, your pending send, paging state, whether it shows a historical
 * context window rather than the newest messages, the Go to message revision whose target it contains, and the
 * reactions on the last [FOLLOW_REACTION_ROWS] messages.
 */
internal data class FollowFrame(
    val keys: List<String>, val pendingId: String? = null, val loadingOlder: Boolean = false,
    val historical: Boolean = false, val loadingNewer: Boolean = false, val jump: Long? = null,
    val reactions: List<List<MessageReaction>> = emptyList(),
    val clientIds: List<String> = emptyList(),
)

/** Only a genuine live append, never a pending confirmation, page or focus jump. */
internal fun liveAppend(before: FollowFrame?, now: FollowFrame): Boolean {
    if (before == null || before.historical || now.historical || before.loadingOlder || now.loadingOlder ||
        before.loadingNewer || now.loadingNewer || before.jump != now.jump || before.clientIds.isEmpty()) return false
    // A remote row may arrive just before the pending row; confirmation retains its client ID.
    val previousRows = before.clientIds.filterNot { it == before.pendingId }
    val currentRows = now.clientIds.filterNot { it == now.pendingId }
    return currentRows.take(previousRows.size) == previousRows && now.clientIds.any { it !in before.clientIds }
}

/** Recent rows whose reaction chips can grow the bottom of the list: more than a screen of short messages. */
internal const val FOLLOW_REACTION_ROWS = 20

/**
 * Web's timeline following, as the `(index, offset)` to request (or null to stay), updating [follow]. A Go to
 * message jump owns the viewport. Otherwise the list opens at the latest row once, always reveals your own send,
 * returns to the bottom when it goes back to the newest page (but not when you paged forward into it), keeps the
 * rows you were reading when an older page lands ([anchor]), and follows new rows only near the bottom ([near]) of
 * the live conversation.
 */
internal fun followScroll(follow: TimelineFollow, ready: Boolean, now: FollowFrame, lastIndex: Int, near: () -> Boolean, anchor: () -> Pair<Int, Int>?): Pair<Int, Int>? {
    val last = follow.last
    val jumped = now.jump != null && now.jump != follow.jump
    follow.last = now
    if (now.jump != null) follow.jump = now.jump
    val latest = lastIndex to PAST_END
    return when {
        !ready -> null
        jumped -> { follow.settled = true; null }
        !follow.settled -> if (now.keys.isEmpty()) null else latest.also { follow.settled = true; follow.sent = false }
        // Recreated: the restored position stands.
        last == null -> null
        now.pendingId != null && now.pendingId != last.pendingId -> latest.also { follow.sent = now.historical }
        last.historical && !now.historical -> if (last.loadingNewer && !follow.sent) null else latest.also { follow.sent = false }
        else -> (if (last.loadingOlder && !now.loadingOlder) anchor() else null)
            // Reaction chips can add/wrap a row without changing its key.
            ?: latest.takeIf { !now.historical && !last.historical &&
                (now.keys.lastOrNull() != last.keys.lastOrNull() || now.reactions != last.reactions) && near() }
    }
}

/** Applies [followScroll] before the next measure, so nothing flashes at the old position, and keeps a reader at the bottom when the viewport shrinks (the keyboard opening) or a shown row grows. */
@Composable private fun FollowLatest(list: LazyListState, follow: TimelineFollow, now: FollowFrame, firstRow: Int, lastIndex: Int, ready: Boolean, displacement: TimelineDisplacement) {
    val threshold = with(LocalDensity.current) { 80.dp.roundToPx() }
    val still = reducedMotion()
    LaunchedEffect(list, displacement) {
        snapshotFlow { list.isScrollInProgress }.collect { scrolling ->
            if (scrolling) displacement.clear()
        }
    }
    LaunchedEffect(displacement, displacement.revision, still) {
        if (still) displacement.clear()
        else if (displacement.remaining > 0f) {
            val start = withFrameNanos { it }
            do {
                val elapsed = withFrameNanos { it } - start
                if (displacement.remaining == 0f) break
                displacement.remaining = 1f - EaseOut.transform((elapsed / (MOTION_SLOW * 1_000_000f)).coerceIn(0f, 1f))
            } while (displacement.remaining > 0f)
        }
    }
    SideEffect {
        // The layout still shows the previous rows here.
        val animate = !still && ready && follow.settled && !list.isScrollInProgress &&
            list.layoutInfo.nearBottom(threshold) && liveAppend(follow.last, now)
        val jumped = now.jump != follow.last?.jump
        val confirmation = follow.last?.clientIds == now.clientIds
        val before = follow.last?.keys
        followScroll(follow, ready, now, lastIndex, { list.layoutInfo.nearBottom(threshold) }) {
            olderPageAnchor(list.layoutInfo.visibleItemsInfo.map { Triple(it.key, it.index, it.offset) }, before?.let { pageBoundary(it, now.keys) }) { key ->
                (key as? String)?.let(now.keys::indexOf)?.takeIf { it >= 0 }?.plus(firstRow)
            }
        }?.let { (index, offset) ->
            if (animate) displacement.capture(list) else if (!confirmation) displacement.clear()
            list.requestScrollToItem(index, offset)
        }
        if (still || jumped || !ready || now.historical || now.loadingOlder || now.loadingNewer) displacement.clear()
    }
    LaunchedEffect(list, follow, displacement) {
        var height = 0
        var near = true
        var sizes = emptyMap<Any, Int>()
        snapshotFlow { list.layoutInfo }.collect { info ->
            val shrank = info.viewportSize.height < height
            height = info.viewportSize.height
            // A shown row measured taller (embeds arriving, a card loading after opening) pushes the end down.
            val grew = info.visibleItemsInfo.any { item -> sizes[item.key]?.let { item.size > it } == true }
            sizes = info.visibleItemsInfo.associate { it.key to it.size }
            val live = follow.settled && follow.last?.historical == false && info.totalItemsCount > 0
            if (live && near && (shrank || grew && !list.isScrollInProgress && !info.nearBottom(0))) {
                displacement.clear()
                list.requestScrollToItem(info.totalItemsCount - 1, PAST_END)
            }
            else near = info.nearBottom(threshold)
        }
    }
}

/** A past-the-end offset bottom-aligns the last row, even one taller than the viewport. */
internal const val PAST_END = Int.MAX_VALUE / 2

private fun LazyListLayoutInfo.nearBottom(threshold: Int): Boolean = visibleItemsInfo.lastOrNull().let { last ->
    nearBottom(totalItemsCount, last?.index ?: -1, last?.let { it.offset + it.size } ?: 0, viewportEndOffset - afterContentPadding, threshold)
}

/** Web's at-bottom test: the last item ends within [threshold] of the content's end. An empty list is at the bottom. */
internal fun nearBottom(totalItems: Int, lastVisibleIndex: Int, lastVisibleEnd: Int, contentEnd: Int, threshold: Int): Boolean =
    totalItems == 0 || lastVisibleIndex == totalItems - 1 && lastVisibleEnd - contentEnd <= threshold

/**
 * After an older page lands: the first visible `(key, index, offset)` item whose index moved, as the new index and
 * the scroll offset that keeps it where it was. A header or thread root above the page keeps its index. The row the
 * page landed against ([boundary]) is passed over while another moved row is visible: it gains a previous message,
 * so its day divider or full header can drop, and holding its top would pull everything below it up (web's jump).
 */
internal fun olderPageAnchor(visible: List<Triple<Any, Int, Int>>, boundary: Any? = null, newIndex: (Any) -> Int?): Pair<Int, Int>? {
    fun anchor(skip: Any?) = visible.firstNotNullOfOrNull { (key, index, offset) ->
        if (skip != null && key == skip) null else newIndex(key)?.takeIf { it != index }?.let { it to -offset }
    }
    return anchor(boundary) ?: anchor(null)
}

/** The earlier rows' first key whose index moved: the row an older page landed against (after any thread root). */
internal fun pageBoundary(before: List<String>, now: List<String>): String? =
    before.withIndex().firstOrNull { (index, key) -> now.indexOf(key).let { it >= 0 && it != index } }?.value

/** Web's `.chat-history` buttons: older and newer messages or replies, and Back to latest, all alike. */
@Composable private fun HistoryButton(label: String, enabled: Boolean = true, onClick: () -> Unit) = OutlinedButton(
    onClick, enabled = enabled, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border),
    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp), colors = ButtonDefaults.outlinedButtonColors(contentColor = TextMuted),
) { Text(label, fontSize = 11.sp, fontWeight = FontWeight.Bold) }

/** A centred row of [HistoryButton]s (and their status text), as tall as the timeline's first row. */
@Composable private fun HistoryRow(content: @Composable RowScope.() -> Unit) = Row(
    Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 14.dp),
    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically, content = content,
)

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
    openPicker: (ChatMessage) -> Unit = {},
    retryPin: (String) -> Unit = {},
    dismissPinError: (String) -> Unit = {},
    openConversation: (ChatMessage) -> Unit = {},
    openHistory: (ChatMessage) -> Unit = {},
    onAttachmentFailed: (ChatAttachment, Int?) -> Unit = { _, _ -> },
    openMention: (MessageMention) -> Unit = {},
    openAuthor: ((ChatAuthor) -> Unit)? = null,
    openReference: ((MessageReply) -> Unit)? = null,
    /** Compact under the same person's previous message (`data/Grouping.kt`). */
    grouped: Boolean = false,
    inPins: Boolean = false,
    /** Only pinned cards opt in; nested links and controls consume their own taps. */
    openMessage: (() -> Unit)? = null,
    openActions: (ChatMessage) -> Unit,
) {
    val own = state.chatAuthorId ?: state.account?.id
    val canReact = state.canParticipate && own != null
    val saves = state.reactionSaves.filterKeys { it.startsWith("${message.id}:") }.values
    val interaction = remember(message.id) { MutableInteractionSource() }
    val pressing by interaction.collectIsPressedAsState()
    // A deleted message keeps its header and place, without its content, quote, forward, pin or reactions.
    val deleted = message.deletedAt != null
    // Pin ownership takes precedence over the mention wash; inline mention pills remain visible.
    val mentioned = message.pin == null && mentionsMe(message.content.mentions, message.author.id, state.account?.id ?: state.chatAuthorId)
    val replyWash = !deleted && message.replyTo != null && message.pin == null && state.focusedMessageId != message.id && state.thread?.rootId != message.id
    val replyToMe = replyWash && own != null && message.author.id != own && message.replyTo?.author?.id == own
    Column((if (!mentioned && !replyToMe) Modifier.background(if (replyWash) MaterialTheme.colorScheme.onSurface.copy(alpha = 0.03f) else Color.Transparent) else Modifier.background(Terracotta.copy(alpha = if (replyToMe) 0.12f else 0.08f)).drawBehind {
        val edge = 2.dp.toPx()
        drawRect(Terracotta, topLeft = Offset(if (layoutDirection == LayoutDirection.Rtl) size.width - edge else 0f, 0f), size = Size(edge, size.height))
    }).drawBehind {
        if (pressing) drawRect(Text.copy(alpha = 0.08f))
    }) {
        message.pin?.takeIf { !deleted }?.let { pin ->
            val pinInteraction = remember(message.id) { MutableInteractionSource() }
            val hovered by pinInteraction.collectIsHoveredAsState()
            LaunchedEffect(hovered) { if (hovered) openAuthor?.invoke(pin.author) }
            Text("Pinned by ${if (pin.author.id == (state.account?.id ?: state.chatAuthorId)) "you" else pin.author.name}", Modifier.padding(start = 62.dp, end = 18.dp)
                .heightIn(min = 44.dp).wrapContentHeight()
                .hoverable(pinInteraction)
                .clip(MaterialTheme.shapes.small).combinedClickable(enabled = openAuthor != null, role = Role.Button, onClick = { openAuthor?.invoke(pin.author) }, onLongClick = { openAuthor?.invoke(pin.author) })
                .semantics { contentDescription = "Open profile for ${pin.author.name}" },
                color = TextMuted, fontSize = 10.sp, fontWeight = FontWeight.Bold)
        }
        if (inPins) Text(fullDateLabel(message.createdAt), Modifier.padding(start = 62.dp, end = 18.dp), color = TextMuted, fontSize = 10.sp)
        message.replyTo?.takeIf { !deleted }?.let { reference ->
            // A deleted original has nothing to show, so its quote does not open it.
            ReplyContext(reference, state, message.clientMessageId, openReference?.takeIf { !reference.deleted }?.let { open -> { open(reference) } })
        }
        // A grouped row stays a full-width long-press target, but drops the 48dp minimum so a run reads as one block.
        Box(Modifier.heightIn(min = if (grouped) 0.dp else 48.dp).combinedClickable(
            interactionSource = interaction,
            indication = null,
            onClick = { openMessage?.invoke() },
            onLongClick = { openActions(message) },
            onLongClickLabel = "Message actions for ${message.author.name}",
        )) { MessageRow(message, state, onAttachmentFailed, openMention, { mentionCard(it.id, it.username.orEmpty(), state).title }, openAuthor, grouped) { if (message.forward == null) openHistory(message) } }
        // A team mention (`@feedback`, `@admin`, …): the server told the Caper team, who can read and reply here.
        if (!deleted && notifiesCaperTeam(message.content)) Text(CAPER_TEAM_NOTIFIED, Modifier.padding(start = 62.dp, end = 18.dp, bottom = 2.dp), color = TextMuted, fontSize = 11.sp)
        if (!deleted) Box(Modifier.combinedClickable(
            onClick = { openConversation(message) },
            onLongClick = { openActions(message) },
            onLongClickLabel = "Message actions for ${message.author.name}",
        )) { ForwardCard(message, state, onAttachmentFailed) { openConversation(message) } }
        if (!deleted) FlowRow(Modifier.padding(start = 62.dp, end = 18.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            message.reactions.forEach { reaction ->
                val selected = own != null && own in reaction.authorIds
                ReactionChip(
                    reaction, selected, canReact,
                    toggle = { setReaction(message.id, reaction.emoji, !selected) },
                    showReactors = { openReactors(message, reaction.emoji) },
                )
            }
            if (message.reactions.isNotEmpty() && canReact) {
                IconButton({ openPicker(message) }, modifier = Modifier.size(48.dp)) {
                    Icon(painterResource(R.drawable.lucide_smile), "Add reaction", Modifier.size(18.dp), tint = TextMuted)
                }
            }
        }
        saves.filter { it.error != null }.forEach { save ->
            Column(Modifier.padding(start = 62.dp, end = 18.dp, top = 4.dp)) {
                Text(save.error ?: "Reaction could not be saved.", color = ErrorText, fontSize = 11.sp)
                Row {
                    TextButton({ retryReaction(message.id, save.emoji) }, enabled = canReact, modifier = Modifier.heightIn(min = 48.dp), shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Retry") }
                    TextButton({ dismissReactionError(message.id, save.emoji) }, modifier = Modifier.heightIn(min = 48.dp), shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Dismiss") }
                }
            }
        }
        state.pinSaves[message.id]?.takeIf { it.error != null }?.let { save ->
            Row(Modifier.padding(start = 62.dp, end = 18.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(save.error ?: "Pin could not be saved.", Modifier.weight(1f), color = ErrorText, fontSize = 11.sp)
                TextButton({ retryPin(message.id) }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Retry") }
                TextButton({ dismissPinError(message.id) }, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Dismiss") }
            }
        }
    }
}

private val quickReactions = listOf("👍", "❤️", "😂")

@OptIn(ExperimentalMaterial3Api::class)
@Composable internal fun MessageActionsSheet(
    message: ChatMessage,
    state: AppUiState,
    onDismiss: () -> Unit,
    setReaction: (String, String, Boolean) -> Unit,
    openPicker: () -> Unit,
    setPin: (String, Boolean) -> Unit = { _, _ -> },
    onReply: (() -> Unit)? = null,
    onReplyMessage: (() -> Unit)? = null,
    forward: () -> Unit = {},
    onEdit: (() -> Unit)? = null,
    onHistory: (() -> Unit)? = null,
    /** Opens who reacted, shown while the message has reactions. */
    onViewReactions: (() -> Unit)? = null,
    blockLabel: String? = null,
    onBlock: () -> Unit = {},
    /** Asks to confirm deleting; null unless the viewer may delete it. */
    onDelete: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    val uriHandler = LocalUriHandler.current
    // Include both a forwarding note and its visible original, but never deleted text.
    val links = if (message.deletedAt != null) emptyList() else
        listOfNotNull(message, message.forward?.message).filter { it.deletedAt == null }.flatMap { linkedMessage ->
            val text = linkedMessage.content.text
            linkRanges(text, highlightedMentions(text, linkedMessage.content.mentions).map { it.start until it.end })
                .map { link -> text.substring(link.start, link.end) to link.href }
        }.distinct()
    val own = state.chatAuthorId ?: state.account?.id
    // A deleted message keeps only its thread and its ID.
    val deleted = message.deletedAt != null
    val canReact = state.canParticipate && own != null && !deleted
    fun copy(label: String, value: String) {
        clipboard.setPrimaryClip(ClipData.newPlainText(label, value))
        onDismiss()
    }
    // Opens fully, and scrolls when the actions outgrow a short or landscape screen.
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = SurfaceRaised, contentColor = Text) {
        Column(Modifier.fillMaxWidth().swipeCancelsPresses().verticalScroll(rememberScrollState()).padding(start = 18.dp, end = 18.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
                                contentDescription = "React with $emoji"
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
                    // Web's order: replies, forward, edit, history, reactions, pin, then copy and block.
                    if (!deleted) onReplyMessage?.let { SheetAction("Reply", icon = R.drawable.lucide_reply, onClick = it); HorizontalDivider(color = Border) }
                    if (!deleted || (message.thread?.replyCount ?: 0) > 0 || message.threadRootId != null) {
                        onReply?.let { SheetAction("Reply in thread", icon = R.drawable.lucide_thread, onClick = it); HorizontalDivider(color = Border) }
                    }
                    if (!deleted && state.account != null && (message.forward == null || message.forward.message != null)) {
                        SheetAction("Forward message", onClick = forward)
                        HorizontalDivider(color = Border)
                    }
                    if (!deleted) onEdit?.let { SheetAction("Edit message", onClick = it); HorizontalDivider(color = Border) }
                    if (!deleted && message.forward == null && message.revision > 1) onHistory?.let { SheetAction("Message history", onClick = it); HorizontalDivider(color = Border) }
                    // Web, Apple and desktop: who reacted, from the first reaction's tab.
                    if (!deleted && message.reactions.isNotEmpty()) onViewReactions?.let { SheetAction("View reactions", onClick = it); HorizontalDivider(color = Border) }
                    if (canReact) {
                        val saving = state.pinSaves[message.id]?.saving == true
                        SheetAction(if (saving) "Saving…" else if (message.pin == null) "Pin message" else "Unpin message", enabled = !saving) { setPin(message.id, message.pin == null) }
                        HorizontalDivider(color = Border)
                    }
                    // An attachment-only message has no text to copy.
                    links.forEach { (label, href) ->
                        SheetAction("Open link: $label") { openExternalLink(uriHandler, href); onDismiss() }
                        SheetAction("Copy link: $label") { copy("Link", href) }
                        HorizontalDivider(color = Border)
                    }
                    if (message.content.text.isNotBlank()) {
                        SheetAction("Copy text") { copy("Message text", message.content.text) }
                        HorizontalDivider(color = Border)
                    }
                    SheetAction("Copy message ID") { copy("Message ID", message.id) }
                    blockLabel?.let { label ->
                        HorizontalDivider(color = Border)
                        SheetAction(label, if (label.startsWith("Block")) ErrorText else Text, onClick = onBlock)
                    }
                    if (!deleted) onDelete?.let {
                        HorizontalDivider(color = Border)
                        SheetAction("Delete message", ErrorText, icon = R.drawable.lucide_trash_2, onClick = it)
                    }
                }
            }
        }
    }
}

/**
 * One item of the message actions list: neutral like its siblings, and pressed edge to edge (the
 * list's rounded corners clip it) rather than as a pill inside the row.
 */
@Composable private fun SheetAction(label: String, color: Color = Text, enabled: Boolean = true, icon: Int? = null, onClick: () -> Unit) = TextButton(
    onClick, Modifier.fillMaxWidth().heightIn(min = 48.dp), enabled = enabled, shape = RectangleShape,
    colors = ButtonDefaults.textButtonColors(contentColor = color),
) {
    icon?.let { Icon(painterResource(it), contentDescription = null, Modifier.size(16.dp)); Spacer(Modifier.width(8.dp)) }
    Text(label, Modifier.fillMaxWidth())
}

@Composable private fun PinnedMessages(state: AppUiState, viewModel: CaperViewModel, modifier: Modifier, close: () -> Unit) {
    var conversationTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var editTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var deleteTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var historyTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var actionTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var pickerTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var forwardTarget by remember { mutableStateOf<ChatMessage?>(null) }
    var reactorsTarget by remember { mutableStateOf<Pair<String, String>?>(null) }
    var pinnerTarget by remember { mutableStateOf<ChatAuthor?>(null) }
    LaunchedEffect(state.account?.id, state.selectedChannel?.id, state.selectedDirectId) { conversationTarget = null; editTarget = null; deleteTarget = null; historyTarget = null; pinnerTarget = null }
    // Each pin is its own card: a gap keeps their washes from running together.
    LazyColumn(modifier.fillMaxWidth(), contentPadding = PaddingValues(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        state.messageContextError?.let { error -> item { Text(error, Modifier.padding(horizontal = 18.dp, vertical = 8.dp), color = ErrorText) } }
        if (state.displayedPins.isEmpty()) item {
            Text("No pinned messages.", Modifier.fillMaxWidth().padding(24.dp), color = TextMuted, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        }
        items(state.displayedPins, key = { "pin:${it.id}" }) { message ->
            val goToMessage: () -> Unit = {
                if (!state.loadingMessageContext) {
                    pinnerTarget = null
                    viewModel.goToMessage(message, onLoaded = close)
                }
            }
            Column(Modifier.clip(MaterialTheme.shapes.small).background(if (message.pin?.author?.id == (state.account?.id ?: state.chatAuthorId)) Terracotta.copy(alpha = 0.12f) else PinGoldWash)) {
                ReactionMessageRow(message, state, viewModel::setReaction, viewModel::retryReaction, viewModel::dismissReactionError,
                    openReactors = { target, emoji -> reactorsTarget = target.id to emoji }, openActions = { actionTarget = it },
                    openPicker = { pickerTarget = it },
                    inPins = true, openMessage = goToMessage,
                    openAuthor = { pinnerTarget = it },
                    openConversation = { target -> viewModel.openForwardedConversation(target, { conversationTarget = target }, close) }, openHistory = { historyTarget = it },
                    openReference = { reference -> viewModel.goToMessage(message.copy(id = reference.id, threadRootId = reference.threadRootId), onLoaded = close) },
                    retryPin = viewModel::retryPin, dismissPinError = viewModel::dismissPinError,
                    onAttachmentFailed = viewModel::reportAttachmentFailure)
                Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = 18.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton({ actionTarget = message }) { Icon(painterResource(R.drawable.lucide_ellipsis), "Message actions for ${message.author.name}", tint = TextMuted) }
                    Spacer(Modifier.weight(1f))
                    OutlinedButton(goToMessage, Modifier.heightIn(min = 44.dp), enabled = !state.loadingMessageContext, shape = MaterialTheme.shapes.small,
                        border = BorderStroke(1.dp, Border), colors = neutralOutlinedButtonColors()) {
                        Text(if (state.loadingMessageContext) "Loading message…" else "Go to message")
                        Spacer(Modifier.width(6.dp))
                        Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(14.dp))
                    }
                }
            }
        }
    }
    pinnerTarget?.let { author ->
        val card = authorCard(author, state)
        key(author.id) {
            MentionCardSheet(card, { done, failed -> card.username?.let { viewModel.messageMentioned(card.id, it, { pinnerTarget = null; close(); done() }, failed) } }, state = state, viewModel = viewModel,
                onMessageSent = { pinnerTarget = null; close() }) { pinnerTarget = null }
        }
    }
    actionTarget?.let { target ->
        val current = state.pinnedMessages.find { it.id == target.id } ?: target
        MessageActionsSheet(current, state, onDismiss = { actionTarget = null }, setReaction = viewModel::setReaction,
            openPicker = { actionTarget = null; pickerTarget = current },
            setPin = { id, active -> actionTarget = null; viewModel.setPin(id, active) },
            onReply = { actionTarget = null; close(); viewModel.replyInThread(current) },
            onReplyMessage = { actionTarget = null; close(); viewModel.replyTo(current) },
            forward = { actionTarget = null; forwardTarget = current },
            onEdit = if (viewModel.canEdit(current)) ({ actionTarget = null; editTarget = current }) else null,
            onDelete = if (viewModel.canDelete(current)) ({ actionTarget = null; deleteTarget = current }) else null,
            onHistory = { actionTarget = null; historyTarget = current },
            onViewReactions = { current.reactions.firstOrNull()?.let { reactorsTarget = current.id to it.emoji }; actionTarget = null })
    }
    pickerTarget?.let { target -> EmojiPicker({ pickerTarget = null }) { emoji -> pickerTarget = null; viewModel.setReaction(target.id, emoji, true) } }
    forwardTarget?.let { target -> key(target.id) { ForwardPickerSheet(target, viewModel) { forwardTarget = null } } }
    reactorsTarget?.let { (id, emoji) ->
        state.pinnedMessages.find { it.id == id }?.let { current ->
            ReactorsSheet(current, emoji, state.chatAuthorId ?: state.account?.id, state.account, viewModel::reactors, state.profileRevision) { reactorsTarget = null }
        }
    }
    conversationTarget?.let { target ->
        state.displayedPins.firstOrNull { it.id == target.id && it.deletedAt == null && it.forward?.message != null }?.let { current ->
            key(target.id) { ForwardConversationSheet(current, viewModel) { conversationTarget = null } }
        }
    }
    editTarget?.let { target ->
        if (viewModel.canEdit(state.pinnedMessages.find { it.id == target.id } ?: target)) MessageEditorDialog(target, viewModel) { editTarget = null }
        else LaunchedEffect(target.id) { editTarget = null }
    }
    deleteTarget?.let { target -> key(target.id) { DeleteMessageDialog(target, viewModel) { deleteTarget = null } } }
    historyTarget?.let { target ->
        val current = state.pinnedMessages.find { it.id == target.id } ?: target
        if (current.deletedAt == null) MessageHistoryDialog(current, viewModel) { historyTarget = null }
        else LaunchedEffect(target.id) { historyTarget = null }
    }
}

private val emojiPickerCategories = listOf(
    Triple("smileys_people", "Smileys & people", "😀"), Triple("animals_nature", "Animals & nature", "🐱"),
    Triple("food_drink", "Food & drink", "🍔"), Triple("travel_places", "Travel & places", "🚌"),
    Triple("activities", "Activities", "⚽"), Triple("objects", "Objects", "👕"),
    Triple("symbols", "Symbols", "🎵"), Triple("flags", "Flags", "🏁"),
)
/**
 * Picker order approximating web's: smileys (the emoticon block, then other faces) before people, then the other
 * categories with flags last. The catalog itself is in code point order; sorting is stable within each group.
 */
internal val emojiPickerOrder: Comparator<EmojiEntry> = compareBy<EmojiEntry>(
    { entry -> emojiPickerCategories.indexOfFirst { it.first == entry.category }.let { rank -> if (rank < 0) emojiPickerCategories.size else rank } },
    {
        val point = it.id.substringBefore('-').toIntOrNull(16) ?: 0
        when {
            it.category != "smileys_people" || point in 0x1f600..0x1f64f -> 0
            "face" in it.name || "face" in it.keywords.split(' ') -> 1
            else -> 2
        }
    },
)

@Composable internal fun EmojiPicker(onDismiss: () -> Unit, title: String = "Add reaction", select: (String) -> Unit) {
    Dialog(onDismissRequest = onDismiss) {
        Surface(shape = MaterialTheme.shapes.medium, color = SurfaceRaised, border = BorderStroke(1.dp, Border), modifier = Modifier.swipeCancelsPresses().fillMaxWidth().heightIn(min = 420.dp, max = 520.dp)) {
            EmojiPickerContent(onDismiss, title, select)
        }
    }
}

@Composable internal fun EmojiPickerContent(onDismiss: () -> Unit, title: String = "Insert emoji", select: (String) -> Unit) {
    val context = LocalContext.current
    // Picker order, as on web: smileys and people first, flags last.
    val catalog = remember { EmojiArtwork.catalog(context).filter { it.selectable }.sortedWith(emojiPickerOrder) }
    var query by rememberSaveable { mutableStateOf("") }
    var category by rememberSaveable { mutableStateOf(emojiPickerCategories.first().first) }
    val grid = rememberLazyGridState()
    LaunchedEffect(category, query) { grid.scrollToItem(0) }
    val shown = remember(query, category, catalog) {
        val needle = query.trim().lowercase(Locale.ROOT)
        if (needle.isEmpty()) catalog.filter { it.category == category }
        else catalog.filter { it.name.lowercase(Locale.ROOT).contains(needle) || it.keywords.lowercase(Locale.ROOT).contains(needle) || it.category.replace('_', ' ').contains(needle) }
    }
    Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) { Text(title, Modifier.weight(1f), fontWeight = FontWeight.Bold); TextButton(onDismiss, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Close") } }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            emojiPickerCategories.forEach { (id, name, icon) ->
                val active = query.isBlank() && category == id
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    IconButton({ category = id; query = "" }, Modifier.size(48.dp).semantics { contentDescription = name; selected = active }) {
                        EmojiImage(icon, null, Modifier.size(22.dp))
                    }
                    Box(Modifier.width(44.dp).height(2.dp).background(if (active) TerracottaBright else Color.Transparent))
                }
            }
        }
        OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth(), placeholder = { Text("Search emoji") }, singleLine = true)
        Text(if (query.isBlank()) emojiPickerCategories.first { it.first == category }.second else "Search results", color = TextMuted, fontWeight = FontWeight.Bold)
        if (shown.isEmpty()) Box(Modifier.fillMaxWidth().height(120.dp), contentAlignment = Alignment.Center) { Text("No emoji found.", color = TextMuted) }
        else LazyVerticalGrid(GridCells.Adaptive(48.dp), state = grid, modifier = Modifier.weight(1f)) {
            gridItems(shown, key = { it.id }) { entry ->
                IconButton({ select(entry.emoji) }, Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp).semantics { contentDescription = entry.name }) { EmojiImage(entry.emoji, null, Modifier.size(30.dp)) }
            }
        }
    }
}
@Composable private fun ReplyContext(reference: MessageReply, state: AppUiState, motionKey: String, onOpen: (() -> Unit)? = null) {
    val hidden = reference.author.id in state.blockedIds && reference.author.id != (state.chatAuthorId ?: state.account?.id)
    val text = if (reference.deleted) DELETED_REPLY else (reference.fullText ?: reference.text).ifEmpty { VoiceMessages.excerptLabel(reference.attachments, reference.hasAttachments) }
    var expanded by rememberSaveable(reference.id, hidden) { mutableStateOf(false) }
    var clipped by remember(reference.id, hidden, text) { mutableStateOf(false) }
    val context = LocalContext.current
    val still by produceState(false, context) {
        val resolver = context.contentResolver
        val setting = android.provider.Settings.Global.ANIMATOR_DURATION_SCALE
        fun disabled() = android.provider.Settings.Global.getFloat(resolver, setting, 1f) == 0f
        value = disabled()
        val observer = object : android.database.ContentObserver(android.os.Handler(android.os.Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { value = disabled() }
        }
        resolver.registerContentObserver(android.provider.Settings.Global.getUriFor(setting), false, observer)
        awaitDispose { resolver.unregisterContentObserver(observer) }
    }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current.density
    val canExpand = !hidden && (expanded || clipped)
    // Match the other clients: each replying message has stable spring settings,
    // including when several messages quote the same original.
    val seed = remember(motionKey) { motionKey.toByteArray(Charsets.UTF_8).fold(2166136261u) { hash, byte -> (hash xor byte.toUByte().toUInt()) * 16777619u } }
    val stiffness = 560f + (seed and 255u).toFloat() / 255f * 400f
    val damping = 28f + ((seed shr 8) and 255u).toFloat() / 255f * 14f
    val impulse = 1f + ((seed shr 16) and 255u).toFloat() / 255f * 0.4f
    val limit = 5f + (seed shr 24).toFloat() / 255f * 2f
    val speedLimit = limit * 16f
    var previousY by remember(motionKey) { mutableStateOf<Float?>(null) }
    var displacement by remember(motionKey) { mutableFloatStateOf(0f) }
    var velocity by remember(motionKey) { mutableFloatStateOf(0f) }
    var motion by remember(motionKey) { mutableStateOf<Job?>(null) }
    DisposableEffect(motionKey, hidden, still) {
        onDispose { motion?.cancel(); displacement = 0f; velocity = 0f }
    }
    val action = if (onOpen == null) Modifier else Modifier.clickable(role = Role.Button, onClickLabel = "View original message", onClick = onOpen)
    Column(Modifier.fillMaxWidth().onGloballyPositioned { coordinates ->
        val y = coordinates.positionInWindow().y / density
        val delta = (previousY ?: y) - y
        previousY = y
        if (!hidden && !still && delta != 0f) {
            velocity = (velocity + delta * impulse).coerceIn(-speedLimit, speedLimit)
            if (motion?.isActive != true) motion = scope.launch {
                var previousTime = withFrameNanos { it }
                while (isActive) {
                    val time = withFrameNanos { it }
                    val dt = ((time - previousTime) / 1_000_000_000f).coerceAtMost(0.032f)
                    previousTime = time
                    velocity -= (stiffness * displacement + damping * velocity) * dt
                    displacement = (displacement + velocity * dt).coerceIn(-limit, limit)
                    if (abs(displacement) < 0.01f && abs(velocity) < 0.05f) break
                }
                displacement = 0f; velocity = 0f
            }
        }
    }.drawWithContent {
        drawContent()
        if (hidden) return@drawWithContent
        val sourceX = 74.dp.toPx(); val sourceY = 24.dp.toPx()
        val targetX = 18.dp.toPx(); val bottom = size.height + 27.dp.toPx()
        val angle = displacement * 0.025f
        val dx = 12.dp.toPx() * cos(angle); val dy = 12.dp.toPx() * sin(angle)
        val start = Offset(targetX - dx, bottom + dy)
        val end = Offset(sourceX - dx, sourceY - dy)
        val span = start.y - end.y
        val offset = displacement.dp.toPx()
        val path = Path().apply {
            moveTo(start.x, start.y)
            // Keep the lower bend inward of the cup back throughout its sway.
            cubicTo(start.x + 8.dp.toPx() + offset, start.y - span * 0.3f,
                end.x - 30.dp.toPx() + offset, end.y + span * 0.3f, end.x, end.y)
        }
        drawPath(path, TextMuted.copy(alpha = 0.75f), style = Stroke(1.1.dp.toPx(), cap = StrokeCap.Round))
        replySocket(Offset(sourceX, sourceY), angle * 180f / kotlin.math.PI.toFloat())
        replySocket(Offset(targetX, bottom), -angle * 180f / kotlin.math.PI.toFloat())
    }.padding(start = 62.dp, end = 18.dp, top = 4.dp, bottom = 8.dp)) {
        Surface(color = MaterialTheme.colorScheme.surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border.copy(alpha = 0.7f))) {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 10.dp)) {
                Column(Modifier.fillMaxWidth().then(action).heightIn(min = 44.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Row(Modifier.heightIn(min = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (!hidden) {
                            Avatar(reference.author.name, 16.dp, avatarId = reference.author.avatarId, avatarAsset = reference.author.avatarAsset, avatarUrl = reference.author.avatarUrl)
                            Spacer(Modifier.width(6.dp))
                        }
                        Text(if (hidden) "Blocked message" else reference.author.name, Modifier.weight(1f), color = TextMuted, fontWeight = FontWeight.Bold, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (onOpen != null) { Spacer(Modifier.width(8.dp)); Text("View original", color = TextMuted, fontSize = 11.sp) }
                    }
                    if (!hidden) Text(text, color = TextMuted, fontSize = 12.sp, lineHeight = 18.sp, fontStyle = if (reference.deleted) FontStyle.Italic else null,
                        maxLines = if (expanded) Int.MAX_VALUE else 3, overflow = TextOverflow.Ellipsis,
                        onTextLayout = { if (!expanded) clipped = it.hasVisualOverflow })
                    if (!hidden && !reference.deleted && reference.attachments.isNotEmpty()) ReplyAttachmentPreviews(reference.attachments)
                }
                if (canExpand) {
                    TextButton({ expanded = !expanded }, Modifier.heightIn(min = 48.dp),
                        contentPadding = PaddingValues(0.dp), colors = neutralTextButtonColors()) {
                        Text(if (expanded) "Show less" else "Show more", color = LinkBlue, fontSize = 11.sp, textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline)
                    }
                }
            }
        }
    }
}

private fun DrawScope.replySocket(position: Offset, angle: Float) {
    withTransform({ translate(position.x, position.y); rotate(angle, Offset.Zero); scale(this@replySocket.density, this@replySocket.density, Offset.Zero) }) {
        val cup = Path().apply {
            moveTo(-12f, -2.4f); quadraticTo(-13f, 0f, -12f, 2.4f)
            lineTo(-1f, 4.5f); lineTo(-1f, -4.5f); close()
        }
        drawPath(cup, Brush.verticalGradient(0f to Color(0xFF646B70), 0.32f to Color(0xFFA2A9AD), 0.55f to Color(0xFF70787D), 1f to Color(0xFF30363A), startY = -4.5f, endY = 4.5f))
        drawPath(cup, Border, style = Stroke(0.6f))
        drawLine(Color(0xFFC3C8CB).copy(alpha = 0.35f), Offset(-11f, -1.6f), Offset(-3f, -3f), strokeWidth = 0.6f)
        drawOval(Color(0xFF252A2D), Offset(-2.6f, -4.5f), Size(3.2f, 9f))
        drawOval(Terracotta, Offset(-2.6f, -4.5f), Size(3.2f, 9f), style = Stroke(0.8f))
        drawOval(Color(0xFF151719), Offset(-1.7f, -3.4f), Size(1.4f, 6.8f))
    }
}

@Composable private fun ReplyDraft(reference: MessageReply, state: AppUiState, onCancel: () -> Unit) {
    val hidden = reference.author.id in state.blockedIds && reference.author.id != (state.chatAuthorId ?: state.account?.id)
    Row(Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite }.drawBehind {
        drawLine(Terracotta, Offset(1.dp.toPx(), 0f), Offset(1.dp.toPx(), size.height), strokeWidth = 2.dp.toPx())
    }.padding(start = 10.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.Top) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text("Replying to ${if (hidden) "blocked message" else reference.author.name}", color = TextMuted, fontSize = 12.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!hidden) Text((reference.fullText ?: reference.text).ifEmpty { VoiceMessages.excerptLabel(reference.attachments, reference.hasAttachments) },
                color = TextMuted, fontSize = 12.sp, lineHeight = 18.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        IconButton(onCancel, Modifier.size(48.dp)) {
            Icon(painterResource(R.drawable.lucide_x), "Cancel reply", Modifier.size(16.dp), tint = TextMuted)
        }
    }
}

internal const val DELETED_MESSAGE = "This message was deleted."
internal const val DELETED_REPLY = "Original message was deleted."

/** A delivered message: its files on fresh URLs with their live processing state, and its resolved mentions. */
@Composable private fun MessageRow(
    message: ChatMessage, state: AppUiState, onAttachmentFailed: (ChatAttachment, Int?) -> Unit = { _, _ -> },
    openMention: ((MessageMention) -> Unit)? = null, mentionLabel: (MessageMention) -> String = { "@${it.username}" },
    openAuthor: ((ChatAuthor) -> Unit)? = null,
    grouped: Boolean = false, openHistory: () -> Unit = {},
) = MessageRow(
    message.author.name, message.author.isGuest, message.createdAt, message.content.text, false, message.author.avatarId,
    message.content.attachments.map { it.withFreshUrls(state.freshAttachmentUrls[it.id]) }, onAttachmentFailed,
    state.attachmentProgress, state.localAttachmentPreviews,
    edited = message.forward == null && message.revision > 1 && message.deletedAt == null, openHistory = openHistory,
    mentions = highlightedMentions(message.content.text, message.content.mentions), entries = message.content.mentions,
    openMention = openMention, mentionLabel = mentionLabel, grouped = grouped,
    openAuthor = if (message.author.isGuest || openAuthor == null) null else ({ openAuthor(message.author) }),
    admin = message.author.admin,
    avatarAsset = message.author.avatarAsset, avatarUrl = message.author.avatarUrl,
    deleted = message.deletedAt != null,
    embeds = message.content.embeds,
    gif = message.content.gif,
)
@Composable private fun MessageRow(
    author: String, guest: Boolean, createdAt: String, text: String, pending: Boolean, avatarId: Int? = null,
    attachments: List<ChatAttachment> = emptyList(), onAttachmentFailed: (ChatAttachment, Int?) -> Unit = { _, _ -> },
    attachmentProgress: Map<String, Int> = emptyMap(), localAttachmentPreviews: Map<String, String> = emptyMap(),
    edited: Boolean = false, openHistory: () -> Unit = {}, mentions: List<MentionSpan> = emptyList(), entries: List<MessageMention> = emptyList(),
    openMention: ((MessageMention) -> Unit)? = null, mentionLabel: (MessageMention) -> String = { "@${it.username}" },
    /** No avatar or header under the same person's previous message; the time shows on pointer hover. */
    grouped: Boolean = false,
    openAuthor: (() -> Unit)? = null,
    /** A feedback team member's message (`author.admin`): an ADMIN badge after the name. */
    admin: Boolean = false,
    /** The author's custom picture, over their [avatarId] mascot. */
    avatarAsset: String? = null, avatarUrl: String? = null,
    /** The author deleted it: a muted note in place of its text and files. */
    deleted: Boolean = false,
    /** Link preview cards, under the files (`EmbedViews.kt`). */
    embeds: List<ChatEmbed> = emptyList(),
    /** A KLIPY GIF or sticker, under the text. */
    gif: KlipyGif? = null,
) {
    val hover = remember { MutableInteractionSource() }
    val hovered by hover.collectIsHoveredAsState()
    val context = LocalContext.current
    // The system's 24-hour setting, as the status bar clock shows it.
    val hour24 = remember(context) { android.text.format.DateFormat.is24HourFormat(context) }
    // TalkBack keeps reading the name; the click label names the action.
    val profile = if (openAuthor == null) Modifier else Modifier
        .clickable(role = Role.Button, onClickLabel = "Open profile for $author", onClick = openAuthor)
    // A round ripple and a 48dp touch target that spills into the row's padding,
    // so the layout still matches grouped rows. TalkBack uses the name instead.
    val avatarProfile = if (openAuthor == null) Modifier else Modifier
        .size(34.dp).wrapContentSize(unbounded = true).minimumInteractiveComponentSize()
        .clearAndSetSemantics {}.clip(CircleShape)
        .clickable(role = Role.Button, onClickLabel = "Open profile for $author", onClick = openAuthor)
    Row(Modifier.fillMaxWidth().hoverable(hover).padding(start = 18.dp, end = 18.dp, top = if (grouped) 2.dp else 10.dp, bottom = 10.dp)) {
        if (grouped) Box(
            // The empty avatar column still names the author and time for screen readers.
            Modifier.width(34.dp).clearAndSetSemantics {
                contentDescription = listOfNotNull(author, ADMIN_BADGE.takeIf { admin }, "Guest".takeIf { guest }, timeLabel(createdAt, hour24).ifEmpty { null }).joinToString(", ")
            },
            contentAlignment = Alignment.TopCenter,
        ) {
            if (hovered) Text(shortTimeLabel(createdAt, hour24), color = TextMuted, fontSize = 10.sp, lineHeight = 21.sp, maxLines = 1, softWrap = false)
        } else Avatar(author, 34.dp, modifier = avatarProfile, avatarId = avatarId, avatarAsset = avatarAsset, avatarUrl = avatarUrl)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            if (!grouped) Row(verticalAlignment = Alignment.CenterVertically) {
                // A long name gives way first, so the time and tags stay whole.
                Text(author, Modifier.weight(1f, fill = false).then(profile), fontSize = 13.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (admin) { Spacer(Modifier.width(7.dp)); AdminBadge() }
                if (guest) { Spacer(Modifier.width(7.dp)); Surface(color = Color.Transparent, border = BorderStroke(1.dp, Border), shape = MaterialTheme.shapes.extraSmall) { Text("GUEST", Modifier.padding(horizontal = 5.dp, vertical = 2.dp), color = TextMuted, fontSize = 8.sp, fontWeight = FontWeight.Bold) } }
                Spacer(Modifier.width(7.dp)); Text(timeLabel(createdAt, hour24), color = TextMuted, fontSize = 10.sp)
                if (edited) { Spacer(Modifier.width(7.dp)); Text("(edited)", Modifier.clickable(onClickLabel = "Open message history", onClick = openHistory), color = TextMuted, fontSize = 10.sp) }
            }
            // A grouped row's "(edited)" follows its text instead.
            val inlineEdited = grouped && edited
            // A file-only message has empty text: show just its files.
            if (deleted) Text(DELETED_MESSAGE, color = TextMuted, fontSize = 14.sp, lineHeight = 21.sp, fontStyle = FontStyle.Italic)
            else if (text.isNotEmpty() || inlineEdited) {
                // Pending messages have emphasis but no mention or link actions.
                val links = remember(text, mentions, pending) { if (pending) emptyList() else linkRanges(text, mentions.map { it.start until it.end }) }
                if (mentions.isEmpty() && links.isEmpty() && !inlineEdited) Text(messageText(text, emptyList()), color = if (pending) TextMuted else MessageText, fontSize = 14.sp, lineHeight = 21.sp)
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
                    MessageLinkText(
                        annotated,
                        if (people.isEmpty() && links.isEmpty() && !inlineEdited) Modifier else Modifier.semantics {
                            customActions = people.map { user -> CustomAccessibilityAction("Open profile for ${mentionLabel(user)}") { open?.invoke(user); true } } +
                                links.map { link -> CustomAccessibilityAction("Open link ${text.substring(link.start, link.end)}") { openExternalLink(uriHandler, link.href); true } } +
                                listOfNotNull(if (inlineEdited) CustomAccessibilityAction("Open message history") { history(); true } else null)
                        },
                        color = if (pending) TextMuted else MessageText, fontSize = 14.sp, lineHeight = 21.sp,
                    )
                }
            }
            if (!deleted) gif?.let { MessageGif(it) }
            if (!deleted) MessageAttachments(
                attachments, pending, onAttachmentFailed, attachmentProgress, localAttachmentPreviews,
                caption = listOf(author, timeLabel(createdAt, hour24)).filter { it.isNotEmpty() }.joinToString(" · "),
            )
            if (!deleted) MessageEmbeds(embeds)
        }
    }
}

/** Static, noninteractive placeholders shared by channel and thread loading. */
@Composable private fun MessageSkeleton(modifier: Modifier = Modifier, rows: Int = 5, label: String = "Loading messages") {
    Column(modifier.fillMaxWidth().clearAndSetSemantics {
        contentDescription = label
        progressBarRangeInfo = ProgressBarRangeInfo.Indeterminate
    }) {
        repeat(rows) { index ->
            Row(Modifier.fillMaxWidth().height(70.dp).padding(horizontal = 18.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Box(Modifier.size(34.dp).background(Border, CircleShape))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Box(Modifier.width(96.dp).height(12.dp).background(Border, RoundedCornerShape(4.dp)))
                    Box(Modifier.fillMaxWidth(if (index % 2 == 0) 0.76f else 0.54f).height(10.dp).background(Border, RoundedCornerShape(4.dp)))
                }
            }
        }
    }
}

/**
 * Why Attach is disabled, or null when it can be used, worded as on every client: checking while upload settings
 * or the conversation load, unavailable once either fails, and the attachment limit.
 */
internal fun attachDisabledReason(state: AppUiState, target: AttachmentDraftTarget? = state.selectedChannel?.id?.let { AttachmentDraftTarget(it) }): String? = when {
    (state.uploadsChecked && !state.uploadsEnabled) || state.messagesError != null || state.sessionError != null -> "File uploads are unavailable"
    target?.threadRootId != null && state.thread?.error != null -> "File uploads are unavailable"
    !state.uploadsEnabled || state.chatAuthorId == null || state.messagesLoading || (target?.threadRootId != null && state.thread?.loading == true) -> "Checking file uploads…"
    state.attachmentDrafts[target].orEmpty().size >= AttachmentPolicy.MAX_ATTACHMENTS -> "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files."
    else -> null
}

/** Replaces the field's selection (or inserts at the caret), within the 4,000-character limit. */
internal fun insertEmojiAtSelection(value: TextFieldValue, emoji: String): TextFieldValue? =
    insertEmoji(value, EmojiToken(value.selection.min, value.selection.max, ""), emoji)

/** A composer's emoji control: the emoji picker, inserting at the field's selection. */
@Composable internal fun EmojiInsertButton(
    enabled: Boolean, interaction: (Boolean) -> Unit = {},
    load: (suspend (chat.caper.android.data.KlipyKind, String, Int, chat.caper.android.data.KlipyAdRequest?) -> chat.caper.android.data.KlipyPage)? = null,
    mediaAvailable: Boolean = false,
    sendMedia: (chat.caper.android.model.KlipyGif, chat.caper.android.data.KlipyKind, String) -> Boolean = { _, _, _ -> false },
    insert: (String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    IconButton({ open = true; interaction(true) }, Modifier.size(48.dp).semantics { contentDescription = "Emojis, GIFs and stickers" }, enabled = enabled) {
        Icon(painterResource(R.drawable.lucide_smile), null, Modifier.size(20.dp), tint = if (enabled) TextMuted else TextMuted.copy(alpha = 0.4f))
    }
    if (open) ComposerMediaPicker(load, mediaAvailable, { open = false; interaction(false) }, { gif, kind, query -> enabled && sendMedia(gif, kind, query) }) { emoji ->
        open = false; interaction(false)
        if (enabled) insert(emoji)
    }
}

/**
 * The composer's attach control. With a [drawer] it opens the attach drawer (photo library, camera,
 * Files and Send), which stays openable at the attachment limit to explain it; otherwise a menu of
 * the system photo picker, the document picker and Paste.
 */
@Composable internal fun AttachButton(
    disabledReason: String?, add: (List<android.net.Uri>) -> Unit, interaction: (Boolean) -> Unit = {},
    /** Imports files copied to the clipboard, where a composer supports it. */
    paste: (() -> Unit)? = null,
    /** Conversation launchers live outside transient thread panels and retain their original target. */
    pick: ((Boolean) -> Unit)? = null,
    drawer: AttachDrawerHost? = null,
) {
    val limitReason = "You can attach up to ${AttachmentPolicy.MAX_ATTACHMENTS} files."
    val enabled = disabledReason == null || (drawer != null && disabledReason == limitReason)
    var open by remember { mutableStateOf(false) }
    var picking by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(picking) { interaction(open || picking) }
    val media = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(AttachmentPolicy.MAX_ATTACHMENTS)) {
        add(it); picking = false; interaction(open)
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        add(it); picking = false; interaction(open)
    }
    // The camera writes into a private file; saved so a capture survives the activity being recreated.
    val context = LocalContext.current
    var capture by rememberSaveable { mutableStateOf<String?>(null) }
    val captured: (Boolean) -> Unit = { ok ->
        capture?.let { if (ok) add(listOf(android.net.Uri.parse(it))) }
        capture = null; picking = false; interaction(open)
    }
    val takePhoto = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture(), captured)
    val recordVideo = rememberLauncherForActivityResult(ActivityResultContracts.CaptureVideo(), captured)
    fun launch(photos: Boolean) {
        if (pick != null) { pick(photos); return }
        picking = true
        runCatching {
            if (photos) media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
            else documents.launch(arrayOf("*/*"))
        }.onFailure { picking = false; interaction(open) }
    }
    if (drawer != null) {
        // Prepare the gallery while the composer is up, so the drawer opens filled.
        LaunchedEffect(Unit) { AttachDrawerCache.warm(context) }
        IconButton({ open = true; interaction(true) }, Modifier.size(48.dp).semantics {
            contentDescription = "Attach files"
            (disabledReason?.takeIf { it != limitReason } ?: AttachCapacity.hint(drawer.files.size))?.let { stateDescription = it }
        }, enabled = enabled) {
            Icon(painterResource(R.drawable.lucide_plus), null, Modifier.size(20.dp),
                tint = if (!enabled) TextMuted.copy(alpha = 0.4f) else if (open) Text else TextMuted)
        }
        if (open && enabled) AttachDrawer(
            drawer, add,
            gallery = { open = false; launch(true) },
            files = { open = false; launch(false) },
            camera = { video ->
                val uri = cameraCaptureUri(context, video)
                if (uri != null) {
                    capture = uri.toString(); picking = true
                    runCatching { if (video) recordVideo.launch(uri) else takePhoto.launch(uri) }
                        .onFailure { capture = null; picking = false }
                }
            },
            onDismiss = { open = false; interaction(picking) },
        )
        return
    }
    Box {
        IconButton({ open = true; interaction(true) }, Modifier.size(48.dp).semantics {
            contentDescription = "Attach files"
            if (disabledReason != null) stateDescription = disabledReason
        }, enabled = enabled) {
            Icon(painterResource(R.drawable.lucide_plus), null, Modifier.size(20.dp), tint = if (enabled) TextMuted else TextMuted.copy(alpha = 0.4f))
        }
        DropdownMenu(open, { open = false; interaction(picking) }, containerColor = SurfaceRaised, border = BorderStroke(1.dp, Border)) {
            DropdownMenuItem(text = { Text("Photos and videos", fontSize = 13.sp) }, onClick = {
                if (pick != null) { open = false; pick(true); return@DropdownMenuItem }
                open = false; picking = true
                runCatching { media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) }
                    .onFailure { picking = false; interaction(false) }
            })
            DropdownMenuItem(text = { Text("Files", fontSize = 13.sp) }, onClick = {
                if (pick != null) { open = false; pick(false); return@DropdownMenuItem }
                open = false; picking = true
                runCatching { documents.launch(arrayOf("*/*")) }
                    .onFailure { picking = false; interaction(false) }
            })
            paste?.let { paste ->
                DropdownMenuItem(text = { Text("Paste from clipboard", fontSize = 13.sp) }, onClick = {
                    open = false; interaction(false)
                    paste()
                })
            }
        }
    }
}

/**
 * Message text: resolved mentions as pills (text #F3F4F5, one weight step bolder, terracotta at 24%),
 * then [links] (found outside the mentions, `data/Links.kt`) in baby-blue rounded pills.
 * Inline icon placeholders are presentation only; message actions use the original content text.
 * URL backgrounds are drawn by [MessageLinkText]; mention backgrounds remain span styles.
 * [openHistory] appends a grouped row's "(edited)" marker right after the text.
 */
internal fun messageText(
    text: String, mentions: List<MentionSpan>, entries: List<MessageMention>? = null, links: List<LinkRange> = emptyList(),
    open: (MessageMention) -> Unit = {}, openLink: (String) -> Unit = {}, openHistory: (() -> Unit)? = null,
) = buildAnnotatedString {
    val formatted = chat.caper.android.data.formattedMessage(text)
    val rendered = formatted.text
    val renderedLinks = links.map { it.copy(start = formatted.displayed(it.start), end = formatted.displayed(it.end)) }
    var cursor = 0
    renderedLinks.forEach { link ->
        append(rendered.substring(cursor, link.end))
        appendInlineContent(ExternalLinkInline)
        cursor = link.end
    }
    append(rendered.substring(cursor))
    fun renderedOffset(offset: Int) = offset + renderedLinks.count { it.end <= offset }
    fun displayed(offset: Int) = renderedOffset(formatted.displayed(offset))
    mentions.forEach { span ->
        // With [entries], a person's pill is a link (focusable, Enter-activatable; 32% while hovered, focused or pressed).
        val user = entries?.let { mentionedUser(span, it) }
        if (user == null) addStyle(MentionPill, displayed(span.start), displayed(span.end))
        else addLink(LinkAnnotation.Clickable("mention:${span.name}", MentionPillLink) { open(user) }, displayed(span.start), displayed(span.end))
    }
    // Only http(s) URLs reach here, and they open outside the app.
    links.forEach { link -> addLink(LinkAnnotation.Url(link.href, MessageLink) { openLink(link.href) }, displayed(link.start), displayed(link.end)) }
    var runOffset = 0
    formatted.runs.forEach { run ->
        addStyle(SpanStyle(
            fontWeight = if (run.bold) FontWeight.Bold else null,
            fontStyle = if (run.italic) androidx.compose.ui.text.font.FontStyle.Italic else null,
        ), renderedOffset(runOffset), renderedOffset(runOffset + run.text.length))
        runOffset += run.text.length
    }
    openHistory?.let { history ->
        append(" ")
        withLink(LinkAnnotation.Clickable("edited", EditedMarker) { history() }) { append("(edited)") }
    }
}

private val MentionPill = SpanStyle(color = Text, fontWeight = FontWeight.Medium, background = Terracotta.copy(alpha = 0.24f))
private val MentionPillActive = SpanStyle(background = Terracotta.copy(alpha = 0.32f))
private val MentionPillLink = TextLinkStyles(MentionPill, focusedStyle = MentionPillActive, hoveredStyle = MentionPillActive, pressedStyle = MentionPillActive)
internal const val ExternalLinkInline = "external-link"
@Composable internal fun messageInlineContent() = mapOf(ExternalLinkInline to InlineTextContent(
    Placeholder(14.sp, 12.sp, PlaceholderVerticalAlign.TextCenter),
) {
    Icon(painterResource(R.drawable.lucide_external_link), null, Modifier.fillMaxSize().padding(start = 3.dp), tint = LinkBlue)
})
private val MessageLinkActive = SpanStyle(color = LinkBlueActive, textDecoration = TextDecoration.Underline)
internal val MessageLink = TextLinkStyles(
    SpanStyle(color = LinkBlue),
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
    if (shown.isNotEmpty()) {
        Row(Modifier.fillMaxWidth().height(20.dp).graphicsLayer { this.alpha = alpha }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            TypingDots()
            Text(shown, color = TextMuted, fontSize = 10.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Web's three 4px dots: 1.2 s bounce staggered by .15 s, still when animations are off. */
@Composable private fun TypingDots() {
    val still = reducedMotion()
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
    count >= 3500 -> PinGold
    else -> TextMuted
}

/**
 * A person's avatar: their custom picture ([avatarAsset]/[avatarUrl], see [pictureOf]) aspect-filled
 * in a rounded square ([PictureShape]) over the saved mascot ([avatarId]), which shows (in its
 * circle) while the picture loads, when it fails and when there is none; initials only without a
 * valid mascot. The speaking ring and an [outline] (voice stacks) follow whichever shape shows;
 * presence dots stay outside, at the bottom-right corner. [voiceRing] (your call's faces) draws the speaking
 * ring as a 2dp gap in the surrounding colour and 2dp of voice green, outside the avatar.
 */
@Composable internal fun Avatar(
    name: String, size: Dp, modifier: Modifier = Modifier, avatarId: Int? = null, speaking: Boolean = false,
    avatarAsset: String? = null, avatarUrl: String? = null, outline: BorderStroke? = null, voiceRing: Boolean = false,
) {
    val index = caperAvatarIndex(avatarId)
    val picture = pictureOf(avatarAsset, avatarUrl)
    // The picture is on screen: transparent pixels in it must not reveal the mascot.
    var pictureShown by remember(picture) { mutableStateOf(false) }
    val shape = if (pictureShown) PictureShape else CircleShape
    Box(
        modifier
            // Web: caper-green border with a soft outer ring while speaking.
            .size(size)
            .then(if (speaking) Modifier.drawBehind {
                // Your call's faces: the gap shows the surface behind, then a solid voice-green ring.
                val ring = if (voiceRing) VoiceSessionGreen else CaperGreen.copy(alpha = .2f)
                val stroke = (if (voiceRing) 2.dp else 3.dp).toPx()
                val gap = if (voiceRing) 2.dp.toPx() + stroke / 2 else 1.5.dp.toPx()
                if (pictureShown) {
                    val outer = Size(this.size.width + gap * 2, this.size.height + gap * 2)
                    drawRoundRect(ring, topLeft = Offset(-gap, -gap), size = outer,
                        cornerRadius = androidx.compose.ui.geometry.CornerRadius(outer.minDimension * .3f), style = Stroke(stroke))
                } else drawCircle(ring, radius = this.size.minDimension / 2 + gap, style = Stroke(stroke))
            } else Modifier)
            .then(if (outline != null) Modifier.border(outline, shape) else Modifier)
            .clip(shape)
            // Bundled image avatars retain their transparent backing. Initials
            // still need contrast against every surface.
            .then(if (index == null) Modifier.background(if (size > 32.dp) SurfaceRaised else SurfaceComposer) else Modifier)
            .then(if (speaking && !voiceRing) Modifier.border(2.dp, CaperGreen, shape) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        if (!pictureShown) {
            if (index == null) Text(initialOf(name), fontWeight = FontWeight.Black, fontSize = (size.value * .38f).sp)
            else {
                Image(
                    painter = painterResource(caperAvatarResources[index]),
                    contentDescription = null,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
        if (picture != null) PictureImage(picture, size, Modifier.fillMaxSize()) { pictureShown = it }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable private fun AuthFrame(content: @Composable ColumnScope.() -> Unit) {
    // The keyboard shrinks this frame (the root pads for it), and sign-in keeps the field focused
    // after a failed request. Scroll rather than clip, and keep the end of the form (its error and
    // primary button) above the keyboard; otherwise the button sits hidden behind it.
    val scroll = rememberScrollState()
    val keyboard = WindowInsets.isImeVisible
    LaunchedEffect(keyboard, scroll.maxValue) { if (keyboard) scroll.scrollTo(scroll.maxValue) }
    BoxWithConstraints(Modifier.fillMaxSize().background(Blackout).verticalScroll(scroll)) {
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

/**
 * Web's one primary button for sign-in and the profile (account/controls.tsx): compact, with a
 * spinner beside the pending label and a minimum width so that label never resizes it. Disabled
 * or pending, it steps back to neutral chrome, as on web.
 */
@Composable private fun PrimaryButton(
    label: String, pendingLabel: String, pending: Boolean, enabled: Boolean, modifier: Modifier = Modifier,
    minWidth: Dp = 200.dp, contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp, vertical = 16.dp), onClick: () -> Unit,
) = Button(
    onClick, modifier.widthIn(min = minWidth), enabled = enabled && !pending, shape = MaterialTheme.shapes.small,
    colors = primaryButtonColors(),
    contentPadding = contentPadding,
) {
    if (pending) {
        CircularProgressIndicator(Modifier.size(16.dp), color = LocalContentColor.current, strokeWidth = 2.dp)
        Spacer(Modifier.width(12.dp))
        Text(pendingLabel)
    } else {
        Text(label)
        Spacer(Modifier.width(12.dp))
        Icon(painterResource(R.drawable.lucide_arrow_right), null, Modifier.size(20.dp))
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
        PrimaryButton("Email me a code", "Sending…", busy, email.contains('@'), Modifier.align(Alignment.End).padding(top = 12.dp)) { submit(email) }
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
        // Done submits a complete code, like Continue (and the email field's Send).
        OutlinedTextField(code, { code = it.uppercase().filter { character -> character in "ABCDEFGHJKMNPQRSTWXYZ23456789" }.take(6); if (error != null) clearError() }, singleLine = true,
            placeholder = { Text("e.g. ABC234") },
            readOnly = busy, enabled = !exhausted, modifier = Modifier.fillMaxWidth().focusRequester(codeFocus),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (code.length == 6 && !busy) submit(screen.challengeId, code) }))
        if (error != null) Surface(Modifier.fillMaxWidth().padding(top = 20.dp), color = Color.Transparent, border = BorderStroke(1.dp, Terracotta), shape = MaterialTheme.shapes.small) { Text(error, Modifier.padding(14.dp)) }
        if (screen.attemptsRemaining == 1) Text("One attempt left. Check the code carefully.", Modifier.padding(top = 12.dp), fontSize = 14.sp, fontWeight = FontWeight.Bold)
        // Shown after a resend, announced politely; the new challenge also cleared the field and the attempts.
        if (screen.resends > 0) Text(RESENT_CODE_STATUS, Modifier.padding(top = 12.dp).semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 14.sp)
        // "Email me a new code" counts as a resend too, and restarts the wait.
        // At the resend limit the API would send nothing, so the button is hidden (as on web).
        // Only the request that is running says so: a resend leaves Continue as it was.
        if (exhausted) { if (screen.resends < MAX_CODE_RESENDS) PrimaryButton("Email me a new code", "Sending…", busy, true,
            Modifier.align(Alignment.End).padding(top = 12.dp), minWidth = 232.dp, onClick = sendNewCode) }
        else PrimaryButton("Continue", "Checking…", busy && !resending, code.length == 6 && !busy,
            Modifier.align(Alignment.End).padding(top = 12.dp)) { submit(screen.challengeId, code) }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(20.dp), itemVerticalAlignment = Alignment.CenterVertically) {
            TextButton(back, enabled = !busy, contentPadding = PaddingValues(vertical = 16.dp), shape = MaterialTheme.shapes.small) { Text("Use a different email", color = TextMuted) }
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
                TextButton(sendNewCode, enabled = ready, contentPadding = PaddingValues(vertical = 16.dp), shape = MaterialTheme.shapes.small) {
                    Text(if (resending) "Sending…" else resendLabel(secondsLeft), color = if (ready) TextMuted else TextMuted.copy(alpha = 0.6f), style = TabularNumbers)
                }
            }
        }
    }
}

/** [picture] (Edit profile only) shows and changes your profile picture above the fields. */
@Composable internal fun ProfileScreen(
    account: Account, busy: Boolean, error: String?, close: (() -> Unit)?, picture: (@Composable () -> Unit)? = null,
    submit: (String, String) -> Unit,
) {
    var username by remember(account.id) { mutableStateOf(account.username.orEmpty()) }
    var name by remember(account.id) { mutableStateOf(account.displayName.orEmpty()) }
    // `error` is app-wide: Edit profile shows only its own save's failure, never an earlier one.
    var submitted by remember { mutableStateOf(close == null) }
    val form: @Composable ColumnScope.() -> Unit = {
        if (close == null) Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("ONE LAST THING", color = TextMuted, fontSize = 11.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.5.sp)
            Text("Choose how you show up.", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
        }
        if (picture != null) { picture(); HorizontalDivider(color = Border) }
        // Web's field hints (account/ProfileForm.tsx).
        OutlinedTextField(username, { username = normalizeUsername(it) }, label = { Text("Username") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ImeAction.Next),
            supportingText = { Text("Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores.", color = TextMuted) })
        val nameError = displayNameError(name, account.displayName)
        OutlinedTextField(name, { name = it.codePointTake(64) }, label = { Text("Display name") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            isError = nameError != null,
            supportingText = { Text(nameError ?: "This is what people see in conversations.", color = if (nameError != null) ErrorText else TextMuted) })
        Box(Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            error?.takeIf { submitted }?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        }
        PrimaryButton(if (account.username.isNullOrEmpty()) "Finish setup" else "Save profile", "Saving…", busy, profileValid(username, name, account.displayName),
            Modifier.align(Alignment.End), minWidth = if (close == null) 200.dp else 148.dp,
            contentPadding = PaddingValues(horizontal = if (close == null) 20.dp else 16.dp, vertical = if (close == null) 16.dp else 8.dp)) { submitted = true; submit(username, name) }
    }
    // Onboarding spaces the form as the Edit profile dialog does.
    if (close == null) AuthFrame { Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(14.dp)) { form() } }
    else CaperDialog("Edit profile", close) { form() }
}

@Composable private fun FirstSpaceScreen(state: AppUiState, viewModel: CaperViewModel) {
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
        OutlinedTextField(name, { name = it.codePointTake(80); error = null }, label = { Text("Space name") }, placeholder = { Text("Gaming nights") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        if (!allowed) Text("You have reached your space limit.", color = TextMuted, fontSize = 12.sp)
        (error ?: state.error)?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        Spacer(Modifier.height(16.dp))
        Button({ spaceNameError(name)?.let { error = it } ?: viewModel.createSpace(name.trim()) },
            enabled = allowed && !state.busy && name.isNotBlank(), modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.small, colors = primaryButtonColors(),
        ) { Text(if (state.busy) "Creating…" else "Create space") }
        TextButton(viewModel::logout, Modifier.align(Alignment.End), shape = MaterialTheme.shapes.small) { Text("Log out", color = TextMuted) }
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
        // Web validates on submit and says why (spaces/client.ts spaceNameError).
        fun submit() { spaceNameError(name)?.let { error = it } ?: run { submitted = true; create(name.trim()) } }
        OutlinedTextField(name, { name = it.codePointTake(80); error = null; if (submitted && serverError != null) clearError() }, label = { Text("Space name") }, placeholder = { Text("Gaming nights") },
            modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { if (!busy) submit() }))
        (error ?: serverError?.takeIf { submitted })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        DialogActions(close, "Create space", busy, pendingLabel = "Creating…") { submit() }
    }
}

/** [people] suggests usernames (`/api/people`, refreshed by [loadPeople] on open); the lookup stays exact. */
@Composable private fun StartDirectDialog(
    busy: Boolean, people: List<MentionCandidate>, loadPeople: () -> Unit, close: () -> Unit,
    lookup: (String, (Person) -> Unit, (String) -> Unit) -> Unit, start: (String, (String) -> Unit) -> Unit,
) {
    var username by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var resolved by remember { mutableStateOf<Person?>(null) }
    val recipientName = resolved?.displayName ?: people.firstOrNull { it.username == username }?.label
    LaunchedEffect(Unit) { loadPeople() }
    CaperDialog("New direct message", close, busy = busy) {
        val field = remember { FocusRequester() }
        LaunchedEffect(Unit) { runCatching { field.requestFocus() } }
        Text("Enter an account’s exact username.", color = TextMuted, fontSize = 12.sp)
        // The field is read-only while the conversation opens, so suggestions wait too.
        UsernameSuggestions(username, { username = it; error = null; resolved = null }, if (busy) emptyList() else people) { suggestions, imeAction ->
            OutlinedTextField(username, { username = normalizeUsername(it); error = null; resolved = null }, label = { Text("Username") }, placeholder = { Text("username") }, readOnly = busy,
                modifier = Modifier.fillMaxWidth().focusRequester(field).then(suggestions), singleLine = true,
                keyboardActions = KeyboardActions(onDone = { imeAction { defaultKeyboardAction(ImeAction.Done) } }))
        }
        Box(Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            if (error != null) Text(error!!, color = ErrorText, fontSize = 12.sp)
            else recipientName?.let { Text("$it (@${resolved?.username ?: username})", color = TextMuted, fontSize = 12.sp) }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            TextButton(close, enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
            Button({
                if (!Regex("^[a-z0-9_]{3,32}$").matches(username)) error = "Enter an exact valid username."
                else {
                    error = null
                    if (recipientName != null) start(resolved?.username ?: username) { error = it }
                    else lookup(username, { resolved = it }, { error = it })
                }
            }, enabled = !busy && Regex("^[a-z0-9_]{3,32}$").matches(username), shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) {
                PendingLabel(if (recipientName != null) "Open conversation" else "Find person", if (recipientName != null) "Opening…" else "Finding…", busy)
            }
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
        fun submit() { channelNameError(name.removeSuffix("-"))?.let { error = it } ?: run { submitted = true; create(name.removeSuffix("-"), private) } }
        OutlinedTextField(name, { name = normalizeChannel(it); error = null; if (submitted && serverError != null) clearError() }, label = { Text("Channel name") }, placeholder = { Text("project-updates") },
            leadingIcon = { Icon(painterResource(if (private) R.drawable.lucide_lock_keyhole else R.drawable.lucide_hash), null, Modifier.size(18.dp)) },
            modifier = Modifier.fillMaxWidth().focusRequester(field), singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (!busy && name.removeSuffix("-").isNotEmpty()) submit() }))
        Text("Channels are where conversations happen around a topic. Use a name that is easy to find and understand.", color = TextMuted, fontSize = 12.sp, lineHeight = 18.sp)
        PrivacyToggle(private, detail.space.name) { private = it }
        (error ?: serverError?.takeIf { submitted })?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        DialogActions(close, "Create channel", busy, pendingLabel = "Creating…", canSubmit = name.removeSuffix("-").isNotEmpty()) { submit() }
    }
}

@Composable private fun InvitePeopleDialog(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, initialChannelId: String?, close: () -> Unit) {
    LaunchedEffect(detail.space.id) { viewModel.loadSpaceInvitations(); viewModel.refreshPeople() }
    CaperDialog("Invite people to ${detail.space.name}", close, wide = true, busy = state.busy) {
        state.error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        DefaultInviteLinkSection(state, detail, viewModel, initialChannelId)
        HorizontalDivider(color = Border)
        InviteManager(detail.members, state.pendingSpaceInvitations, state.busy, viewModel::addSpaceMember,
            {}, viewModel::cancelSpaceInvitation, channel = false,
            suggestions = spaceInviteCandidates(state.people, detail.members, state.pendingSpaceInvitations), inviteOnly = true)
    }
}

@Composable private fun ManageSpaceDialog(state: AppUiState, detail: SpaceDetail, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(detail.space.id) { mutableStateOf(detail.space.name) }
    var nameError by remember(detail.space.id) { mutableStateOf<String?>(null) }
    var confirmingDelete by remember { mutableStateOf(false) }
    var removing by remember(detail.space.id) { mutableStateOf<Member?>(null) }
    // Invite suggestions come from `/api/people`; the previous list stays while it refreshes.
    LaunchedEffect(detail.space.id) { viewModel.loadSpaceInvitations(); viewModel.refreshPeople() }
    CaperDialog("Manage space", close, wide = true, description = "Only the owner can change this space and its membership.") {
        state.error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        SpaceIconSection(state, detail, viewModel)
        val canRename = !state.busy && name.isNotBlank() && name.trim() != detail.space.name
        // Web validates on submit and says why (spaces/client.ts spaceNameError); Done saves too.
        fun rename() { spaceNameError(name)?.let { nameError = it } ?: viewModel.renameSpace(name) }
        OutlinedTextField(name, { name = it.codePointTake(80); nameError = null; if (state.error != null) viewModel.clearError() }, label = { Text("Space name") }, placeholder = { Text("Gaming nights") },
            modifier = Modifier.fillMaxWidth(), singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { if (canRename) rename() }))
        nameError?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        Button(::rename, enabled = canRename, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text("Save name") }
        HorizontalDivider(color = Border)
        InviteLinkSettingsDisclosure(state, detail, viewModel)
        HorizontalDivider(color = Border)
        InviteManager(detail.members, state.pendingSpaceInvitations, state.busy, viewModel::addSpaceMember,
            { removing = it }, viewModel::cancelSpaceInvitation, channel = false,
            suggestions = spaceInviteCandidates(state.people, detail.members, state.pendingSpaceInvitations))
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
            OutlinedButton({ accepting = false; decline() }, enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralOutlinedButtonColors()) {
                PendingLabel("Decline", "Declining…", busy && accepting == false)
            }
            Spacer(Modifier.width(8.dp))
            Button({ accepting = true; accept() }, enabled = !busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) {
                PendingLabel("Accept", "Accepting…", busy && accepting == true)
            }
        }
    }
}

@Composable internal fun InviteManager(
    members: List<Member>, pending: List<Member>, busy: Boolean, add: (String, () -> Unit, (String) -> Unit) -> Unit,
    remove: (Member) -> Unit, cancel: (Member) -> Unit, channel: Boolean, suggestions: List<MentionCandidate>,
    inviteOnly: Boolean = false,
) {
    Text("Invite", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    Text("Pending invitations", fontSize = 13.sp, fontWeight = FontWeight.Bold)
    if (pending.isEmpty()) Text("No pending invitations.", color = TextMuted, fontSize = 11.sp)
    pending.forEach { member ->
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(member.displayName, 30.dp, avatarId = member.avatarId, avatarAsset = member.avatarAsset, avatarUrl = member.avatarUrl); Spacer(Modifier.width(9.dp))
            Column(Modifier.weight(1f)) {
                Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("@${member.username}", color = TextMuted, fontSize = 10.sp)
            }
            TextButton({ cancel(member) }, enabled = !busy, shape = MaterialTheme.shapes.small) { Text("Cancel", color = ErrorText) }
        }
    }
    MemberManager(members, pending, busy, add, remove, channel, suggestions, heading = "Members", action = "Invite", inviteOnly = inviteOnly)
}

@Composable private fun ManageChannelDialog(state: AppUiState, channel: Channel, viewModel: CaperViewModel, close: () -> Unit) {
    var name by remember(channel.id) { mutableStateOf(channel.name) }; var private by remember(channel.id) { mutableStateOf(channel.private) }
    var confirmingDelete by remember { mutableStateOf(false) }
    var removing by remember(channel.id) { mutableStateOf<Member?>(null) }
    LaunchedEffect(channel.id, channel.private) { viewModel.loadChannelGrants(channel) }
    val dirty = name.removeSuffix("-") != channel.name || private != channel.private
    // Keep the sticky save bar's footprint stable while hiding clean controls.
    CaperDialog("Channel settings", close, wide = true, footer = {
        val hiddenSemantics = if (dirty) Modifier else Modifier.clearAndSetSemantics { }
        Row(Modifier.fillMaxWidth().then(hiddenSemantics).graphicsLayer { alpha = if (dirty) 1f else 0f }.background(Blackout).padding(horizontal = 22.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("You have unsaved changes.", Modifier.weight(1f), fontSize = 12.sp)
            OutlinedButton({ name = channel.name; private = channel.private }, enabled = dirty && !state.busy, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border), colors = neutralOutlinedButtonColors()) { Text("Reset") }
            Spacer(Modifier.width(8.dp))
            Button({ viewModel.updateChannel(channel, name.removeSuffix("-"), private) }, enabled = dirty && !state.busy && !channelInvalid(name), shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { PendingLabel("Save changes", "Saving…", state.busy) }
        }
    }) {
        state.error?.let { Text(it, color = ErrorText, fontSize = 12.sp) }
        OutlinedTextField(name, { name = normalizeChannel(it); if (state.error != null) viewModel.clearError() }, label = { Text("Channel name") }, placeholder = { Text("project-updates") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        PrivacyToggle(private, state.selectedSpace?.space?.name ?: "this space", stableSwitch = true) { private = it }
        if (channel.private) {
            HorizontalDivider(color = Border)
            val (grants, invited) = state.channelGrantsFor(channel.id)
            InviteManager(grants, invited, state.busy, { username, done, failed -> viewModel.addChannelGrant(channel, username, done, failed) },
                { removing = it }, { viewModel.cancelChannelInvitation(channel, it) }, channel = true,
                suggestions = channelGrantCandidates(state.selectedSpace?.members.orEmpty(), state.account?.id ?: state.chatAuthorId, grants, invited))
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

/**
 * Web's MemberManager: exact-username invite (with [suggestions] for the username), kept until it is sent,
 * then "@username · Owner" rows.
 */
@Composable private fun MemberManager(
    members: List<Member>, invitations: List<Member>, busy: Boolean, add: (String, () -> Unit, (String) -> Unit) -> Unit, remove: (Member) -> Unit,
    channel: Boolean, suggestions: List<MentionCandidate>, heading: String = "Members", action: String = "Add",
    inviteOnly: Boolean = false,
) {
    var username by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var success by remember { mutableStateOf<String?>(null) }
    if (!inviteOnly) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(heading, fontSize = 13.sp, fontWeight = FontWeight.Bold)
        Surface(color = SurfaceRaised, shape = CircleShape) { Text(members.size.toString(), Modifier.padding(horizontal = 6.dp, vertical = 2.dp), color = TextMuted, fontSize = 10.sp) }
    }
    UsernameSuggestions(username, { username = it; error = null; success = null }, suggestions) { field, imeAction ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            val submitMember = {
                success = null
                error = memberInviteError(username, members, invitations, channel)
                if (error == null) add(username, { username = ""; success = "Invitation sent. They must accept before joining." }, { error = memberAddError(it) })
            }
            OutlinedTextField(username, { username = normalizeUsername(it); error = null; success = null }, label = { Text("Exact username") }, modifier = Modifier.weight(1f).then(field), singleLine = true,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { imeAction { if (!busy) submitMember() } }))
            Spacer(Modifier.width(8.dp))
            Button(submitMember, enabled = !busy, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { Text(action) }
        }
    }
    success?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = TextMuted, fontSize = 12.sp) }
    error?.let { Text(it, Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = ErrorText, fontSize = 12.sp) }
    if (!inviteOnly) members.forEach { member ->
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(member.displayName, 30.dp, avatarId = member.avatarId, avatarAsset = member.avatarAsset, avatarUrl = member.avatarUrl); Spacer(Modifier.width(9.dp))
            Column(Modifier.weight(1f)) {
                Text(member.displayName, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("@${member.username}${if (member.owner) " · Owner" else ""}", color = TextMuted, fontSize = 10.sp)
            }
            if (!member.owner) TextButton({ remove(member) }, enabled = !busy, shape = MaterialTheme.shapes.small) { Text("Remove", color = ErrorText) }
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

@Composable private fun PrivacyToggle(value: Boolean, spaceName: String, stableSwitch: Boolean = false, changed: (Boolean) -> Unit) = Row(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small).clickable { CaperEffects.toggle(!value); changed(!value) }, verticalAlignment = if (stableSwitch) Alignment.Top else Alignment.CenterVertically) {
    Icon(painterResource(R.drawable.lucide_lock_keyhole), null, Modifier.size(17.dp), tint = TextMuted); Spacer(Modifier.width(8.dp)); Column(Modifier.weight(1f)) { Text("Private channel", fontWeight = FontWeight.Bold, fontSize = 13.sp); Text(if (value) "Only you and the people you add can view or join." else "Anyone in $spaceName can view or join this channel.", color = TextMuted, fontSize = 11.sp) }; Switch(value, { CaperEffects.toggle(it); changed(it) }, colors = caperSwitchColors())
}

@Composable internal fun ConfirmDialog(
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
        TextButton(close, Modifier.focusRequester(cancelFocus), enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
        Button({ confirmed = true; confirm() }, enabled = !busy, shape = MaterialTheme.shapes.small, colors = ButtonDefaults.buttonColors(containerColor = Danger)) {
            PendingLabel(action, pendingLabel, busy)
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
    /** Drawn above the title, like What's new's Caper face. */
    heroIcon: Int? = null,
    /** Right under the title, e.g. What's new's linked version. */
    subtitle: (@Composable () -> Unit)? = null,
    /** A request is in flight: X, back and outside taps wait, so it can't finish behind a closed dialog. */
    busy: Boolean = false,
    content: @Composable ColumnScope.() -> Unit,
) {
    Dialog({ if (!busy) close() }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        BoxWithConstraints(Modifier.swipeCancelsPresses().padding(16.dp).widthIn(max = if (wide) 600.dp else 460.dp).fillMaxWidth(), contentAlignment = Alignment.Center) {
            // Short dialogs fit their content; long ones scroll below a fixed heading (web).
            Surface(Modifier.fillMaxWidth().heightIn(max = minOf(maxHeight, 760.dp)), color = Surface, shape = MaterialTheme.shapes.small, border = BorderStroke(1.dp, Border)) {
                Column {
                    Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 10.dp, top = 10.dp, bottom = if (description != null || subtitle != null) 16.dp else 10.dp), verticalAlignment = Alignment.Top) {
                        Column(Modifier.weight(1f)) {
                            // Centered on the close button's row; the title follows below.
                            heroIcon?.let { Image(painterResource(it), contentDescription = null, modifier = Modifier.padding(top = 6.dp, bottom = 10.dp).size(36.dp)) }
                            // As tall as the close button, so the X stays level with the title.
                            Row(Modifier.heightIn(min = if (heroIcon != null) 0.dp else 48.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                titleIcon?.let { Image(painterResource(it), contentDescription = null, modifier = Modifier.size(32.dp)) }
                                Text(title, Modifier.semantics { heading() }, fontSize = 19.sp, fontWeight = FontWeight.Bold)
                            }
                            description?.let { Text(it, color = TextMuted, fontSize = 12.sp, lineHeight = 17.sp) }
                            subtitle?.let { Box(Modifier.padding(top = 2.dp)) { it() } }
                        }
                        Spacer(Modifier.width(8.dp))
                        IconButton(close, Modifier.size(48.dp), enabled = !busy) { Icon(painterResource(R.drawable.lucide_x), "Close", tint = if (busy) TextMuted.copy(alpha = 0.4f) else TextMuted) }
                    }
                    HorizontalDivider(color = Border)
                    Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(22.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                        content()
                    }
                    if (footer != null) { HorizontalDivider(color = Border); footer() }
                }
            }
        }
    }
}

/**
 * A button's [label], or [pendingLabel] while its request runs, sized for the wider of the two so
 * the button never resizes (and its neighbours never shift) as the label changes (web).
 */
@Composable internal fun PendingLabel(label: String, pendingLabel: String, pending: Boolean) = Box(contentAlignment = Alignment.Center) {
    Text(if (pending) label else pendingLabel, Modifier.alpha(0f).clearAndSetSemantics {})
    Text(if (pending) pendingLabel else label)
}

/** [pendingLabel] says what the request is doing; [canSubmit] is false while the required field is empty. */
@Composable private fun DialogActions(
    close: () -> Unit, label: String, busy: Boolean, pendingLabel: String = "Saving…", canSubmit: Boolean = true, action: () -> Unit,
) = Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
    TextButton(close, enabled = !busy, shape = MaterialTheme.shapes.small, colors = neutralTextButtonColors()) { Text("Cancel") }; Spacer(Modifier.width(8.dp))
    Button(action, enabled = !busy && canSubmit, shape = MaterialTheme.shapes.small, colors = primaryButtonColors()) { PendingLabel(label, pendingLabel, busy) }
}

/** Web's spaces/client.ts validation copy. */
internal fun spaceNameError(name: String): String? {
    val trimmed = name.trim()
    return when {
        trimmed.isEmpty() -> "Enter a space name."
        trimmed.codePointCount(0, trimmed.length) > 80 -> "Space names can be at most 80 characters."
        trimmed.any { it.isISOControl() } -> "Space names cannot contain control characters."
        hasHiddenNameCharacters(trimmed) -> "Space names cannot contain invisible or text-direction characters."
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

/** Web's MemberManager checks before inviting [username] to a space or, with [channel], a private channel. */
internal fun memberInviteError(username: String, members: List<Member>, invitations: List<Member>, channel: Boolean): String? = when {
    !usernameValid(username) -> "Use 3–32 lowercase letters, numbers, or underscores."
    members.any { it.username == username } -> if (channel) "This person already has access to this channel." else "This person is already in the space."
    invitations.any { it.username == username } -> "This person already has a pending invitation."
    else -> null
}

/** Web's copy for the server's member errors. */
internal fun memberAddError(message: String): String = when (message) {
    "user not found" -> "User not found. Check the username and try again."
    "user already in space" -> "This person is already in the space."
    "user already in channel" -> "This person already has access to this channel."
    "user already invited" -> "This person already has a pending invitation."
    "invalid username" -> "Use 3–32 lowercase letters, numbers, or underscores."
    else -> message
}
/** Web's displayNameError: [saved], the account's current name, stays savable unchanged even if it predates the invisible-character rule. */
internal fun displayNameError(displayName: String, saved: String?): String? =
    displayName.trim().takeIf { it != saved?.trim() && hasHiddenNameCharacters(it) }
        ?.let { "Display names cannot contain invisible or text-direction characters." }
internal fun profileValid(username: String, displayName: String, saved: String? = null) =
    usernameValid(username) && displayName.isNotBlank() &&
        displayName.codePointCount(0, displayName.length) <= 64 && displayName.none { it.isISOControl() } &&
        displayNameError(displayName, saved) == null
private fun normalizeChannel(value: String) = value.lowercase().replace(Regex("\\s+"), "-").filter { it in 'a'..'z' || it == '-' }.replace(Regex("-+"), "-").removePrefix("-").take(80)
private fun channelInvalid(value: String) = !Regex("^[a-z]+(?:-[a-z]+)*$").matches(value.removeSuffix("-"))
private fun String.codePointTake(max: Int): String = if (codePointCount(0, length) <= max) this else substring(0, offsetByCodePoints(0, max))
private fun localDate(value: String, zoneId: ZoneId): LocalDate? = runCatching { Instant.parse(value).atZone(zoneId).toLocalDate() }.getOrNull()
internal fun sameLocalDay(first: String, second: String, zoneId: ZoneId = ZoneId.systemDefault()): Boolean =
    localDate(first, zoneId)?.let { it == localDate(second, zoneId) } == true
internal fun fullDateLabel(value: String, zoneId: ZoneId = ZoneId.systemDefault(), locale: Locale = Locale.getDefault()): String = runCatching {
    DateTimeFormatter.ofLocalizedDate(FormatStyle.FULL).withLocale(locale).format(Instant.parse(value).atZone(zoneId))
}.getOrDefault("")
// Message times follow the system's 12/24-hour setting (`DateFormat.is24HourFormat`); AM/PM uses the current locale.
private val clock12: DateTimeFormatter = DateTimeFormatter.ofPattern("h:mm a")
private val clock24: DateTimeFormatter = DateTimeFormatter.ofPattern("HH:mm")
private val clock12Short: DateTimeFormatter = DateTimeFormatter.ofPattern("h:mm")
private fun timeLabel(value: String, hour24: Boolean): String = runCatching {
    (if (hour24) clock24 else clock12).withLocale(Locale.getDefault()).format(Instant.parse(value).atZone(ZoneId.systemDefault()))
}.getOrDefault("")
/** A grouped row's hover time, without AM/PM ("8:28"; "20:28" in 24-hour time). */
private fun shortTimeLabel(value: String, hour24: Boolean): String = runCatching {
    (if (hour24) clock24 else clock12Short).format(Instant.parse(value).atZone(ZoneId.systemDefault()))
}.getOrDefault("")

@Composable
private fun DirectUnreadBadge(count: Long?) {
    if (count != null && count > 0) {
        Text(if (count > 99) "99+" else count.toString(),
            Modifier.background(Terracotta, CircleShape).padding(horizontal = 6.dp, vertical = 2.dp)
                .clearAndSetSemantics { contentDescription = "$count unread messages" },
            color = Text, fontSize = 10.sp, fontWeight = FontWeight.Bold, maxLines = 1)
    } else {
        Box(Modifier.size(8.dp).background(TerracottaBright, CircleShape).semantics { contentDescription = "Unread" })
    }
}
